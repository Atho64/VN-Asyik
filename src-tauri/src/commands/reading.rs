//! Reading checkpoint commands. Journal writes never alter playtime or save files.

use tauri::State;
use vn_core::{EntityId, reading::ReadingEntry};

use super::CommandResult;
use crate::{db::reading, state::AppState};

#[tauri::command]
pub async fn reading_context(
    state: State<'_, AppState>,
    game_id: String,
    offset: Option<i64>,
) -> CommandResult<reading::ReadingContext> {
    reading::context(
        state.db().pool(),
        EntityId::parse(&game_id)?,
        offset.unwrap_or(0),
    )
    .await
}

#[tauri::command]
pub async fn reading_session_context(
    state: State<'_, AppState>,
    game_id: String,
    session_id: String,
    offset: Option<i64>,
) -> CommandResult<reading::SessionReadingContext> {
    reading::session_context(
        state.db().pool(),
        EntityId::parse(&game_id)?,
        EntityId::parse(&session_id)?,
        offset.unwrap_or(0),
    )
    .await
}

#[tauri::command]
pub async fn reading_save(
    state: State<'_, AppState>,
    input: reading::SaveReadingInput,
) -> CommandResult<ReadingEntry> {
    reading::save(state.db().pool(), state.device(), input).await
}

#[tauri::command]
pub async fn reading_delete(
    state: State<'_, AppState>,
    game_id: String,
    id: String,
    expected_updated_at: i64,
) -> CommandResult<()> {
    reading::delete(
        state.db().pool(),
        state.device(),
        EntityId::parse(&game_id)?,
        EntityId::parse(&id)?,
        expected_updated_at,
    )
    .await
}
