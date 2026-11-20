//! Tauri commands for custom reading goals.

use tauri::State;
use vn_core::ids::EntityId;

use crate::commands::CommandResult;
use crate::db::goals::{self, ReadingGoalView, SaveReadingGoalInput};
use crate::state::AppState;

/// List live reading goals with their computed progress values.
///
/// # Errors
/// Returns [`CommandError`] on database failure.
#[tauri::command]
pub async fn reading_goals_list(state: State<'_, AppState>) -> CommandResult<Vec<ReadingGoalView>> {
    Ok(goals::list(state.db().pool()).await?)
}

/// Create or update a reading goal.
///
/// # Errors
/// Returns [`CommandError`] on validation, conflict, or database failure.
#[tauri::command]
pub async fn reading_goal_save(
    state: State<'_, AppState>,
    input: SaveReadingGoalInput,
) -> CommandResult<ReadingGoalView> {
    goals::save(state.db().pool(), state.device(), input).await
}

/// Pin or unpin a reading goal.
///
/// # Errors
/// Returns [`CommandError`] if the goal is missing or the write fails.
#[tauri::command]
pub async fn reading_goal_set_pinned(
    state: State<'_, AppState>,
    id: EntityId,
    pinned: bool,
) -> CommandResult<ReadingGoalView> {
    goals::set_pinned(state.db().pool(), state.device(), id, pinned).await
}

/// Set the manual progress value of a `Manual` reading goal.
///
/// # Errors
/// Returns [`CommandError`] if the goal is missing, not manual, or invalid.
#[tauri::command]
pub async fn reading_goal_set_progress(
    state: State<'_, AppState>,
    id: EntityId,
    progress: i64,
) -> CommandResult<ReadingGoalView> {
    goals::set_manual_progress(state.db().pool(), state.device(), id, progress).await
}

/// Soft-delete a reading goal.
///
/// # Errors
/// Returns [`CommandError`] if the goal is missing or already deleted.
#[tauri::command]
pub async fn reading_goal_delete(state: State<'_, AppState>, id: EntityId) -> CommandResult<()> {
    goals::delete(state.db().pool(), state.device(), id).await
}
