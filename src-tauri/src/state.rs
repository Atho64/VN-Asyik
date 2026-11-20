//! Shared application state, constructed once at startup.

use std::path::PathBuf;

use tauri::{AppHandle, Manager};
use vn_core::ids::DeviceId;

use crate::db::{Db, DbError, queries};
use crate::tracking::TrackingService;
use crate::vndb::VndbClient;

pub struct AppState {
    db: Db,
    device: DeviceId,
    data_dir: PathBuf,
    vndb: VndbClient,
    tracker: TrackingService,
    import_drafts: crate::commands::data::ImportDraftStore,
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
        // How time is measured is the user's choice, and it is read here so the
        // first poll already respects it rather than starting in the default.
        let mode = crate::tracking::mode::load(db.pool()).await;
        tracker.set_mode(mode).await;
        let auto_resume = crate::tracking::auto_route::load_enabled(db.pool()).await;
        tracker.set_auto_resume_route(auto_resume).await;
        let recovered = crate::tracking::recover(db.pool(), &device, now).await?;
        if recovered > 0 {
            queries::record_recovery(db.pool(), recovered, now).await?;
        }

        // Retroactive achievement scan over the recovered history. Spawned so
        // boot never waits on it, and a failure is logged, never fatal.
        {
            let app = app.clone();
            let pool = db.pool().clone();
            let device = device.clone();
            tauri::async_runtime::spawn(async move {
                crate::commands::achievements::refresh_quietly(&app, &pool, &device).await;
            });
        }

        let presence_settings = queries::load_presence_settings(db.pool())
            .await
            .unwrap_or_default();
        let presence =
            std::sync::Arc::new(crate::presence::PresenceService::new(presence_settings));

        Ok(Self {
            db,
            device,
            data_dir,
            vndb,
            tracker,
            import_drafts: crate::commands::data::ImportDraftStore::default(),
            presence,
        })
    }

    #[must_use]
    pub fn db(&self) -> &Db {
        &self.db
    }

    #[must_use]
    pub fn presence(&self) -> &std::sync::Arc<crate::presence::PresenceService> {
        &self.presence
    }

    #[must_use]
    pub fn device(&self) -> &DeviceId {
        &self.device
    }

    /// Directory where backups and cached art are stored.
    #[must_use]
    pub fn data_dir(&self) -> &PathBuf {
        &self.data_dir
    }

    #[must_use]
    pub fn tracker(&self) -> &TrackingService {
        &self.tracker
    }

    #[must_use]
    pub fn vndb(&self) -> &VndbClient {
        &self.vndb
    }

    /// Ephemeral document awaiting an explicit import confirmation.
    pub fn import_drafts(&self) -> &crate::commands::data::ImportDraftStore {
        &self.import_drafts
    }
}

#[derive(Debug, thiserror::Error)]
pub enum BootError {
    #[error("could not resolve application paths: {0}")]
    Path(#[from] tauri::Error),
    #[error(transparent)]
    Db(#[from] DbError),
    #[error("network client could not start: {0}")]
    Vndb(crate::vndb::VndbError),
}
