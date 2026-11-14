//! Script IPC: import user-extracted scripts, list/delete them, and run
//! compaction steps against the configured chat provider.
use super::{CommandError, CommandResult};
use crate::{script, state::AppState};
use tauri::State;

fn error(code: String) -> CommandError {
    CommandError::new("script", code)
}

/// Import script file(s) for a game, auto-detecting plain text, VNTP `[{name, message}]` JSON, or Asyik JSON.
/// Multiple files merge in natural filename order so split scripts import in one pick.
#[tauri::command]
pub async fn script_import(
    state: State<'_, AppState>,
    game_id: String,
    paths: Vec<String>,
) -> CommandResult<script::ScriptDocumentInfo> {
    script::import(state.db().pool(), &game_id, &paths)
        .await
        .map_err(error)
}

#[tauri::command]
pub async fn script_list(
    state: State<'_, AppState>,
    game_id: Option<String>,
) -> CommandResult<Vec<script::ScriptDocumentInfo>> {
    script::list(state.db().pool(), game_id.as_deref())
        .await
        .map_err(error)
}

#[tauri::command]
pub async fn script_delete(state: State<'_, AppState>, id: String) -> CommandResult<()> {
    script::delete(state.db().pool(), &id).await.map_err(error)
}

/// Summarize up to `max_chunks` pending chunks per agent and reduce into a digest once complete (resumable; `rebuild` clears stored progress).
/// Uses the shared chat provider settings (local endpoints work without an API key).
#[tauri::command]
pub async fn script_compact(
    state: State<'_, AppState>,
    id: String,
    max_chunks: Option<u32>,
    rebuild: Option<bool>,
) -> CommandResult<script::CompactionInfo> {
    let pool = state.db().pool().clone();
    let chat = crate::ai::profiles::resolve(&pool).await;
    let config = chat.config;
    // The active saved profile already resolved its key, off the UI thread.
    let key = chat.key;
    if key.is_none() && !crate::commands::ai::is_local_endpoint(&config.base_url) {
        return Err(error("chat_key_missing".into()));
    }
    let client = provider_client()?;
    let agents = script::load_agent_settings(&pool)
        .await
        .map_err(error)?
        .agents;
    script::compact(
        &pool,
        &client,
        &config,
        key.as_ref(),
        &id,
        script::RunOptions {
            step: max_chunks.unwrap_or(script::MAX_CHUNKS_PER_CALL),
            agents,
            rebuild: rebuild.unwrap_or(false),
        },
    )
    .await
    .map_err(error)
}

/// How many summarizer agents compaction runs in parallel.
///
/// # Errors
/// `db_error` when the store cannot be read, `agent_settings_invalid` when the
/// stored blob does not parse (re-saving the setting rewrites it).
#[tauri::command]
pub async fn script_agent_settings(
    state: State<'_, AppState>,
) -> CommandResult<script::AgentSettings> {
    script::load_agent_settings(state.db().pool())
        .await
        .map_err(error)
}

/// Normalize (1..=10), persist and return the agent settings. A change takes
/// effect on the next compaction run: an in-flight one keeps resuming against
/// its stored agent layout until it finishes.
#[tauri::command]
pub async fn script_save_agent_settings(
    state: State<'_, AppState>,
    settings: script::AgentSettings,
) -> CommandResult<script::AgentSettings> {
    script::save_agent_settings(state.db().pool(), settings)
        .await
        .map_err(error)
}

fn provider_client() -> Result<reqwest::Client, CommandError> {
    crate::vndb::http_client_builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        // One map call summarizes one chunk; 90 s matches the chat transport's
        // patience so slow local models are not cut off mid-summary.
        .timeout(std::time::Duration::from_secs(90))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| error("provider_offline".into()))
}
