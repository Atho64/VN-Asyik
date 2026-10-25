//! Executable-to-game mapping, scan folders, and tracker persistence.
//!
//! The mapping is intentional rather than heuristic: the user says "this exe is
//! this game". Auto-suggestion only proposes candidates; it never decides. That
//! keeps attribution honest when several games share an engine executable name,
//! which is common for Japanese visual novels.

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use vn_core::clock::Timestamp;
use vn_core::ids::{DeviceId, EntityId};
use vn_core::tracker::Segment;

use crate::db::DbError;

/// An executable mapped to a game.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameExecutable {
    /// Row id.
    pub id: String,
    /// Owning game.
    pub game_id: String,
    /// Lowercased file name.
    pub exe_name: String,
    /// Full path when known.
    pub exe_path: Option<String>,
    /// Whether tracking is enabled for this executable.
    pub enabled: bool,
    /// What runs the game: a [`vn_core::emulator::RunnerKind`] string. `native`
    /// for every pre-emulator mapping.
    pub runner_kind: String,
    /// What the host is told to open (ROM path, title id, game folder).
    pub launch_target: Option<String>,
    /// Caption text that recognises this game inside a shared host.
    pub title_match: Option<String>,
    /// Host-specific options as JSON.
    pub extra_args: Option<String>,
}

impl GameExecutable {
    /// The package this mapping can open on Android. Desktop mappings remain
    /// usable on their own device after a cross-platform library restore.
    #[must_use]
    pub fn android_package(&self) -> Option<&str> {
        let runner = vn_core::emulator::RunnerKind::parse(&self.runner_kind)?;
        let package = self.exe_path.as_deref()?;
        (runner.platforms().android && vn_core::emulator::is_android_package(package))
            .then_some(package)
    }

    /// The caption text that identifies this game inside its host, if any.
    #[must_use]
    pub fn needle(&self) -> Option<String> {
        vn_core::emulator::title_needle(self.title_match.as_deref(), self.launch_target.as_deref())
    }
}

/// Database row for an executable.
#[derive(Debug, sqlx::FromRow)]
struct ExecutableRow {
    id: String,
    game_id: String,
    exe_name: String,
    exe_path: Option<String>,
    enabled: i64,
    runner_kind: String,
    launch_target: Option<String>,
    title_match: Option<String>,
    extra_args: Option<String>,
}

impl From<ExecutableRow> for GameExecutable {
    fn from(row: ExecutableRow) -> Self {
        Self {
            id: row.id,
            game_id: row.game_id,
            exe_name: row.exe_name,
            exe_path: row.exe_path,
            enabled: row.enabled != 0,
            runner_kind: row.runner_kind,
            launch_target: row.launch_target,
            title_match: row.title_match,
            extra_args: row.extra_args,
        }
    }
}

/// An emulator or runner mapping, as the user describes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunnerTarget {
    /// Which runner. Must not be `native`: those go through [`add_executable`].
    pub runner_kind: String,
    /// The emulator's executable on Windows.
    pub exe_path: Option<String>,
    /// The Android package name, for Android runners.
    pub package: Option<String>,
    /// What the host opens. Required: it is what tells the games apart.
    pub launch_target: Option<String>,
    /// Caption text to recognise the game while its siblings share the host.
    pub title_match: Option<String>,
    /// Host-specific options as JSON.
    pub extra_args: Option<String>,
}

/// A candidate found on disk, before the user decides.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutableCandidate {
    /// Full path.
    pub path: String,
    /// File name.
    pub exe_name: String,
    /// Size in bytes, a weak hint that this is a game and not a config tool.
    pub size_bytes: u64,
    /// Game it is already mapped to, when it is.
    pub mapped_game_id: Option<String>,
}

/// Add or replace an executable mapping.
pub async fn add_executable(
    pool: &SqlitePool,
    device: &DeviceId,
    now: Timestamp,
    game_id: EntityId,
    exe_path: &str,
) -> Result<GameExecutable, DbError> {
    let exe_name = crate::platform::probe::exe_name_of(exe_path);
    if exe_name.is_empty() {
        return Err(DbError::Corrupt("executable name is empty".to_owned()));
    }
    let normalized = crate::platform::probe::normalize_path(exe_path);
    // One mapping per path: re-adding the same exe to the same game is a no-op,
    // and adding it to a different game moves it, which is what a user means.
    //
    // The comparison normalises separators in SQL, not in Rust. Comparing the
    // stored `F:\Games\game.exe` against a pre-normalised `f:/games/game.exe`
    // never matched, so every add inserted a new row: this produced duplicate
    // mappings, including the same executable pointing at two different games.
    let existing: Option<(String, String)> = sqlx::query_as(
        r"SELECT id, game_id FROM game_executables
          WHERE replace(replace(lower(exe_path), '\', '/'), '//', '/') = ?
            AND deleted = 0 LIMIT 1",
    )
    .bind(&normalized)
    .fetch_optional(pool)
    .await?;

    let id = existing
        .as_ref()
        .map_or_else(|| EntityId::new().to_string(), |(id, _)| id.clone());

    sqlx::query(
        "INSERT INTO game_executables (id, game_id, exe_name, exe_path, added_at, enabled, updated_at, device_id, deleted, runner_kind) \
         VALUES (?, ?, ?, ?, ?, 1, ?, ?, 0, 'native') \
         ON CONFLICT(id) DO UPDATE SET game_id = excluded.game_id, exe_name = excluded.exe_name, \
            exe_path = excluded.exe_path, enabled = 1, updated_at = excluded.updated_at, \
            device_id = excluded.device_id, deleted = 0, runner_kind = 'native'",
    )
    .bind(&id)
    .bind(game_id.to_string())
    .bind(&exe_name)
    .bind(exe_path)
    .bind(now.as_millis())
    .bind(now.as_millis())
    .bind(device.as_str())
    .execute(pool)
    .await?;

    Ok(GameExecutable {
        id,
        game_id: game_id.to_string(),
        exe_name,
        exe_path: Some(exe_path.to_owned()),
        enabled: true,
        runner_kind: "native".to_owned(),
        launch_target: None,
        title_match: None,
        extra_args: None,
    })
}

fn validate_runner_package(
    target: &RunnerTarget,
    runner: vn_core::emulator::RunnerKind,
) -> Result<(), DbError> {
    if let Some(package) = target
        .package
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        if !runner.platforms().android || !vn_core::emulator::is_android_package(package.trim()) {
            return Err(DbError::Corrupt(
                "invalid Android runner package".to_owned(),
            ));
        }
        if target
            .exe_path
            .as_deref()
            .is_some_and(|path| !path.trim().is_empty())
        {
            return Err(DbError::Corrupt(
                "choose a desktop executable or an Android package".to_owned(),
            ));
        }
    }
    Ok(())
}

/// Add or update an emulator or runner target.
///
/// Unlike native executables, shared hosts (`Vita3K`, RPCS3, PCSX2, Eden, Citron,
/// PPSSPP, Winlator, Kirikiroid2, etc.) can be mapped to many games at the same
/// time. The identity is therefore `(game_id, runner_kind, launch_target)`: adding
/// a game to `Vita3K` never moves or unbinds another game on `Vita3K`.
pub async fn add_runner_target(
    pool: &SqlitePool,
    device: &DeviceId,
    now: Timestamp,
    game_id: EntityId,
    target: &RunnerTarget,
) -> Result<GameExecutable, DbError> {
    let runner = vn_core::emulator::RunnerKind::parse(&target.runner_kind)
        .ok_or_else(|| DbError::Corrupt(format!("unknown runner kind: {}", target.runner_kind)))?;

    validate_runner_package(target, runner)?;

    let (exe_name, exe_path) =
        if let Some(path) = target.exe_path.as_deref().filter(|p| !p.trim().is_empty()) {
            (
                crate::platform::probe::exe_name_of(path),
                Some(path.to_owned()),
            )
        } else if let Some(pkg) = target.package.as_deref().filter(|p| !p.trim().is_empty()) {
            (pkg.trim().to_lowercase(), Some(pkg.trim().to_owned()))
        } else {
            let default_exe = runner.windows_exe_names().first().map(|s| (*s).to_owned());
            let default_pkg = runner.android_packages().first().map(|s| (*s).to_owned());
            if let Some(exe) = default_exe {
                (exe.clone(), Some(exe))
            } else if let Some(pkg) = default_pkg {
                (pkg.clone(), Some(pkg))
            } else {
                return Err(DbError::Corrupt(
                    "missing executable path or package for runner".to_owned(),
                ));
            }
        };

    let launch_target = target
        .launch_target
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let title_match = target
        .title_match
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let extra_args = target
        .extra_args
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let existing: Option<String> = sqlx::query_scalar(
        "SELECT id FROM game_executables \
         WHERE game_id = ? AND runner_kind = ? \
           AND coalesce(lower(launch_target), '') = coalesce(lower(?), '') \
           AND coalesce(exe_path, '') = coalesce(?, '') \
           AND deleted = 0 LIMIT 1",
    )
    .bind(game_id.to_string())
    .bind(runner.as_str())
    .bind(launch_target)
    .bind(&exe_path)
    .fetch_optional(pool)
    .await?;

    let id = existing.unwrap_or_else(|| EntityId::new().to_string());

    sqlx::query(
        "INSERT INTO game_executables (id, game_id, exe_name, exe_path, added_at, enabled, updated_at, device_id, deleted, runner_kind, launch_target, title_match, extra_args) \
         VALUES (?, ?, ?, ?, ?, 1, ?, ?, 0, ?, ?, ?, ?) \
         ON CONFLICT(id) DO UPDATE SET game_id = excluded.game_id, exe_name = excluded.exe_name, \
            exe_path = excluded.exe_path, enabled = 1, updated_at = excluded.updated_at, \
            device_id = excluded.device_id, deleted = 0, runner_kind = excluded.runner_kind, \
            launch_target = excluded.launch_target, title_match = excluded.title_match, \
            extra_args = excluded.extra_args",
    )
    .bind(&id)
    .bind(game_id.to_string())
    .bind(&exe_name)
    .bind(&exe_path)
    .bind(now.as_millis())
    .bind(now.as_millis())
    .bind(device.as_str())
    .bind(runner.as_str())
    .bind(launch_target)
    .bind(title_match)
    .bind(extra_args)
    .execute(pool)
    .await?;

    Ok(GameExecutable {
        id,
        game_id: game_id.to_string(),
        exe_name,
        exe_path,
        enabled: true,
        runner_kind: runner.as_str().to_owned(),
        launch_target: launch_target.map(str::to_owned),
        title_match: title_match.map(str::to_owned),
        extra_args: extra_args.map(str::to_owned),
    })
}

/// Every executable mapping, for the UI and the tracker.
pub async fn list_executables(pool: &SqlitePool) -> Result<Vec<GameExecutable>, DbError> {
    let rows: Vec<ExecutableRow> = sqlx::query_as(
        "SELECT e.id, e.game_id, e.exe_name, e.exe_path, e.enabled, e.runner_kind, e.launch_target, e.title_match, e.extra_args \
         FROM game_executables e JOIN games g ON g.id = e.game_id \
         WHERE e.deleted = 0 AND g.deleted = 0 ORDER BY e.exe_name ASC",
    )
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(GameExecutable::from).collect())
}

/// Executables mapped to one game.
pub async fn executables_for_game(
    pool: &SqlitePool,
    game_id: EntityId,
) -> Result<Vec<GameExecutable>, DbError> {
    Ok(list_executables(pool)
        .await?
        .into_iter()
        .filter(|entry| entry.game_id == game_id.to_string())
        .collect())
}

/// Remove a mapping.
pub async fn remove_executable(
    pool: &SqlitePool,
    id: &str,
    now: Timestamp,
    device: &DeviceId,
) -> Result<(), DbError> {
    sqlx::query(
        "UPDATE game_executables SET deleted = 1, updated_at = ?, device_id = ? WHERE id = ?",
    )
    .bind(now.as_millis())
    .bind(device.as_str())
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Every executable mapping matching a process path or name.
pub async fn executables_for_process(
    pool: &SqlitePool,
    exe_path: Option<&str>,
    exe_name: &str,
) -> Result<Vec<GameExecutable>, DbError> {
    if let Some(path) = exe_path {
        let normalized = crate::platform::probe::normalize_path(path);
        // The join is what stops a deleted game from being tracked: the
        // executable rows survive soft-delete (they are not synced rows), so
        // without it the poll keeps writing sessions for a game the user
        // removed — time no UI aggregate shows.
        let rows: Vec<ExecutableRow> = sqlx::query_as(
            "SELECT ge.id, ge.game_id, ge.exe_name, ge.exe_path, ge.enabled, ge.runner_kind, ge.launch_target, ge.title_match, ge.extra_args \
             FROM game_executables ge \
             JOIN games g ON g.id = ge.game_id AND g.deleted = 0 \
             WHERE (lower(ge.exe_path) = ? OR replace(replace(lower(ge.exe_path), '\\', '/'), '//', '/') = ?) \
               AND ge.deleted = 0 AND ge.enabled = 1",
        )
        .bind(&normalized)
        .bind(&normalized)
        .fetch_all(pool)
        .await?;
        if !rows.is_empty() {
            return Ok(rows.into_iter().map(GameExecutable::from).collect());
        }
    }

    // Same join as the path query: a name match must not resurrect a deleted
    // game either.
    let rows: Vec<ExecutableRow> = sqlx::query_as(
        "SELECT ge.id, ge.game_id, ge.exe_name, ge.exe_path, ge.enabled, ge.runner_kind, ge.launch_target, ge.title_match, ge.extra_args \
         FROM game_executables ge \
         JOIN games g ON g.id = ge.game_id AND g.deleted = 0 \
         WHERE ge.exe_name = ? AND ge.deleted = 0 AND ge.enabled = 1",
    )
    .bind(exe_name.to_lowercase())
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(GameExecutable::from).collect())
}

/// Find the game an executable belongs to, disambiguating by window caption
/// when one or more games use a shared-host emulator.
pub async fn game_for_executable_with_title(
    pool: &SqlitePool,
    exe_path: Option<&str>,
    exe_name: &str,
    window_title: Option<&str>,
) -> Result<Option<GameExecutable>, DbError> {
    let candidates = executables_for_process(pool, exe_path, exe_name).await?;
    if candidates.is_empty() {
        return Ok(None);
    }

    if candidates.len() == 1 && candidates[0].runner_kind == "native" {
        return Ok(Some(candidates[0].clone()));
    }

    if let Some(title) = window_title {
        let mut needles: Vec<Option<String>> = Vec::with_capacity(candidates.len());
        for candidate in &candidates {
            if let Some(needle) = candidate.needle() {
                needles.push(Some(needle));
            } else if let Ok(gid) = EntityId::parse(&candidate.game_id) {
                needles.push(game_title(pool, gid).await?);
            } else {
                needles.push(None);
            }
        }
        if let Some(best) = vn_core::emulator::best_title_match(title, &needles) {
            return Ok(Some(candidates[best].clone()));
        }
        // A shared-host emulator whose window title did not match any mapped game
        // (e.g. sitting in the Vita3K / RPCS3 / PCSX2 / Eden / PPSSPP main menu)
        // must never be attributed to a game just because only one game is mapped.
        return Ok(None);
    }

    if candidates.len() == 1 {
        let is_shared = vn_core::emulator::RunnerKind::parse(&candidates[0].runner_kind)
            .is_some_and(vn_core::emulator::RunnerKind::is_shared_host);
        if !is_shared {
            return Ok(Some(candidates[0].clone()));
        }
    }

    Ok(None)
}

/// Find the game an executable belongs to.
///
/// Path match wins over name match: two games can ship the same `game.exe`, and
/// the full path is what disambiguates them.
pub async fn game_for_executable(
    pool: &SqlitePool,
    exe_path: Option<&str>,
    exe_name: &str,
) -> Result<Option<GameExecutable>, DbError> {
    game_for_executable_with_title(pool, exe_path, exe_name, None).await
}

// scan folders
/// Remember a folder to scan.
pub async fn add_scan_folder(pool: &SqlitePool, path: &str, now: Timestamp) -> Result<(), DbError> {
    sqlx::query(
        "INSERT INTO scan_folders (path, added_at) VALUES (?, ?) ON CONFLICT(path) DO NOTHING",
    )
    .bind(path)
    .bind(now.as_millis())
    .execute(pool)
    .await?;
    Ok(())
}

/// Folders the user asked to scan.
pub async fn list_scan_folders(pool: &SqlitePool) -> Result<Vec<String>, DbError> {
    let rows: Vec<(String,)> = sqlx::query_as("SELECT path FROM scan_folders ORDER BY path")
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(|(path,)| path).collect())
}

/// Forget a folder.
pub async fn remove_scan_folder(pool: &SqlitePool, path: &str) -> Result<(), DbError> {
    sqlx::query("DELETE FROM scan_folders WHERE path = ?")
        .bind(path)
        .execute(pool)
        .await?;
    Ok(())
}

// tracker state
/// The open run, as persisted for crash recovery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenRunState {
    /// Game being tracked.
    pub game_id: String,
    /// Session id the checkpoints belong to.
    pub session_id: String,
    /// Chapter, when known.
    pub chapter_id: Option<String>,
    /// When the run started, UTC ms.
    pub started_at: i64,
    /// Active time so far.
    pub active_ms: i64,
    /// Away time so far.
    pub afk_ms: i64,
    /// Executable path, so the run can be matched on recovery.
    pub exe_path: String,
    /// When the last checkpoint was written.
    pub checkpoint_at: i64,
}

/// Persist the open run.
pub async fn save_open_run(pool: &SqlitePool, state: &OpenRunState) -> Result<(), DbError> {
    sqlx::query(
        "INSERT INTO tracker_state (id, game_id, session_id, chapter_id, started_at, active_ms, afk_ms, exe_path, checkpoint_at) \
         VALUES (1, ?, ?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT(id) DO UPDATE SET game_id = excluded.game_id, session_id = excluded.session_id, \
            chapter_id = excluded.chapter_id, started_at = excluded.started_at, \
            active_ms = excluded.active_ms, afk_ms = excluded.afk_ms, \
            exe_path = excluded.exe_path, checkpoint_at = excluded.checkpoint_at",
    )
    .bind(&state.game_id)
    .bind(&state.session_id)
    .bind(&state.chapter_id)
    .bind(state.started_at)
    .bind(state.active_ms)
    .bind(state.afk_ms)
    .bind(&state.exe_path)
    .bind(state.checkpoint_at)
    .execute(pool)
    .await?;
    Ok(())
}

/// Read the open run, if the last shutdown was not clean.
pub async fn load_open_run(pool: &SqlitePool) -> Result<Option<OpenRunState>, DbError> {
    let row: Option<OpenRunRow> = sqlx::query_as(
        "SELECT game_id, session_id, chapter_id, started_at, active_ms, afk_ms, exe_path, checkpoint_at \
         FROM tracker_state WHERE id = 1",
    )
    .fetch_optional(pool)
    .await?;

    Ok(row.map(OpenRunState::from))
}

/// Database row for the open run.
#[derive(Debug, sqlx::FromRow)]
struct OpenRunRow {
    game_id: String,
    session_id: String,
    chapter_id: Option<String>,
    started_at: i64,
    active_ms: i64,
    afk_ms: i64,
    exe_path: String,
    checkpoint_at: i64,
}

impl From<OpenRunRow> for OpenRunState {
    fn from(row: OpenRunRow) -> Self {
        Self {
            game_id: row.game_id,
            session_id: row.session_id,
            chapter_id: row.chapter_id,
            started_at: row.started_at,
            active_ms: row.active_ms,
            afk_ms: row.afk_ms,
            exe_path: row.exe_path,
            checkpoint_at: row.checkpoint_at,
        }
    }
}

/// Clear the open run after a clean close.
pub async fn clear_open_run(pool: &SqlitePool) -> Result<(), DbError> {
    sqlx::query("DELETE FROM tracker_state WHERE id = 1")
        .execute(pool)
        .await?;
    Ok(())
}

/// Write a pending session row.
///
/// Pending rows exist so a checkpoint survives a crash without pretending the run
/// has finished. Recovery promotes them into real sessions.
pub async fn write_pending_session(
    pool: &SqlitePool,
    segment: &Segment,
    device: &DeviceId,
    now: Timestamp,
) -> Result<(), DbError> {
    sqlx::query(
        "INSERT INTO pending_sessions (id, game_id, chapter_id, started_at, ended_at, counted_ms, afk_ms, source, device_id, written_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, 'desktop', ?, ?) \
         ON CONFLICT(id) DO UPDATE SET ended_at = excluded.ended_at, counted_ms = excluded.counted_ms, \
            afk_ms = excluded.afk_ms, written_at = excluded.written_at",
    )
    .bind(segment.session_id.to_string())
    .bind(segment.game_id.to_string())
    .bind(segment.chapter_id.map(|id| id.to_string()))
    .bind(segment.started_at.as_millis())
    .bind(segment.ended_at.as_millis())
    .bind(segment.active_ms)
    .bind(segment.afk_ms)
    .bind(device.as_str())
    .bind(now.as_millis())
    .execute(pool)
    .await?;
    Ok(())
}

/// Promote a checkpointed segment into a real play session and drop the pending row.
///
/// Uses the same id as the checkpoint, so a session written from a checkpoint and
/// later finalised updates in place rather than duplicating.
pub async fn promote_pending_session(
    pool: &SqlitePool,
    segment: &Segment,
    device: &DeviceId,
    now: Timestamp,
) -> Result<(), DbError> {
    let mut tx = pool.begin().await?;

    sqlx::query(
        "INSERT INTO play_sessions (id, game_id, chapter_id, route_id, started_at, ended_at, counted_ms, afk_ms, source, updated_at, device_id, deleted) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'desktop', ?, ?, 0) \
         ON CONFLICT(id) DO UPDATE SET ended_at = excluded.ended_at, counted_ms = excluded.counted_ms, \
            afk_ms = excluded.afk_ms, chapter_id = excluded.chapter_id, \
            route_id = excluded.route_id, updated_at = excluded.updated_at",
    )
    .bind(segment.session_id.to_string())
    .bind(segment.game_id.to_string())
    .bind(segment.chapter_id.map(|id| id.to_string()))
    // Written from the segment, so a route-timed run persists its route rather than
    // silently landing as an unattributed session.
    .bind(segment.route_id.map(|id| id.to_string()))
    .bind(segment.started_at.as_millis())
    .bind(segment.ended_at.as_millis())
    .bind(segment.active_ms)
    .bind(segment.afk_ms)
    .bind(now.as_millis())
    .bind(device.as_str())
    .execute(&mut *tx)
    .await?;

    // A desktop-tracked game must sort as started exactly like an Android one. Only
    // the Android path used to set this, so a library built on desktop showed
    // "never started" and no start date while the same game on Android had both.
    sqlx::query(
        "UPDATE games SET started_at = COALESCE(started_at, ?),          updated_at = MAX(updated_at, ?), device_id = ?          WHERE id = ? AND deleted = 0",
    )
    .bind(segment.started_at.as_millis())
    .bind(now.as_millis())
    .bind(device.as_str())
    .bind(segment.game_id.to_string())
    .execute(&mut *tx)
    .await?;

    sqlx::query("DELETE FROM pending_sessions WHERE id = ?")
        .bind(segment.session_id.to_string())
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;
    Ok(())
}

/// Pending sessions left behind by a crash.
pub async fn list_pending_sessions(pool: &SqlitePool) -> Result<Vec<Segment>, DbError> {
    let rows: Vec<PendingRow> = sqlx::query_as(
        "SELECT id, game_id, chapter_id, started_at, ended_at, counted_ms, afk_ms \
         FROM pending_sessions ORDER BY started_at ASC",
    )
    .fetch_all(pool)
    .await?;

    // A corrupt id means the row cannot be attributed to a game; dropping it is
    // the honest option, since a session with no game is meaningless.
    Ok(rows
        .into_iter()
        .filter_map(PendingRow::into_segment)
        .collect())
}

/// Database row for a checkpointed segment.
#[derive(Debug, sqlx::FromRow)]
struct PendingRow {
    id: String,
    game_id: String,
    chapter_id: Option<String>,
    started_at: i64,
    ended_at: i64,
    counted_ms: i64,
    afk_ms: i64,
}

impl PendingRow {
    /// Turn the row into a segment, or `None` when its ids are unreadable.
    fn into_segment(self) -> Option<Segment> {
        let session_id = EntityId::parse(&self.id).ok()?;
        let game_id = EntityId::parse(&self.game_id).ok()?;
        Some(Segment {
            session_id,
            game_id,
            chapter_id: self
                .chapter_id
                .as_deref()
                .and_then(|value| EntityId::parse(value).ok()),
            // A recovered pending session carries no route: this struct predates
            // route-timed sessions and a run interrupted mid-timer is not a state
            // the recovery path needs to know about.
            route_id: None,
            started_at: Timestamp::from_millis(self.started_at),
            ended_at: Timestamp::from_millis(self.ended_at),
            active_ms: self.counted_ms,
            afk_ms: self.afk_ms,
            checkpoint: false,
        })
    }
}

/// The library title of a game, which the engine may or may not use verbatim in
/// its window caption.
pub async fn game_title(pool: &SqlitePool, game_id: EntityId) -> Result<Option<String>, DbError> {
    let title: Option<String> =
        sqlx::query_scalar("SELECT title FROM games WHERE id = ? AND deleted = 0")
            .bind(game_id.to_string())
            .fetch_optional(pool)
            .await?;
    Ok(title)
}

/// A chapter as detection found it, before it is stored.
///
/// Carries both the raw engine scene name (the identity detection matches on) and
/// the human parts derived from it (name, route, order).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectedChapter {
    /// The engine's own scene name, e.g. `70_終章_0726`. Never renamed.
    pub raw_name: String,
    /// The name with the route prefix stripped.
    pub name: String,
    /// The engine's ordering code, when it has one.
    pub order: Option<u32>,
    /// Stable route identifier, e.g. `route_70`.
    pub route_key: Option<String>,
    /// Human route label, when known.
    pub route_name: Option<String>,
    /// The position marker as the engine reported it, e.g. `7月26日`.
    pub position: Option<String>,
}

/// Create a chapter the user named themselves.
///
/// Manual chapters are marked so they are never treated as detected scenes, and
/// so re-detection cannot deduplicate against them by accident.
pub async fn insert_manual_chapter(
    pool: &SqlitePool,
    game_id: EntityId,
    name: &str,
    route_name: Option<&str>,
    route_id: Option<EntityId>,
    device: &DeviceId,
    now: Timestamp,
) -> Result<EntityId, DbError> {
    let next_index: i64 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(sort_index), -1) + 1 FROM chapters WHERE game_id = ?",
    )
    .bind(game_id.to_string())
    .fetch_one(pool)
    .await?;

    let id = EntityId::new();
    sqlx::query(
        "INSERT INTO chapters (id, game_id, name, sort_index, reached, first_seen_at,             updated_at, device_id, deleted, display_name, route_key, route_name, manual, route_id)          VALUES (?, ?, ?, ?, 1, ?, ?, ?, 0, ?, ?, ?, 1, ?)",
    )
    .bind(id.to_string())
    .bind(game_id.to_string())
    .bind(name)
    .bind(next_index)
    .bind(now.as_millis())
    .bind(now.as_millis())
    .bind(device.as_str())
    .bind(name)
    // Shared with detection so a manual and an engine route with the same label
    // are one group. The prefix keeps the two key spaces apart.
    .bind(route_name.map(vn_core::chapter::manual_route_key))
    .bind(route_name)
    // The route row this chapter belongs to, when the user added it from inside
    // one. `route_key` above is the grouping key; this is the link the Routes card
    // actually groups on, so without it the chapter lands in the unlinked bucket
    // even though it was created from a route.
    .bind(route_id.map(|id| id.to_string()))
    .execute(pool)
    .await?;

    Ok(id)
}

/// Record that a game reached a named chapter, creating the row on first sight.
///
/// A chapter is identified by its name within one game, so passing the same scene
/// twice does not create duplicates and re-entering a chapter does not reset when
/// it was first seen. `sort_index` follows the engine's own ordering prefix, which
/// is what the game uses to order its script.
pub async fn upsert_chapter(
    pool: &SqlitePool,
    game_id: EntityId,
    scene: &DetectedChapter,
    device: &DeviceId,
    now: Timestamp,
) -> Result<EntityId, DbError> {
    // Reuse the existing row when this scene has been seen before. The match is on
    // the raw scene name, which detection re-reads every poll, so a renamed
    // chapter keeps its row instead of gaining a duplicate.
    let existing: Option<String> = sqlx::query_scalar(
        "SELECT id FROM chapters WHERE game_id = ? AND name = ? AND deleted = 0 LIMIT 1",
    )
    .bind(game_id.to_string())
    .bind(&scene.raw_name)
    .fetch_optional(pool)
    .await?;

    if let Some(id) = existing {
        // A stored row always holds an id we wrote, so a parse failure means the
        // database was edited by hand; treat it as a fresh chapter rather than
        // failing the poll.
        if let Ok(id) = EntityId::parse(&id) {
            // Only touch bookkeeping, and only when it changes: this runs on
            // every 2-second poll while the chapter stays detected, and chapters
            // sync by `updated_at` last-write-wins — writing on every poll would
            // let this device's no-op bookkeeping out-date a rename or route
            // link made on another device and silently revert it. The display
            // name is the user's, and route labels may have been learned since;
            // neither is overwritten here.
            sqlx::query(
                "UPDATE chapters SET reached = 1, updated_at = ? WHERE id = ? AND reached = 0",
            )
            .bind(now.as_millis())
            .bind(id.to_string())
            .execute(pool)
            .await?;
            return Ok(id);
        }
    }

    insert_chapter(pool, game_id, scene, device, now).await
}

/// Insert a chapter row, ordering it after the ones already known.
async fn insert_chapter(
    pool: &SqlitePool,
    game_id: EntityId,
    scene: &DetectedChapter,
    device: &DeviceId,
    now: Timestamp,
) -> Result<EntityId, DbError> {
    // Append after the chapters already known for this game.
    let next_index: i64 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(sort_index), -1) + 1 FROM chapters WHERE game_id = ?",
    )
    .bind(game_id.to_string())
    .fetch_one(pool)
    .await?;

    let id = EntityId::new();
    // `sort_index` is the position in this game's chapter list, and the engine's
    // route code used to be written here instead. That collided with the counter
    // manual chapters use: a chapter from route 10 landed on `sort_index` 10 while
    // the eleventh manual chapter also landed on 10, so the list interleaved two
    // unrelated orderings. The code is an ordering *within* a route, so it belongs
    // in `route_order`; `sort_index` stays a plain append counter for both.
    sqlx::query(
        // `display_name` starts as the detected name so a new chapter is readable
        // immediately; a user rename overwrites it later and is never reset here.
        "INSERT INTO chapters (id, game_id, name, sort_index, reached, first_seen_at,             updated_at, device_id, deleted, display_name, route_key, route_name, route_order, position, manual)          VALUES (?, ?, ?, ?, 1, ?, ?, ?, 0, ?, ?, ?, ?, ?, 0)",
    )
    .bind(id.to_string())
    .bind(game_id.to_string())
    .bind(&scene.raw_name)
    .bind(next_index)
    .bind(now.as_millis())
    .bind(now.as_millis())
    .bind(device.as_str())
    .bind(&scene.name)
    .bind(scene.route_key.as_deref())
    .bind(scene.route_name.as_deref())
    .bind(scene.order.map(i64::from))
    .bind(scene.position.as_deref())
    .execute(pool)
    .await?;

    Ok(id)
}

/// Rename a chapter for this device.
///
/// The engine's raw name stays stored in `name` because detection matches on it;
/// only `display_name` changes. Passing `None` clears the rename and shows the
/// engine's own name again.
pub async fn rename_chapter(
    pool: &SqlitePool,
    chapter_id: EntityId,
    display_name: Option<&str>,
    device: &DeviceId,
    now: Timestamp,
) -> Result<(), DbError> {
    // An empty rename is the same as clearing it: showing a blank row would be a
    // chapter the user cannot identify.
    let display_name = display_name.map(str::trim).filter(|name| !name.is_empty());

    // `renamed` records the act, so the UI never has to infer it from values.
    let renamed = i64::from(display_name.is_some());
    sqlx::query(
        "UPDATE chapters SET display_name = ?, renamed = ?, updated_at = ?, device_id = ? WHERE id = ?",
    )
        .bind(display_name)
        .bind(renamed)
        .bind(now.as_millis())
        .bind(device.as_str())
        .bind(chapter_id.to_string())
        .execute(pool)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;

    fn now() -> Timestamp {
        Timestamp::from_millis(1_700_000_000_000)
    }

    async fn fixture() -> (Db, DeviceId, EntityId) {
        let db = Db::open_memory().await.expect("open");
        let device = crate::db::queries::ensure_device_id(db.pool())
            .await
            .expect("device");
        let game = vn_core::model::Game::new(
            &device,
            now(),
            "Test Game",
            vn_core::model::GameStatus::Playing,
        )
        .expect("game");
        crate::db::queries::upsert_game(db.pool(), &game)
            .await
            .expect("insert");
        (db, device, game.meta.id)
    }

    fn detected(raw: &str, name: &str, order: u32, route: Option<(&str, &str)>) -> DetectedChapter {
        DetectedChapter {
            raw_name: raw.to_owned(),
            name: name.to_owned(),
            order: Some(order),
            route_key: route.map(|(key, _)| key.to_owned()),
            route_name: route.map(|(_, label)| label.to_owned()),
            position: None,
        }
    }

    #[tokio::test]
    async fn adding_the_same_path_twice_does_not_duplicate_it() {
        // Regression: the lookup compared stored `F:\Games\game.exe` against a
        // pre-normalised `f:/games/game.exe`, so it never matched and every add
        // inserted a new row. Real libraries ended up with the same executable
        // mapped to two different games.
        let (db, device, game_id) = fixture().await;

        let first = add_executable(
            db.pool(),
            &device,
            now(),
            game_id,
            r"F:\Games\Sample\game.exe",
        )
        .await
        .expect("first");
        let again = add_executable(
            db.pool(),
            &device,
            now(),
            game_id,
            r"F:\Games\Sample\game.exe",
        )
        .await
        .expect("again");

        assert_eq!(first.id, again.id, "the same path reuses its row");
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM game_executables")
            .fetch_one(db.pool())
            .await
            .expect("count");
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn adding_a_path_with_the_other_separator_is_the_same_executable() {
        // A path pasted from elsewhere uses forward slashes; it is the same file.
        let (db, device, game_id) = fixture().await;

        let first = add_executable(
            db.pool(),
            &device,
            now(),
            game_id,
            r"F:\Games\Sample\game.exe",
        )
        .await
        .expect("first");
        let second = add_executable(
            db.pool(),
            &device,
            now(),
            game_id,
            "F:/Games/Sample/game.exe",
        )
        .await
        .expect("second");

        assert_eq!(first.id, second.id);
    }

    #[tokio::test]
    async fn adding_the_same_path_to_another_game_moves_the_mapping() {
        // One executable belongs to one game; re-pointing it must not leave the
        // old mapping behind, or tracking attributes time to both.
        let (db, device, game_a) = fixture().await;
        let game_b = vn_core::model::Game::new(
            &device,
            now(),
            "Second Game",
            vn_core::model::GameStatus::Playing,
        )
        .expect("game b");
        crate::db::queries::upsert_game(db.pool(), &game_b)
            .await
            .expect("insert b");

        add_executable(db.pool(), &device, now(), game_a, r"F:\Games\x.exe")
            .await
            .expect("first");
        add_executable(db.pool(), &device, now(), game_b.meta.id, r"F:\Games\x.exe")
            .await
            .expect("second");

        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM game_executables")
            .fetch_one(db.pool())
            .await
            .expect("count");
        assert_eq!(count, 1, "the mapping moved rather than duplicated");
        let owner: String = sqlx::query_scalar("SELECT game_id FROM game_executables")
            .fetch_one(db.pool())
            .await
            .expect("owner");
        assert_eq!(owner, game_b.meta.id.to_string());
    }

    #[tokio::test]
    async fn detecting_the_same_scene_twice_does_not_duplicate_the_chapter() {
        let (db, device, game_id) = fixture().await;
        let scene = detected("70_終章_0726", "終章_0726", 70, Some(("route_70", "Final")));

        let first = upsert_chapter(db.pool(), game_id, &scene, &device, now())
            .await
            .expect("first");
        let second = upsert_chapter(db.pool(), game_id, &scene, &device, now())
            .await
            .expect("second");

        assert_eq!(first, second, "the same scene reuses its chapter row");
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM chapters")
            .fetch_one(db.pool())
            .await
            .expect("count");
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn a_new_chapter_stores_its_route() {
        let (db, device, game_id) = fixture().await;
        let scene = detected(
            "20_しろは0815",
            "しろは0815",
            20,
            Some(("route_20", "Shiroha")),
        );
        upsert_chapter(db.pool(), game_id, &scene, &device, now())
            .await
            .expect("insert");

        let (name, route_key, route_name, display): (
            String,
            Option<String>,
            Option<String>,
            Option<String>,
        ) = sqlx::query_as("SELECT name, route_key, route_name, display_name FROM chapters")
            .fetch_one(db.pool())
            .await
            .expect("row");
        assert_eq!(
            name, "20_しろは0815",
            "the raw scene name is the stored identity"
        );
        assert_eq!(route_key.as_deref(), Some("route_20"));
        assert_eq!(route_name.as_deref(), Some("Shiroha"));
        // Seeded with the detected name so a new chapter is readable at once.
        // A "renamed" flag is therefore "display differs from name", not "is set".
        assert_eq!(
            display.as_deref(),
            Some("しろは0815"),
            "a new chapter shows the detected name until the user renames it"
        );
    }

    #[tokio::test]
    async fn renaming_changes_the_display_name_but_not_the_identity() {
        let (db, device, game_id) = fixture().await;
        let scene = detected("70_終章_0726", "終章_0726", 70, Some(("route_70", "Final")));
        let id = upsert_chapter(db.pool(), game_id, &scene, &device, now())
            .await
            .expect("insert");

        rename_chapter(db.pool(), id, Some("Final Chapter"), &device, now())
            .await
            .expect("rename");

        let (name, display): (String, Option<String>) =
            sqlx::query_as("SELECT name, display_name FROM chapters WHERE id = ?")
                .bind(id.to_string())
                .fetch_one(db.pool())
                .await
                .expect("row");
        assert_eq!(name, "70_終章_0726", "the raw name still matches detection");
        assert_eq!(display.as_deref(), Some("Final Chapter"));
    }

    #[tokio::test]
    async fn a_renamed_chapter_is_not_recreated_by_detection() {
        // The reason the raw name stays the key: detection runs every poll, and a
        // rename that replaced it would spawn a duplicate on the next read.
        let (db, device, game_id) = fixture().await;
        let scene = detected("70_終章_0726", "終章_0726", 70, None);
        let id = upsert_chapter(db.pool(), game_id, &scene, &device, now())
            .await
            .expect("insert");
        rename_chapter(db.pool(), id, Some("My name"), &device, now())
            .await
            .expect("rename");

        let again = upsert_chapter(db.pool(), game_id, &scene, &device, now())
            .await
            .expect("re-detect");
        assert_eq!(again, id, "re-detection matched the renamed chapter");

        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM chapters")
            .fetch_one(db.pool())
            .await
            .expect("count");
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn clearing_a_rename_restores_the_engine_name() {
        let (db, device, game_id) = fixture().await;
        let scene = detected("10_プロローグ0725", "プロローグ0725", 10, None);
        let id = upsert_chapter(db.pool(), game_id, &scene, &device, now())
            .await
            .expect("insert");

        rename_chapter(db.pool(), id, Some("Prologue"), &device, now())
            .await
            .expect("rename");
        rename_chapter(db.pool(), id, None, &device, now())
            .await
            .expect("clear");

        let display: Option<String> =
            sqlx::query_scalar("SELECT display_name FROM chapters WHERE id = ?")
                .bind(id.to_string())
                .fetch_one(db.pool())
                .await
                .expect("row");
        assert_eq!(display, None);
    }

    #[tokio::test]
    async fn a_blank_rename_is_treated_as_no_rename() {
        // An empty display name would render as an unidentifiable row.
        let (db, device, game_id) = fixture().await;
        let scene = detected("10_プロローグ0725", "プロローグ0725", 10, None);
        let id = upsert_chapter(db.pool(), game_id, &scene, &device, now())
            .await
            .expect("insert");
        rename_chapter(db.pool(), id, Some("   "), &device, now())
            .await
            .expect("rename");

        let display: Option<String> =
            sqlx::query_scalar("SELECT display_name FROM chapters WHERE id = ?")
                .bind(id.to_string())
                .fetch_one(db.pool())
                .await
                .expect("row");
        assert_eq!(display, None);
    }

    #[tokio::test]
    async fn the_renamed_flag_records_the_act_not_the_values() {
        // `display_name` is seeded on insert, so it can never be used to tell a
        // rename from the detected name. This asserts the explicit flag.
        let (db, device, game_id) = fixture().await;
        let scene = detected("70_終章_0726", "終章_0726", 70, None);
        let id = upsert_chapter(db.pool(), game_id, &scene, &device, now())
            .await
            .expect("insert");

        let seeded: i64 = sqlx::query_scalar("SELECT renamed FROM chapters WHERE id = ?")
            .bind(id.to_string())
            .fetch_one(db.pool())
            .await
            .expect("row");
        assert_eq!(seeded, 0, "a freshly detected chapter is not renamed");

        rename_chapter(db.pool(), id, Some("Final Chapter"), &device, now())
            .await
            .expect("rename");
        let renamed: i64 = sqlx::query_scalar("SELECT renamed FROM chapters WHERE id = ?")
            .bind(id.to_string())
            .fetch_one(db.pool())
            .await
            .expect("row");
        assert_eq!(renamed, 1);

        rename_chapter(db.pool(), id, None, &device, now())
            .await
            .expect("clear");
        let cleared: i64 = sqlx::query_scalar("SELECT renamed FROM chapters WHERE id = ?")
            .bind(id.to_string())
            .fetch_one(db.pool())
            .await
            .expect("row");
        assert_eq!(cleared, 0, "clearing a rename makes it un-renamed again");
    }

    #[tokio::test]
    async fn a_detected_chapter_does_not_collide_with_a_manual_chapter_index() {
        // Regression: `insert_chapter` wrote the engine's route code into
        // `sort_index` (route 10 became sort_index 10), while manual chapters used
        // MAX+1. Ten manual chapters and a route-10 scene therefore shared an index,
        // so the list interleaved two unrelated orderings. The code now lives in
        // `route_order` and `sort_index` is a plain append for both.
        let (db, device, game_id) = fixture().await;

        let scene = detected(
            "10_プロローグ0725",
            "プロローグ0725",
            10,
            Some(("route_10", "Common")),
        );
        upsert_chapter(db.pool(), game_id, &scene, &device, now())
            .await
            .expect("detected");

        let manual = insert_manual_chapter(db.pool(), game_id, "Mine", None, None, &device, now())
            .await
            .expect("manual");

        let (detected_index, detected_route_order): (i64, Option<i64>) = sqlx::query_as(
            "SELECT sort_index, route_order FROM chapters WHERE game_id = ? AND manual = 0",
        )
        .bind(game_id.to_string())
        .fetch_one(db.pool())
        .await
        .expect("detected row");
        let manual_index: i64 = sqlx::query_scalar("SELECT sort_index FROM chapters WHERE id = ?")
            .bind(manual.to_string())
            .fetch_one(db.pool())
            .await
            .expect("manual row");

        assert_ne!(
            detected_index, manual_index,
            "a detected chapter and a manual one must not claim the same position"
        );
        assert_eq!(
            detected_route_order,
            Some(10),
            "the engine's code is an ordering within the route, so it belongs in route_order"
        );
        assert_eq!(
            detected_index, 0,
            "the first chapter is appended at 0, not at its route code"
        );
        assert_eq!(manual_index, 1, "the next chapter appends after it");
    }

    #[tokio::test]
    async fn a_chapter_added_inside_a_route_lands_on_that_route() {
        // The Routes card creates a chapter from inside a route. The route label and
        // the route row are two different things, so a chapter that carried only the
        // label fell into the unlinked bucket even though it was added from a route,
        // which left the new route with nothing to time.
        let (db, device, game_id) = fixture().await;
        let route = vn_core::model::Route::new(&device, now(), game_id, "Shiroha").unwrap();
        crate::db::queries::upsert_route(db.pool(), &route)
            .await
            .expect("route");

        let id = insert_manual_chapter(
            db.pool(),
            game_id,
            "Chapter One",
            Some("Shiroha"),
            Some(route.meta.id),
            &device,
            now(),
        )
        .await
        .expect("insert");

        let (route_id, route_name): (Option<String>, Option<String>) =
            sqlx::query_as("SELECT route_id, route_name FROM chapters WHERE id = ?")
                .bind(id.to_string())
                .fetch_one(db.pool())
                .await
                .expect("row");
        assert_eq!(
            route_id.as_deref(),
            Some(route.meta.id.to_string().as_str()),
            "the chapter must be linked to the route it was created from"
        );
        assert_eq!(route_name.as_deref(), Some("Shiroha"));
    }

    #[tokio::test]
    async fn a_manual_chapter_is_marked_and_keeps_its_route() {
        let (db, device, game_id) = fixture().await;
        let id = insert_manual_chapter(
            db.pool(),
            game_id,
            "Chapter One",
            Some("Shiroha"),
            None,
            &device,
            now(),
        )
        .await
        .expect("insert");

        let (manual, route_name, display): (i64, Option<String>, Option<String>) =
            sqlx::query_as("SELECT manual, route_name, display_name FROM chapters WHERE id = ?")
                .bind(id.to_string())
                .fetch_one(db.pool())
                .await
                .expect("row");
        assert_eq!(manual, 1, "manual chapters are marked");
        assert_eq!(route_name.as_deref(), Some("Shiroha"));
        assert_eq!(display.as_deref(), Some("Chapter One"));
    }

    #[tokio::test]
    async fn executables_round_trip_and_lowercase_the_name() {
        let (db, device, game_id) = fixture().await;
        let entry = add_executable(
            db.pool(),
            &device,
            now(),
            game_id,
            r"C:\Games\Test\SiglusEngine.exe",
        )
        .await
        .expect("add");
        assert_eq!(entry.exe_name, "siglusengine.exe");
        assert!(entry.enabled);

        let listed = list_executables(db.pool()).await.expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].game_id, game_id.to_string());
    }

    #[tokio::test]
    async fn the_same_path_maps_once_and_can_move_between_games() {
        let (db, device, game_id) = fixture().await;
        let other = vn_core::model::Game::new(
            &device,
            now(),
            "Other Game",
            vn_core::model::GameStatus::Wishlist,
        )
        .expect("game");
        crate::db::queries::upsert_game(db.pool(), &other)
            .await
            .expect("insert");

        // Backslashes, because that is what a Windows path looks like. This test
        // used to pass forward slashes, which matched the lookup by accident and
        // let a separator bug ship: real paths never deduplicated.
        let first = add_executable(db.pool(), &device, now(), game_id, r"C:\g\game.exe")
            .await
            .expect("add");
        let second = add_executable(db.pool(), &device, now(), other.meta.id, r"C:\g\game.exe")
            .await
            .expect("re-add");

        assert_eq!(first.id, second.id, "same path reuses the row");
        let listed = list_executables(db.pool()).await.expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].game_id, other.meta.id.to_string());
    }

    #[tokio::test]
    async fn lookup_prefers_the_full_path_over_the_file_name() {
        let (db, device, game_id) = fixture().await;
        let other = vn_core::model::Game::new(
            &device,
            now(),
            "Same Exe Other Folder",
            vn_core::model::GameStatus::Wishlist,
        )
        .expect("game");
        crate::db::queries::upsert_game(db.pool(), &other)
            .await
            .expect("insert");

        add_executable(db.pool(), &device, now(), game_id, "C:/a/game.exe")
            .await
            .expect("a");
        add_executable(db.pool(), &device, now(), other.meta.id, "C:/b/game.exe")
            .await
            .expect("b");

        let found = game_for_executable(db.pool(), Some("C:/b/game.exe"), "game.exe")
            .await
            .expect("lookup")
            .expect("found");
        assert_eq!(
            found.game_id,
            other.meta.id.to_string(),
            "path disambiguates"
        );
    }

    #[tokio::test]
    async fn lookup_falls_back_to_the_name_when_the_path_is_unknown() {
        let (db, device, game_id) = fixture().await;
        add_executable(db.pool(), &device, now(), game_id, "C:/a/unique.exe")
            .await
            .expect("add");
        let found = game_for_executable(db.pool(), None, "UNIQUE.EXE")
            .await
            .expect("lookup")
            .expect("found");
        assert_eq!(found.game_id, game_id.to_string());
    }

    #[tokio::test]
    async fn an_unknown_executable_is_not_attributed() {
        let (db, _device, _game) = fixture().await;
        assert!(
            game_for_executable(db.pool(), Some("C:/nope.exe"), "nope.exe")
                .await
                .expect("lookup")
                .is_none()
        );
    }

    #[tokio::test]
    async fn removing_a_mapping_stops_attribution() {
        let (db, device, game_id) = fixture().await;
        let entry = add_executable(db.pool(), &device, now(), game_id, "C:/a/gone.exe")
            .await
            .expect("add");
        remove_executable(db.pool(), &entry.id, now(), &device)
            .await
            .expect("remove");
        assert!(
            game_for_executable(db.pool(), Some("C:/a/gone.exe"), "gone.exe")
                .await
                .expect("lookup")
                .is_none()
        );
    }

    #[tokio::test]
    async fn scan_folders_are_unique_and_removable() {
        let (db, _device, _game) = fixture().await;
        add_scan_folder(db.pool(), "C:/Games", now())
            .await
            .expect("add");
        add_scan_folder(db.pool(), "C:/Games", now())
            .await
            .expect("dup");
        add_scan_folder(db.pool(), "D:/VN", now())
            .await
            .expect("add");

        let folders = list_scan_folders(db.pool()).await.expect("list");
        assert_eq!(folders.len(), 2);

        remove_scan_folder(db.pool(), "C:/Games")
            .await
            .expect("remove");
        assert_eq!(list_scan_folders(db.pool()).await.expect("list").len(), 1);
    }

    #[tokio::test]
    async fn open_run_state_survives_and_clears() {
        let (db, _device, game_id) = fixture().await;
        assert!(load_open_run(db.pool()).await.expect("load").is_none());

        let state = OpenRunState {
            game_id: game_id.to_string(),
            session_id: "s1".to_owned(),
            chapter_id: None,
            started_at: 1_000,
            active_ms: 5_000,
            afk_ms: 1_000,
            exe_path: "C:/a/game.exe".to_owned(),
            checkpoint_at: 1_500,
        };
        save_open_run(db.pool(), &state).await.expect("save");
        assert_eq!(
            load_open_run(db.pool())
                .await
                .expect("load")
                .expect("present"),
            state
        );

        clear_open_run(db.pool()).await.expect("clear");
        assert!(load_open_run(db.pool()).await.expect("load").is_none());
    }

    fn segment(game_id: EntityId, active: i64, afk: i64) -> Segment {
        Segment {
            session_id: EntityId::new(),
            game_id,
            chapter_id: None,
            route_id: None,
            started_at: Timestamp::from_millis(0),
            ended_at: Timestamp::from_millis(active + afk),
            active_ms: active,
            afk_ms: afk,
            checkpoint: false,
        }
    }

    #[tokio::test]
    async fn a_checkpoint_updates_in_place_and_promotes_once() {
        let (db, device, game_id) = fixture().await;
        let mut seg = segment(game_id, 5_000, 0);

        // Two checkpoints for the same session id.
        write_pending_session(db.pool(), &seg, &device, now())
            .await
            .expect("cp1");
        seg.active_ms = 20_000;
        seg.ended_at = Timestamp::from_millis(20_000);
        write_pending_session(db.pool(), &seg, &device, now())
            .await
            .expect("cp2");

        let pending = list_pending_sessions(db.pool()).await.expect("list");
        assert_eq!(pending.len(), 1, "one row per session, not per checkpoint");
        assert_eq!(pending[0].active_ms, 20_000);

        promote_pending_session(db.pool(), &seg, &device, now())
            .await
            .expect("promote");

        assert!(
            list_pending_sessions(db.pool())
                .await
                .expect("list")
                .is_empty(),
            "promotion clears the pending row"
        );
        let sessions = crate::db::queries::list_sessions(db.pool(), game_id)
            .await
            .expect("sessions");
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].counted_ms, 20_000);
    }

    #[tokio::test]
    async fn active_and_away_time_both_survive_promotion() {
        let (db, device, game_id) = fixture().await;
        let seg = segment(game_id, 30_000, 60_000);
        promote_pending_session(db.pool(), &seg, &device, now())
            .await
            .expect("promote");

        let sessions = crate::db::queries::list_sessions(db.pool(), game_id)
            .await
            .expect("sessions");
        assert_eq!(sessions[0].counted_ms, 30_000, "active only");
        assert_eq!(sessions[0].afk_ms, 60_000, "away kept separately");
    }

    #[tokio::test]
    async fn a_deleted_game_hides_its_executables() {
        let (db, device, game_id) = fixture().await;
        add_executable(db.pool(), &device, now(), game_id, "C:/a/x.exe")
            .await
            .expect("add");
        crate::db::queries::soft_delete_game(db.pool(), game_id, now(), &device)
            .await
            .expect("delete game");

        // The executable row survives soft-delete (it is not a synced row), but
        // the lookup joins on a live game, so a deleted game is never
        // attributed: the poll must not keep writing sessions for a game the
        // user removed. The row is kept so a restore does not need remapping.
        let found = game_for_executable(db.pool(), Some("C:/a/x.exe"), "x.exe")
            .await
            .expect("lookup");
        assert!(found.is_none(), "a deleted game must not be attributed");

        // The name fallback is joined the same way: game.exe cannot resurrect a
        // deleted game when the path no longer matches.
        let by_name = game_for_executable(db.pool(), None, "x.exe")
            .await
            .expect("lookup");
        assert!(by_name.is_none(), "the name fallback must hide it too");
    }

    #[tokio::test]
    async fn shared_hosts_map_multiple_games_without_moving_each_other() {
        let (db, device, game_a) = fixture().await;
        let game_b = vn_core::model::Game::new(
            &device,
            now(),
            "Second Vita Game",
            vn_core::model::GameStatus::Playing,
        )
        .expect("game b");
        crate::db::queries::upsert_game(db.pool(), &game_b)
            .await
            .expect("insert b");

        let target_a = RunnerTarget {
            runner_kind: "vita3k".to_owned(),
            exe_path: Some(r"C:\Emulators\Vita3K\Vita3K.exe".to_owned()),
            package: None,
            launch_target: Some("PCSG00001".to_owned()),
            title_match: Some("Game Alpha".to_owned()),
            extra_args: None,
        };
        let target_b = RunnerTarget {
            runner_kind: "vita3k".to_owned(),
            exe_path: Some(r"C:\Emulators\Vita3K\Vita3K.exe".to_owned()),
            package: None,
            launch_target: Some("PCSG00002".to_owned()),
            title_match: Some("Game Beta".to_owned()),
            extra_args: None,
        };

        let mapped_a = add_runner_target(db.pool(), &device, now(), game_a, &target_a)
            .await
            .expect("add a");
        let mapped_b = add_runner_target(db.pool(), &device, now(), game_b.meta.id, &target_b)
            .await
            .expect("add b");

        assert_ne!(
            mapped_a.id, mapped_b.id,
            "different games on the same host have separate rows"
        );

        let listed = list_executables(db.pool()).await.expect("list");
        assert_eq!(listed.len(), 2, "both games remain mapped to the emulator");

        // Window title disambiguation
        let found_a = game_for_executable_with_title(
            db.pool(),
            Some(r"C:\Emulators\Vita3K\Vita3K.exe"),
            "vita3k.exe",
            Some("Vita3K | Game Alpha (PCSG00001)"),
        )
        .await
        .expect("lookup a")
        .expect("found a");
        assert_eq!(found_a.game_id, game_a.to_string());

        let found_b = game_for_executable_with_title(
            db.pool(),
            Some(r"C:\Emulators\Vita3K\Vita3K.exe"),
            "vita3k.exe",
            Some("Vita3K | Game Beta (PCSG00002)"),
        )
        .await
        .expect("lookup b")
        .expect("found b");
        assert_eq!(found_b.game_id, game_b.meta.id.to_string());

        // Unmatched window title (e.g. idle in emulator GUI) credits nothing
        let idle = game_for_executable_with_title(
            db.pool(),
            Some(r"C:\Emulators\Vita3K\Vita3K.exe"),
            "vita3k.exe",
            Some("Vita3K v0.2.0"),
        )
        .await
        .expect("lookup idle");
        assert!(
            idle.is_none(),
            "idle emulator GUI does not falsely attribute to any game"
        );
    }
}
