//! Preference commands. Values are validated against the domain enums before
//! they are stored, so a bad string never reaches the UI.

use serde::Deserialize;
use tauri::{AppHandle, Emitter, State};
use vn_core::model::{NsfwMode, ThemeMode};

use super::{CommandError, CommandResult};
use crate::db::{models::PrefsSnapshot, queries};
use crate::state::AppState;

/// Read all preferences.
#[tauri::command]
pub async fn get_prefs(state: State<'_, AppState>) -> CommandResult<PrefsSnapshot> {
    Ok(queries::load_prefs(state.db().pool()).await?)
}

pub(crate) async fn require_beta(state: &AppState) -> CommandResult<()> {
    if !queries::load_prefs(state.db().pool()).await?.beta_features {
        return Err(CommandError::new(
            "beta_disabled",
            "Enable Beta features in Settings first.",
        ));
    }
    Ok(())
}

/// Hide experimental features without deleting their configuration or history.
#[tauri::command]
pub async fn set_beta_features(
    app: AppHandle,
    state: State<'_, AppState>,
    value: bool,
) -> CommandResult<PrefsSnapshot> {
    queries::set_pref(
        state.db().pool(),
        "beta_features",
        if value { "true" } else { "false" },
    )
    .await?;
    state.luna().apply_beta(state.db().pool(), value).await;
    // Notify every webview even if cleanup reports an error after the save.
    let _ = app.emit("prefs://beta-features", value);
    if !value {
        state.chat().cancel_all();
        let suspended = state.companion().suspend_beta(&state).await;
        #[cfg(desktop)]
        crate::companion::window::companion_hide_overlay(app).await?;
        suspended.map_err(|code| CommandError::new("companion", code))?;
    }
    Ok(queries::load_prefs(state.db().pool()).await?)
}

/// Set the theme mode: `dark`, `light` or `system`.
#[tauri::command]
pub async fn set_theme(state: State<'_, AppState>, value: String) -> CommandResult<PrefsSnapshot> {
    let mode = ThemeMode::parse(&value)
        .map_err(|_| CommandError::new("invalid_theme", format!("unknown theme: {value}")))?;
    queries::set_pref(state.db().pool(), "theme", mode.as_str()).await?;
    Ok(queries::load_prefs(state.db().pool()).await?)
}

/// Set the interface language tag.
#[tauri::command]
pub async fn set_language(
    state: State<'_, AppState>,
    value: String,
) -> CommandResult<PrefsSnapshot> {
    let supported = ["en", "id"];
    if !supported.contains(&value.as_str()) {
        return Err(CommandError::new(
            "invalid_language",
            format!("unsupported language: {value}"),
        ));
    }
    queries::set_pref(state.db().pool(), "language", &value).await?;
    Ok(queries::load_prefs(state.db().pool()).await?)
}

/// Set the NSFW art policy: `show`, `blur` or `hide`.
#[tauri::command]
pub async fn set_nsfw_mode(
    state: State<'_, AppState>,
    value: String,
) -> CommandResult<PrefsSnapshot> {
    let mode = NsfwMode::parse(&value)
        .map_err(|_| CommandError::new("invalid_nsfw", format!("unknown mode: {value}")))?;
    queries::set_pref(state.db().pool(), "nsfw", mode.as_str()).await?;
    Ok(queries::load_prefs(state.db().pool()).await?)
}

/// Turn the reduced-effects rendering mode on or off.
#[tauri::command]
pub async fn set_reduce_effects(
    state: State<'_, AppState>,
    value: bool,
) -> CommandResult<PrefsSnapshot> {
    queries::set_pref(
        state.db().pool(),
        "reduce_effects",
        if value { "true" } else { "false" },
    )
    .await?;
    Ok(queries::load_prefs(state.db().pool()).await?)
}

/// Backup payload describing the current preferences, used by import/export.
#[derive(Debug, Deserialize)]
pub struct PrefsInput {
    /// Theme key.
    pub theme: String,
    /// Language tag.
    pub language: String,
    /// NSFW mode key.
    pub nsfw: String,
}

/// Apply a validated set of preferences at once (used by backup import).
pub async fn apply_prefs(state: &AppState, input: &PrefsInput) -> Result<(), CommandError> {
    let theme = ThemeMode::parse(&input.theme)
        .map_err(|_| CommandError::new("invalid_theme", "unknown theme"))?;
    let nsfw = NsfwMode::parse(&input.nsfw)
        .map_err(|_| CommandError::new("invalid_nsfw", "unknown nsfw mode"))?;
    if !["en", "id"].contains(&input.language.as_str()) {
        return Err(CommandError::new(
            "invalid_language",
            "unsupported language",
        ));
    }
    let pool = state.db().pool();
    queries::set_pref(pool, "theme", theme.as_str()).await?;
    queries::set_pref(pool, "nsfw", nsfw.as_str()).await?;
    queries::set_pref(pool, "language", &input.language).await?;
    Ok(())
}
