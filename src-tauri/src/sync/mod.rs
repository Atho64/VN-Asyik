//! Folder-based synchronisation.
//!
//! # Why a folder rather than a server
//!
//! The library is personal data. Pushing it through someone else's service means
//! trusting that service with it, and running one means asking the user to host
//! something. A folder the user already syncs (Syncthing, Nextcloud, a network
//! share, a USB stick) needs neither, works offline, and keeps the peer-to-peer
//! story honest: no account, no telemetry, no hidden requests.
//!
//! # The protocol
//!
//! Each device writes one file, `vn-asyik-<device_id>.json`, containing its whole
//! library. Sync reads every peer file in the folder, merges each into local state
//! with [`vn_core::sync`], then rewrites its own file with the merged result.
//!
//! Whole-library payloads are deliberate. A delta protocol would be smaller, but
//! it needs a change journal that survives crashes and handles compaction; with a
//! personal library of a few thousand rows the JSON is small, and correctness is
//! worth more than bytes here. A merge is also idempotent, so a repeated or
//! interrupted sync converges rather than corrupting.
//!
//! # Ordering
//!
//! Merging is commutative: every peer's rows are merged in, and the local file is
//! rewritten from the result. Two devices that sync in either order end with the
//! same rows, which is what makes an eventually-consistent folder safe. Sessions
//! are append-only and merge by identity, never overwritten.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use vn_core::clock::Timestamp;
use vn_core::ids::DeviceId;
use vn_core::migration::SCHEMA_VERSION;
use vn_core::model::{Chapter, Ending, Game, PlaySession, Route};
use vn_core::sync::{self, Conflict, MetaFields, Reason, Side};

use crate::db::{DbError, queries};

/// Prefix and suffix identifying a payload file in a shared folder.
const FILE_PREFIX: &str = "vn-asyik-";
const FILE_SUFFIX: &str = ".json";

/// The on-disk payload one device writes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncPayload {
    /// Schema this payload was written with. A newer one is refused, not guessed.
    pub schema_version: u32,
    /// Which device wrote the file.
    pub device_id: String,
    /// When it was written, for the UI to show freshness.
    pub written_at_ms: i64,
    /// Incremented by the writer each time it rewrites the file.
    pub generation: u64,
    /// Live and deleted games. Tombstones travel so deletes propagate.
    pub games: Vec<Game>,
    /// Live and deleted routes.
    ///
    /// Defaulted so a version-20 file written before this field existed still
    /// loads: the incoming library simply has no routes to contribute, and the
    /// local ones survive untouched.
    #[serde(default)]
    pub routes: Vec<Route>,
    /// Live and deleted chapters.
    pub chapters: Vec<Chapter>,
    /// Sessions are append-only and are never tombstoned.
    pub sessions: Vec<PlaySession>,
    /// Live and deleted endings.
    pub endings: Vec<Ending>,
}

/// What one sync run did.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncOutcome {
    /// Peers read from the folder.
    pub peers: usize,
    /// Rows written locally because a peer's copy won.
    pub pulled: usize,
    /// Rows this device's file now carries that a peer did not have.
    pub pushed: usize,
    /// Conflicts resolved, in the order they were applied.
    pub conflicts: Vec<AppliedConflict>,
    /// Peer files that could not be read, with the reason.
    pub skipped: Vec<SkippedPeer>,
}

/// A conflict that was resolved and logged.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppliedConflict {
    pub table: String,
    pub record_id: String,
    pub kept: String,
    pub reason: String,
    pub discarded_updated_at_ms: i64,
}

/// A peer file that was ignored.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkippedPeer {
    pub file: String,
    pub reason: String,
}

/// Errors a sync can fail with.
#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    /// The folder does not exist or is not a directory.
    #[error("sync folder is not usable: {0}")]
    BadFolder(String),
    /// A payload was written by a newer version of the app.
    #[error("payload schema {found} is newer than this app supports ({max})")]
    PayloadTooNew {
        /// Schema found in the file.
        found: u32,
        /// Highest schema this build understands.
        max: u32,
    },
    /// Filesystem trouble.
    #[error("sync i/o failed: {0}")]
    Io(#[from] std::io::Error),
    /// Database trouble.
    #[error(transparent)]
    Db(#[from] DbError),
    /// Serialisation trouble.
    #[error("could not encode payload: {0}")]
    Encode(#[from] serde_json::Error),
    /// No device id has been created yet.
    #[error("device id is not initialised")]
    NoDeviceId,
}

/// The file name for a device's payload.
#[must_use]
pub fn payload_file_name(device: &str) -> String {
    format!("{FILE_PREFIX}{device}{FILE_SUFFIX}")
}

/// Whether a path is a payload file.
#[must_use]
fn is_payload(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            (name.starts_with(FILE_PREFIX) || name.starts_with("vntracker-"))
                && name.ends_with(FILE_SUFFIX)
        })
}

/// Read every local row, including tombstones.
///
/// The UI queries hide deleted rows, which is right for the UI and wrong for
/// sync: a delete has to travel, or the row comes back on the next pull.
pub async fn collect_local(pool: &SqlitePool, device: &DeviceId) -> Result<SyncPayload, SyncError> {
    let games = queries::list_games_including_deleted(pool).await?;
    let mut routes = Vec::new();
    let mut chapters = Vec::new();
    let mut sessions = Vec::new();
    let mut endings = Vec::new();
    for game in &games {
        routes.extend(queries::list_routes_including_deleted(pool, game.meta.id).await?);
        chapters.extend(queries::list_chapters_including_deleted(pool, game.meta.id).await?);
        sessions.extend(queries::list_sessions_including_deleted(pool, game.meta.id).await?);
        endings.extend(queries::list_endings_including_deleted(pool, game.meta.id).await?);
    }

    Ok(SyncPayload {
        schema_version: SCHEMA_VERSION,
        device_id: device.as_str().to_owned(),
        written_at_ms: Timestamp::now().map_or(0, Timestamp::as_millis),
        generation: 0,
        games,
        routes,
        chapters,
        sessions,
        endings,
    })
}

/// Merge one explicitly selected transfer file with the same rules as folder sync.
pub async fn merge_transfer(
    pool: &SqlitePool,
    device: &DeviceId,
    payload: &SyncPayload,
) -> Result<SyncOutcome, SyncError> {
    if payload.schema_version > SCHEMA_VERSION {
        return Err(SyncError::PayloadTooNew {
            found: payload.schema_version,
            max: SCHEMA_VERSION,
        });
    }
    if payload.device_id == device.as_str() {
        return Ok(SyncOutcome::default());
    }
    merge_library_snapshot(pool, payload).await
}

/// Merge a user-selected library snapshot, including a backup from this device.
/// Legacy transfer commands still skip their own file through `merge_transfer`.
pub async fn merge_library_snapshot(
    pool: &SqlitePool,
    payload: &SyncPayload,
) -> Result<SyncOutcome, SyncError> {
    if payload.schema_version > SCHEMA_VERSION {
        return Err(SyncError::PayloadTooNew {
            found: payload.schema_version,
            max: SCHEMA_VERSION,
        });
    }
    let mut outcome = SyncOutcome {
        peers: 1,
        ..SyncOutcome::default()
    };
    apply_payload(pool, payload, &mut outcome).await?;
    Ok(outcome)
}

/// Merge every peer payload in `folder` into the local database.
///
/// Returns what happened, and rewrites this device's own payload with the merged
/// result so peers pick the change up next time they sync.
///
/// # Errors
/// [`SyncError::BadFolder`] when the folder is unusable, and any I/O or database
/// error. An unreadable *peer* file is not an error: it is reported in
/// [`SyncOutcome::skipped`] and the run continues, because one corrupt file should
/// not block a full library of good ones.
pub async fn sync_folder(
    pool: &SqlitePool,
    device: &DeviceId,
    folder: &Path,
) -> Result<SyncOutcome, SyncError> {
    if !folder.is_dir() {
        return Err(SyncError::BadFolder(folder.display().to_string()));
    }

    let mut outcome = SyncOutcome::default();
    let own_file = payload_file_name(device.as_str());

    let mut entries: Vec<PathBuf> = std::fs::read_dir(folder)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && is_payload(path))
        .collect();
    // Deterministic order so a run is reproducible, and so a log reads the same
    // each time.
    entries.sort();

    for path in entries {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_owned();
        if name == own_file {
            continue;
        }

        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(err) => {
                outcome.skipped.push(SkippedPeer {
                    file: name,
                    reason: err.to_string(),
                });
                continue;
            }
        };
        let payload: SyncPayload = match serde_json::from_str(&text) {
            Ok(payload) => payload,
            Err(err) => {
                outcome.skipped.push(SkippedPeer {
                    file: name,
                    reason: err.to_string(),
                });
                continue;
            }
        };
        if payload.schema_version > SCHEMA_VERSION {
            outcome.skipped.push(SkippedPeer {
                file: name,
                reason: format!(
                    "written by a newer version (schema {}, this build reads {SCHEMA_VERSION})",
                    payload.schema_version
                ),
            });
            continue;
        }
        // A file claiming to be ours is stale or copied; skipping avoids merging
        // our own rows back onto ourselves.
        if payload.device_id == device.as_str() {
            continue;
        }

        outcome.peers += 1;
        apply_payload(pool, &payload, &mut outcome).await?;
    }

    // Always publish our own file, even when there was no peer to merge with.
    // Writing only after a successful merge left the first device on a fresh
    // folder with nothing to publish, so the second device had nothing to read and
    // the two never converged.
    {
        let local = collect_local(pool, device).await?;
        let generation = read_generation(folder, &own_file).map_or(1, |value| value + 1);
        let payload = SyncPayload {
            generation,
            ..local
        };
        let text = serde_json::to_string_pretty(&payload)?;
        write_atomically(&folder.join(&own_file), &text)?;
    }

    outcome.pushed = local_row_count(pool).await?;
    Ok(outcome)
}

/// Read the generation counter from our existing file, if any.
fn read_generation(folder: &Path, own_file: &str) -> Option<u64> {
    let text = std::fs::read_to_string(folder.join(own_file)).ok()?;
    let payload: SyncPayload = serde_json::from_str(&text).ok()?;
    Some(payload.generation)
}

/// Write via a temporary file and a rename.
///
/// A peer may read the folder at any moment. Writing in place would let it see a
/// half-written payload, so the rename publishes the new content atomically.
fn write_atomically(path: &Path, text: &str) -> std::io::Result<()> {
    let temp = path.with_extension("json.tmp");
    std::fs::write(&temp, text)?;
    std::fs::rename(&temp, path)
}

async fn local_row_count(pool: &SqlitePool) -> Result<usize, DbError> {
    let games = queries::list_games_including_deleted(pool).await?.len();
    Ok(games)
}

/// Merge one peer's payload into the database.
async fn apply_payload(
    pool: &SqlitePool,
    payload: &SyncPayload,
    outcome: &mut SyncOutcome,
) -> Result<(), SyncError> {
    if payload.sessions.iter().any(|session| session.afk_ms < 0) {
        return Err(DbError::Corrupt("session idle time cannot be negative".to_owned()).into());
    }

    // Reads and writes use the same connection so a failed peer cannot leave
    // earlier overwrites/tombstones behind. Publish counts only after commit.
    let mut tx = pool.begin().await.map_err(DbError::from)?;
    let mut delta = SyncOutcome::default();
    let local_games = queries::list_games_including_deleted(&mut *tx).await?;
    let (games, conflicts) = sync::merge_rows("games", &local_games, &payload.games);
    record(&mut delta, conflicts);
    for game in &games {
        if should_write(&local_games, game) {
            queries::upsert_game(&mut *tx, game).await?;
            delta.pulled += 1;
        }
    }

    // Routes merge first, before the children that point at them. A chapter whose
    // `route_id` names a route this device has not stored yet would be a dangling
    // reference the route view cannot resolve.
    let mut local_routes = Vec::new();
    for game in &games {
        local_routes.extend(queries::list_routes_including_deleted(&mut *tx, game.meta.id).await?);
    }
    let peer_routes: Vec<Route> = payload
        .routes
        .iter()
        .filter(|route| games.iter().any(|game| game.meta.id == route.game_id))
        .cloned()
        .collect();
    let (routes, conflicts) = sync::merge_rows("routes", &local_routes, &peer_routes);
    record(&mut delta, conflicts);
    for route in &routes {
        if should_write(&local_routes, route) {
            queries::upsert_route(&mut *tx, route).await?;
            delta.pulled += 1;
        }
    }

    // Chapters are merged per game so a chapter never lands without its game.
    let mut local_chapters = Vec::new();
    for game in &games {
        local_chapters
            .extend(queries::list_chapters_including_deleted(&mut *tx, game.meta.id).await?);
    }
    let peer_chapters: Vec<Chapter> = payload
        .chapters
        .iter()
        .filter(|chapter| games.iter().any(|game| game.meta.id == chapter.game_id))
        .cloned()
        .collect();
    let (chapters, conflicts) = sync::merge_rows("chapters", &local_chapters, &peer_chapters);
    record(&mut delta, conflicts);
    for chapter in &chapters {
        if should_write(&local_chapters, chapter) {
            queries::upsert_chapter(&mut *tx, chapter).await?;
            delta.pulled += 1;
        }
    }

    let mut local_sessions = Vec::new();
    for game in &games {
        local_sessions
            .extend(queries::list_sessions_including_deleted(&mut *tx, game.meta.id).await?);
    }
    let (sessions, added) = sync::merge_sessions(&local_sessions, &payload.sessions);
    for session in &sessions {
        if added.contains(&session.meta.id.to_string()) {
            queries::insert_session(&mut *tx, session).await?;
            delta.pulled += 1;
        }
    }

    let mut local_endings = Vec::new();
    for game in &games {
        local_endings
            .extend(queries::list_endings_including_deleted(&mut *tx, game.meta.id).await?);
    }
    let (endings, conflicts) = sync::merge_rows("endings", &local_endings, &payload.endings);
    record(&mut delta, conflicts);
    for ending in &endings {
        if should_write(&local_endings, ending) {
            queries::upsert_ending(&mut *tx, ending).await?;
            delta.pulled += 1;
        }
    }

    tx.commit().await.map_err(DbError::from)?;
    outcome.pulled += delta.pulled;
    outcome.conflicts.extend(delta.conflicts);
    Ok(())
}

/// Whether `merged` differs from what is stored, so a write is needed.
fn should_write<T: MetaFields + PartialEq>(local: &[T], merged: &T) -> bool {
    match local.iter().find(|row| row.meta().id == merged.meta().id) {
        None => true,
        Some(existing) => existing != merged,
    }
}

fn record(outcome: &mut SyncOutcome, conflicts: Vec<Conflict>) {
    for conflict in conflicts {
        outcome.conflicts.push(AppliedConflict {
            table: conflict.table.to_owned(),
            record_id: conflict.id,
            kept: match conflict.kept {
                Side::Local => "local".to_owned(),
                Side::Remote => "remote".to_owned(),
            },
            reason: match conflict.reason {
                Reason::OnlyOneSide => "only_one_side".to_owned(),
                Reason::Identical => "identical".to_owned(),
                Reason::Newer => "newer".to_owned(),
                Reason::TieBreak => "tie_break".to_owned(),
                Reason::DeleteWins => "delete_wins".to_owned(),
            },
            discarded_updated_at_ms: conflict.discarded_updated_at_ms,
        });
    }
}

/// Explain why a peer file would be skipped, without touching anything.
///
/// Used by the UI to show what a sync would do before the user runs it.
#[must_use]
pub fn describe_folder(folder: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return vec!["folder cannot be read".to_owned()];
    };
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && is_payload(path))
        .filter_map(|path| path.file_name()?.to_str().map(str::to_owned))
        .collect();
    names.sort();
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("vntracker-sync-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create");
        dir
    }

    #[test]
    fn payload_file_names_are_recognisable_and_unique_per_device() {
        let a = payload_file_name("device-a");
        let b = payload_file_name("device-b");
        assert!(a.starts_with(FILE_PREFIX));
        assert!(a.ends_with(FILE_SUFFIX));
        assert_ne!(a, b);
        assert!(is_payload(Path::new(&a)));
    }

    #[test]
    fn unrelated_files_in_the_shared_folder_are_ignored() {
        // The folder is shared with the user's other files; only ours may be read.
        assert!(!is_payload(Path::new("notes.txt")));
        assert!(!is_payload(Path::new("vntracker-backup.zip")));
        assert!(!is_payload(Path::new(".sync/id")));
    }

    #[test]
    fn writing_is_atomic_so_a_peer_never_reads_a_partial_file() {
        let dir = temp_folder("atomic");
        let target = dir.join(payload_file_name("dev"));
        write_atomically(&target, "{\"a\":1}").expect("write");
        assert_eq!(std::fs::read_to_string(&target).expect("read"), "{\"a\":1}");
        // No temporary file is left behind.
        let leftovers: Vec<String> = std::fs::read_dir(&dir)
            .expect("dir")
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.to_lowercase().ends_with(".tmp"))
            .collect();
        assert_eq!(leftovers.len(), 0, "left {leftovers:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_folder_is_reported_rather_than_treated_as_empty() {
        // An empty folder and a mistyped path look identical to a merge, and
        // treating the second as the first would silently do nothing forever.
        let missing = std::env::temp_dir().join("vntracker-does-not-exist");
        assert!(!missing.is_dir());
        assert_eq!(describe_folder(&missing), vec!["folder cannot be read"]);
    }

    #[test]
    fn describe_lists_only_payload_files_sorted() {
        let dir = temp_folder("describe");
        std::fs::write(dir.join("vntracker-b.json"), "{}").expect("b");
        std::fs::write(dir.join("vntracker-a.json"), "{}").expect("a");
        std::fs::write(dir.join("readme.txt"), "x").expect("txt");
        let names = describe_folder(&dir);
        assert_eq!(names, vec!["vntracker-a.json", "vntracker-b.json"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
