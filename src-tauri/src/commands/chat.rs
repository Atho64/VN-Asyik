//! Chat IPC. Settings remain shared with the existing AI provider.
use super::{CommandError, CommandResult};
use crate::{
    ai::conversation::{self, ChatEvent, ChatMessage, ChatThread, store},
    state::AppState,
};
use tauri::State;
use vn_core::chat::ChatOptions;
fn error(code: String) -> CommandError {
    CommandError::new("chat", code)
}

#[tauri::command]
pub async fn chat_list(state: State<'_, AppState>) -> CommandResult<Vec<ChatThread>> {
    store::list(state.db().pool()).await.map_err(error)
}
#[tauri::command]
pub async fn chat_create(
    state: State<'_, AppState>,
    options: ChatOptions,
) -> CommandResult<ChatThread> {
    store::create(state.db().pool(), options)
        .await
        .map_err(error)
}
#[tauri::command]
pub async fn chat_messages(
    state: State<'_, AppState>,
    id: String,
) -> CommandResult<Vec<ChatMessage>> {
    store::get(state.db().pool(), &id).await.map_err(error)?;
    store::messages(state.db().pool(), &id).await.map_err(error)
}
#[tauri::command]
pub async fn chat_update(
    state: State<'_, AppState>,
    id: String,
    title: String,
    options: ChatOptions,
) -> CommandResult<ChatThread> {
    let (_lease, _) = state.chat().enter(&id).map_err(error)?;
    options.validate().map_err(error)?;
    if title.chars().count() > 120 {
        return Err(error("invalid_title".into()));
    }
    let old = store::get(state.db().pool(), &id).await.map_err(error)?;
    // Scope/spoilers are fixed after the first message: older answers would leak
    // information when narrowing scope or reducing the spoiler level.
    let history = store::messages(state.db().pool(), &id)
        .await
        .map_err(error)?;
    if !history.is_empty()
        && (old.options.game_id != options.game_id
            || old.options.spoilers != options.spoilers
            || old.options.boundary != options.boundary)
    {
        return Err(error("new_chat_for_context".into()));
    }
    sqlx::query("UPDATE chat_threads SET title=?,options_json=?,updated_at=? WHERE id=?")
        .bind(title.trim())
        .bind(serde_json::to_string(&options).map_err(|_| error("invalid_options".into()))?)
        .bind(store::now())
        .bind(&id)
        .execute(state.db().pool())
        .await
        .map_err(|_| error("db_error".into()))?;
    store::get(state.db().pool(), &id).await.map_err(error)
}
#[tauri::command]
pub async fn chat_delete(state: State<'_, AppState>, id: String) -> CommandResult<()> {
    let (_lease, _) = state.chat().enter(&id).map_err(error)?;
    let mut tx = state
        .db()
        .pool()
        .begin()
        .await
        .map_err(|_| error("db_error".into()))?;
    sqlx::query("DELETE FROM chat_messages WHERE thread_id=?")
        .bind(&id)
        .execute(&mut *tx)
        .await
        .map_err(|_| error("db_error".into()))?;
    sqlx::query("DELETE FROM chat_threads WHERE id=?")
        .bind(&id)
        .execute(&mut *tx)
        .await
        .map_err(|_| error("db_error".into()))?;
    tx.commit().await.map_err(|_| error("db_error".into()))?;
    Ok(())
}
#[tauri::command]
pub async fn chat_send(
    state: State<'_, AppState>,
    id: String,
    text: String,
    retry: bool,
    on_event: tauri::ipc::Channel<ChatEvent>,
) -> CommandResult<ChatMessage> {
    conversation::send(&state, &id, &text, retry, &on_event)
        .await
        .map_err(error)
}
#[tauri::command]
pub async fn chat_cancel(state: State<'_, AppState>, id: String) -> CommandResult<()> {
    state.chat().cancel(&id).map_err(error)
}
