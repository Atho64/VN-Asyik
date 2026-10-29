//! System tray icon and window visibility management for desktop.
//!
//! Provides a system tray icon in the notification area so VN Asyik can run silently
//! in the background while tracking visual novel playtime and receiving `LunaTranslator`
//! text hooks.
//!
//! The menu labels come from the UI (`tray_set_labels`), not from this file: every
//! other string in the app goes through the i18n dictionaries, and the tray was the
//! one place that shipped Indonesian text to an English-first app.

/// The labels the tray menu shows, supplied by the frontend in the user's locale.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayLabels {
    /// The entry that restores the window.
    pub show: String,
    /// The entry that hides the window.
    pub hide: String,
    /// The entry that ends the app.
    pub quit: String,
}

impl Default for TrayLabels {
    fn default() -> Self {
        // English, like the app's default locale. This is what the menu reads for
        // the few milliseconds between the tray existing and the frontend
        // reporting its language.
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

/// Id of the tray icon, so the labels can be replaced later without guessing
/// which tray belongs to this app.
#[cfg(desktop)]
pub const TRAY_ID: &str = "vn-asyik";

#[cfg(desktop)]
fn get_main_window(app: &tauri::AppHandle) -> Option<tauri::WebviewWindow> {
    app.get_webview_window("main")
        .or_else(|| app.webview_windows().into_values().next())
}

/// Build the tray menu with the given labels.
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

/// Set up the desktop system tray icon with menu and left-click toggle.
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

/// Fallback for non-desktop targets (e.g. Android).
#[cfg(not(desktop))]
pub fn setup_tray(_app: &tauri::AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    Ok(())
}
