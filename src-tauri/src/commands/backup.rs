//! `JSON` backup export and import with schema versioning.

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tauri::State;
use vn_core::clock::Timestamp;
use vn_core::migration::{BACKUP_FORMAT, BackupHeader, MAX_READABLE_SCHEMA, MigrationError};
use vn_core::model::{NsfwMode, ThemeMode};

use super::{CommandError, CommandResult};
use crate::db::{DbError, models::PrefsSnapshot, queries};
use crate::state::AppState;

/// The full contents of a backup file.
#[derive(Debug, Serialize, Deserialize)]
pub struct BackupFile {
    /// Format and schema declaration. Validated before any rows are used.
    pub header: BackupHeader,
    /// Every live game.
    pub games: Vec<vn_core::model::Game>,
    /// Every live route.
    ///
    /// Defaulted so a backup written before routes existed still imports; it
    /// simply carries no routes, and the chapters/endings that mention one keep
    /// `route_id = NULL` rather than failing validation.
    #[serde(default)]
    pub routes: Vec<vn_core::model::Route>,
    /// Every live chapter.
    pub chapters: Vec<vn_core::model::Chapter>,
    /// Every live session.
    pub sessions: Vec<vn_core::model::PlaySession>,
    /// Every live ending.
    pub endings: Vec<vn_core::model::Ending>,
    /// Preferences at export time.
    pub prefs: PrefsSnapshot,
}

/// Build the backup payload without touching the filesystem.
///
/// The frontend owns file placement via a save dialog, so this stays a pure
/// data command that is trivially testable.
#[tauri::command]
pub async fn export_backup(state: State<'_, AppState>) -> CommandResult<BackupFile> {
    let pool = state.db().pool();
    let games = queries::list_games(pool).await?;

    let mut routes = Vec::new();
    let mut chapters = Vec::new();
    let mut sessions = Vec::new();
    let mut endings = Vec::new();
    for game in &games {
        routes.extend(queries::list_routes(pool, game.meta.id).await?);
        chapters.extend(queries::list_chapters(pool, game.meta.id).await?);
        sessions.extend(queries::list_sessions(pool, game.meta.id).await?);
        endings.extend(queries::list_endings(pool, game.meta.id).await?);
    }

    let created_at = Timestamp::now()?;
    Ok(BackupFile {
        header: BackupHeader {
            format: BACKUP_FORMAT,
            schema_version: vn_core::migration::SCHEMA_VERSION,
            created_at_ms: created_at.as_millis(),
            app_version: env!("CARGO_PKG_VERSION").to_owned(),
        },
        games,
        routes,
        chapters,
        sessions,
        endings,
        prefs: queries::load_prefs(pool).await?,
    })
}

/// Result of an import.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct ImportSummary {
    /// Games written.
    pub games: usize,
    /// Routes written.
    pub routes: usize,
    /// Chapters written.
    pub chapters: usize,
    /// Sessions written (new ids only).
    pub sessions: usize,
    /// Endings written.
    pub endings: usize,
    /// Sessions skipped because their id already existed (append-only rule).
    pub sessions_skipped: usize,
}

/// Save a backup through the platform document picker, including Android content URIs.
#[tauri::command]
pub async fn export_backup_file(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> CommandResult<bool> {
    let payload = export_backup(state).await?;
    let filename = format!("vn-asyik-backup-{}.json", payload.header.created_at_ms);
    let contents = serde_json::to_string_pretty(&payload)
        .map_err(|err| CommandError::new("backup", err.to_string()))?;
    crate::platform::storage::save_document(app, filename, contents).await
}

/// Read a user-selected backup; cancellation leaves the library untouched.
#[tauri::command]
pub async fn import_backup_file(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> CommandResult<Option<ImportSummary>> {
    let Some(contents) = crate::platform::storage::open_document(app).await? else {
        return Ok(None);
    };
    let payload = serde_json::from_str(&contents)
        .map_err(|_| CommandError::new("invalid_backup", "Choose a VN Asyik backup JSON file"))?;
    import_backup(state, payload).await.map(Some)
}

/// Import a backup payload.
///
/// Existing rows are upserted by id; sessions are append-only, so duplicates are
/// skipped rather than overwritten. A newer schema is refused outright.
#[tauri::command]
pub async fn import_backup(
    state: State<'_, AppState>,
    payload: BackupFile,
) -> CommandResult<ImportSummary> {
    import_payload(state.db().pool(), &payload).await
}

/// Restore one backup atomically, including its preferences.
///
/// Validation or storage failure rolls back every row, including earlier
/// overwrites. Runtime tracking state, credentials and device identity stay local.
pub async fn import_payload(
    pool: &SqlitePool,
    payload: &BackupFile,
) -> CommandResult<ImportSummary> {
    payload.header.validate().map_err(|err| match err {
        MigrationError::BackupTooNew { found, max } => CommandError::new(
            "backup_too_new",
            format!("backup schema {found} is newer than this app supports ({max})"),
        ),
        other => CommandError::new("invalid_backup", other.to_string()),
    })?;

    if payload.header.schema_version > MAX_READABLE_SCHEMA {
        return Err(CommandError::new(
            "backup_too_new",
            "backup was written by a newer version",
        ));
    }

    let theme = ThemeMode::parse(&payload.prefs.theme)
        .map_err(|_| CommandError::new("invalid_theme", "unknown theme"))?;
    let nsfw = NsfwMode::parse(&payload.prefs.nsfw)
        .map_err(|_| CommandError::new("invalid_nsfw", "unknown nsfw mode"))?;
    if !["en", "id"].contains(&payload.prefs.language.as_str()) {
        return Err(CommandError::new(
            "invalid_language",
            "unsupported language",
        ));
    }
    if payload.sessions.iter().any(|session| session.afk_ms < 0) {
        return Err(CommandError::new(
            "invalid_backup",
            "session idle time cannot be negative",
        ));
    }

    let mut tx = pool.begin().await.map_err(DbError::from)?;
    let mut summary = ImportSummary {
        games: 0,
        routes: 0,
        chapters: 0,
        sessions: 0,
        endings: 0,
        sessions_skipped: 0,
    };

    for game in &payload.games {
        queries::upsert_game(&mut *tx, game).await?;
        summary.games += 1;
    }
    // Routes before the children that reference them, or a chapter could name a
    // route row that does not exist yet.
    for route in &payload.routes {
        queries::upsert_route(&mut *tx, route).await?;
        summary.routes += 1;
    }
    for chapter in &payload.chapters {
        queries::upsert_chapter(&mut *tx, chapter).await?;
        summary.chapters += 1;
    }
    for ending in &payload.endings {
        queries::upsert_ending(&mut *tx, ending).await?;
        summary.endings += 1;
    }
    for session in &payload.sessions {
        // A duplicate id is the ordinary case on a second import (sessions are
        // append-only and merge by uuid), so it is `Ok(false)`, not an error. It used
        // to arrive as `Corrupt`, which made a genuinely damaged row count as a skip.
        match queries::insert_session(&mut *tx, session).await {
            Ok(true) => summary.sessions += 1,
            Ok(false) => summary.sessions_skipped += 1,
            Err(other) => return Err(other.into()),
        }
    }

    queries::set_pref(&mut *tx, "theme", theme.as_str()).await?;
    queries::set_pref(&mut *tx, "nsfw", nsfw.as_str()).await?;
    queries::set_pref(&mut *tx, "language", &payload.prefs.language).await?;
    queries::set_pref(
        &mut *tx,
        "reduce_effects",
        if payload.prefs.reduce_effects {
            "true"
        } else {
            "false"
        },
    )
    .await?;
    tx.commit().await.map_err(DbError::from)?;

    Ok(summary)
}
