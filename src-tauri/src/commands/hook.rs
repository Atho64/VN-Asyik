//! The live text ring, as the UI and the companion chat see it.
//!
//! Text comes from the `LunaTranslator` network service (see `tracking/luna.rs`).
//! There is no attach command because nothing is ever loaded into a game: the
//! app reads a local websocket the user enabled, and that is all.

use serde::Serialize;

use crate::commands::CommandResult;
use crate::tracking::hook::{CapturedLine, HookStatus};

/// Current ring state plus the captured lines.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HookView {
    #[serde(flatten)]
    pub status: HookStatus,
    pub lines: Vec<CapturedLine>,
}

/// The recent live text, for the settings preview and the chat tool.
#[tauri::command]
pub async fn hook_status(
    state: tauri::State<'_, crate::state::AppState>,
) -> CommandResult<HookView> {
    let host = state.hook();
    Ok(HookView {
        status: host.status().await,
        lines: host.lines().await,
    })
}
