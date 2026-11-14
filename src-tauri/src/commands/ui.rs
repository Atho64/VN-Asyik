//! UI presentation and desktop window launch preference commands.

use serde::Deserialize;
use tauri::State;
#[cfg(desktop)]
use vn_core::ui_prefs::StartupPrefsPatch;
use vn_core::ui_prefs::{UiPrefs, UiPrefsPatch};
#[cfg(desktop)]
use vn_core::window::{WindowLaunchMode, WindowLaunchPrefs};

use crate::commands::{CommandError, CommandResult};
use crate::db::queries;
use crate::state::AppState;
use crate::window_launch::WindowLaunchView;

#[cfg(desktop)]
use tauri::Manager;

/// Input payload for updating desktop launch position preferences.
#[derive(Debug, Deserialize)]
pub struct WindowLaunchInput {
    /// Launch positioning mode (`centered` or `customized`).
    pub mode: vn_core::window::WindowLaunchMode,
    /// Optional custom top-left X coordinate (physical pixels).
    pub x: Option<i32>,
    /// Optional custom top-left Y coordinate (physical pixels).
    pub y: Option<i32>,
    /// When `true`, also position the currently open main window immediately.
    #[serde(default)]
    pub apply_now: bool,
}

/// Load persisted UI shell and library startup preferences.
#[tauri::command]
pub async fn ui_prefs_get(state: State<'_, AppState>) -> CommandResult<UiPrefs> {
    queries::load_ui_prefs(state.db().pool())
        .await
        .map_err(Into::into)
}

/// Apply a partial update to persisted UI shell and library startup preferences.
#[tauri::command]
pub async fn ui_prefs_set(
    state: State<'_, AppState>,
    patch: UiPrefsPatch,
) -> CommandResult<UiPrefs> {
    let pool = state.db().pool();
    let mut prefs = queries::load_ui_prefs(pool).await?;
    prefs.apply_patch(patch);
    queries::save_ui_prefs(pool, &prefs).await?;
    Ok(prefs)
}

/// Read the desktop window launch position preference and live main-window coordinates.
#[tauri::command]
pub async fn window_launch_get(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> CommandResult<WindowLaunchView> {
    #[cfg(not(desktop))]
    {
        let _ = (app, state);
        return Ok(WindowLaunchView {
            mode: vn_core::window::WindowLaunchMode::Centered,
            x: None,
            y: None,
            current_x: None,
            current_y: None,
            supported: false,
        });
    }

    #[cfg(desktop)]
    {
        let prefs = queries::load_window_launch_prefs(state.db().pool()).await?;
        let tracker = app.state::<crate::window_launch::WindowLaunchTracker>();
        tracker.sync_from_prefs(&prefs);
        let live = app
            .get_webview_window("main")
            .and_then(|win| crate::window_launch::live_position(&win));
        Ok(WindowLaunchView {
            mode: prefs.mode,
            x: prefs.x,
            y: prefs.y,
            current_x: live.map(|(x, _)| x),
            current_y: live.map(|(_, y)| y),
            supported: true,
        })
    }
}

/// Update the desktop window launch position preference.
#[tauri::command]
pub async fn window_launch_set(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    input: WindowLaunchInput,
) -> CommandResult<WindowLaunchView> {
    #[cfg(not(desktop))]
    {
        let _ = (app, state, input);
        return Err(CommandError::new(
            "unsupported",
            "Window positioning is unsupported on this platform.",
        ));
    }

    #[cfg(desktop)]
    {
        let pool = state.db().pool();
        let previous = queries::load_window_launch_prefs(pool).await?;
        let main = app.get_webview_window("main");
        let live = main.as_ref().and_then(crate::window_launch::live_position);

        let (x, y) = match input.mode {
            WindowLaunchMode::Centered => (previous.x, previous.y),
            WindowLaunchMode::Customized => {
                let x = input.x.or(previous.x).or(live.map(|(lx, _)| lx));
                let y = input.y.or(previous.y).or(live.map(|(_, ly)| ly));
                (x, y)
            }
        };

        let next = WindowLaunchPrefs {
            mode: input.mode,
            x,
            y,
        };

        let tracker = app.state::<crate::window_launch::WindowLaunchTracker>();
        if let Some(window) = main.as_ref() {
            crate::window_launch::apply_window_setting(window, &tracker, &next, input.apply_now)
                .map_err(|msg| CommandError::new("invalid_position", msg))?;
        } else {
            tracker.sync_from_prefs(&next);
        }

        queries::save_window_launch_prefs(pool, &next).await?;
        let refreshed_live = main.as_ref().and_then(crate::window_launch::live_position);

        Ok(WindowLaunchView {
            mode: next.mode,
            x: next.x,
            y: next.y,
            current_x: refreshed_live.map(|(lx, _)| lx),
            current_y: refreshed_live.map(|(_, ly)| ly),
            supported: true,
        })
    }
}

/// Capture the current main window coordinates as the customized launch position.
#[tauri::command]
pub async fn window_launch_capture(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> CommandResult<WindowLaunchView> {
    #[cfg(not(desktop))]
    {
        let _ = (app, state);
        return Err(CommandError::new(
            "unsupported",
            "Window positioning is unsupported on this platform.",
        ));
    }

    #[cfg(desktop)]
    {
        let window = app
            .get_webview_window("main")
            .ok_or_else(|| CommandError::new("no_window", "Main window is not open."))?;
        let (x, y) = crate::window_launch::live_position(&window).ok_or_else(|| {
            CommandError::new(
                "window_minimized",
                "Restore the window before capturing its position.",
            )
        })?;

        let next = WindowLaunchPrefs {
            mode: WindowLaunchMode::Customized,
            x: Some(x),
            y: Some(y),
        };
        let tracker = app.state::<crate::window_launch::WindowLaunchTracker>();
        crate::window_launch::apply_window_setting(&window, &tracker, &next, false)
            .map_err(|msg| CommandError::new("invalid_position", msg))?;
        queries::save_window_launch_prefs(state.db().pool(), &next).await?;

        Ok(WindowLaunchView {
            mode: next.mode,
            x: next.x,
            y: next.y,
            current_x: Some(x),
            current_y: Some(y),
            supported: true,
        })
    }
}

/// One startup-behaviour read: the saved prefs plus whether this platform
/// acts on them at all.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartupPrefsView {
    /// Launch presentation and close action.
    pub prefs: vn_core::ui_prefs::StartupPrefs,
    /// Whether this platform applies startup modes and the autostart toggle.
    pub supported: bool,
    /// Whether the OS currently launches this app at login. `None` where the
    /// answer is unavailable.
    pub autostart: Option<bool>,
}

/// Read the startup-mode prefs and the OS autostart registration.
///
/// # Errors
/// `db_error` for the prefs, `unsupported` where the platform has neither.
#[tauri::command]
pub async fn startup_prefs_get(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> CommandResult<StartupPrefsView> {
    let prefs = queries::load_startup_prefs(state.db().pool()).await?;
    #[cfg(not(desktop))]
    {
        let _ = app;
        return Ok(StartupPrefsView {
            prefs,
            supported: false,
            autostart: None,
        });
    }

    #[cfg(desktop)]
    {
        use tauri_plugin_autostart::ManagerExt as _;
        let autostart = app
            .autolaunch()
            .is_enabled()
            .map_err(|err| CommandError::new("autostart", err.to_string()))?;
        Ok(StartupPrefsView {
            prefs,
            supported: true,
            autostart: Some(autostart),
        })
    }
}

/// Input payload for the startup-behaviour prefs.
#[derive(Debug, Deserialize)]
pub struct StartupPrefsInput {
    /// Optional launch-mode change.
    pub startup_mode: Option<vn_core::ui_prefs::StartupMode>,
    /// Optional close-action change.
    pub close_to_tray: Option<bool>,
    /// Optional autostart change; applied to the OS when given.
    pub autostart: Option<bool>,
}

/// Update the startup-mode prefs, the close action and the autostart toggle.
///
/// # Errors
/// `db_error` for the prefs, `unsupported` where the platform has neither,
/// `autostart` when the OS registration fails.
#[tauri::command]
pub async fn startup_prefs_set(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    input: StartupPrefsInput,
) -> CommandResult<StartupPrefsView> {
    #[cfg(not(desktop))]
    {
        let _ = (app, state, input);
        // A phone has neither a tray nor an OS autostart to toggle; the
        // settings section is hidden there and the read reports the same.
        return Err(CommandError::new(
            "unsupported",
            "Startup behaviour is unsupported on this platform.",
        ));
    }

    #[cfg(desktop)]
    {
        use tauri_plugin_autostart::ManagerExt as _;
        let pool = state.db().pool();
        let mut prefs = queries::load_startup_prefs(pool).await?;
        prefs.apply_patch(StartupPrefsPatch {
            startup_mode: input.startup_mode,
            close_to_tray: input.close_to_tray,
        });
        queries::save_startup_prefs(pool, &prefs).await?;

        // The close handler is synchronous and reads this mirror instead of
        // the database.
        if let Some(tray) = input.close_to_tray
            && let Some(behavior) = app.try_state::<crate::startup::CloseBehavior>()
            && let Ok(mut guard) = behavior.0.write()
        {
            *guard = tray;
        }

        let autostart = if let Some(enabled) = input.autostart {
            let manager = app.autolaunch();
            let result = if enabled {
                manager.enable()
            } else {
                manager.disable()
            };
            result.map_err(|err| CommandError::new("autostart", err.to_string()))?;
            Some(
                manager
                    .is_enabled()
                    .map_err(|err| CommandError::new("autostart", err.to_string()))?,
            )
        } else {
            None
        };

        Ok(StartupPrefsView {
            prefs,
            supported: true,
            autostart,
        })
    }
}

/// Toggle the main window's fullscreen state, used to drop the fullscreen the
/// Big Picture launch mode set once the reader leaves it.
///
/// # Errors
/// `unsupported` on mobile.
#[tauri::command]
pub async fn window_set_fullscreen(app: tauri::AppHandle, fullscreen: bool) -> CommandResult<()> {
    #[cfg(not(desktop))]
    {
        let _ = (app, fullscreen);
        return Err(CommandError::new(
            "unsupported",
            "Fullscreen control is unsupported on this platform.",
        ));
    }

    #[cfg(desktop)]
    {
        let window = app
            .get_webview_window("main")
            .ok_or_else(|| CommandError::new("no_window", "Main window is not open."))?;
        window
            .set_fullscreen(fullscreen)
            .map_err(|err| CommandError::new("window", err.to_string()))
    }
}
