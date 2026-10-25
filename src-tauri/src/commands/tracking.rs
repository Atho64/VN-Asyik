//! Commands for desktop playtime tracking.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};
use vn_core::clock::Timestamp;
use vn_core::ids::EntityId;
use vn_core::tracker::{Event, Phase};

use super::{CommandError, CommandResult};
use crate::db::queries;
use crate::state::AppState;
use crate::tracking::mapping::{self, ExecutableCandidate};

use crate::tracking::{TrackingError, TrackingStatus, phase_name};

fn now() -> CommandResult<Timestamp> {
    Timestamp::now().map_err(Into::into)
}

fn map_err(err: &TrackingError) -> CommandError {
    let code = match err {
        TrackingError::Unsupported => "unsupported",
        TrackingError::Denied(_) => "denied",
        TrackingError::Probe(_) => "probe",
        TrackingError::Db(_) => "db",
    };
    CommandError::new(code, err.to_string())
}

/// Whether tracking works here, and why not when it does not.
#[derive(Debug, Clone, Serialize)]
pub struct TrackerCapability {
    /// Whether the loop can run.
    pub available: bool,
    /// Human-readable reason when it cannot.
    pub reason: Option<String>,
    /// Whether the user has automatic tracking switched on.
    pub enabled: bool,
    /// Poll interval in milliseconds, for honesty about granularity.
    pub poll_interval_ms: i64,
}

/// Report tracking capability.
#[tauri::command]
pub async fn tracking_capability(state: State<'_, AppState>) -> CommandResult<TrackerCapability> {
    let status = state.tracker().status(state.db().pool()).await;
    Ok(TrackerCapability {
        available: status.available,
        reason: status.unavailable_reason.clone(),
        enabled: state.tracker().is_enabled().await,
        poll_interval_ms: i64::try_from(crate::tracking::POLL_INTERVAL.as_millis())
            .unwrap_or(2_000),
    })
}

/// Android usage-access state, and where the user can grant it.
///
/// Reported separately from `tracking_capability` because on Android the two
/// questions differ: the tracker is available in principle, but the platform
/// permission that makes it work is the user's to grant in system Settings, and
/// there is no runtime dialog for it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageAccessView {
    /// False on platforms that do not use the Android usage log at all.
    pub applicable: bool,
    /// True when the permission is held.
    pub granted: bool,
    /// The Settings action to open, when one is available.
    pub settings_action: Option<String>,
    /// Why it could not be determined, if that is the case.
    pub reason: Option<String>,
}

/// Report Android usage-access state.
#[tauri::command]
#[allow(
    clippy::needless_pass_by_value,
    reason = "Tauri commands cannot take &AppHandle; the macro requires a value"
)]
pub fn tracking_usage_access(app: tauri::AppHandle) -> CommandResult<UsageAccessView> {
    use crate::platform::android::{AndroidUsage, AndroidUsageError};

    match AndroidUsage::status(&app) {
        Ok(access) => Ok(UsageAccessView {
            applicable: true,
            granted: access.granted,
            settings_action: Some(access.settings_action),
            reason: None,
        }),
        // Not Android. The UI hides the card rather than showing a dead button.
        Err(AndroidUsageError::Unsupported) => Ok(UsageAccessView {
            applicable: false,
            granted: false,
            settings_action: None,
            reason: None,
        }),
        Err(err) => Ok(UsageAccessView {
            applicable: true,
            granted: false,
            settings_action: None,
            reason: Some(err.to_string()),
        }),
    }
}

/// Open the Android Usage access settings screen.
///
/// # Errors
/// Returns `unsupported` off Android, so the UI never shows a button that cannot
/// do anything.
#[tauri::command]
pub fn tracking_open_usage_settings() -> CommandResult<()> {
    use crate::platform::android::{AndroidUsage, AndroidUsageError};

    AndroidUsage::open_settings().map_err(|err| match err {
        AndroidUsageError::Unsupported => CommandError::new("unsupported", err.to_string()),
        _ => CommandError::new("usage_settings", err.to_string()),
    })
}

/// Installed launcher apps, including Android emulator and runner variants.
#[tauri::command]
pub async fn tracking_android_apps() -> CommandResult<Vec<crate::platform::android::AndroidApp>> {
    crate::platform::android::AndroidUsage::installed_apps()
        .await
        .map_err(|err| CommandError::new("android_apps", err.to_string()))
}

/// Read the active Android game session without closing or reconciling it.
#[tauri::command]
pub async fn tracking_android_status(
    state: State<'_, AppState>,
) -> CommandResult<crate::tracking::android::AndroidSessionView> {
    if !cfg!(target_os = "android") {
        return Ok(crate::tracking::android::AndroidSessionView::default());
    }
    Ok(crate::tracking::android::status(state.db().pool()).await?)
}

/// Reconcile the selected Android game's time when the app returns to the foreground.
#[tauri::command]
pub async fn tracking_android_session(
    state: State<'_, AppState>,
) -> CommandResult<crate::tracking::android::AndroidSessionView> {
    if !cfg!(target_os = "android") {
        return Ok(crate::tracking::android::AndroidSessionView::default());
    }
    let timestamp = now()?;
    let result = match crate::tracking::android::reconcile(
        state.db().pool(),
        state.device(),
        timestamp.as_millis(),
    )
    .await
    {
        Ok(view) => Ok(view),
        Err(reason) => {
            let mut view = crate::tracking::android::status(state.db().pool()).await?;
            view.reason = Some(reason);
            Ok(view)
        }
    };
    sync_presence_loop(&state, timestamp).await;
    result
}

/// Stop the explicitly selected Android game and save its last foreground interval.
#[tauri::command]
pub async fn tracking_android_stop(
    state: State<'_, AppState>,
) -> CommandResult<crate::tracking::android::AndroidSessionView> {
    if !cfg!(target_os = "android") {
        return Err(CommandError::new(
            "unsupported",
            "Android package tracking is not available here",
        ));
    }
    let timestamp = now()?;
    let view =
        crate::tracking::android::stop(state.db().pool(), state.device(), timestamp.as_millis())
            .await
            .map_err(|err| CommandError::new("android_tracking", err))?;
    sync_presence_loop(&state, timestamp).await;
    Ok(view)
}

/// Live tracking status for the Now Playing bar.
#[tauri::command]
pub async fn tracking_status(state: State<'_, AppState>) -> CommandResult<TrackingStatus> {
    Ok(state.tracker().status(state.db().pool()).await)
}

/// Turn automatic tracking on or off.
#[tauri::command]
pub async fn tracking_set_enabled(
    state: State<'_, AppState>,
    enabled: bool,
) -> CommandResult<TrackerCapability> {
    state.tracker().set_enabled(enabled).await;
    let status = state.tracker().status(state.db().pool()).await;
    Ok(TrackerCapability {
        available: status.available,
        reason: status.unavailable_reason.clone(),
        enabled,
        poll_interval_ms: i64::try_from(crate::tracking::POLL_INTERVAL.as_millis())
            .unwrap_or(2_000),
    })
}

/// Payload for mapping an executable to a game.
#[derive(Debug, Deserialize)]
pub struct MapExecutableInput {
    /// Game the executable belongs to.
    pub game_id: String,
    /// Full path to the executable.
    pub exe_path: String,
}

/// Map an executable to a game.
#[tauri::command]
pub async fn tracking_map_executable(
    state: State<'_, AppState>,
    input: MapExecutableInput,
) -> CommandResult<mapping::GameExecutable> {
    let game_id = EntityId::parse(&input.game_id)?;
    if queries::get_game(state.db().pool(), game_id)
        .await?
        .is_none()
    {
        return Err(CommandError::new("not_found", "game not found"));
    }
    Ok(mapping::add_executable(
        state.db().pool(),
        state.device(),
        now()?,
        game_id,
        &input.exe_path,
    )
    .await?)
}

/// Payload for mapping an emulator or runner target to a game.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapRunnerTargetInput {
    /// Game the mapping belongs to.
    pub game_id: String,
    /// Which runner (e.g. vita3k, rpcs3, pcsx2, eden, citron, shadps4, ppsspp, winlator, kirikiroid2, tyranor, `android_apk`).
    pub runner_kind: String,
    /// Host executable path on desktop.
    pub exe_path: Option<String>,
    /// Package name on Android.
    pub package: Option<String>,
    /// What the host opens (ROM path, title id, container folder).
    pub launch_target: Option<String>,
    /// Caption text for disambiguating window titles.
    pub title_match: Option<String>,
    /// Host-specific extra options as JSON.
    pub extra_args: Option<String>,
}

/// Map an emulator or runner target to a game.
#[tauri::command]
pub async fn tracking_map_runner_target(
    state: State<'_, AppState>,
    input: MapRunnerTargetInput,
) -> CommandResult<mapping::GameExecutable> {
    let game_id = EntityId::parse(&input.game_id)?;
    if cfg!(target_os = "android")
        && input
            .package
            .as_deref()
            .is_none_or(|value| value.trim().is_empty())
    {
        return Err(CommandError::new(
            "invalid",
            "Choose the installed Android game or runner package.",
        ));
    }
    if queries::get_game(state.db().pool(), game_id)
        .await?
        .is_none()
    {
        return Err(CommandError::new("not_found", "game not found"));
    }
    let target = mapping::RunnerTarget {
        runner_kind: input.runner_kind,
        exe_path: input.exe_path,
        package: input.package,
        launch_target: input.launch_target,
        title_match: input.title_match,
        extra_args: input.extra_args,
    };
    Ok(
        mapping::add_runner_target(state.db().pool(), state.device(), now()?, game_id, &target)
            .await?,
    )
}

/// Every executable mapping.
#[tauri::command]
pub async fn tracking_list_executables(
    state: State<'_, AppState>,
) -> CommandResult<Vec<mapping::GameExecutable>> {
    Ok(mapping::list_executables(state.db().pool()).await?)
}

/// The manual chapter timer, as the UI shows it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManualTimerView {
    /// Whether a timer is running or paused.
    pub active: bool,
    /// The timed chapter, when the target is a chapter.
    pub chapter_id: Option<String>,
    /// The timed route, when the target is a route rather than a chapter.
    pub route_id: Option<String>,
    /// What is being timed, chapter or route name, so the UI needs no second lookup.
    pub chapter_name: Option<String>,
    /// Milliseconds accumulated, excluding paused time.
    pub elapsed_ms: i64,
    /// Whether time is currently accumulating.
    pub running: bool,
}

/// Current state of the manual timer.
#[tauri::command]
pub async fn tracking_manual_status(state: State<'_, AppState>) -> CommandResult<ManualTimerView> {
    let Some(run) = state.tracker().manual_state().await else {
        return Ok(ManualTimerView {
            active: false,
            chapter_id: None,
            route_id: None,
            elapsed_ms: 0,
            chapter_name: None,
            running: false,
        });
    };

    // One lookup, whichever kind of target is running. A route-timed run has no
    // chapter, and asking the chapters table for a route id would return nothing and
    // leave the UI showing an empty name.
    let name: Option<String> = match (run.chapter_id, run.route_id) {
        (Some(chapter), _) => sqlx::query_scalar(
            "SELECT COALESCE(NULLIF(display_name, ''), name) FROM chapters WHERE id = ?",
        )
        .bind(chapter.to_string())
        .fetch_optional(state.db().pool())
        .await
        .map_err(|err| CommandError::new("db", err.to_string()))?,
        (None, Some(route)) => sqlx::query_scalar("SELECT name FROM routes WHERE id = ?")
            .bind(route.to_string())
            .fetch_optional(state.db().pool())
            .await
            .map_err(|err| CommandError::new("db", err.to_string()))?,
        (None, None) => None,
    };

    Ok(ManualTimerView {
        active: true,
        chapter_id: run.chapter_id.map(|id| id.to_string()),
        route_id: run.route_id.map(|id| id.to_string()),
        chapter_name: name,
        elapsed_ms: run.elapsed_at(now()?),
        running: run.is_running(),
    })
}

/// Start timing a chapter by hand.
#[tauri::command]
pub async fn tracking_manual_start(
    state: State<'_, AppState>,
    chapter_id: String,
    game_id: String,
) -> CommandResult<ManualTimerView> {
    let chapter = EntityId::parse(&chapter_id)?;
    let game = EntityId::parse(&game_id)?;
    let timestamp = now()?;
    state
        .tracker()
        .manual_start(chapter, game, timestamp)
        .await
        .map_err(|err| CommandError::new("manual", err.to_string()))?;
    sync_presence_loop(&state, timestamp).await;
    tracking_manual_status(state).await
}

/// Start timing a route by hand, with no chapter named.
///
/// Separate from [`tracking_manual_start`] rather than one command with two optional
/// ids: a caller must say which kind of thing it is timing, and "both unset" has no
/// meaning here.
#[tauri::command]
pub async fn tracking_manual_start_route(
    state: State<'_, AppState>,
    route_id: String,
    game_id: String,
) -> CommandResult<ManualTimerView> {
    let route = EntityId::parse(&route_id)?;
    let game = EntityId::parse(&game_id)?;
    let timestamp = now()?;
    state
        .tracker()
        .manual_start_route(route, game, timestamp)
        .await
        .map_err(|err| CommandError::new("manual", err.to_string()))?;
    sync_presence_loop(&state, timestamp).await;
    tracking_manual_status(state).await
}

/// Pause the manual timer.
#[tauri::command]
pub async fn tracking_manual_pause(state: State<'_, AppState>) -> CommandResult<ManualTimerView> {
    let timestamp = now()?;
    state
        .tracker()
        .manual_pause(timestamp)
        .await
        .map_err(|err| CommandError::new("manual", err.to_string()))?;
    sync_presence_loop(&state, timestamp).await;
    tracking_manual_status(state).await
}

/// Resume the manual timer.
#[tauri::command]
pub async fn tracking_manual_resume(state: State<'_, AppState>) -> CommandResult<ManualTimerView> {
    let timestamp = now()?;
    state
        .tracker()
        .manual_resume(timestamp)
        .await
        .map_err(|err| CommandError::new("manual", err.to_string()))?;
    sync_presence_loop(&state, timestamp).await;
    tracking_manual_status(state).await
}

/// Stop the manual timer and record the time it measured.
///
/// The result is an ordinary play session, so stats, exports and sync treat it
/// exactly like detected time.
#[tauri::command]
pub async fn tracking_manual_stop(state: State<'_, AppState>) -> CommandResult<ManualTimerView> {
    let run = state
        .tracker()
        .manual_stop()
        .await
        .map_err(|err| CommandError::new("manual", err.to_string()))?;

    let finished = now()?;
    let elapsed = run.finish(finished);
    if elapsed > 0 {
        let segment = vn_core::tracker::Segment {
            session_id: EntityId::new(),
            game_id: run.game_id,
            chapter_id: run.chapter_id,
            route_id: run.route_id,
            started_at: run.started_at,
            ended_at: finished,
            active_ms: elapsed,
            afk_ms: 0,
            checkpoint: false,
        };
        mapping::promote_pending_session(state.db().pool(), &segment, state.device(), finished)
            .await?;
    }

    sync_presence_loop(&state, finished).await;
    tracking_manual_status(state).await
}

/// Create a chapter by hand, for a game whose engine cannot be detected.
#[tauri::command]
pub async fn tracking_create_chapter(
    state: State<'_, AppState>,
    game_id: String,
    name: String,
    route_name: Option<String>,
    route_id: Option<String>,
) -> CommandResult<String> {
    let game = EntityId::parse(&game_id)?;
    let name = name.trim();
    if name.is_empty() {
        return Err(CommandError::new(
            "invalid",
            "a chapter needs a name to be identifiable",
        ));
    }
    let route = route_name
        .map(|value| value.trim().to_owned())
        .filter(|v| !v.is_empty());
    let link = route_id.as_deref().map(EntityId::parse).transpose()?;

    // Adding a chapter from inside a route should not also require typing the route
    // name: the route row already knows it.
    let route = match (route, link) {
        (Some(label), _) => Some(label),
        (None, Some(id)) => queries::route_by_id(state.db().pool(), id)
            .await?
            .map(|row| row.name),
        (None, None) => None,
    };

    mapping::insert_manual_chapter(
        state.db().pool(),
        game,
        name,
        route.as_deref(),
        link,
        state.device(),
        now()?,
    )
    .await
    .map(|id| id.to_string())
    .map_err(Into::into)
}

/// A chapter as the UI shows it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChapterView {
    /// Chapter id.
    pub id: String,
    /// The engine's raw scene name, kept so the UI can show what was detected.
    pub raw_name: String,
    /// What to show: the user's rename when set, otherwise the engine's name.
    pub display_name: String,
    /// Whether the display name is a user rename.
    pub renamed: bool,
    /// Whether the user created this chapter rather than detection finding it.
    pub manual: bool,
    /// Ordering code from the engine.
    pub order: Option<i64>,
    /// Stable route identifier, when the engine's naming reveals a route.
    pub route_key: Option<String>,
    /// Human route label, when known.
    pub route_name: Option<String>,
    /// What to show as the route heading.
    ///
    /// Computed here rather than in the panel: an engine route whose code is not in
    /// the verified table has a `route_key` and no `route_name`, and grouping on
    /// the key while labelling from the name made every such route render as
    /// "No route" — the same text as a chapter with no route at all.
    pub route_label: String,
    /// Whether this chapter has been reached.
    pub reached: bool,
    /// Time attributed to this chapter, summed from play sessions.
    ///
    /// Chapters are the detail; routes are the unit people track. This is what
    /// route totals are built from.
    pub playtime_ms: i64,
    /// The route this chapter is linked to, when it has one.
    pub route_id: Option<String>,
}

/// What chapter detection can do on this machine, and why.
///
/// The panel shows manual timers either way, but it must be able to say whether
/// automatic detection is expected to work rather than leave a user guessing why
/// chapters never appear.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChapterCapabilityView {
    /// Platform this build runs on, e.g. `windows`.
    pub platform: String,
    /// Sources that will actually be tried, cheapest first.
    pub sources: Vec<String>,
    /// True when a title read or a scene-table read may run here.
    pub automatic: bool,
    /// The engine family found for this game, when one was.
    pub engine: Option<String>,
    /// Why automatic detection is unavailable, when the platform permits it but
    /// nothing can be read.
    ///
    /// One of `noExecutable`, `unknownEngine`, `engineHasNoVerifiedSource`. Absent
    /// when detection is available or when the platform itself is the reason.
    pub unavailable_reason: Option<String>,
}

/// The executable name that will run a game, if it has been mapped.
///
async fn mapped_executable_name(pool: &sqlx::SqlitePool, game_id: &str) -> Option<String> {
    crate::tracking::mapping::executables_for_game(
        pool,
        vn_core::ids::EntityId::parse(game_id).ok()?,
    )
    .await
    .ok()?
    .into_iter()
    .find(|mapping| mapping.enabled)
    .map(|mapping| mapping.exe_name)
}

/// Which engine family an executable name belongs to, if any.
///
/// The probe answers this for a *running* process, by reading its modules. When no
/// game is running the only thing available is the name the user mapped, and the
/// verified profiles are keyed by module name, so a profile match is the honest
/// test. `None` means no engine has a profile for this build.
fn engine_for_module(module: &str) -> Option<String> {
    vn_core::adapter::KNOWN_ENGINES
        .iter()
        .find(|engine| vn_core::scene::profile_for(engine, module).is_some())
        .map(|engine| (*engine).to_owned())
}

/// Report which chapter sources are usable for a game here.
///
/// `game_id` is optional because the panel can be mounted before a game exists.
/// Without it the answer is the platform's, which is all that can honestly be said.
#[tauri::command]
#[allow(
    clippy::unused_async,
    reason = "the command reads the database, which needs an await once a game is named"
)]
pub async fn tracking_chapter_capability(
    state: tauri::State<'_, crate::state::AppState>,
    game_id: Option<String>,
) -> CommandResult<ChapterCapabilityView> {
    let platform = vn_core::adapter::Platform::current();

    let module = match game_id.as_deref() {
        Some(id) => mapped_executable_name(state.db().pool(), id).await,
        None => None,
    };
    let engine = module.as_deref().and_then(engine_for_module);

    let sources = vn_core::adapter::source_order(engine.as_deref(), platform);
    let labels: Vec<String> = sources
        .iter()
        .map(|source| match source {
            vn_core::adapter::ChapterSource::WindowTitle => "window_title".to_owned(),
            vn_core::adapter::ChapterSource::MemoryScene => "memory_scene".to_owned(),
        })
        .collect();

    // Only the platform is worth blaming when the platform is the obstacle; a
    // desktop with an unmapped game has a different problem and a different fix.
    let platform_ok = vn_core::adapter::ChapterSource::WindowTitle
        .unavailable_reason(platform)
        .is_available()
        || vn_core::adapter::ChapterSource::MemoryScene
            .unavailable_reason(platform)
            .is_available();

    let unavailable_reason = if labels.is_empty() {
        if !platform_ok {
            Some("platformCannotRead".to_owned())
        } else if module.is_none() {
            // Nothing mapped: the fix is to map the executable, not to read faster.
            Some("noExecutable".to_owned())
        } else {
            // Named, with no verified profile to read and no way to scan for
            // one. That is an honest gap, not a missing mapping.
            Some("engineHasNoVerifiedSource".to_owned())
        }
    } else {
        None
    };

    Ok(ChapterCapabilityView {
        platform: match platform {
            vn_core::adapter::Platform::Windows => "windows",
            vn_core::adapter::Platform::MacOs => "macos",
            vn_core::adapter::Platform::Linux => "linux",
            vn_core::adapter::Platform::Android => "android",
        }
        .to_owned(),
        automatic: !labels.is_empty(),
        sources: labels,
        engine,
        unavailable_reason,
    })
}

/// Chapters detected for a game, grouped for display.
#[tauri::command]
pub async fn tracking_list_chapters(
    state: State<'_, AppState>,
    game_id: String,
) -> CommandResult<Vec<ChapterView>> {
    chapter_views(state.db(), &game_id).await
}

/// The body of [`tracking_list_chapters`], callable from other commands.
///
/// Split out rather than duplicated: `game_routes` needs the same playtime subquery
/// and the same rename/manual logic, and a second copy of it is exactly how two
/// screens start disagreeing about the same chapter. A `#[tauri::command]` cannot be
/// called internally without a `State`, hence the plain function behind it.
pub async fn chapter_views(db: &crate::db::Db, game_id: &str) -> CommandResult<Vec<ChapterView>> {
    use sqlx::Row as _;

    let rows = sqlx::query(
        // Left-correlated sum so a chapter with no sessions still appears, at zero.
        // The `games` join is not decoration: `PRAGMA foreign_keys` is per connection,
        // so a chapter whose game was removed can outlive it, and summing sessions
        // through it would report playtime for a game that no longer exists.
        "SELECT c.id, c.name, c.display_name, c.sort_index, c.route_key, c.route_name, \
                c.reached, c.renamed, c.manual, c.route_id, \
                COALESCE((SELECT SUM(ps.counted_ms) FROM play_sessions ps \
                          JOIN games g2 ON g2.id = ps.game_id AND g2.deleted = 0 \
                          WHERE ps.chapter_id = c.id AND ps.deleted = 0), 0) AS playtime_ms \
         FROM chapters c JOIN games g ON g.id = c.game_id AND g.deleted = 0 \
         WHERE c.game_id = ? AND c.deleted = 0 \
         ORDER BY COALESCE(c.route_order, c.sort_index), c.sort_index",
    )
    .bind(game_id)
    .fetch_all(db.pool())
    .await
    .map_err(|err| CommandError::new("db", err.to_string()))?;

    Ok(rows
        .into_iter()
        .map(|row| {
            let raw_name: String = row.get("name");
            let display: Option<String> = row.get("display_name");
            // Recorded explicitly by `rename_chapter`, not inferred: the display
            // name is seeded with the detected name, so comparing fields cannot
            // tell "seeded" from "renamed".
            let renamed: i64 = row.get("renamed");
            let route_name: Option<String> = row.get("route_name");
            ChapterView {
                id: row.get("id"),
                display_name: display
                    .filter(|value| !value.is_empty())
                    .unwrap_or_else(|| raw_name.clone()),
                raw_name,
                renamed: renamed != 0,
                manual: row.get::<i64, _>("manual") != 0,
                order: row.get("sort_index"),
                route_label: vn_core::chapter::route_label(
                    row.get::<Option<String>, _>("route_key").as_deref(),
                    route_name.as_deref(),
                ),
                route_key: row.get("route_key"),
                route_name,
                reached: row.get::<i64, _>("reached") != 0,
                playtime_ms: row.get("playtime_ms"),
                route_id: row.get("route_id"),
            }
        })
        .collect())
}

/// Rename a chapter for this device.
///
/// The engine's raw name stays the identity detection matches on, so a rename
/// changes only what is shown. Passing an empty name clears the rename.
#[tauri::command]
pub async fn tracking_rename_chapter(
    state: State<'_, AppState>,
    chapter_id: String,
    display_name: Option<String>,
) -> CommandResult<()> {
    let id = EntityId::parse(&chapter_id)?;
    mapping::rename_chapter(
        state.db().pool(),
        id,
        display_name.as_deref(),
        state.device(),
        now()?,
    )
    .await?;
    Ok(())
}

/// Remove an executable mapping.
#[tauri::command]
pub async fn tracking_remove_executable(
    state: State<'_, AppState>,
    id: String,
) -> CommandResult<()> {
    mapping::remove_executable(state.db().pool(), &id, now()?, state.device()).await?;
    Ok(())
}

/// Ask the tracker to pause or resume.
#[tauri::command]
pub async fn tracking_pause(state: State<'_, AppState>) -> CommandResult<()> {
    state
        .tracker()
        .send(state.db().pool(), state.device(), Event::Pause, now()?)
        .await
        .map_err(|err| map_err(&err))?;
    Ok(())
}

/// Resume after a pause.
#[tauri::command]
pub async fn tracking_resume(state: State<'_, AppState>) -> CommandResult<()> {
    state
        .tracker()
        .send(state.db().pool(), state.device(), Event::Resume, now()?)
        .await
        .map_err(|err| map_err(&err))?;
    Ok(())
}

/// Stop the current run.
#[tauri::command]
pub async fn tracking_stop(state: State<'_, AppState>) -> CommandResult<()> {
    state
        .tracker()
        .send(state.db().pool(), state.device(), Event::Stop, now()?)
        .await
        .map_err(|err| map_err(&err))?;
    Ok(())
}

/// Folders the user wants scanned for executables.
#[tauri::command]
pub async fn tracking_list_scan_folders(state: State<'_, AppState>) -> CommandResult<Vec<String>> {
    Ok(mapping::list_scan_folders(state.db().pool()).await?)
}

/// Add a folder to scan.
#[tauri::command]
pub async fn tracking_add_scan_folder(
    state: State<'_, AppState>,
    path: String,
) -> CommandResult<()> {
    if !std::path::Path::new(&path).is_dir() {
        return Err(CommandError::new("invalid", "that folder does not exist"));
    }
    mapping::add_scan_folder(state.db().pool(), &path, now()?).await?;
    Ok(())
}

/// Remove a folder from the scan list.
#[tauri::command]
pub async fn tracking_remove_scan_folder(
    state: State<'_, AppState>,
    path: String,
) -> CommandResult<()> {
    mapping::remove_scan_folder(state.db().pool(), &path).await?;
    Ok(())
}

/// Scan a folder for candidate executables.
///
/// Read-only: it lists candidates and marks the ones already mapped. It never
/// maps anything itself, because guessing which executable in a Visual Novel
/// folder is "the game" is exactly the kind of guess that produces wrong playtime.
#[tauri::command]
pub async fn tracking_scan_folder(
    state: State<'_, AppState>,
    path: String,
    max_depth: Option<u32>,
) -> CommandResult<Vec<ExecutableCandidate>> {
    let root = std::path::PathBuf::from(&path);
    if !root.is_dir() {
        return Err(CommandError::new("invalid", "that folder does not exist"));
    }

    let depth = max_depth.unwrap_or(3).min(6);
    let folder_name = root
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default();

    // The walk lives in `vn-core` so its traversal rules are unit-testable; a
    // too-deep directory must skip only itself, never the sibling folders still
    // queued behind it.
    let found = vn_core::scan_rank::walk(&root, depth);
    let mut candidates = Vec::with_capacity(found.len());
    for item in vn_core::scan_rank::order_found(&found, &folder_name) {
        let mapped = mapping::game_for_executable(state.db().pool(), Some(&item.path), &item.name)
            .await?
            .map(|entry| entry.game_id);
        candidates.push(ExecutableCandidate {
            path: item.path.clone(),
            exe_name: item.name.clone(),
            size_bytes: item.size_bytes,
            mapped_game_id: mapped,
        });
    }
    Ok(candidates)
}

/// Start the background polling loop.
///
/// Runs for the lifetime of the app. Every tick feeds one sample into the state
/// machine and persists whatever it emits, so a crash loses at most one poll.
pub fn spawn_loop(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut ticker =
            tokio::time::interval(Duration::from_millis(if cfg!(target_os = "android") {
                15_000
            } else {
                u64::try_from(crate::tracking::POLL_INTERVAL.as_millis()).unwrap_or(2_000)
            }));
        // Skip missed ticks rather than bursting to catch up, which would credit
        // a suspended machine with hours of phantom playtime.
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            ticker.tick().await;
            let Some(state) = app.try_state::<AppState>() else {
                continue;
            };
            let Ok(timestamp) = Timestamp::now() else {
                continue;
            };
            if cfg!(target_os = "android") {
                // UsageStats survives our process being suspended/killed. Replay
                // while alive and again on app return, with one persisted cursor.
                if let Err(err) = crate::tracking::android::reconcile(
                    state.db().pool(),
                    state.device(),
                    timestamp.as_millis(),
                )
                .await
                {
                    eprintln!("android usage reconcile failed: {err}");
                }
                sync_presence_loop(&state, timestamp).await;
                continue;
            }
            // A failing poll must never kill the loop; the next tick may succeed.
            // It is still reported: `poll` advances the machine before persisting, so
            // a swallowed error hides a slice of playtime that will not be retried.
            if let Err(err) = state
                .tracker()
                .poll(state.db().pool(), state.device(), timestamp)
                .await
            {
                eprintln!("playtime poll failed: {err}");
            }
            sync_presence_loop(&state, timestamp).await;
        }
    });
}

/// What the presence loop reports: one game, and where in it.
struct ActivePresenceTarget {
    game_id: EntityId,
    chapter_id: Option<EntityId>,
    exe_name: Option<String>,
    active_ms: i64,
    start_epoch: Option<i64>,
    /// Whether the card is being shown for a focused game or a backgrounded one.
    phase: Phase,
}

/// The bare file name of a tracked executable.
///
/// The tracker reports a full path, which contains the user's Windows account
/// name. `{exe_name}` is a template token anyone can put on a public profile, so
/// only the file name may leave the machine. The split is by hand on both
/// separators because the path is a Windows path even where this is compiled
/// for another OS.
fn exe_file_name(path: &str) -> Option<String> {
    path.rsplit(['/', '\\'])
        .next()
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
}

/// Start of a session shown to Discord as "elapsed": now minus the time already
/// played, so the timer Discord draws matches the active time we track.
fn start_epoch(now: Timestamp, active_ms: i64) -> i64 {
    now.as_millis() / 1000 - active_ms / 1000
}

/// A running manual timer, if there is one.
///
/// Elapsed time comes from [`crate::tracking::manual::ManualRun::elapsed_at`]. The
/// `elapsed_ms` field only holds time up to the last pause, so reading it
/// directly froze the shown time at zero while the timer ran, and anchoring on
/// `started_at` counted paused time as played.
fn target_from_manual(
    manual: &crate::tracking::manual::ManualRun,
    now: Timestamp,
) -> Option<ActivePresenceTarget> {
    manual.is_running().then(|| {
        let elapsed = manual.elapsed_at(now);
        ActivePresenceTarget {
            game_id: manual.game_id,
            chapter_id: manual.chapter_id,
            exe_name: None,
            active_ms: elapsed,
            start_epoch: Some(start_epoch(now, elapsed)),
            phase: Phase::Tracking,
        }
    })
}

/// The automatically tracked game, while it is being played.
///
/// The phase is compared with [`phase_name`], the function that produced it. It
/// was once compared with a literal `"Tracking"` while the tracker reports
/// `"tracking"`, so an automatically tracked game never reached Discord.
///
/// `show_when_unfocused` widens what counts as "being played" from
/// [`Phase::Tracking`] to also include [`Phase::Unfocused`]. Note what it does
/// **not** widen to: `Paused` is the user asking for nothing to be recorded, and
/// `Idle` means the process is gone. Neither may publish, so the switch can never
/// advertise a novel that is closed or deliberately put down.
fn target_from_status(
    status: &TrackingStatus,
    now: Timestamp,
    show_when_unfocused: bool,
) -> Option<ActivePresenceTarget> {
    let phase = if status.phase == phase_name(Phase::Tracking) {
        Phase::Tracking
    } else if show_when_unfocused && status.phase == phase_name(Phase::Unfocused) {
        Phase::Unfocused
    } else {
        return None;
    };
    let game_id = EntityId::parse(status.game_id.as_deref()?).ok()?;
    Some(ActivePresenceTarget {
        game_id,
        chapter_id: status
            .chapter_id
            .as_deref()
            .and_then(|c| EntityId::parse(c).ok()),
        exe_name: status.exe_path.as_deref().and_then(exe_file_name),
        // While unfocused nothing accrues, so `active_ms` is frozen. That is
        // exactly what the card should show: the elapsed timer stops rather than
        // counting minutes the user was not reading.
        active_ms: status.active_ms,
        start_epoch: Some(start_epoch(now, status.active_ms)),
        phase,
    })
}

pub(crate) async fn sync_presence_now(state: &AppState) {
    if let Ok(timestamp) = Timestamp::now() {
        sync_presence_loop(state, timestamp).await;
    }
}

async fn sync_presence_loop(state: &AppState, now: Timestamp) {
    // While presence is off there is nothing to send, and no reason to query the
    // database every poll. Switching it off already cleared Discord.
    let presence_settings = state.presence().get_settings().await;
    if !presence_settings.enabled {
        return;
    }

    let pool = state.db().pool();
    let status = state.tracker().status(pool).await;
    let manual = state.tracker().manual_state().await;
    let target = manual
        .as_ref()
        .and_then(|run| target_from_manual(run, now))
        .or_else(|| target_from_status(&status, now, presence_settings.show_when_unfocused));

    #[cfg(target_os = "android")]
    let target = match target {
        Some(t) => Some(t),
        None => crate::tracking::android::active_target(pool, now.as_millis())
            .await
            .map(|active| ActivePresenceTarget {
                game_id: active.game_id,
                chapter_id: None,
                exe_name: Some(active.package),
                active_ms: active.active_ms,
                start_epoch: Some(active.start_epoch),
                // `active_target` answers only while the game is in the
                // foreground — the last counted interval is still fresh — so
                // Android has no equivalent of "running but another window has
                // focus" and this branch is not gated by the setting.
                phase: Phase::Tracking,
            }),
    };

    let Some(target) = target else {
        state.presence().clear();
        return;
    };
    let Ok(Some(game)) = queries::get_game(pool, target.game_id).await else {
        state.presence().clear();
        return;
    };

    let (chapter_name, route_name) = match target.chapter_id {
        Some(cid) => match queries::chapter_by_id(pool, cid).await {
            Ok(Some(chap)) => (
                Some(chap.display_name.unwrap_or(chap.name)),
                chap.route_name,
            ),
            _ => (None, None),
        },
        None => (None, None),
    };

    let mut base = crate::presence::context_for_game(pool, &game).await;
    // `{date}` / `{time}` are wall-clock display values, refreshed on every poll;
    // the debouncer only pushes an update when a template actually renders them.
    let (date, time) = crate::presence::local_date_time(now);
    // A configured image host turns a concealed or local cover into a hosted
    // URL the card can show; cached, so the common poll is a map lookup.
    if let Some(url) =
        crate::presence::ensure_rpc_image(state.data_dir(), &game, &presence_settings).await
    {
        base.rpc_image = Some(url);
    }
    state
        .presence()
        .update_context(vn_core::presence::PresenceContext {
            exe_name: target.exe_name.unwrap_or_default(),
            elapsed_time: vn_core::time_fmt::format_duration(target.active_ms),
            date,
            time,
            chapter: chapter_name,
            route: route_name,
            session_start_epoch: target.start_epoch,
            // Lets `{game_state}` say "paused" without the card vanishing, so the
            // reader can see the difference between "I tabbed away" and "the game
            // is not running".
            is_unfocused: target.phase == Phase::Unfocused,
            ..base
        });
}

#[cfg(test)]
mod presence_target_tests {
    use super::*;
    use crate::tracking::manual::ManualRun;
    use vn_core::ids::EntityId;

    fn status(phase: &str, game: Option<EntityId>) -> TrackingStatus {
        TrackingStatus {
            available: true,
            unavailable_reason: None,
            phase: phase.to_owned(),
            game_id: game.map(|g| g.to_string()),
            exe_path: Some(r"C:\Users\Atho\Games\Clannad\clannad.exe".to_owned()),
            pid: Some(1),
            active_ms: 90_000,
            afk_ms: 0,
            chapter_id: None,
            candidates: Vec::new(),
        }
    }

    fn at(secs: i64) -> Timestamp {
        Timestamp::from_millis(secs * 1000)
    }

    #[test]
    fn a_game_the_tracker_reports_as_tracking_reaches_presence() {
        // The tracker's own phase string, not a hand-written one: this is the
        // regression for presence never firing for automatic tracking.
        let tracking = phase_name(Phase::Tracking);
        let target = target_from_status(&status(tracking, Some(EntityId::new())), at(1_000), true)
            .expect("a tracked game must produce a target");
        assert_eq!(target.active_ms, 90_000);
        assert_eq!(target.start_epoch, Some(1_000 - 90));
        assert_eq!(target.phase, Phase::Tracking);
    }

    #[test]
    fn an_unfocused_game_publishes_only_when_the_setting_allows_it() {
        // The setting's whole point: alt-tabbing to a walkthrough must not make
        // the card vanish for someone who asked for it to stay.
        let unfocused = status(phase_name(Phase::Unfocused), Some(EntityId::new()));
        assert!(
            target_from_status(&unfocused, at(1_000), false).is_none(),
            "off must keep the strict behaviour"
        );
        let target = target_from_status(&unfocused, at(1_000), true)
            .expect("on must keep the card while the game runs");
        assert_eq!(target.phase, Phase::Unfocused);
    }

    #[test]
    fn the_setting_never_widens_to_paused_or_idle() {
        // Paused is the user asking for nothing to be recorded, and Idle means the
        // process is gone. Neither may publish even with the setting on, or the
        // card would advertise a novel that is closed or deliberately put down.
        for phase in [Phase::Idle, Phase::Paused] {
            let s = status(phase_name(phase), Some(EntityId::new()));
            assert!(
                target_from_status(&s, at(1_000), true).is_none(),
                "{phase:?} must not publish even when show_when_unfocused is on"
            );
        }
    }

    #[test]
    fn an_unfocused_game_stops_accruing_time_on_the_card() {
        // Nothing accrues while unfocused, so the elapsed timer must freeze rather
        // than count minutes the user was not reading.
        let mut unfocused = status(phase_name(Phase::Unfocused), Some(EntityId::new()));
        unfocused.active_ms = 90_000;
        let target = target_from_status(&unfocused, at(10_000), true).expect("target");
        assert_eq!(target.active_ms, 90_000);
        assert_eq!(target.start_epoch, Some(10_000 - 90));
    }

    #[test]
    fn a_missing_or_malformed_game_id_shows_nothing() {
        let tracking = phase_name(Phase::Tracking);
        assert!(target_from_status(&status(tracking, None), at(0), true).is_none());
        let mut bad = status(tracking, Some(EntityId::new()));
        bad.game_id = Some("not-a-uuid".to_owned());
        assert!(target_from_status(&bad, at(0), true).is_none());
    }

    #[test]
    fn only_the_executable_file_name_leaves_the_machine() {
        let tracking = phase_name(Phase::Tracking);
        let target = target_from_status(&status(tracking, Some(EntityId::new())), at(0), true)
            .expect("target");
        assert_eq!(target.exe_name.as_deref(), Some("clannad.exe"));
        assert_eq!(
            exe_file_name("/home/atho/games/vn.exe").as_deref(),
            Some("vn.exe")
        );
        assert_eq!(exe_file_name("bare.exe").as_deref(), Some("bare.exe"));
        assert_eq!(exe_file_name(r"C:\Users\Atho\"), None);
    }

    #[test]
    fn a_running_manual_timer_shows_its_live_elapsed_time() {
        let run = ManualRun::start(EntityId::new(), EntityId::new(), at(100));
        // 60 s into the stretch. `elapsed_ms` itself is still 0 here.
        let target = target_from_manual(&run, at(160)).expect("running timer");
        assert_eq!(run.elapsed_ms, 0);
        assert_eq!(target.active_ms, 60_000);
        assert_eq!(target.start_epoch, Some(100));
    }

    #[test]
    fn a_paused_manual_timer_does_not_count_the_pause_as_played() {
        let mut run = ManualRun::start(EntityId::new(), EntityId::new(), at(100));
        run.pause(at(160)).expect("pause");
        assert!(
            target_from_manual(&run, at(500)).is_none(),
            "paused shows nothing"
        );

        run.resume(at(500)).expect("resume");
        let target = target_from_manual(&run, at(530)).expect("running again");
        // 60 s before the pause + 30 s after, not 430 s since `started_at`.
        assert_eq!(target.active_ms, 90_000);
        assert_eq!(target.start_epoch, Some(530 - 90));
    }
}

#[cfg(test)]
mod capability_tests {
    use super::engine_for_module;

    #[test]
    fn a_verified_module_names_its_engine() {
        assert_eq!(
            engine_for_module("SiglusEngine.exe").as_deref(),
            Some("siglus")
        );
        assert_eq!(
            engine_for_module("siglusengine.EXE").as_deref(),
            Some("siglus")
        );
    }

    #[test]
    fn an_engine_with_no_verified_profile_names_nothing() {
        // Regression: the capability reported `memory_scene` as available on every
        // Windows game, because it only asked whether the platform allows reading
        // another process. A KiriKiri game then promised detection that
        // `profile_for` returns `None` for, on the first line of the reader.
        assert_eq!(engine_for_module("Kirikiri.exe"), None);
        assert_eq!(engine_for_module("game.exe"), None);
        assert_eq!(engine_for_module(""), None);
    }
}

/// The window-title parsing configuration, as the UI shows it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TitleConfigView {
    /// `off`, `siglus` or `custom`.
    pub format: String,
    /// The user's pattern, when set.
    pub pattern: Option<String>,
    /// A caption the user can test against, if a game is running.
    pub sample: Option<String>,
}

/// Read the title-parsing choice.
///
/// # Errors
/// Database failures.
#[tauri::command]
pub async fn tracking_title_config(state: State<'_, AppState>) -> CommandResult<TitleConfigView> {
    let settings = crate::tracking::title_config::load(state.db().pool()).await;
    Ok(TitleConfigView {
        format: settings.format.as_str().to_owned(),
        pattern: settings.pattern,
        // Offering the focused caption makes the tester useful without asking the
        // user to copy a window title by hand, which few can do.
        sample: state
            .tracker()
            .probe()
            .focused_window()
            .ok()
            .flatten()
            .map(|window| window.title),
    })
}

/// Save the title-parsing choice.
///
/// A custom pattern is compiled before it is stored. Refusing a broken one here
/// means the tracker never runs with a pattern that cannot match, which would be
/// indistinguishable from an engine that writes no titles.
///
/// # Errors
/// A message when the pattern is unusable.
#[tauri::command]
pub async fn tracking_save_title_config(
    state: State<'_, AppState>,
    format: String,
    pattern: Option<String>,
) -> CommandResult<TitleConfigView> {
    let format = vn_core::title_settings::TitleFormat::parse(&format);
    if format == vn_core::title_settings::TitleFormat::Custom {
        let source = pattern.as_deref().unwrap_or_default();
        vn_core::title_settings::CustomPattern::new(source)
            .map_err(|err| CommandError::new("bad_pattern", err.to_string()))?;
    }
    let settings = vn_core::title_settings::TitleSettings { format, pattern };
    crate::tracking::title_config::save(state.db().pool(), &settings).await;
    tracking_title_config(state).await
}

/// Test a pattern against captions without saving it.
///
/// # Errors
/// A message when the pattern is unusable, so the UI can say why instead of
/// reporting "no match" for a pattern that never compiled.
#[tauri::command]
#[allow(
    clippy::needless_pass_by_value,
    reason = "Tauri commands cannot take borrowed arguments; the macro requires owned values"
)]
pub fn tracking_test_title_pattern(
    pattern: String,
    titles: Vec<String>,
) -> CommandResult<Vec<vn_core::title_settings::PatternTest>> {
    vn_core::title_settings::test_pattern(&pattern, &titles)
        .map_err(|err| CommandError::new("bad_pattern", err.to_string()))
}

/// Derived statistics plus the time series for the requested period.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatsPayload {
    /// The headline figures.
    pub summary: vn_core::stats::DerivedStats,
    /// One entry per bucket, oldest first.
    pub series: Vec<vn_core::stats::TimeBucket>,
}

/// Derived library statistics.
///
/// Everything here comes from rows the app already stores. A figure with no data
/// behind it is `null` rather than zero, so the screen can say "not recorded"
/// instead of making a claim.
///
/// # Errors
/// Database failures.
#[tauri::command]
pub async fn stats_derived(
    state: State<'_, AppState>,
    period: String,
) -> CommandResult<StatsPayload> {
    use sqlx::Row as _;

    let pool = state.db().pool();

    let game_rows = sqlx::query(
        "SELECT g.status AS status, g.rating AS rating, g.finished_at AS finished_at, \
                COALESCE((SELECT SUM(ps.counted_ms) FROM play_sessions ps \
                          WHERE ps.game_id = g.id AND ps.deleted = 0), 0) AS playtime_ms \
         FROM games g WHERE g.deleted = 0",
    )
    .fetch_all(pool)
    .await
    .map_err(|err| CommandError::new("db", err.to_string()))?;

    let games: Vec<vn_core::stats::GameSample> = game_rows
        .iter()
        .map(|row| vn_core::stats::GameSample {
            status: row.get("status"),
            rating: row
                .get::<Option<i64>, _>("rating")
                .and_then(|value| u8::try_from(value).ok()),
            playtime_ms: row.get("playtime_ms"),
            finished_at: row.get("finished_at"),
        })
        .collect();

    // Joined to `games`: a hard-deleted game leaves its sessions behind, and an
    // unjoined sum made the stats page report hours of playtime for games that
    // no longer existed.
    let session_rows = sqlx::query(
        "SELECT ps.started_at, ps.counted_ms FROM play_sessions ps \
         JOIN games g ON g.id = ps.game_id AND g.deleted = 0 \
         WHERE ps.deleted = 0",
    )
    .fetch_all(pool)
    .await
    .map_err(|err| CommandError::new("db", err.to_string()))?;

    let sessions: Vec<vn_core::stats::SessionSample> = session_rows
        .iter()
        .map(|row| vn_core::stats::SessionSample {
            started_at: row.get("started_at"),
            counted_ms: row.get("counted_ms"),
        })
        .collect();

    // Route totals reuse the same sum the chapters panel shows, so the two
    // screens cannot disagree. Joined to `games` like every aggregate over a
    // child table (AGENTS.md §5): orphan rows — chapters whose game row is gone
    // — must not inflate route stats or `favourite_route`.
    let route_rows = sqlx::query(
        "SELECT c.route_name AS name, SUM(ps.counted_ms) AS playtime_ms \
         FROM chapters c \
         JOIN games g ON g.id = c.game_id AND g.deleted = 0 \
         JOIN play_sessions ps ON ps.chapter_id = c.id AND ps.deleted = 0 \
         WHERE c.deleted = 0 AND c.route_name IS NOT NULL \
         GROUP BY c.route_name",
    )
    .fetch_all(pool)
    .await
    .map_err(|err| CommandError::new("db", err.to_string()))?;

    let routes: Vec<vn_core::stats::RouteStat> = route_rows
        .iter()
        .map(|row| vn_core::stats::RouteStat {
            name: row.get("name"),
            playtime_ms: row.get("playtime_ms"),
        })
        .collect();

    let timestamp = now()?;
    Ok(StatsPayload {
        summary: vn_core::stats::derive(&games, &sessions, &routes, timestamp),
        series: vn_core::stats::time_series(
            &sessions,
            vn_core::stats::Period::parse(&period),
            timestamp,
        ),
    })
}
