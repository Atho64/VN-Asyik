//! Tauri commands for the achievement catalog (phase P1, backend only).
//!
//! The frontend screen and i18n arrive in P2. These commands expose the full
//! catalog with live progress, run an explicit evaluation, and record the
//! Discover-import marker. Automatic triggers (startup scan, tracking loop,
//! post-sync, exports) all go through [`refresh_and_emit`] so the unlock event
//! and storage rules live in one place.

use sqlx::SqlitePool;
use tauri::{Emitter, State};
use vn_core::achievements::AchievementUnlock;
use vn_core::clock::Timestamp;
use vn_core::ids::DeviceId;

use super::CommandResult;
use crate::db::DbError;
use crate::db::achievements::{self, AchievementView};
use crate::state::AppState;

/// `local_prefs` marker: the first backup/library export was made (F7).
pub(crate) const BACKUP_MARKER: &str = "achievements.backupMade";
/// `local_prefs` marker: a game was imported from the Discover screen (F4).
pub(crate) const DISCOVER_MARKER: &str = "achievements.discoverImported";

/// List all 65 catalog items with progress; locked items still show progress.
///
/// # Errors
/// Returns [`CommandError`] on database failure.
#[tauri::command]
pub async fn achievements_list(state: State<'_, AppState>) -> CommandResult<Vec<AchievementView>> {
    Ok(achievements::list(state.db().pool()).await?)
}

/// Evaluate the catalog now and return the slugs unlocked by this call.
///
/// # Errors
/// Returns [`CommandError`] on database failure.
#[tauri::command]
pub async fn achievements_evaluate(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> CommandResult<Vec<String>> {
    let fresh = refresh_and_emit(&app, state.db().pool(), state.device()).await?;
    Ok(fresh
        .into_iter()
        .map(|unlock| unlock.achievement_id)
        .collect())
}

/// Record that a game was imported from the Discover screen (F4 marker).
///
/// P2 wiring: the Discover import flow calls this after a successful import.
/// Until then F4 stays locked; the backend is ready and evaluates the marker.
///
/// # Errors
/// Returns [`CommandError`] on database failure.
#[tauri::command]
pub async fn achievements_note_discover_import(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> CommandResult<()> {
    note_marker(state.db().pool(), DISCOVER_MARKER).await?;
    refresh_and_emit(&app, state.db().pool(), state.device()).await?;
    Ok(())
}

/// Write a first-occurrence marker timestamp, keeping the earliest date.
async fn note_marker(pool: &SqlitePool, key: &str) -> Result<(), DbError> {
    let now = Timestamp::now().map_err(|err| DbError::Corrupt(err.to_string()))?;
    sqlx::query(
        "INSERT INTO local_prefs (key, value) VALUES (?, ?) \
         ON CONFLICT(key) DO NOTHING",
    )
    .bind(key)
    .bind(now.as_millis().to_string())
    .execute(pool)
    .await?;
    Ok(())
}

/// Record the first backup export (F7). Called by the export commands.
///
/// # Errors
/// Returns [`DbError`] if the marker cannot be stored.
pub(crate) async fn note_backup_made(pool: &SqlitePool) -> Result<(), DbError> {
    note_marker(pool, BACKUP_MARKER).await
}

/// Evaluate, store new unlocks, and emit `vn:achievements-unlocked` with the
/// new slugs. The single refresh path shared by every automatic trigger.
///
/// # Errors
/// Returns [`DbError`] on clock or database failure; the emit itself is
/// best-effort (a missing listener is not an error).
pub async fn refresh_and_emit(
    app: &tauri::AppHandle,
    pool: &SqlitePool,
    device: &DeviceId,
) -> Result<Vec<AchievementUnlock>, DbError> {
    let now = Timestamp::now().map_err(|err| DbError::Corrupt(err.to_string()))?;
    let fresh = achievements::evaluate_and_store(pool, device, now).await?;
    if !fresh.is_empty() {
        let slugs: Vec<&str> = fresh.iter().map(|u| u.achievement_id.as_str()).collect();
        let _ = app.emit(
            "vn:achievements-unlocked",
            serde_json::json!({ "slugs": slugs }),
        );
    }
    Ok(fresh)
}

/// Fire-and-forget refresh for triggers that must not fail their own operation
/// (boot, tracking loop, sync, imports, exports). Errors are logged only.
pub(crate) async fn refresh_quietly(app: &tauri::AppHandle, pool: &SqlitePool, device: &DeviceId) {
    if let Err(err) = refresh_and_emit(app, pool, device).await {
        eprintln!("achievement refresh failed: {err}");
    }
}
