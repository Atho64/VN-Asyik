pub mod gateway;
pub mod ipc;
pub mod upload;

use serde::Serialize;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::{RwLock, mpsc};
use vn_core::model::{Game, GameStatus};
use vn_core::presence::{
    CycleField, MIN_CYCLE_SECONDS, PresenceActivity, PresenceContext, PresenceDebouncer,
    PresenceDetailsMode, PresenceSettings, build_activity,
};

/// Default application / client id for VN Asyik.
pub const DEFAULT_CLIENT_ID: &str = "1563096795532230756";

/// Whether this game's cover must stay off Discord.
///
/// Fails closed: a cover is public only when **both** content ratings are known
/// and below the explicit level. A cached row can predate the rating columns, and
/// an unrated cover read as "safe" would be broadcast to Discord's CDN, which is
/// the one outcome this rule exists to prevent. Unlike the in-app blur, the user's
/// NSFW display setting is deliberately ignored: this is a public broadcast.
///
/// The threshold itself is `vn_core::model::is_explicit_at_level`, the same one the
/// library and the screenshot gallery use, so a cover cannot be concealed in one place
/// and broadcast from the other.
#[must_use]
pub fn cover_is_explicit(game: &Game) -> bool {
    match (game.cover_sexual, game.cover_violence) {
        (Some(sexual), Some(violence)) => {
            vn_core::model::is_explicit_at_level(Some(sexual))
                || vn_core::model::is_explicit_at_level(Some(violence))
        }
        _ => true,
    }
}

const fn status_label(status: GameStatus) -> &'static str {
    match status {
        GameStatus::Wishlist => "Wishlist",
        GameStatus::Playing => "Playing",
        GameStatus::Completed => "Completed",
        GameStatus::OnHold => "On Hold",
        GameStatus::Dropped => "Dropped",
    }
}

/// VNDB's coarse length bucket, phrased the way `EwE` renders it.
///
/// Shown on the rotating details line, so the text has to read as a card line:
/// `Long (30-50h)` tells the reader what they are in for, where a bare `Long` or
/// a raw `2400` does not. The ranges are VNDB's own published buckets.
///
/// `games.length_minutes` is already stored on that 1..5 scale, so the values are
/// matched as bands rather than converted to minutes first.
fn length_label(bucket: i64) -> String {
    match bucket {
        i64::MIN..=0 => "",
        1 => "Very Short (<2h)",
        2 => "Short (2-10h)",
        3 => "Medium (10-30h)",
        4 => "Long (30-50h)",
        _ => "Very Long (>50h)",
    }
    .to_string()
}

/// Local wall-clock strings for the `{date}` and `{time}` tags.
///
/// Formatting matches `EwE` (`%Y-%m-%d`, `%H:%M`), the card the templates were
/// ported from. Local time is a display concern: the stored cursor stays UTC.
/// An indeterminate system offset falls back to UTC rather than to an empty
/// tag, so a template line keeps rendering near-midnight-wrong instead of
/// disappearing.
#[must_use]
pub fn local_date_time(now: vn_core::clock::Timestamp) -> (String, String) {
    let Ok(utc) = time::OffsetDateTime::from_unix_timestamp(now.as_millis().div_euclid(1_000))
    else {
        return (String::new(), String::new());
    };
    let offset = time::UtcOffset::current_local_offset().unwrap_or(time::UtcOffset::UTC);
    format_date_time(utc.to_offset(offset))
}

/// Formats one resolved local moment as the `{date}` and `{time}` tag values.
///
/// Split from [`local_date_time`] so the format is testable without a machine
/// timezone.
fn format_date_time(local: time::OffsetDateTime) -> (String, String) {
    // `[month]` and `[day]` render zero-padded, matching `%m` / `%d`.
    let date_fmt = time::macros::format_description!("[year]-[month]-[day]");
    let time_fmt = time::macros::format_description!("[hour]:[minute]");
    (
        local.format(&date_fmt).unwrap_or_default(),
        local.format(&time_fmt).unwrap_or_default(),
    )
}

/// The game-derived half of a presence context. Callers fill in the session half
/// (elapsed time, chapter, route, start) with struct-update syntax, so the cover
/// policy above is applied in exactly one place.
///
/// When the settings configure an image host, `ensure_rpc_image` may add a
/// hosted, policy-cleared `rpc_image` on top of this half.
#[must_use]
pub fn base_context(game: &Game) -> PresenceContext {
    PresenceContext {
        vn_name: game.title.clone(),
        activity_name: game.title.clone(),
        game_key: game.meta.id.to_string(),
        window_title: game.title.clone(),
        large_text: game.title.clone(),
        vndb_id: game.vndb_id.clone(),
        vndb_url: game
            .vndb_id
            .as_ref()
            .map(|id| format!("https://vndb.org/{id}")),
        // Two different ratings, two different tags. `{my_rating}` is what the
        // user gave the game locally, one decimal like the library UI;
        // `{vndb_rating}` is VNDB's community rating and is filled from
        // `vndb_details` in `context_for_game`. They used to share one tag,
        // which made `{vndb_rating}` show the user's own number whenever they
        // had rated the game.
        my_rating: game.rating.map(|r| format!("{:.1}", f64::from(r) / 10.0)),
        vndb_status: Some(status_label(game.status).to_string()),
        vndb_length: game
            .length_minutes
            .map(length_label)
            .filter(|label| !label.is_empty()),
        cover_url: game.cover_url.clone(),
        is_cover_explicit: cover_is_explicit(game),
        // A linked VN has facts to show even before `vndb_details` is read; an
        // unlinked one never will, so the card shows the "Reading" placeholders
        // instead of a generic word that looks like a rendering failure.
        vndb_loaded: game.vndb_id.is_some(),
        ..Default::default()
    }
}

/// Build [`base_context`] and enrich it with cached `vndb_details` facts
/// (`{vndb_developer}`, `{vndb_released}`, `{vndb_year}`, and VNDB's community
/// rating for `{vndb_rating}`) when the game is linked to VNDB.
pub async fn context_for_game(pool: &sqlx::SqlitePool, game: &Game) -> PresenceContext {
    let mut ctx = base_context(game);
    let Some(ref vn_id) = game.vndb_id else {
        return ctx;
    };
    let row: Option<(String, Option<String>, Option<f64>)> = sqlx::query_as(
        "SELECT developers_json, released, rating FROM vndb_details WHERE vn_id = ?",
    )
    .bind(vn_id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();

    if let Some((developers_json, released, vndb_rating)) = row {
        let devs: Vec<crate::vndb::dto::Producer> =
            serde_json::from_str(&developers_json).unwrap_or_default();
        let names: Vec<String> = devs
            .into_iter()
            .filter_map(|d| d.name.or(d.original))
            .filter(|s| !s.trim().is_empty())
            .collect();
        if !names.is_empty() {
            ctx.vndb_developer = Some(names.join(" / "));
        }
        if let Some(ref rel) = released
            && rel.len() >= 4
            && rel.as_bytes()[..4].iter().all(u8::is_ascii_digit)
        {
            ctx.vndb_year = Some(rel[..4].to_string());
        }
        ctx.vndb_released = released;
        ctx.vndb_rating = vndb_rating.map(|r| format!("{:.2}", r / 10.0));
    }
    ctx
}

/// Which local file the card's image must come from, and whether only its blur
/// may travel.
///
/// Pure row/file logic so the concealment rule is testable without a network:
/// a user-chosen custom cover always needs hosting, an explicit (or unrated —
/// the rule fails closed) VNDB cover may only travel blurred from the cache,
/// and a safe VNDB cover already has a public URL, so it hosts nothing.
#[must_use]
pub fn rpc_image_source(
    data_dir: &std::path::Path,
    game: &Game,
) -> Option<(std::path::PathBuf, bool)> {
    let store = crate::vndb::images::ImageStore::new(data_dir);
    let custom = game
        .custom_cover_path
        .as_deref()
        .and_then(|stored| crate::vndb::images::ImageStore::resolve_custom(data_dir, stored));
    let explicit = cover_is_explicit(game);
    if let Some(path) = custom {
        return Some((path, explicit));
    }
    if explicit {
        let vn_id = game.vndb_id.as_deref()?;
        let path = store.cover_path(vn_id);
        return path.is_file().then_some((path, true));
    }
    None
}

/// The image Discord cannot fetch on its own, hosted so the card can show it.
///
/// `EwE` sends hosted URLs for exactly the covers a plain asset cannot cover:
/// explicit art (uploaded blurred) and user-chosen images. This is the opt-in
/// port: `RpcImageHost::None` — the default — uploads nothing, and explicit or
/// unrated art is blurred locally before any bytes leave the device, so what is
/// published is the same concealed image the library shows.
///
/// Returns `None` when nothing needs hosting: a safe VNDB cover already has a
/// public URL, a game with no local art has nothing to upload, and a failure
/// (reported to stderr) falls back to the no-image card rather than blocking
/// the text lines.
pub async fn ensure_rpc_image(
    data_dir: &std::path::Path,
    game: &Game,
    settings: &PresenceSettings,
) -> Option<String> {
    use vn_core::presence::RpcImageHost;

    if !settings.show_cover || settings.rpc_image_host == RpcImageHost::None {
        return None;
    }
    let (source, blurred) = rpc_image_source(data_dir, game)?;

    let raw = tokio::fs::read(&source).await.ok()?;
    if raw.is_empty() {
        return None;
    }
    let image_bytes = if blurred {
        crate::vndb::images::blurred_cover_bytes(&raw).ok()?
    } else {
        raw.clone()
    };
    if image_bytes.is_empty() {
        return None;
    }

    // Session-stable key: a changed cover, blur flip or host is a new upload.
    let meta = std::fs::metadata(&source).ok();
    let key = format!(
        "{:?}:{}:{}:{}:{}",
        settings.rpc_image_host,
        blurred,
        source.display(),
        meta.as_ref().map_or(0, std::fs::Metadata::len),
        raw.len()
    );
    let host = settings.rpc_image_host;
    let filename = source.file_name().map_or_else(
        || "cover.jpg".to_owned(),
        |name| name.to_string_lossy().into_owned(),
    );
    // The keychain read belongs inside the attempt, not on the poll path: a
    // cache hit must not touch the credential store at all.
    let url = upload::shared_cache()
        .get_or_upload(&key, async move {
            let api_key = if host == RpcImageHost::Imgbb {
                crate::secrets::load_imgbb_key()
                    .ok()
                    .map(|key| key.expose().to_owned())
            } else {
                None
            };
            upload::upload_to_host(host, api_key.as_deref(), image_bytes, &filename).await
        })
        .await;
    if url.is_none() {
        eprintln!("presence image upload failed; continuing without the card image");
    }
    url
}

/// Transport sink trait for dispatching rich presence updates.
pub trait PresenceSink: Send {
    fn update(&mut self, client_id: &str, activity: &PresenceActivity)
    -> Result<(), PresenceError>;
    fn clear(&mut self) -> Result<(), PresenceError>;
    fn shutdown(&mut self) -> Result<(), PresenceError>;
    fn is_connected(&self) -> bool;
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq, Serialize)]
pub enum PresenceError {
    #[error("Discord is not running")]
    DiscordNotRunning,
    #[error("Presence is disabled")]
    Disabled,
    #[error("Platform unsupported")]
    UnsupportedPlatform,
    #[error("IPC error: {0}")]
    IpcError(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", content = "message", rename_all = "snake_case")]
pub enum PresenceStatus {
    Disabled,
    Disconnected,
    Connecting,
    Connected,
    DiscordNotRunning,
    Error(String),
    UnsupportedPlatform,
}

impl From<PresenceError> for PresenceStatus {
    fn from(err: PresenceError) -> Self {
        match err {
            PresenceError::DiscordNotRunning => Self::DiscordNotRunning,
            PresenceError::UnsupportedPlatform => Self::UnsupportedPlatform,
            PresenceError::IpcError(msg) => Self::Error(msg),
            PresenceError::Disabled => Self::Disabled,
        }
    }
}

/// Builds, throttles and sends one update, advancing the details cycle.
///
/// Returns `None` when the debouncer swallowed the update (nothing changed yet),
/// so the caller knows not to touch the connection status.
///
/// The two rotating lines advance on their own clocks. `EwE` keys a switch time
/// and an index per field, and a single shared clock made a fast details list
/// drag the state list along with it, so a three-entry state list skipped entries
/// whenever the details list was shorter.
fn publish(
    sink: &mut dyn PresenceSink,
    debouncer: &mut PresenceDebouncer,
    settings: &PresenceSettings,
    ctx: &mut PresenceContext,
    cycle: &mut CycleState,
    now_secs: i64,
) -> Option<Result<(), PresenceError>> {
    ctx.cycle_index = cycle.index(CycleField::Details);
    ctx.state_cycle_index = cycle.index(CycleField::State);
    let activity = build_activity(settings, ctx);

    if !debouncer.should_send(&activity, now_secs) {
        return None;
    }

    let effective_id = if settings.client_id.trim().is_empty() {
        DEFAULT_CLIENT_ID
    } else {
        settings.client_id.trim()
    };

    let res = sink.update(effective_id, &activity);
    if res.is_ok() {
        // Advance only after Discord accepted the payload, so a failing connection
        // retries the same entry instead of running the lists blind.
        cycle.tick(
            CycleField::Details,
            details_cycle_seconds(settings),
            now_secs,
        );
        cycle.tick(CycleField::State, state_cycle_seconds(settings), now_secs);
    }
    Some(res)
}

/// How long the details line holds one entry.
fn details_cycle_seconds(settings: &PresenceSettings) -> i64 {
    match settings.details_mode {
        PresenceDetailsMode::Vndb => i64::from(settings.cycle_seconds.max(MIN_CYCLE_SECONDS)),
        _ => i64::from(settings.custom_details_cycle_seconds.max(MIN_CYCLE_SECONDS)),
    }
}

/// How long the state line holds one entry.
fn state_cycle_seconds(settings: &PresenceSettings) -> i64 {
    i64::from(settings.custom_state_cycle_seconds.max(MIN_CYCLE_SECONDS))
}

/// One rotating field's cursor and clock.
#[derive(Debug, Default, Clone, Copy)]
struct CycleCursor {
    index: u32,
    /// When this field last advanced, in epoch seconds. `None` until the first
    /// successful send, so the first entry is shown rather than skipped: a zero
    /// start would make `now - 0` exceed the interval immediately.
    last_at: Option<i64>,
}

/// Both rotating cursors, plus the game they belong to.
///
/// Cleared when the game changes: `EwE` resets its rotation state on stop for the
/// same reason, otherwise resuming a novel would start mid-list at a fact the
/// user has not seen yet.
#[derive(Debug, Default)]
struct CycleState {
    details: CycleCursor,
    state: CycleCursor,
    game_key: String,
}

impl CycleState {
    const fn index(&self, field: CycleField) -> u32 {
        match field {
            CycleField::Details => self.details.index,
            CycleField::State => self.state.index,
        }
    }

    fn cursor_mut(&mut self, field: CycleField) -> &mut CycleCursor {
        match field {
            CycleField::Details => &mut self.details,
            CycleField::State => &mut self.state,
        }
    }

    /// Advances one cursor when its interval has elapsed.
    fn tick(&mut self, field: CycleField, seconds: i64, now_secs: i64) {
        let cursor = self.cursor_mut(field);
        let due = cursor.last_at.is_none_or(|last| now_secs - last >= seconds);
        if !due {
            return;
        }
        if cursor.last_at.is_some() {
            cursor.index = cursor.index.wrapping_add(1);
        }
        cursor.last_at = Some(now_secs);
    }

    /// Forgets both cursors when the tracked game changes.
    fn retarget(&mut self, game_key: &str) {
        if self.game_key == game_key {
            return;
        }
        self.game_key = game_key.to_string();
        self.details = CycleCursor::default();
        self.state = CycleCursor::default();
    }
}

/// Applies a settings replacement and reports what Discord is now showing.
///
/// Extracted from the service loop so the "switched off while something was on
/// screen" rule reads on its own: only a *transition* to disabled may clear, and
/// re-saving settings that were already off must not send anything.
fn apply_settings(
    sink: &mut dyn PresenceSink,
    debouncer: &mut PresenceDebouncer,
    _previous: PresenceSettings,
    incoming: PresenceSettings,
    has_activity: &mut bool,
) -> (PresenceSettings, PresenceStatus) {
    if incoming.enabled {
        // Nothing to clear. The connection status is left for the next update to
        // report: a save is not proof that Discord is reachable.
        return (incoming, PresenceStatus::Disconnected);
    }

    debouncer.reset();
    if *has_activity {
        let _ = sink.clear();
        *has_activity = false;
    }
    (incoming, PresenceStatus::Disabled)
}

enum PresenceMsg {
    Update(Box<PresenceContext>),
    Clear,
    SetSettings(Box<PresenceSettings>),
    Shutdown,
}

/// Service managing Discord Rich Presence connection and update loop.
pub struct PresenceService {
    tx: mpsc::Sender<PresenceMsg>,
    status: Arc<RwLock<PresenceStatus>>,
    settings: Arc<RwLock<PresenceSettings>>,
    /// Which entry each rotating line is showing.
    ///
    /// Lives here, not in `PresenceContext`, because every poll builds a fresh
    /// context from the database: a counter carried in the context would restart
    /// at zero on every poll and the lines would never rotate.
    cycle: Arc<std::sync::Mutex<CycleState>>,
}

impl PresenceService {
    /// Which VNDB fact the rotating details line is currently on.
    ///
    /// Exposed so a caller (and the tests) can observe the cycle advancing
    /// without having to drive a real Discord connection.
    #[must_use]
    pub fn cycle_index(&self) -> u32 {
        self.cycle
            .lock()
            .map_or(0, |state| state.index(CycleField::Details))
    }
}

impl std::fmt::Debug for PresenceService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PresenceService").finish_non_exhaustive()
    }
}

impl PresenceService {
    #[must_use]
    pub fn new(initial_settings: PresenceSettings) -> Self {
        #[cfg(windows)]
        let sink: Box<dyn PresenceSink> = Box::new(ipc::IpcSink::new());
        #[cfg(target_os = "android")]
        let sink: Box<dyn PresenceSink> = Box::new(gateway::AndroidGatewaySink::new());
        #[cfg(not(any(windows, target_os = "android")))]
        let sink: Box<dyn PresenceSink> = Box::new(ipc::IpcSink::new());

        Self::with_sink(initial_settings, sink)
    }

    #[must_use]
    pub fn with_sink(initial_settings: PresenceSettings, mut sink: Box<dyn PresenceSink>) -> Self {
        let (tx, mut rx) = mpsc::channel::<PresenceMsg>(32);
        let initial_status = if initial_settings.enabled {
            PresenceStatus::Disconnected
        } else {
            PresenceStatus::Disabled
        };

        let status = Arc::new(RwLock::new(initial_status));
        let settings = Arc::new(RwLock::new(initial_settings.clone()));
        let cycle = Arc::new(std::sync::Mutex::new(CycleState::default()));

        let task_status = Arc::clone(&status);
        let task_cycle = Arc::clone(&cycle);

        tauri::async_runtime::spawn(async move {
            let mut current_settings = initial_settings;
            let mut debouncer = PresenceDebouncer::default();
            // Whether Discord is currently showing something we sent. The tracking
            // loop reports "nothing is running" every poll, and without this every
            // one of those would become a `clear_activity` call to Discord.
            let mut has_activity = false;

            while let Some(msg) = rx.recv().await {
                match msg {
                    PresenceMsg::Update(mut ctx) => {
                        if !current_settings.enabled {
                            if has_activity {
                                let _ = sink.clear();
                                has_activity = false;
                            }
                            let mut st = task_status.write().await;
                            *st = PresenceStatus::Disabled;
                            continue;
                        }

                        let now_secs = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0));
                        // The lock is taken and released inside this block: holding a
                        // `std::sync::MutexGuard` across the `status` await below makes
                        // the whole service task non-`Send`.
                        let published = {
                            let Ok(mut cycle) = task_cycle.lock() else {
                                continue;
                            };
                            cycle.retarget(&ctx.game_key);
                            publish(
                                &mut *sink,
                                &mut debouncer,
                                &current_settings,
                                &mut ctx,
                                &mut cycle,
                                now_secs,
                            )
                        };
                        let Some(status_now) = published else {
                            continue;
                        };
                        let mut st = task_status.write().await;
                        match status_now {
                            Ok(()) => {
                                has_activity = true;
                                *st = PresenceStatus::Connected;
                            }
                            Err(err) => {
                                // The debouncer already counted this as sent. It did
                                // not arrive, so let the next poll retry at the
                                // minimum interval rather than after 30 s of silence.
                                debouncer.reset();
                                *st = PresenceStatus::from(err);
                            }
                        }
                    }
                    PresenceMsg::Clear => {
                        debouncer.reset();
                        if has_activity {
                            let _ = sink.clear();
                            has_activity = false;
                            let mut st = task_status.write().await;
                            *st = if !current_settings.enabled {
                                PresenceStatus::Disabled
                            } else if sink.is_connected() {
                                PresenceStatus::Connected
                            } else {
                                PresenceStatus::Disconnected
                            };
                        }
                    }
                    PresenceMsg::SetSettings(new_settings) => {
                        let next = apply_settings(
                            &mut *sink,
                            &mut debouncer,
                            current_settings,
                            *new_settings,
                            &mut has_activity,
                        );
                        current_settings = next.0;
                        *task_status.write().await = next.1;
                    }
                    PresenceMsg::Shutdown => {
                        debouncer.reset();
                        let _ = sink.shutdown();
                        break;
                    }
                }
            }
        });

        Self {
            tx,
            status,
            settings,
            cycle,
        }
    }

    /// Offer the latest activity to the service. Never waits.
    ///
    /// The tracking poll loop calls this every two seconds. The IPC calls behind it
    /// are blocking and have no timeout, so a stuck Discord would fill a bounded
    /// queue and, with an awaiting `send`, freeze playtime tracking. A full queue
    /// drops the update instead; the next poll sends a fresh one.
    pub fn update_context(&self, ctx: PresenceContext) {
        let _ = self.tx.try_send(PresenceMsg::Update(Box::new(ctx)));
    }

    /// Ask the service to clear the activity. Never waits, for the reason above.
    pub fn clear(&self) {
        let _ = self.tx.try_send(PresenceMsg::Clear);
    }

    /// Replace the settings.
    ///
    /// The shared copy is written here, before the message is queued. Written only
    /// by the service task it lagged behind: a read straight after a save (the
    /// settings screen does exactly that) returned the old values, and
    /// `presence_set_enabled`'s read-modify-write then saved them back over the
    /// edit.
    pub async fn set_settings(&self, settings: PresenceSettings) {
        *self.settings.write().await = settings.clone();
        let _ = self
            .tx
            .send(PresenceMsg::SetSettings(Box::new(settings)))
            .await;
    }

    pub async fn get_settings(&self) -> PresenceSettings {
        self.settings.read().await.clone()
    }

    pub async fn status(&self) -> PresenceStatus {
        let local_st = self.status.read().await.clone();
        #[cfg(target_os = "android")]
        if self.settings.read().await.enabled {
            if let Ok(live_st) = crate::platform::android_presence::status() {
                return match live_st {
                    PresenceStatus::Disconnected => match local_st {
                        PresenceStatus::Error(_) => local_st,
                        _ => PresenceStatus::Disconnected,
                    },
                    other => other,
                };
            }
        }
        local_st
    }

    pub async fn shutdown(&self) {
        let _ = self.tx.send(PresenceMsg::Shutdown).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::models::GameRow;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    /// A minimal game row the image-source tests can tweak per case.
    fn image_game(sexual: Option<i64>, custom: Option<&str>) -> Game {
        Game::try_from(GameRow {
            id: "0192f0aa-0000-7000-8000-000000000001".to_owned(),
            title: "Umineko".to_owned(),
            title_original: None,
            vndb_id: Some("v24".to_owned()),
            status: "playing".to_owned(),
            rating: Some(10),
            notes: None,
            length_minutes: Some(3_000),
            manual_playtime_ms: 0,
            accent_color: None,
            cover_url: Some("https://t.vndb.org/cv/24.jpg".to_owned()),
            // Both ratings carry the same value: `None` is the unrated case that
            // must fail closed, `Some(0)` is safe, `Some(2)` is explicit.
            cover_sexual: sexual,
            cover_violence: sexual,
            custom_cover_path: custom.map(str::to_owned),
            custom_cover_source_vndb: 0,
            started_at: None,
            finished_at: None,
            updated_at: 5_000,
            device_id: "dev-a".to_owned(),
            deleted: 0,
        })
        .expect("row converts")
    }

    /// A temp data dir holding one custom cover file and one cached VNDB cover.
    fn image_dir(tag: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("vnt-rpcimg-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let store = crate::vndb::images::ImageStore::new(&root);
        std::fs::create_dir_all(store.custom_dir()).expect("custom dir");
        std::fs::write(store.custom_cover_path("gamea"), b"jpeg").expect("custom file");
        std::fs::write(store.cover_path("v24"), b"jpeg").expect("cached file");
        root
    }

    #[test]
    fn a_custom_cover_is_uploaded_blurred_unless_ratings_say_it_is_safe() {
        let dir = image_dir("custom");
        let store = crate::vndb::images::ImageStore::new(&dir);

        // A game with no cover ratings fails closed: blur travels, not the file.
        let unrated = image_game(
            None,
            Some(&crate::vndb::images::ImageStore::custom_relative("gamea")),
        );
        let (path, blurred) = rpc_image_source(&dir, &unrated).expect("custom cover resolves");
        assert!(path.ends_with("custom/gamea.jpg"));
        assert!(path.starts_with(store.custom_dir()));
        assert!(blurred);

        let safe = image_game(
            Some(0),
            Some(&crate::vndb::images::ImageStore::custom_relative("gamea")),
        );
        let (_, blurred) = rpc_image_source(&dir, &safe).expect("safe custom cover");
        assert!(!blurred, "a rated-safe custom cover may travel as-is");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_explicit_vndb_cover_blurs_from_the_cache_and_a_safe_one_needs_nothing() {
        let dir = image_dir("vndb");
        let store = crate::vndb::images::ImageStore::new(&dir);

        let explicit = image_game(Some(2), None);
        let (path, blurred) = rpc_image_source(&dir, &explicit).expect("cached cover");
        assert_eq!(path, store.cover_path("v24"));
        assert!(blurred);

        let safe = image_game(Some(0), None);
        assert_eq!(
            rpc_image_source(&dir, &safe),
            None,
            "the public URL already works"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_cached_file_means_no_image_rather_than_an_error() {
        let dir = std::env::temp_dir().join(format!("vnt-rpcimg-empty-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let explicit = image_game(Some(2), None);
        assert_eq!(rpc_image_source(&dir, &explicit), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn format_utc_at(
        year: i32,
        month: time::Month,
        day: u8,
        hour: u8,
        minute: u8,
        offset_h: i8,
    ) -> (String, String) {
        let utc = time::Date::from_calendar_date(year, month, day)
            .expect("test date is valid")
            .with_hms(hour, minute, 0)
            .expect("test time is valid")
            .assume_utc();
        let zone = time::UtcOffset::from_hms(offset_h, 0, 0).expect("test offset is valid");
        format_date_time(utc.to_offset(zone))
    }

    #[test]
    fn date_and_time_match_the_ewe_formats() {
        assert_eq!(
            format_utc_at(2026, time::Month::October, 24, 14, 30, 0),
            ("2026-10-24".to_string(), "14:30".to_string())
        );
    }

    #[test]
    fn date_and_time_are_zero_padded() {
        assert_eq!(
            format_utc_at(2026, time::Month::March, 5, 9, 7, 0),
            ("2026-03-05".to_string(), "09:07".to_string())
        );
    }

    #[test]
    fn the_date_follows_the_local_offset_not_utc() {
        // 2026-10-24 17:30 UTC is already the next day at +8, so a hardcoded UTC
        // date would show yesterday for much of Asia.
        assert_eq!(
            format_utc_at(2026, time::Month::October, 24, 17, 30, 8),
            ("2026-10-25".to_string(), "01:30".to_string())
        );
    }

    #[test]
    fn local_date_time_renders_a_well_formed_pair() {
        // The offset depends on the machine, so only the shape is pinned here:
        // never empty, always `%Y-%m-%d` and `%H:%M`.
        let (date, time) = local_date_time(vn_core::clock::Timestamp::from_millis(86_400_000));
        let (d, t, m) = (date.as_bytes(), time.as_bytes(), date.len());
        assert_eq!(m, 10, "date: {date:?}");
        assert_eq!(d[4], b'-');
        assert_eq!(d[7], b'-');
        assert_eq!(time.len(), 5, "time: {time:?}");
        assert_eq!(t[2], b':');
        assert!(d[..4].iter().all(u8::is_ascii_digit));
        assert!(t[..2].iter().all(u8::is_ascii_digit));
    }

    struct MockSink {
        updates: Arc<AtomicUsize>,
        clears: Arc<AtomicUsize>,
        fail_connect: bool,
        is_connected: Arc<AtomicBool>,
    }

    impl PresenceSink for MockSink {
        fn update(
            &mut self,
            _client_id: &str,
            _activity: &PresenceActivity,
        ) -> Result<(), PresenceError> {
            if self.fail_connect {
                return Err(PresenceError::DiscordNotRunning);
            }
            self.updates.fetch_add(1, Ordering::SeqCst);
            self.is_connected.store(true, Ordering::SeqCst);
            Ok(())
        }

        fn clear(&mut self) -> Result<(), PresenceError> {
            self.clears.fetch_add(1, Ordering::SeqCst);
            self.is_connected.store(false, Ordering::SeqCst);
            Ok(())
        }

        fn shutdown(&mut self) -> Result<(), PresenceError> {
            self.is_connected.store(false, Ordering::SeqCst);
            Ok(())
        }

        fn is_connected(&self) -> bool {
            self.is_connected.load(Ordering::SeqCst)
        }
    }

    #[tokio::test]
    async fn test_presence_disabled_by_default() {
        let settings = PresenceSettings::default();
        let updates = Arc::new(AtomicUsize::new(0));
        let clears = Arc::new(AtomicUsize::new(0));
        let is_connected = Arc::new(AtomicBool::new(false));

        let sink = Box::new(MockSink {
            updates: Arc::clone(&updates),
            clears: Arc::clone(&clears),
            fail_connect: false,
            is_connected: Arc::clone(&is_connected),
        });

        let service = PresenceService::with_sink(settings, sink);
        assert_eq!(service.status().await, PresenceStatus::Disabled);

        // Update when disabled should not call sink update
        service.update_context(PresenceContext::default());
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        assert_eq!(updates.load(Ordering::SeqCst), 0);
        assert_eq!(service.status().await, PresenceStatus::Disabled);
    }

    fn enabled() -> PresenceSettings {
        PresenceSettings {
            enabled: true,
            ..Default::default()
        }
    }

    fn counting_sink(fail_connect: bool) -> (Box<MockSink>, Arc<AtomicUsize>, Arc<AtomicUsize>) {
        let updates = Arc::new(AtomicUsize::new(0));
        let clears = Arc::new(AtomicUsize::new(0));
        let sink = Box::new(MockSink {
            updates: Arc::clone(&updates),
            clears: Arc::clone(&clears),
            fail_connect,
            is_connected: Arc::new(AtomicBool::new(false)),
        });
        (sink, updates, clears)
    }

    async fn settle() {
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    }

    #[tokio::test]
    async fn an_idle_poll_loop_does_not_flood_discord_with_clears() {
        // The tracking loop reports "nothing running" every two seconds. Only the
        // first clear after something was shown may reach Discord.
        let (sink, updates, clears) = counting_sink(false);
        let service = PresenceService::with_sink(enabled(), sink);

        for _ in 0..10 {
            service.clear();
        }
        settle().await;
        assert_eq!(clears.load(Ordering::SeqCst), 0, "nothing was shown");

        service.update_context(PresenceContext {
            vn_name: "Clannad".to_string(),
            ..Default::default()
        });
        settle().await;
        assert_eq!(updates.load(Ordering::SeqCst), 1);

        for _ in 0..10 {
            service.clear();
        }
        settle().await;
        assert_eq!(
            clears.load(Ordering::SeqCst),
            1,
            "cleared once, not ten times"
        );
    }

    #[tokio::test]
    async fn a_disabled_service_does_not_clear_on_every_update() {
        let (sink, _updates, clears) = counting_sink(false);
        let service = PresenceService::with_sink(PresenceSettings::default(), sink);
        for _ in 0..5 {
            service.update_context(PresenceContext::default());
        }
        settle().await;
        assert_eq!(clears.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn disabling_clears_what_was_shown_exactly_once() {
        let (sink, _updates, clears) = counting_sink(false);
        let service = PresenceService::with_sink(enabled(), sink);
        service.update_context(PresenceContext {
            vn_name: "Clannad".to_string(),
            ..Default::default()
        });
        settle().await;

        service.set_settings(PresenceSettings::default()).await;
        service.set_settings(PresenceSettings::default()).await;
        settle().await;
        assert_eq!(clears.load(Ordering::SeqCst), 1);
        assert_eq!(service.status().await, PresenceStatus::Disabled);
    }

    #[tokio::test]
    async fn saved_settings_are_readable_immediately() {
        // The settings screen saves and then reads back; the read must not see the
        // previous values just because the service task has not run yet.
        let (sink, _updates, _clears) = counting_sink(false);
        let service = PresenceService::with_sink(PresenceSettings::default(), sink);
        let edited = PresenceSettings {
            enabled: true,
            custom_details: "edited".to_string(),
            ..Default::default()
        };
        service.set_settings(edited.clone()).await;
        assert_eq!(service.get_settings().await, edited);
    }

    #[tokio::test]
    async fn a_stuck_service_never_blocks_the_caller() {
        // A sink that never returns models Discord hanging mid-handshake.
        struct StuckSink(std::sync::mpsc::Receiver<()>);
        impl PresenceSink for StuckSink {
            fn update(&mut self, _: &str, _: &PresenceActivity) -> Result<(), PresenceError> {
                let _ = self.0.recv();
                Ok(())
            }
            fn clear(&mut self) -> Result<(), PresenceError> {
                Ok(())
            }
            fn shutdown(&mut self) -> Result<(), PresenceError> {
                Ok(())
            }
            fn is_connected(&self) -> bool {
                false
            }
        }

        let (release, hold) = std::sync::mpsc::channel::<()>();
        let service = PresenceService::with_sink(enabled(), Box::new(StuckSink(hold)));
        // Far more messages than the queue holds. If these awaited a full queue,
        // this test would hang, and so would the playtime poll loop.
        let flood = async {
            for _ in 0..200 {
                service.update_context(PresenceContext {
                    vn_name: "Clannad".to_string(),
                    ..Default::default()
                });
                service.clear();
            }
        };
        tokio::time::timeout(tokio::time::Duration::from_secs(5), flood)
            .await
            .expect("callers must never wait on the service");
        drop(release);
    }

    fn game_with_ratings(sexual: Option<u8>, violence: Option<u8>) -> Game {
        let device = vn_core::ids::DeviceId::parse("test-device").expect("device id");
        let mut game = Game::new(
            &device,
            vn_core::clock::Timestamp::from_millis(0),
            "Clannad",
            vn_core::model::GameStatus::Playing,
        )
        .expect("game");
        game.cover_url = Some("https://t.vndb.org/cv/00/1.jpg".to_string());
        game.cover_sexual = sexual;
        game.cover_violence = violence;
        game
    }

    #[test]
    fn an_unrated_cover_is_withheld_not_assumed_safe() {
        assert!(cover_is_explicit(&game_with_ratings(None, None)));
        assert!(cover_is_explicit(&game_with_ratings(Some(0), None)));
        assert!(cover_is_explicit(&game_with_ratings(None, Some(0))));
    }

    #[test]
    fn only_a_fully_rated_low_cover_is_public() {
        assert!(!cover_is_explicit(&game_with_ratings(Some(0), Some(0))));
        // Level 1 is withheld from Discord as well. It used to be published, because
        // this check had its own `>= 2` threshold while the library concealed level 1 —
        // so a cover the user never saw in full inside the app was uploaded to Discord's
        // CDN. One threshold now decides both surfaces.
        assert!(cover_is_explicit(&game_with_ratings(Some(1), Some(1))));
        assert!(cover_is_explicit(&game_with_ratings(Some(2), Some(0))));
        assert!(cover_is_explicit(&game_with_ratings(Some(0), Some(2))));
    }

    #[test]
    fn the_base_context_carries_the_withheld_flag_into_the_activity() {
        let unrated = base_context(&game_with_ratings(None, None));
        let act = build_activity(&PresenceSettings::default(), &unrated);
        assert_eq!(act.large_image, None, "unrated art must not reach Discord");

        let rated = base_context(&game_with_ratings(Some(0), Some(0)));
        let act = build_activity(&PresenceSettings::default(), &rated);
        assert_eq!(
            act.large_image.as_deref(),
            Some("https://t.vndb.org/cv/00/1.jpg")
        );
    }

    #[tokio::test]
    async fn test_presence_enabled_updates_and_clears() {
        let settings = PresenceSettings {
            enabled: true,
            ..Default::default()
        };

        let updates = Arc::new(AtomicUsize::new(0));
        let clears = Arc::new(AtomicUsize::new(0));
        let is_connected = Arc::new(AtomicBool::new(false));

        let sink = Box::new(MockSink {
            updates: Arc::clone(&updates),
            clears: Arc::clone(&clears),
            fail_connect: false,
            is_connected: Arc::clone(&is_connected),
        });

        let service = PresenceService::with_sink(settings, sink);
        service.update_context(PresenceContext {
            vn_name: "Clannad".to_string(),
            ..Default::default()
        });

        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        assert_eq!(updates.load(Ordering::SeqCst), 1);
        assert_eq!(service.status().await, PresenceStatus::Connected);

        service.clear();
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        assert_eq!(clears.load(Ordering::SeqCst), 1);
        assert_eq!(service.status().await, PresenceStatus::Disconnected);
    }

    #[tokio::test]
    async fn test_presence_discord_not_running() {
        let settings = PresenceSettings {
            enabled: true,
            ..Default::default()
        };

        let sink = Box::new(MockSink {
            updates: Arc::new(AtomicUsize::new(0)),
            clears: Arc::new(AtomicUsize::new(0)),
            fail_connect: true,
            is_connected: Arc::new(AtomicBool::new(false)),
        });

        let service = PresenceService::with_sink(settings, sink);
        service.update_context(PresenceContext {
            vn_name: "Fate".to_string(),
            ..Default::default()
        });

        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        assert_eq!(service.status().await, PresenceStatus::DiscordNotRunning);
    }

    /// Records every activity it is handed, so a test can read the payload.
    struct RecordingSink {
        sent: Arc<std::sync::Mutex<Vec<PresenceActivity>>>,
    }

    impl PresenceSink for RecordingSink {
        fn update(
            &mut self,
            _client_id: &str,
            activity: &PresenceActivity,
        ) -> Result<(), PresenceError> {
            self.sent.lock().expect("lock").push(activity.clone());
            Ok(())
        }
        fn clear(&mut self) -> Result<(), PresenceError> {
            Ok(())
        }
        fn shutdown(&mut self) -> Result<(), PresenceError> {
            Ok(())
        }
        fn is_connected(&self) -> bool {
            true
        }
    }

    fn tracking_context(vn_name: &str) -> PresenceContext {
        PresenceContext {
            vn_name: vn_name.to_string(),
            vndb_developer: Some("Nitroplus".to_string()),
            vndb_rating: Some("8.86".to_string()),
            // A linked VN: without this the card shows the "Reading" / "Loading
            // VNDB data..." placeholders, which is the correct behaviour for an
            // unlinked game and would mask what these tests are checking.
            vndb_loaded: true,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn the_tracking_poll_loop_publishes_the_vn_title_as_the_card_name() {
        // The regression: the card was titled with the Discord application name
        // because no activity name was ever sent, even though every other field
        // was populated correctly.
        let sent = Arc::new(std::sync::Mutex::new(Vec::new()));
        let service = PresenceService::with_sink(
            enabled(),
            Box::new(RecordingSink {
                sent: Arc::clone(&sent),
            }),
        );
        service.update_context(tracking_context("Steins;Gate"));
        settle().await;

        let activity = sent.lock().expect("lock").first().cloned().expect("sent");
        assert_eq!(activity.name.as_deref(), Some("Steins;Gate"));
        assert_eq!(
            activity.details.as_deref(),
            Some("Nitroplus"),
            "details must not repeat the title"
        );
    }

    #[test]
    fn the_two_rotating_lines_advance_on_their_own_clocks() {
        // EwE keys a switch time per field. One shared clock made a fast details
        // list drag the state list along, so a longer state list skipped entries.
        let mut cycle = CycleState::default();
        assert_eq!(cycle.index(CycleField::Details), 0);
        assert_eq!(cycle.index(CycleField::State), 0);

        // First tick shows entry 0 on both; nothing advances yet.
        cycle.tick(CycleField::Details, 5, 1_000);
        cycle.tick(CycleField::State, 30, 1_000);
        assert_eq!(cycle.index(CycleField::Details), 0);
        assert_eq!(cycle.index(CycleField::State), 0);

        // Past the details interval only the details line moves.
        cycle.tick(CycleField::Details, 5, 1_006);
        cycle.tick(CycleField::State, 30, 1_006);
        assert_eq!(cycle.index(CycleField::Details), 1);
        assert_eq!(
            cycle.index(CycleField::State),
            0,
            "a 30 s state cycle must not follow a 5 s details cycle"
        );
    }

    /// The pure advance rule, lifted so a mutation test can pin it without a sink.
    /// Mirrors what `publish` calls after Discord accepts a payload.
    fn advance_for(settings: &PresenceSettings, cycle: &mut CycleState, now_secs: i64) {
        cycle.tick(
            CycleField::Details,
            details_cycle_seconds(settings),
            now_secs,
        );
        cycle.tick(CycleField::State, state_cycle_seconds(settings), now_secs);
    }

    #[test]
    fn publish_advances_each_field_at_its_own_configured_interval() {
        // The mixed-up-interval bug: ticking both with the details interval made a
        // 30 s state rotation behave like a 5 s one.
        let settings = PresenceSettings {
            custom_details_cycle_seconds: 5,
            custom_state_cycle_seconds: 30,
            ..Default::default()
        };
        let mut cycle = CycleState::default();
        advance_for(&settings, &mut cycle, 1_000);
        advance_for(&settings, &mut cycle, 1_006);
        assert_eq!(cycle.index(CycleField::Details), 1);
        assert_eq!(cycle.index(CycleField::State), 0);

        advance_for(&settings, &mut cycle, 1_031);
        assert_eq!(cycle.index(CycleField::Details), 2);
        assert_eq!(cycle.index(CycleField::State), 1);
    }

    #[test]
    fn changing_games_resets_the_rotation_to_the_first_entry() {
        // Resuming a novel must not start mid-list on a fact the user has not seen.
        let mut cycle = CycleState::default();
        cycle.retarget("game-a");
        cycle.tick(CycleField::Details, 5, 100);
        cycle.tick(CycleField::Details, 5, 200);
        assert_eq!(cycle.index(CycleField::Details), 1);

        cycle.retarget("game-b");
        assert_eq!(cycle.index(CycleField::Details), 0);
        assert_eq!(cycle.index(CycleField::State), 0);
    }

    #[test]
    fn the_same_game_across_polls_keeps_its_rotation() {
        // Every poll calls `retarget`; only a *different* game may reset.
        let mut cycle = CycleState::default();
        cycle.retarget("game-a");
        cycle.tick(CycleField::Details, 5, 100);
        cycle.tick(CycleField::Details, 5, 200);
        cycle.retarget("game-a");
        assert_eq!(cycle.index(CycleField::Details), 1);
    }

    #[test]
    fn a_cycle_shorter_than_the_floor_is_clamped_up() {
        let settings = PresenceSettings {
            custom_details_cycle_seconds: 1,
            custom_state_cycle_seconds: 0,
            ..Default::default()
        };
        assert_eq!(
            details_cycle_seconds(&settings),
            i64::from(MIN_CYCLE_SECONDS)
        );
        assert_eq!(state_cycle_seconds(&settings), i64::from(MIN_CYCLE_SECONDS));
    }

    #[tokio::test]
    async fn the_vndb_details_cycle_advances_between_polls() {
        // Without a counter that survives between polls, every poll rebuilt the
        // context from scratch and the line was frozen on one fact forever.
        let sent = Arc::new(std::sync::Mutex::new(Vec::new()));
        let service = PresenceService::with_sink(
            PresenceSettings {
                enabled: true,
                cycle_seconds: vn_core::presence::MIN_CYCLE_SECONDS,
                ..Default::default()
            },
            Box::new(RecordingSink {
                sent: Arc::clone(&sent),
            }),
        );

        service.update_context(tracking_context("Steins;Gate"));
        settle().await;
        assert_eq!(service.cycle_index(), 0, "the first fact is the developer");

        // Inside the cycle window nothing moves, even though polls keep arriving.
        service.update_context(tracking_context("Steins;Gate"));
        settle().await;
        assert_eq!(service.cycle_index(), 0);
    }
}
