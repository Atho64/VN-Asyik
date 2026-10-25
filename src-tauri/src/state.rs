//! Shared application state, constructed once at startup.

use std::path::PathBuf;

use tauri::{AppHandle, Manager};
use vn_core::ids::DeviceId;

use crate::db::{Db, DbError, queries};
use crate::tracking::TrackingService;
use crate::vndb::VndbClient;

/// State handed to every command via `tauri::State`.
pub struct AppState {
    db: Db,
    device: DeviceId,
    data_dir: PathBuf,
    vndb: VndbClient,
    tracker: TrackingService,
    hook: std::sync::Arc<crate::tracking::hook::HookHost>,
    luna: std::sync::Arc<crate::tracking::luna::LunaMonitor>,
    import_drafts: crate::commands::data::ImportDraftStore,
    chat: crate::ai::conversation::ChatRuntime,
    presence: std::sync::Arc<crate::presence::PresenceService>,
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState")
            .field("device", &self.device)
            .field("data_dir", &self.data_dir)
            .finish_non_exhaustive()
    }
}

impl AppState {
    /// Open the database in the platform app-data directory and load identity.
    ///
    /// # Errors
    /// Fails when the data directory cannot be resolved or the database cannot
    /// be opened/migrated. Startup must not continue with a broken database.
    pub async fn boot(app: &AppHandle) -> Result<Self, BootError> {
        let data_dir = app.path().app_data_dir()?;
        let db_path = data_dir.join("vn-tracker.sqlite");
        let db = Db::open(&db_path).await?;
        let device = queries::ensure_device_id(db.pool()).await?;
        crate::ai::conversation::store::recover(db.pool())
            .await
            .map_err(|code| BootError::Db(DbError::Corrupt(code)))?;
        // The client is created once so its rate limiter is shared by every
        // command rather than reset per request.
        let vndb = VndbClient::new().map_err(BootError::Vndb)?;

        // Recover anything a crash left behind before the loop starts, so the
        // recovered session exists before new time is added to it.
        let now = vn_core::clock::Timestamp::now()
            .map_err(|err| BootError::Db(DbError::Corrupt(err.to_string())))?;
        let tracker = TrackingService::new(now);
        let afk = queries::load_afk_config(db.pool()).await?;
        tracker.set_afk_config(afk).await;
        let recovered = crate::tracking::recover(db.pool(), &device, now).await?;
        if recovered > 0 {
            queries::record_recovery(db.pool(), recovered, now).await?;
        }

        let presence_settings = queries::load_presence_settings(db.pool())
            .await
            .unwrap_or_default();
        let presence =
            std::sync::Arc::new(crate::presence::PresenceService::new(presence_settings));

        let hook = std::sync::Arc::new(crate::tracking::hook::HookHost::default());
        let luna = std::sync::Arc::new(crate::tracking::luna::LunaMonitor::new(
            hook.clone(),
            crate::tracking::luna::LunaMonitor::load(db.pool()).await,
        ));
        // One supervisor for the app's lifetime: connects to LunaTranslator's
        // websocket when enabled, idles when not, and picks up settings
        // changes without a restart.
        let luna_task = std::sync::Arc::clone(&luna);
        tauri::async_runtime::spawn(async move {
            luna_task.supervise().await;
        });

        Ok(Self {
            db,
            device,
            data_dir,
            vndb,
            tracker,
            hook,
            luna,
            import_drafts: crate::commands::data::ImportDraftStore::default(),
            chat: crate::ai::conversation::ChatRuntime::default(),
            presence,
        })
    }

    /// Borrow the database handle.
    #[must_use]
    pub fn db(&self) -> &Db {
        &self.db
    }

    pub fn chat(&self) -> &crate::ai::conversation::ChatRuntime {
        &self.chat
    }

    #[must_use]
    pub fn presence(&self) -> &std::sync::Arc<crate::presence::PresenceService> {
        &self.presence
    }

    /// This installation's device id.
    #[must_use]
    pub fn device(&self) -> &DeviceId {
        &self.device
    }

    /// Directory backups and cached art are written here.
    #[must_use]
    pub fn data_dir(&self) -> &PathBuf {
        &self.data_dir
    }

    /// Playtime tracking service and its probe.
    #[must_use]
    pub fn tracker(&self) -> &TrackingService {
        &self.tracker
    }

    /// Text hook host: captured lines and connection state.
    #[must_use]
    pub fn hook(&self) -> &std::sync::Arc<crate::tracking::hook::HookHost> {
        &self.hook
    }

    /// `LunaTranslator` network-service monitor: settings, connection state and
    /// the text it feeds into the hook ring.
    #[must_use]
    pub fn luna(&self) -> &std::sync::Arc<crate::tracking::luna::LunaMonitor> {
        &self.luna
    }

    /// Shared VNDB client, with its rate limiter.
    #[must_use]
    pub fn vndb(&self) -> &VndbClient {
        &self.vndb
    }

    /// Ephemeral document awaiting an explicit import confirmation.
    pub fn import_drafts(&self) -> &crate::commands::data::ImportDraftStore {
        &self.import_drafts
    }
}

/// Reasons boot can fail.
#[derive(Debug, thiserror::Error)]
pub enum BootError {
    /// Tauri could not resolve a platform path.
    #[error("could not resolve application paths: {0}")]
    Path(#[from] tauri::Error),
    /// The database layer failed.
    #[error(transparent)]
    Db(#[from] DbError),
    /// The VNDB HTTP client could not be built.
    #[error("network client could not start: {0}")]
    Vndb(crate::vndb::VndbError),
}
