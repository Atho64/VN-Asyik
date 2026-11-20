//! Folder-based peer-to-peer sync (no account, server, or telemetry; works over Syncthing, Nextcloud, shares, or USB).
//! Each device writes `vn-asyik-<device_id>.json` with its whole library, merges peer files via [`vn_core::sync`],
//! and rewrites its own file; merges are commutative and idempotent, and play sessions remain append-only by ID.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use vn_core::achievements::{AchievementUnlock, merge_unlocks};
use vn_core::clock::Timestamp;
use vn_core::goals::{ReadingGoal, validate_reading_goal};
use vn_core::ids::DeviceId;
use vn_core::migration::SCHEMA_VERSION;
use vn_core::model::{Chapter, Ending, Game, PlaySession, Route};
use vn_core::reading::{GlobalNote, ReadingEntry};
use vn_core::sync::{self, Conflict, MetaFields, Reason, Side};

use crate::db::{DbError, queries};

const FILE_PREFIX: &str = "vn-asyik-";
const FILE_SUFFIX: &str = ".json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncPayload {
    /// A newer schema version is refused rather than guessed.
    pub schema_version: u32,
    pub device_id: String,
    pub written_at_ms: i64,
    pub generation: u64,
    /// Includes tombstones so deletes propagate.
    pub games: Vec<Game>,
    /// Defaulted so pre-route v20 payloads load without dropping local routes.
    #[serde(default)]
    pub routes: Vec<Route>,
    pub chapters: Vec<Chapter>,
    /// Append-only; never tombstoned.
    pub sessions: Vec<PlaySession>,
    pub endings: Vec<Ending>,
    /// Absent in pre-journal payloads.
    #[serde(default)]
    pub reading_entries: Vec<ReadingEntry>,
    /// Global notes, including tombstones.
    #[serde(default)]
    pub global_notes: Vec<GlobalNote>,
    /// Custom reading goals, including tombstones.
    #[serde(default)]
    pub reading_goals: Vec<ReadingGoal>,
    /// Achievement unlocks (monotonic merge). Absent in pre-achievement payloads.
    #[serde(default)]
    pub achievement_unlocks: Vec<AchievementUnlock>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncOutcome {
    pub peers: usize,
    pub pulled: usize,
    pub pushed: usize,
    pub conflicts: Vec<AppliedConflict>,
    pub skipped: Vec<SkippedPeer>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppliedConflict {
    pub table: String,
    pub record_id: String,
    pub kept: String,
    pub reason: String,
    pub discarded_updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkippedPeer {
    pub file: String,
    pub reason: String,
}

#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error("sync folder is not usable: {0}")]
    BadFolder(String),
    #[error("payload schema {found} is newer than this app supports ({max})")]
    PayloadTooNew { found: u32, max: u32 },
    #[error("sync i/o failed: {0}")]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Db(#[from] DbError),
    #[error("could not encode payload: {0}")]
    Encode(#[from] serde_json::Error),
    #[error("device id is not initialised")]
    NoDeviceId,
}

#[must_use]
pub fn payload_file_name(device: &str) -> String {
    format!("{FILE_PREFIX}{device}{FILE_SUFFIX}")
}

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
    let mut reading_entries = Vec::new();
    for game in &games {
        routes.extend(queries::list_routes_including_deleted(pool, game.meta.id).await?);
        chapters.extend(queries::list_chapters_including_deleted(pool, game.meta.id).await?);
        sessions.extend(queries::list_sessions_including_deleted(pool, game.meta.id).await?);
        endings.extend(queries::list_endings_including_deleted(pool, game.meta.id).await?);
        reading_entries.extend(crate::db::reading::list_all(pool, game.meta.id).await?);
    }
    let global_notes = crate::db::notes::list_all_including_deleted(pool).await?;
    let reading_goals = crate::db::goals::list_all_including_deleted(pool).await?;
    let achievement_unlocks = crate::db::achievements::list_all_including_deleted(pool).await?;

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
        reading_entries,
        global_notes,
        reading_goals,
        achievement_unlocks,
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

    // Always publish our own file, even with no peers yet, so a second device has something to read.
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

    merge_play_sessions(&mut tx, &games, &payload.sessions, &mut delta).await?;

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

    let mut local_entries = Vec::new();
    for game in &games {
        local_entries.extend(crate::db::reading::list_all(&mut *tx, game.meta.id).await?);
    }
    for entry in &payload.reading_entries {
        entry.validate().map_err(DbError::Corrupt)?;
    }
    let (entries, conflicts) =
        sync::merge_rows("reading_entries", &local_entries, &payload.reading_entries);
    record(&mut delta, conflicts);
    for entry in &entries {
        if should_write(&local_entries, entry) {
            crate::db::reading::upsert(&mut *tx, entry).await?;
            delta.pulled += 1;
        }
    }
    merge_global_notes(&mut tx, &payload.global_notes, &mut delta).await?;
    merge_reading_goals(&mut tx, &payload.reading_goals, &mut delta).await?;
    merge_achievement_unlocks(&mut tx, &payload.achievement_unlocks, &mut delta).await?;

    tx.commit().await.map_err(DbError::from)?;
    outcome.pulled += delta.pulled;
    outcome.conflicts.extend(delta.conflicts);
    Ok(())
}

async fn merge_reading_goals(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    incoming: &[ReadingGoal],
    delta: &mut SyncOutcome,
) -> Result<(), SyncError> {
    let local_goals = crate::db::goals::list_all_including_deleted(&mut **tx).await?;
    for goal in incoming {
        let mut normalized = goal.clone();
        validate_reading_goal(&mut normalized).map_err(DbError::Corrupt)?;
    }
    let (goals, conflicts) = sync::merge_rows("reading_goals", &local_goals, incoming);
    record(delta, conflicts);
    for goal in &goals {
        if should_write(&local_goals, goal) {
            crate::db::goals::upsert(&mut **tx, goal).await?;
            delta.pulled += 1;
        }
    }
    Ok(())
}

async fn merge_global_notes(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    incoming: &[GlobalNote],
    delta: &mut SyncOutcome,
) -> Result<(), SyncError> {
    let local_notes = crate::db::notes::list_all_including_deleted(&mut **tx).await?;
    for note in incoming {
        note.validate().map_err(DbError::Corrupt)?;
    }
    let (notes, conflicts) = sync::merge_rows("global_notes", &local_notes, incoming);
    record(delta, conflicts);
    for note in &notes {
        if should_write(&local_notes, note) {
            crate::db::notes::upsert(&mut **tx, note).await?;
            delta.pulled += 1;
        }
    }
    Ok(())
}

/// Append-only session merge: new UUIDs are inserted, existing ones untouched.
async fn merge_play_sessions(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    games: &[Game],
    incoming: &[PlaySession],
    delta: &mut SyncOutcome,
) -> Result<(), SyncError> {
    let mut local_sessions = Vec::new();
    for game in games {
        local_sessions
            .extend(queries::list_sessions_including_deleted(&mut **tx, game.meta.id).await?);
    }
    let (sessions, added) = sync::merge_sessions(&local_sessions, incoming);
    for session in &sessions {
        if added.contains(&session.meta.id.to_string()) {
            queries::insert_session(&mut **tx, session).await?;
            delta.pulled += 1;
        }
    }
    Ok(())
}

/// Monotonic unlock merge: identity is the slug, not the row UUID, so two
/// devices unlocking the same achievement converge on one row with the
/// earliest unlock date. A tombstone never revokes a live unlock.
async fn merge_achievement_unlocks(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    incoming: &[AchievementUnlock],
    delta: &mut SyncOutcome,
) -> Result<(), SyncError> {
    let local_unlocks = crate::db::achievements::list_all_including_deleted(&mut **tx).await?;
    for unlock in incoming {
        unlock.validate().map_err(DbError::Corrupt)?;
    }
    let (unlocks, conflicts) = merge_unlocks(&local_unlocks, incoming);
    record(delta, conflicts);
    for unlock in &unlocks {
        // Compare by slug: the stored UUID may differ from the merged winner's.
        let changed = match local_unlocks
            .iter()
            .find(|row| row.achievement_id == unlock.achievement_id)
        {
            None => true,
            Some(existing) => existing != unlock,
        };
        if changed {
            crate::db::achievements::upsert(&mut **tx, unlock).await?;
            delta.pulled += 1;
        }
    }
    Ok(())
}

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
                Reason::UnlockWins => "unlock_wins".to_owned(),
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
