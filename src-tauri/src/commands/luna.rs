//! Luna IPC: settings, live status, and a one-shot reachability probe.
use super::{CommandError, CommandResult};
use crate::{
    state::AppState,
    tracking::luna::{LunaSettings, LunaStatus},
};
use tauri::State;

fn error(code: String) -> CommandError {
    CommandError::new("luna", code)
}

#[tauri::command]
pub async fn luna_settings(state: State<'_, AppState>) -> CommandResult<LunaSettings> {
    Ok(state.luna().settings())
}

/// Validate, persist and apply. The monitor reconnects (or disconnects)
/// immediately; no restart is needed.
#[tauri::command]
pub async fn luna_save_settings(
    state: State<'_, AppState>,
    settings: LunaSettings,
) -> CommandResult<LunaSettings> {
    state
        .luna()
        .save(state.db().pool(), settings)
        .await
        .map_err(error)
}

#[tauri::command]
pub async fn luna_status(state: State<'_, AppState>) -> CommandResult<LunaStatus> {
    Ok(state.luna().status().await)
}

/// One-shot probe of a base URL (the saved one, or a candidate the user is
/// typing). Any HTTP response means `LunaTranslator`'s service is listening —
/// including error statuses, since a listener answering is what we want to
/// know before enabling.
#[tauri::command]
pub async fn luna_test(
    state: State<'_, AppState>,
    base_url: Option<String>,
) -> CommandResult<crate::tracking::luna::LunaProbe> {
    let candidate = base_url.unwrap_or_else(|| state.luna().settings().base_url);
    let settings = LunaSettings {
        enabled: true,
        base_url: candidate,
    };
    let probe = crate::tracking::luna::probe(&settings).await;
    Ok(probe)
}
