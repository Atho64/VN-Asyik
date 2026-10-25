//! Sync commands: run a sync, report state, and read the conflict log.

use serde::Serialize;
use tauri::State;

use vn_core::clock::Timestamp;

use super::{CommandError, CommandResult};
use crate::db::queries;
use crate::state::AppState;
use crate::sync;

/// What the Sync screen needs to decide what to show.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatusView {
    /// Device id, so the user can tell two installs apart.
    pub device_id: String,
    /// Folder last used, when there is one.
    pub folder: Option<String>,
    /// When a sync last completed.
    pub last_sync_at_ms: Option<i64>,
    /// Rows that would be exchanged, for a sense of scale.
    pub games: i64,
    pub routes: i64,
    pub chapters: i64,
    pub sessions: i64,
    pub endings: i64,
}

/// One row of the conflict log.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictView {
    pub table: String,
    pub record_id: String,
    /// `local` or `remote`.
    pub kept: String,
    /// `newer`, `tie_break` or `delete_wins`.
    pub reason: String,
    pub discarded_updated_at_ms: i64,
    pub created_at_ms: i64,
}

/// Read a `sync_state` string value.
async fn state_value(pool: &sqlx::SqlitePool, key: &str) -> Option<String> {
    let row: Option<(String,)> = sqlx::query_as("SELECT value FROM sync_state WHERE key = ?")
        .bind(key)
        .fetch_optional(pool)
        .await
        .ok()?;
    row.map(|(value,)| value)
}

/// Sync status for the UI.
#[tauri::command]
pub async fn sync_status(state: State<'_, AppState>) -> CommandResult<SyncStatusView> {
    use sqlx::Row as _;
    let pool = state.db().pool();
    let device = queries::ensure_device_id(pool).await?;

    let counts = sqlx::query(
        "SELECT (SELECT COUNT(*) FROM games) AS games, \
                (SELECT COUNT(*) FROM routes r JOIN games g ON g.id = r.game_id) AS routes, \
                (SELECT COUNT(*) FROM chapters c JOIN games g ON g.id = c.game_id) AS chapters, \
                (SELECT COUNT(*) FROM play_sessions s JOIN games g ON g.id = s.game_id) AS sessions, \
                (SELECT COUNT(*) FROM endings e JOIN games g ON g.id = e.game_id) AS endings",
    )
    .fetch_one(pool)
    .await
    .map_err(|err| CommandError::new("db", err.to_string()))?;

    let last_sync_at_ms = state_value(pool, "sync.last_sync_at")
        .await
        .and_then(|value| value.parse::<i64>().ok());

    Ok(SyncStatusView {
        device_id: device.as_str().to_owned(),
        folder: state_value(pool, "sync.last_folder").await,
        last_sync_at_ms,
        games: counts.get("games"),
        routes: counts.get("routes"),
        chapters: counts.get("chapters"),
        sessions: counts.get("sessions"),
        endings: counts.get("endings"),
    })
}

/// Export a portable snapshot; the user chooses the destination in the system picker.
#[tauri::command]
pub async fn sync_export_file(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> CommandResult<bool> {
    let payload = sync::collect_local(state.db().pool(), state.device())
        .await
        .map_err(|err| CommandError::new("sync_failed", err.to_string()))?;
    let filename = sync::payload_file_name(state.device().as_str());
    let contents = serde_json::to_string_pretty(&payload)
        .map_err(|err| CommandError::new("sync_failed", err.to_string()))?;
    crate::platform::storage::save_document(app, filename, contents).await
}

/// Open Android's share sheet. The destination is selected by the user.
#[tauri::command]
pub async fn sync_share_file(state: State<'_, AppState>) -> CommandResult<()> {
    let payload = sync::collect_local(state.db().pool(), state.device())
        .await
        .map_err(|err| CommandError::new("sync_failed", err.to_string()))?;
    let filename = sync::payload_file_name(state.device().as_str());
    let contents = serde_json::to_string_pretty(&payload)
        .map_err(|err| CommandError::new("sync_failed", err.to_string()))?;
    crate::platform::storage::share_document(filename, contents).await
}

/// Import a transfer without replacing the local database or uploading any data.
#[tauri::command]
pub async fn sync_import_file(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> CommandResult<Option<sync::SyncOutcome>> {
    let Some(contents) = crate::platform::storage::open_document(app).await? else {
        return Ok(None);
    };
    let payload: sync::SyncPayload = serde_json::from_str(&contents).map_err(|_| {
        CommandError::new(
            "invalid_transfer",
            "Choose a VN Asyik transfer file; backups are imported in Settings",
        )
    })?;
    let started = Timestamp::now()?;
    let outcome = sync::merge_transfer(state.db().pool(), state.device(), &payload)
        .await
        .map_err(|err| CommandError::new("sync_failed", err.to_string()))?;
    let finished = Timestamp::now()?;
    record_run(state.db().pool(), &outcome, started, finished).await?;
    Ok(Some(outcome))
}

/// Run a sync against `folder`, and remember it.
///
/// # Errors
/// [`CommandError`] with code `bad_folder` when the folder is not usable, or the
/// underlying failure. Peer files that cannot be read are reported in the outcome
/// instead of failing the run.
#[tauri::command]
pub async fn sync_run(
    state: State<'_, AppState>,
    folder: String,
) -> CommandResult<sync::SyncOutcome> {
    let pool = state.db().pool();
    let device = queries::ensure_device_id(pool).await?;

    // Check the path before storing it, so a typo does not become the remembered
    // folder.
    let path = std::path::Path::new(&folder);
    if !path.is_dir() {
        return Err(CommandError::new(
            "bad_folder",
            format!("{folder} is not a folder that can be read"),
        ));
    }

    let started = Timestamp::now().map_err(|err| CommandError::new("clock", err.to_string()))?;
    let outcome = sync::sync_folder(pool, &device, path)
        .await
        .map_err(|err| match err {
            sync::SyncError::BadFolder(message) => CommandError::new("bad_folder", message),
            other => CommandError::new("sync_failed", other.to_string()),
        })?;

    let finished = Timestamp::now().map_err(|err| CommandError::new("clock", err.to_string()))?;

    for (key, value) in [
        ("sync.last_folder", folder.clone()),
        ("sync.last_sync_at", finished.as_millis().to_string()),
    ] {
        sqlx::query("INSERT INTO sync_state (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value")
            .bind(key)
            .bind(&value)
            .execute(pool)
            .await
            .map_err(|err| CommandError::new("db", err.to_string()))?;
    }

    record_run(pool, &outcome, started, finished).await?;
    Ok(outcome)
}

pub(super) async fn record_run(
    pool: &sqlx::SqlitePool,
    outcome: &sync::SyncOutcome,
    started: Timestamp,
    finished: Timestamp,
) -> CommandResult<()> {
    sqlx::query("INSERT INTO sync_state (key, value) VALUES ('sync.last_sync_at', ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value")
        .bind(finished.as_millis().to_string()).execute(pool).await
        .map_err(|err| CommandError::new("db", err.to_string()))?;
    // Log the run so the UI can show history, and the conflicts so an overwrite is
    // explainable rather than mysterious.
    let run_id = vn_core::ids::EntityId::new().to_string();
    let conflict_count = i64::try_from(outcome.conflicts.len()).unwrap_or(i64::MAX);
    sqlx::query(
        "INSERT INTO sync_runs (id, peer_device, pushed, pulled, conflicts, started_at, finished_at, outcome) \
         VALUES (?, NULL, ?, ?, ?, ?, ?, 'ok')",
    )
    .bind(&run_id)
    .bind(i64::try_from(outcome.pushed).unwrap_or(i64::MAX))
    .bind(i64::try_from(outcome.pulled).unwrap_or(i64::MAX))
    .bind(conflict_count)
    .bind(started.as_millis())
    .bind(finished.as_millis())
    .execute(pool)
    .await
    .map_err(|err| CommandError::new("db", err.to_string()))?;

    for conflict in &outcome.conflicts {
        sqlx::query(
            "INSERT INTO sync_conflicts (id, run_id, table_name, record_id, kept, reason, \
                                         discarded_updated_at, discarded_device, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, NULL, ?)",
        )
        .bind(vn_core::ids::EntityId::new().to_string())
        .bind(&run_id)
        .bind(&conflict.table)
        .bind(&conflict.record_id)
        .bind(&conflict.kept)
        .bind(&conflict.reason)
        .bind(conflict.discarded_updated_at_ms)
        .bind(finished.as_millis())
        .execute(pool)
        .await
        .map_err(|err| CommandError::new("db", err.to_string()))?;
    }

    Ok(())
}

/// The most recent conflicts, newest first.
#[tauri::command]
pub async fn sync_conflicts(
    state: State<'_, AppState>,
    limit: i64,
) -> CommandResult<Vec<ConflictView>> {
    use sqlx::Row as _;
    let rows = sqlx::query(
        "SELECT table_name, record_id, kept, reason, discarded_updated_at, created_at \
         FROM sync_conflicts ORDER BY created_at DESC LIMIT ?",
    )
    .bind(limit.clamp(1, 500))
    .fetch_all(state.db().pool())
    .await
    .map_err(|err| CommandError::new("db", err.to_string()))?;

    Ok(rows
        .into_iter()
        .map(|row| ConflictView {
            table: row.get("table_name"),
            record_id: row.get("record_id"),
            kept: row.get("kept"),
            reason: row.get("reason"),
            discarded_updated_at_ms: row.get("discarded_updated_at"),
            created_at_ms: row.get("created_at"),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use crate::db::Db;

    #[tokio::test]
    async fn the_conflict_log_survives_and_is_ordered_newest_first() {
        // The log is the only record that an overwrite happened, so it has to be
        // readable after the fact.
        let db = Db::open_memory().await.expect("db");
        let pool = db.pool();

        sqlx::query(
            "INSERT INTO sync_runs (id, peer_device, pushed, pulled, conflicts, started_at, finished_at, outcome) \
             VALUES ('run-1', NULL, 1, 2, 2, 0, 100, 'ok')",
        )
        .execute(pool)
        .await
        .expect("run");

        for (id, created) in [("c1", 100_i64), ("c2", 200)] {
            sqlx::query(
                "INSERT INTO sync_conflicts (id, run_id, table_name, record_id, kept, reason, \
                                             discarded_updated_at, discarded_device, created_at) \
                 VALUES (?, 'run-1', 'games', ?, 'remote', 'newer', 5, NULL, ?)",
            )
            .bind(id)
            .bind(format!("rec-{id}"))
            .bind(created)
            .execute(pool)
            .await
            .expect("conflict");
        }

        let rows: Vec<(String, i64)> = sqlx::query_as(
            "SELECT record_id, created_at FROM sync_conflicts ORDER BY created_at DESC",
        )
        .fetch_all(pool)
        .await
        .expect("read");
        assert_eq!(rows[0].0, "rec-c2", "newest first");
    }

    #[tokio::test]
    async fn deleting_a_run_removes_its_conflicts() {
        // The log must not accumulate orphans once a run is pruned.
        let db = Db::open_memory().await.expect("db");
        let pool = db.pool();
        sqlx::query(
            "INSERT INTO sync_runs (id, peer_device, pushed, pulled, conflicts, started_at, finished_at, outcome) \
             VALUES ('run-1', NULL, 0, 0, 1, 0, 1, 'ok')",
        )
        .execute(pool)
        .await
        .expect("run");
        sqlx::query(
            "INSERT INTO sync_conflicts (id, run_id, table_name, record_id, kept, reason, \
                                         discarded_updated_at, discarded_device, created_at) \
             VALUES ('c1', 'run-1', 'games', 'r', 'local', 'newer', 1, NULL, 1)",
        )
        .execute(pool)
        .await
        .expect("conflict");

        sqlx::query("DELETE FROM sync_runs WHERE id = 'run-1'")
            .execute(pool)
            .await
            .expect("delete");

        let remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sync_conflicts")
            .fetch_one(pool)
            .await
            .expect("count");
        assert_eq!(remaining, 0, "cascade should have removed the conflicts");
    }
}
