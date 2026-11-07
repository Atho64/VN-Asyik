//! Desktop companion surface. Window visibility never changes the reading runtime.
use crate::commands::{CommandError, CommandResult};

pub const LABEL: &str = "companion-overlay";
pub const NAVIGATE_EVENT: &str = "companion://open-main";

#[derive(Debug, Default)]
pub struct OverlayWindow {
    #[cfg(desktop)]
    creation: tokio::sync::Mutex<()>,
}

#[cfg(desktop)]
fn window_error(_: tauri::Error) -> CommandError {
    CommandError::new("companion", "overlay_window_error")
}

/// Async is required by `WebviewWindowBuilder` on Windows (Tauri 2.12.4 docs).
/// Serialize lookup + creation so rapid requests always reuse one window.
#[tauri::command]
pub async fn companion_open_overlay(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::state::AppState>,
    windows: tauri::State<'_, OverlayWindow>,
) -> CommandResult<()> {
    crate::commands::prefs::require_beta(&state).await?;
    #[cfg(desktop)]
    {
        use tauri::Manager;
        let _gate = windows.creation.lock().await;
        let window = match app.get_webview_window(LABEL) {
            Some(window) => window,
            None => tauri::WebviewWindowBuilder::new(
                &app,
                LABEL,
                tauri::WebviewUrl::App("companion-overlay".into()),
            )
            .title("VN Asyik")
            .inner_size(420.0, 640.0)
            .min_inner_size(360.0, 480.0)
            .resizable(true)
            .decorations(true)
            .always_on_top(true)
            .disable_drag_drop_handler()
            .visible(false)
            .focused(false)
            .center()
            .build()
            .map_err(window_error)?,
        };
        window.show().map_err(window_error)?;
        window.unminimize().map_err(window_error)?;
        // Only an explicit open request focuses it; notifications never do.
        window.set_focus().map_err(window_error)
    }
    #[cfg(not(desktop))]
    {
        let _ = (app, windows);
        Err(CommandError::new("companion", "overlay_unsupported"))
    }
}

#[tauri::command]
pub async fn companion_hide_overlay(app: tauri::AppHandle) -> CommandResult<()> {
    #[cfg(desktop)]
    {
        use tauri::Manager;
        if let Some(window) = app.get_webview_window(LABEL) {
            window.hide().map_err(window_error)?;
        }
        Ok(())
    }
    #[cfg(not(desktop))]
    {
        let _ = app;
        Err(CommandError::new("companion", "overlay_unsupported"))
    }
}

fn allowed_main_path(path: &str) -> bool {
    matches!(
        path,
        "/companion" | "/companion?memory=1" | "/companion?vision=1" | "/settings"
    ) || path
        .strip_prefix("/game/")
        .is_some_and(|id| uuid::Uuid::parse_str(id).is_ok())
}

#[tauri::command]
pub async fn companion_show_main(app: tauri::AppHandle, path: String) -> CommandResult<()> {
    if !allowed_main_path(&path) {
        return Err(CommandError::new("companion", "overlay_invalid_path"));
    }
    #[cfg(desktop)]
    {
        use tauri::{Emitter, Manager};
        let main = app
            .get_webview_window("main")
            .ok_or_else(|| CommandError::new("companion", "overlay_window_error"))?;
        app.emit_to("main", NAVIGATE_EVENT, &path)
            .map_err(window_error)?;
        main.show().map_err(window_error)?;
        main.unminimize().map_err(window_error)?;
        main.set_focus().map_err(window_error)
    }
    #[cfg(not(desktop))]
    {
        let _ = app;
        Err(CommandError::new("companion", "overlay_unsupported"))
    }
}

#[cfg(test)]
mod tests {
    use super::allowed_main_path;

    #[test]
    fn navigation_accepts_only_companion_destinations() {
        assert!(allowed_main_path("/companion"));
        assert!(allowed_main_path("/settings"));
        assert!(allowed_main_path(&format!(
            "/game/{}",
            uuid::Uuid::new_v4()
        )));
        for path in [
            "https://example.com",
            "//example.com",
            "/chat",
            "/game/../settings",
            "/companion?game=x",
            "/settings\n",
        ] {
            assert!(!allowed_main_path(path), "{path}");
        }
    }
}
