//! One library file for backup and transfer, with merge as the import default.

use std::collections::HashSet;
use std::sync::Mutex;

use serde::de::{IgnoredAny, MapAccess, Visitor};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tauri::State;
use vn_core::clock::Timestamp;
use vn_core::ids::{DeviceId, EntityId};
use vn_core::migration::{BACKUP_FORMAT, SCHEMA_VERSION};
use vn_core::model::{Chapter, Ending, Game, NsfwMode, PlaySession, Route, ThemeMode};

use super::backup::{BackupFile, ImportSummary};
use super::{CommandError, CommandResult, backup};
use crate::db::DbError;
use crate::db::{models::PrefsSnapshot, queries};
use crate::state::AppState;
use crate::sync::{self, SyncOutcome, SyncPayload};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DataImportKind {
    Library,
    Backup,
    Transfer,
}

/// Counts describe the selected file, including deletion tombstones.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DataImportPreview {
    pub id: String,
    pub kind: DataImportKind,
    pub created_at_ms: i64,
    pub games: usize,
    pub routes: usize,
    pub chapters: usize,
    pub sessions: usize,
    pub endings: usize,
    pub same_device: bool,
    pub can_restore: bool,
}

/// A portable library snapshot. Credentials and device mappings are excluded.
#[derive(Debug, Serialize, Deserialize)]
pub struct LibraryFile {
    pub format: String,
    pub version: u32,
    pub payload: SyncPayload,
    pub prefs: PrefsSnapshot,
}

pub const LIBRARY_FORMAT: &str = "vn-asyik-library";

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DataImportMode {
    #[default]
    Merge,
    Restore,
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum DataImportResult {
    Backup { summary: ImportSummary },
    Transfer { outcome: SyncOutcome },
}

#[derive(Debug)]
enum ImportPayload {
    Library(LibraryFile),
    Backup(BackupFile),
    Transfer(SyncPayload),
}

#[derive(Debug)]
struct ImportSlot {
    id: String,
    payload: Option<ImportPayload>,
}

/// Ephemeral state only: file contents never round-trip through frontend IPC.
#[derive(Debug, Default)]
pub struct ImportDraftStore(Mutex<Option<ImportSlot>>);

impl ImportDraftStore {
    /// Supersede the old preview before opening another native picker.
    pub fn begin_open(&self) -> CommandResult<String> {
        let id = EntityId::new().to_string();
        *self.0.lock().map_err(|_| draft_unavailable())? = Some(ImportSlot {
            id: id.clone(),
            payload: None,
        });
        Ok(id)
    }

    /// Cancelling an old dialog cannot discard a newer preview.
    pub fn cancel(&self, id: &str) -> CommandResult<()> {
        let mut slot = self.0.lock().map_err(|_| draft_unavailable())?;
        if slot.as_ref().is_some_and(|draft| draft.id == id) {
            *slot = None;
        }
        Ok(())
    }

    fn publish(&self, id: &str, payload: ImportPayload) -> CommandResult<()> {
        let mut slot = self.0.lock().map_err(|_| draft_unavailable())?;
        let draft = slot
            .as_mut()
            .filter(|draft| draft.id == id)
            .ok_or_else(stale_import)?;
        draft.payload = Some(payload);
        Ok(())
    }

    fn take(&self, id: &str) -> CommandResult<ImportPayload> {
        let mut slot = self.0.lock().map_err(|_| draft_unavailable())?;
        if !slot
            .as_ref()
            .is_some_and(|draft| draft.id == id && draft.payload.is_some())
        {
            return Err(stale_import());
        }
        slot.take()
            .and_then(|draft| draft.payload)
            .ok_or_else(stale_import)
    }
}

fn stale_import() -> CommandError {
    CommandError::new("stale_import", "Select the file again before importing")
}

fn draft_unavailable() -> CommandError {
    CommandError::new(
        "import_unavailable",
        "The import preview is unavailable; select the file again",
    )
}

fn invalid_file() -> CommandError {
    CommandError::new(
        "invalid_file",
        "Choose a valid VN Asyik backup or transfer JSON file",
    )
}

/// Scan keys without losing duplicate fields through `serde_json::Value`'s map.
/// `MapAccess`/`Visitor` contracts are verified from `serde_core`'s shipped source.
struct RootKeys(HashSet<String>);

impl<'de> Deserialize<'de> for RootKeys {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct KeysVisitor;
        impl<'de> Visitor<'de> for KeysVisitor {
            type Value = RootKeys;
            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("one backup or transfer object")
            }
            fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
                let mut keys = HashSet::new();
                while let Some(key) = map.next_key::<String>()? {
                    if !keys.insert(key) {
                        return Err(serde::de::Error::custom("duplicate file field"));
                    }
                    map.next_value::<IgnoredAny>()?;
                }
                Ok(RootKeys(keys))
            }
        }
        deserializer.deserialize_map(KeysVisitor)
    }
}

fn parse_file(contents: &str) -> CommandResult<ImportPayload> {
    let keys: RootKeys = serde_json::from_str(contents).map_err(|_| invalid_file())?;
    if keys.0.contains("format") || keys.0.contains("payload") {
        if [
            "header",
            "schema_version",
            "device_id",
            "written_at_ms",
            "generation",
        ]
        .iter()
        .any(|key| keys.0.contains(*key))
        {
            return Err(invalid_file());
        }
        let file: LibraryFile = serde_json::from_str(contents).map_err(|_| invalid_file())?;
        if file.format != LIBRARY_FORMAT || file.version != 1 {
            return Err(CommandError::new(
                "unsupported_file",
                "This library format is not supported",
            ));
        }
        validate_transfer(&file.payload)?;
        validate_prefs(&file.prefs)?;
        return Ok(ImportPayload::Library(file));
    }
    let backup_marker = keys.0.contains("header") || keys.0.contains("prefs");
    let transfer_marker = ["schema_version", "device_id", "written_at_ms", "generation"]
        .iter()
        .any(|key| keys.0.contains(*key));
    // The payload's existing Serde shape handles required/default fields and
    // tolerates harmless metadata. A file carrying both formats is ambiguous.
    if backup_marker && !transfer_marker {
        let payload: BackupFile = serde_json::from_str(contents).map_err(|_| invalid_file())?;
        if payload.header.format != BACKUP_FORMAT {
            return Err(CommandError::new(
                "unsupported_file",
                "This backup format is not supported",
            ));
        }
        payload.header.validate().map_err(|_| {
            CommandError::new("unsupported_file", "This backup schema is not supported")
        })?;
        validate_prefs(&payload.prefs)?;
        validate_rows(
            &payload.games,
            &payload.routes,
            &payload.chapters,
            &payload.sessions,
            &payload.endings,
        )?;
        Ok(ImportPayload::Backup(payload))
    } else if transfer_marker && !backup_marker {
        let payload: SyncPayload = serde_json::from_str(contents).map_err(|_| invalid_file())?;
        validate_transfer(&payload)?;
        Ok(ImportPayload::Transfer(payload))
    } else {
        Err(invalid_file())
    }
}

fn validate_prefs(prefs: &PrefsSnapshot) -> CommandResult<()> {
    ThemeMode::parse(&prefs.theme).map_err(|_| invalid_file())?;
    NsfwMode::parse(&prefs.nsfw).map_err(|_| invalid_file())?;
    if !["en", "id"].contains(&prefs.language.as_str()) {
        return Err(invalid_file());
    }
    Ok(())
}

fn validate_transfer(payload: &SyncPayload) -> CommandResult<()> {
    if payload.schema_version == 0 || payload.schema_version > SCHEMA_VERSION {
        return Err(CommandError::new(
            "unsupported_file",
            "This library schema is not supported",
        ));
    }
    DeviceId::parse(&payload.device_id).map_err(|_| invalid_file())?;
    validate_rows(
        &payload.games,
        &payload.routes,
        &payload.chapters,
        &payload.sessions,
        &payload.endings,
    )
}

fn unique_ids(ids: impl Iterator<Item = EntityId>) -> CommandResult<()> {
    let mut seen = HashSet::new();
    if ids.into_iter().any(|id| !seen.insert(id)) {
        return Err(invalid_file());
    }
    Ok(())
}

fn validate_rows(
    games: &[Game],
    routes: &[Route],
    chapters: &[Chapter],
    sessions: &[PlaySession],
    endings: &[Ending],
) -> CommandResult<()> {
    unique_ids(games.iter().map(|row| row.meta.id))?;
    unique_ids(routes.iter().map(|row| row.meta.id))?;
    unique_ids(chapters.iter().map(|row| row.meta.id))?;
    unique_ids(sessions.iter().map(|row| row.meta.id))?;
    unique_ids(endings.iter().map(|row| row.meta.id))?;
    for game in games {
        // Mirror existing storage constraints; do not invent stricter limits
        // for historical exports that the legacy importer can already read.
        if game.title.trim().is_empty()
            || game
                .rating
                .is_some_and(|rating| !(10..=100).contains(&rating))
        {
            return Err(invalid_file());
        }
    }
    for session in sessions {
        if session.ended_at < session.started_at || session.counted_ms < 0 || session.afk_ms < 0 {
            return Err(invalid_file());
        }
    }
    Ok(())
}

/// Validate relationships against the file and existing rows without writing.
async fn validate_references(
    pool: &SqlitePool,
    games: &[Game],
    routes: &[Route],
    chapters: &[Chapter],
    sessions: &[PlaySession],
    endings: &[Ending],
) -> CommandResult<()> {
    let local_games: Vec<(String,)> = sqlx::query_as("SELECT id FROM games")
        .fetch_all(pool)
        .await
        .map_err(DbError::from)?;
    let mut game_ids: HashSet<EntityId> = local_games
        .iter()
        .map(|(id,)| EntityId::parse(id))
        .collect::<Result<_, _>>()?;
    game_ids.extend(games.iter().map(|row| row.meta.id));
    let local_chapters: Vec<(String,)> = sqlx::query_as("SELECT id FROM chapters")
        .fetch_all(pool)
        .await
        .map_err(DbError::from)?;
    let mut chapter_ids: HashSet<EntityId> = local_chapters
        .iter()
        .map(|(id,)| EntityId::parse(id))
        .collect::<Result<_, _>>()?;
    chapter_ids.extend(chapters.iter().map(|row| row.meta.id));
    let local_routes: Vec<(String,)> = sqlx::query_as("SELECT id FROM routes")
        .fetch_all(pool)
        .await
        .map_err(DbError::from)?;
    let mut route_ids: HashSet<EntityId> = local_routes
        .iter()
        .map(|(id,)| EntityId::parse(id))
        .collect::<Result<_, _>>()?;
    route_ids.extend(routes.iter().map(|row| row.meta.id));
    if chapters.iter().any(|row| !game_ids.contains(&row.game_id))
        || endings.iter().any(|row| !game_ids.contains(&row.game_id))
        || routes.iter().any(|row| !game_ids.contains(&row.game_id))
        || chapters
            .iter()
            .any(|row| row.route_id.is_some_and(|id| !route_ids.contains(&id)))
        || endings
            .iter()
            .any(|row| row.route_id.is_some_and(|id| !route_ids.contains(&id)))
    {
        return Err(invalid_file());
    }
    for session in sessions {
        if !game_ids.contains(&session.game_id)
            || session
                .chapter_id
                .is_some_and(|id| !chapter_ids.contains(&id))
        {
            return Err(invalid_file());
        }
    }
    Ok(())
}

/// Testable seam used after the native picker returns. It only reads the DB.
pub async fn prepare_import(
    store: &ImportDraftStore,
    id: &str,
    pool: &SqlitePool,
    device: &DeviceId,
    contents: &str,
) -> CommandResult<DataImportPreview> {
    let payload = parse_file(contents)?;
    let (kind, at, same_device, games, routes, chapters, sessions, endings) = match &payload {
        ImportPayload::Library(file) => (
            DataImportKind::Library,
            file.payload.written_at_ms,
            file.payload.device_id == device.as_str(),
            &file.payload.games,
            &file.payload.routes,
            &file.payload.chapters,
            &file.payload.sessions,
            &file.payload.endings,
        ),
        ImportPayload::Backup(file) => (
            DataImportKind::Backup,
            file.header.created_at_ms,
            false,
            &file.games,
            &file.routes,
            &file.chapters,
            &file.sessions,
            &file.endings,
        ),
        ImportPayload::Transfer(file) => (
            DataImportKind::Transfer,
            file.written_at_ms,
            file.device_id == device.as_str(),
            &file.games,
            &file.routes,
            &file.chapters,
            &file.sessions,
            &file.endings,
        ),
    };
    validate_references(pool, games, routes, chapters, sessions, endings).await?;
    let preview = DataImportPreview {
        id: id.to_owned(),
        kind,
        created_at_ms: at,
        games: games.len(),
        routes: routes.len(),
        chapters: chapters.len(),
        sessions: sessions.len(),
        endings: endings.len(),
        same_device,
        can_restore: !matches!(payload, ImportPayload::Transfer(_)),
    };
    store.publish(id, payload)?;
    Ok(preview)
}

/// Consume the preview before writing, including when an import subsequently fails.
pub async fn apply_import(
    store: &ImportDraftStore,
    pool: &SqlitePool,
    device: &DeviceId,
    id: &str,
) -> CommandResult<DataImportResult> {
    apply_import_with_mode(store, pool, device, id, DataImportMode::Merge).await
}

/// Restore is explicit; all formats merge through the same rules by default.
pub async fn apply_import_with_mode(
    store: &ImportDraftStore,
    pool: &SqlitePool,
    device: &DeviceId,
    id: &str,
    mode: DataImportMode,
) -> CommandResult<DataImportResult> {
    let payload = store.take(id)?;
    let (snapshot, prefs) = match payload {
        ImportPayload::Library(file) => (file.payload, Some(file.prefs)),
        ImportPayload::Transfer(file) => (file, None),
        ImportPayload::Backup(file) => (
            SyncPayload {
                schema_version: file.header.schema_version,
                device_id: device.as_str().to_owned(),
                written_at_ms: file.header.created_at_ms,
                generation: 0,
                games: file.games,
                routes: file.routes,
                chapters: file.chapters,
                sessions: file.sessions,
                endings: file.endings,
            },
            Some(file.prefs),
        ),
    };
    if matches!(mode, DataImportMode::Restore) {
        let prefs = prefs.ok_or_else(|| {
            CommandError::new(
                "restore_unavailable",
                "This legacy transfer has no appearance preferences; merge it instead",
            )
        })?;
        let file = BackupFile {
            header: vn_core::migration::BackupHeader {
                format: BACKUP_FORMAT,
                schema_version: snapshot.schema_version,
                created_at_ms: snapshot.written_at_ms,
                app_version: env!("CARGO_PKG_VERSION").to_owned(),
            },
            games: snapshot.games,
            routes: snapshot.routes,
            chapters: snapshot.chapters,
            sessions: snapshot.sessions,
            endings: snapshot.endings,
            prefs,
        };
        return backup::import_payload(pool, &file)
            .await
            .map(|summary| DataImportResult::Backup { summary });
    }
    let started = Timestamp::now()?;
    let outcome = sync::merge_library_snapshot(pool, &snapshot)
        .await
        .map_err(|err| CommandError::new("sync_failed", err.to_string()))?;
    super::sync::record_run(pool, &outcome, started, Timestamp::now()?).await?;
    Ok(DataImportResult::Transfer { outcome })
}

#[tauri::command]
pub async fn data_open_file(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> CommandResult<Option<DataImportPreview>> {
    let id = state.import_drafts().begin_open()?;
    let contents = match crate::platform::storage::open_document(app).await {
        Ok(Some(contents)) => contents,
        Ok(None) => {
            state.import_drafts().cancel(&id)?;
            return Ok(None);
        }
        Err(error) => {
            state.import_drafts().cancel(&id)?;
            return Err(error);
        }
    };
    let result = prepare_import(
        state.import_drafts(),
        &id,
        state.db().pool(),
        state.device(),
        &contents,
    )
    .await;
    if result.is_err() {
        state.import_drafts().cancel(&id)?;
    }
    result.map(Some)
}

#[tauri::command]
pub async fn data_apply_import(
    state: State<'_, AppState>,
    id: String,
    mode: Option<DataImportMode>,
) -> CommandResult<DataImportResult> {
    apply_import_with_mode(
        state.import_drafts(),
        state.db().pool(),
        state.device(),
        &id,
        mode.unwrap_or_default(),
    )
    .await
}

/// Collect the same complete snapshot for Save and Share.
pub async fn collect_library_file(
    pool: &SqlitePool,
    device: &DeviceId,
) -> CommandResult<LibraryFile> {
    Ok(LibraryFile {
        format: LIBRARY_FORMAT.to_owned(),
        version: 1,
        payload: sync::collect_local(pool, device)
            .await
            .map_err(|err| CommandError::new("sync_failed", err.to_string()))?,
        prefs: queries::load_prefs(pool).await?,
    })
}

#[tauri::command]
pub async fn data_export_file(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> CommandResult<bool> {
    let file = collect_library_file(state.db().pool(), state.device()).await?;
    let filename = format!("vn-asyik-library-{}.json", file.payload.written_at_ms);
    let contents = serde_json::to_string_pretty(&file)
        .map_err(|err| CommandError::new("document", err.to_string()))?;
    crate::platform::storage::save_document(app, filename, contents).await
}

#[tauri::command]
pub async fn data_share_file(state: State<'_, AppState>) -> CommandResult<()> {
    let file = collect_library_file(state.db().pool(), state.device()).await?;
    let filename = format!("vn-asyik-library-{}.json", file.payload.written_at_ms);
    let contents = serde_json::to_string_pretty(&file)
        .map_err(|err| CommandError::new("document", err.to_string()))?;
    crate::platform::storage::share_document(filename, contents).await
}

#[tauri::command]
pub async fn data_cancel_import(state: State<'_, AppState>, id: String) -> CommandResult<()> {
    state.import_drafts().cancel(&id)
}

#[tauri::command]
pub async fn share_backup_file(state: State<'_, AppState>) -> CommandResult<()> {
    let payload = backup::export_backup(state).await?;
    let filename = format!("vn-asyik-backup-{}.json", payload.header.created_at_ms);
    let contents = serde_json::to_string_pretty(&payload)
        .map_err(|err| CommandError::new("backup", err.to_string()))?;
    crate::platform::storage::share_document(filename, contents).await
}
