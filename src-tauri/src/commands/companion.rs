//! Thin companion IPC. A send acknowledges dispatch; Rust owns the work.
use super::{CommandError, CommandResult};
use crate::{
    ai::conversation::ChatMessage,
    companion::{
        Action, Snapshot,
        store::{self, Session},
    },
    state::AppState,
};
use tauri::{AppHandle, Manager, State};
use vn_core::companion::SessionOptions;

fn error(code: String) -> CommandError {
    CommandError::new("companion", code)
}

#[tauri::command]
pub async fn companion_profiles(
    state: State<'_, AppState>,
) -> CommandResult<crate::companion::profiles::Catalogue> {
    crate::companion::profiles::list(state.db().pool())
        .await
        .map_err(error)
}

#[tauri::command]
pub async fn companion_save_profile(
    state: State<'_, AppState>,
    id: Option<String>,
    expected_version: Option<i32>,
    settings: vn_core::companion::profile::ProfileSettings,
) -> CommandResult<vn_core::companion::profile::ProfileSnapshot> {
    super::prefs::require_beta(&state).await?;
    crate::companion::profiles::save(state.db().pool(), id.as_deref(), expected_version, settings)
        .await
        .map_err(error)
}

#[tauri::command]
pub async fn companion_delete_profile(
    state: State<'_, AppState>,
    id: String,
    version: i32,
) -> CommandResult<()> {
    super::prefs::require_beta(&state).await?;
    crate::companion::profiles::delete(state.db().pool(), &id, version)
        .await
        .map_err(error)
}

#[tauri::command]
pub async fn companion_apply_profile(
    state: State<'_, AppState>,
    id: String,
    profile: vn_core::companion::profile::ProfileSnapshot,
) -> CommandResult<Snapshot> {
    super::prefs::require_beta(&state).await?;
    state
        .companion()
        .apply_profile(&state, &id, profile)
        .await
        .map_err(error)
}

#[tauri::command]
pub async fn companion_snapshot(state: State<'_, AppState>) -> CommandResult<Snapshot> {
    state.companion().snapshot().map_err(error)
}

#[tauri::command]
pub async fn companion_start(
    state: State<'_, AppState>,
    id: String,
    options: SessionOptions,
) -> CommandResult<Snapshot> {
    super::prefs::require_beta(&state).await?;
    state
        .companion()
        .start(&state, &id, options)
        .await
        .map_err(error)
}

#[tauri::command]
pub async fn companion_control(
    state: State<'_, AppState>,
    id: String,
    action: Action,
) -> CommandResult<Snapshot> {
    // Resume needs Beta; Pause and End stay reachable while Beta is off so a
    // disabled session can still be torn down (AGENTS section 9c).
    if matches!(action, Action::Resume) {
        super::prefs::require_beta(&state).await?;
    }
    state
        .companion()
        .control(&state, &id, action)
        .await
        .map_err(error)
}

#[tauri::command]
pub async fn companion_send(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    request_id: String,
    text: String,
) -> CommandResult<Snapshot> {
    super::prefs::require_beta(&state).await?;
    if let Some(work) = state
        .companion()
        .prepare(&state, &id, &request_id, &text)
        .await
        .map_err(error)?
    {
        tauri::async_runtime::spawn(async move {
            let state = app.state::<AppState>();
            state.companion().run(&state, work).await;
        });
    }
    state.companion().snapshot().map_err(error)
}

#[tauri::command]
pub async fn companion_list(state: State<'_, AppState>) -> CommandResult<Vec<Session>> {
    store::list(state.db().pool()).await.map_err(error)
}

#[tauri::command]
pub async fn companion_messages(
    state: State<'_, AppState>,
    id: String,
) -> CommandResult<Vec<ChatMessage>> {
    store::messages(state.db().pool(), &id).await.map_err(error)
}

#[tauri::command]
pub async fn companion_delete(state: State<'_, AppState>, id: String) -> CommandResult<()> {
    super::prefs::require_beta(&state).await?;
    state.companion().delete(&state, &id).await.map_err(error)
}

#[tauri::command]
pub async fn companion_memory_view(
    state: State<'_, AppState>,
    session: Option<String>,
) -> CommandResult<crate::companion::memory::View> {
    state
        .companion()
        .memory_view(&state, session.as_deref())
        .await
        .map_err(error)
}

#[tauri::command]
pub async fn companion_memory_save(
    state: State<'_, AppState>,
    request: crate::companion::memory::Save,
) -> CommandResult<vn_core::companion::memory::Memory> {
    super::prefs::require_beta(&state).await?;
    state
        .companion()
        .memory_save(&state, request)
        .await
        .map_err(error)
}

#[tauri::command]
pub async fn companion_memory_forget(
    state: State<'_, AppState>,
    target: crate::companion::memory::Forget,
    revision: i64,
) -> CommandResult<()> {
    super::prefs::require_beta(&state).await?;
    state
        .companion()
        .memory_forget(&state, target, revision)
        .await
        .map_err(error)
}

#[tauri::command]
pub async fn companion_memory_settings(
    state: State<'_, AppState>,
    retrieval: bool,
    automatic: bool,
    revision: i64,
) -> CommandResult<()> {
    super::prefs::require_beta(&state).await?;
    state
        .companion()
        .memory_settings(&state, retrieval, automatic, revision)
        .await
        .map_err(error)
}

#[tauri::command]
pub async fn companion_memory_sources(
    state: State<'_, AppState>,
    id: String,
    version: i64,
) -> CommandResult<Vec<crate::companion::memory::Source>> {
    crate::companion::memory::sources(state.db().pool(), &id, version)
        .await
        .map_err(error)
}

#[tauri::command]
pub async fn companion_recap(
    app: AppHandle,
    state: State<'_, AppState>,
    session: String,
) -> CommandResult<()> {
    super::prefs::require_beta(&state).await?;
    let id = state
        .companion()
        .suggest_recap(&state, &session)
        .await
        .map_err(error)?;
    start_summary(app, &state, Some(&id)).await
}

#[tauri::command]
pub async fn companion_recap_resume(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> CommandResult<()> {
    super::prefs::require_beta(&state).await?;
    start_summary(app, &state, Some(&id)).await
}

#[tauri::command]
pub async fn companion_recap_review(
    state: State<'_, AppState>,
    id: String,
    body: Option<String>,
    pinned: bool,
    revision: i64,
) -> CommandResult<()> {
    super::prefs::require_beta(&state).await?;
    state
        .companion()
        .review_recap(&state, &id, body.as_deref(), pinned, revision)
        .await
        .map_err(error)
}

async fn start_summary(app: AppHandle, state: &AppState, id: Option<&str>) -> CommandResult<()> {
    super::prefs::require_beta(state).await?;
    if let Some(work) = state
        .companion()
        .prepare_summary(state, id)
        .await
        .map_err(error)?
    {
        tauri::async_runtime::spawn(async move {
            let state = app.state::<AppState>();
            state.companion().run_summary(&state, work).await;
        });
    }
    Ok(())
}

/// The native timer continues while the main page is hidden or disconnected.
pub fn spawn_monitor(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            let state = app.state::<AppState>();
            if super::prefs::require_beta(&state).await.is_err() {
                if state.companion().snapshot().is_ok_and(|snapshot| {
                    matches!(
                        snapshot.lifecycle,
                        vn_core::companion::Lifecycle::Active
                            | vn_core::companion::Lifecycle::Starting
                    )
                }) {
                    let _ = state.companion().suspend_beta(&state).await;
                }
                // Nothing below this line needs 500 ms reactivity while Beta
                // is off; idle at a slower rate until the pref turns on.
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                continue;
            }
            let _ = state.companion().refresh(&state).await;
            if let Ok(Some(work)) = state.companion().vision_tick(&state).await {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    let state = app.state::<AppState>();
                    state.companion().run_visual(&state, work).await;
                });
            }
            let _ = start_summary(app.clone(), &state, None).await;
        }
    });
}

#[tauri::command]
pub async fn companion_vision_windows(
    state: State<'_, AppState>,
) -> CommandResult<Vec<crate::companion::vision::Window>> {
    super::prefs::require_beta(&state).await?;
    state.companion().vision_windows().map_err(error)
}
#[tauri::command]
pub async fn companion_vision_select(
    state: State<'_, AppState>,
    id: String,
    window: String,
    crop: vn_core::companion::vision::Crop,
) -> CommandResult<crate::companion::vision::Preview> {
    super::prefs::require_beta(&state).await?;
    state
        .companion()
        .vision_select(&state, &id, &window, crop)
        .await
        .map_err(error)
}
#[tauri::command]
pub async fn companion_vision_preview(
    state: State<'_, AppState>,
    id: String,
    generation: u64,
) -> CommandResult<crate::companion::vision::Preview> {
    super::prefs::require_beta(&state).await?;
    state
        .companion()
        .vision_preview(&id, generation)
        .await
        .map_err(error)
}
#[tauri::command]
pub async fn companion_vision_probe(
    state: State<'_, AppState>,
    id: String,
) -> CommandResult<Snapshot> {
    super::prefs::require_beta(&state).await?;
    state
        .companion()
        .vision_probe(&state, &id)
        .await
        .map_err(error)
}
#[tauri::command]
pub async fn companion_vision_control(
    state: State<'_, AppState>,
    id: String,
    action: String,
    generation: u64,
    preview: Option<String>,
) -> CommandResult<Snapshot> {
    // "watch" arms capture and needs Beta; the release actions stay reachable
    // while Beta is off so capture can be torn down (AGENTS section 9c).
    if action == "watch" {
        super::prefs::require_beta(&state).await?;
    }
    state
        .companion()
        .vision_control(&state, &id, &action, generation, preview.as_deref())
        .await
        .map_err(error)
}
#[tauri::command]
pub async fn companion_vision_send(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    preview: String,
    text: String,
) -> CommandResult<Snapshot> {
    super::prefs::require_beta(&state).await?;
    if let Some(work) = state
        .companion()
        .prepare_visual(&state, &id, Some(&preview), &text, false)
        .await
        .map_err(error)?
    {
        tauri::async_runtime::spawn(async move {
            let state = app.state::<AppState>();
            state.companion().run_visual(&state, work).await;
        });
    }
    state.companion().snapshot().map_err(error)
}
