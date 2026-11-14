//! Desktop system tray icon and window visibility management.
//!
//! Menu labels come from the frontend (`tray_set_labels`) so the tray follows
//! the active i18n locale.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayLabels {
    pub show: String,
    pub hide: String,
    pub quit: String,
}

impl Default for TrayLabels {
    fn default() -> Self {
        // Fallback strings shown briefly before the frontend reports its locale.
        Self {
            show: "Show VN Asyik".to_owned(),
            hide: "Hide to tray".to_owned(),
            quit: "Quit".to_owned(),
        }
    }
}

#[cfg(desktop)]
use tauri::{
    Manager,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};

/// Fixed tray icon ID so `set_labels` can rebuild the menu in place.
#[cfg(desktop)]
pub const TRAY_ID: &str = "vn-asyik";

#[cfg(desktop)]
fn get_main_window(app: &tauri::AppHandle) -> Option<tauri::WebviewWindow> {
    app.get_webview_window("main")
        .or_else(|| app.webview_windows().into_values().next())
}

#[cfg(desktop)]
fn build_menu(
    app: &tauri::AppHandle,
    labels: &TrayLabels,
) -> Result<Menu<tauri::Wry>, tauri::Error> {
    let show_i = MenuItem::with_id(app, "show", &labels.show, true, None::<&str>)?;
    let hide_i = MenuItem::with_id(app, "hide", &labels.hide, true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let quit_i = MenuItem::with_id(app, "quit", &labels.quit, true, None::<&str>)?;
    Menu::with_items(app, &[&show_i, &hide_i, &sep, &quit_i])
}

#[cfg(desktop)]
pub fn setup_tray(app: &tauri::AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let menu = build_menu(app, &TrayLabels::default())?;

    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .menu(&menu)
        .tooltip("VN Asyik")
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => {
                if let Some(window) = get_main_window(app) {
                    let _ = window.show();
                    let _ = window.unminimize();
                    let _ = window.set_focus();
                }
            }
            "hide" => {
                if let Some(window) = get_main_window(app) {
                    let _ = window.hide();
                }
            }
            "quit" => {
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                if let Some(window) = get_main_window(app) {
                    let is_visible = window.is_visible().unwrap_or(false);
                    let is_minimized = window.is_minimized().unwrap_or(false);
                    if is_visible && !is_minimized {
                        let _ = window.hide();
                    } else {
                        let _ = window.show();
                        let _ = window.unminimize();
                        let _ = window.set_focus();
                    }
                }
            }
        });

    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }

    builder.build(app)?;
    Ok(())
}

/// Replace the tray menu with one written in the user's language.
///
/// Called when the frontend boots and whenever the locale changes. A missing tray
/// is not an error: on a platform without one there is nothing to relabel.
#[cfg(desktop)]
pub fn set_labels(
    app: &tauri::AppHandle,
    labels: &TrayLabels,
) -> Result<(), Box<dyn std::error::Error>> {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return Ok(());
    };
    tray.set_menu(Some(build_menu(app, labels)?))?;
    Ok(())
}

#[cfg(not(desktop))]
pub fn setup_tray(_app: &tauri::AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    Ok(())
}
