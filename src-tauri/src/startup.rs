//! Desktop startup mode, close-to-tray behavior, and OS autostart.
//!
//! Prefs live in `local_prefs` (`startup_prefs`); autostart stays OS state.
//! Mobile targets return `"unsupported"`.

use tauri::{AppHandle, Manager};
#[cfg(desktop)]
use vn_core::ui_prefs::StartupMode;
use vn_core::ui_prefs::StartupPrefs;

/// In-memory close-to-tray mirror for the synchronous `CloseRequested` handler.
/// Defaults to hiding to tray if a panicked writer poisons the lock.
#[derive(Debug)]
pub struct CloseBehavior(pub std::sync::RwLock<bool>);

impl CloseBehavior {
    #[must_use]
    pub fn hides_to_tray(&self) -> bool {
        self.0.read().map_or(true, |guard| *guard)
    }
}

/// Apply the saved startup mode; the window starts hidden so tray mode avoids a taskbar flash.
#[cfg(desktop)]
pub fn apply_startup_mode(app: &AppHandle, prefs: &StartupPrefs) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    match prefs.startup_mode {
        StartupMode::Tray => {
            // Stay hidden; tray left-click or Ctrl+Shift+H restores it.
        }
        StartupMode::BigPicture => {
            // Navigation belongs to the frontend; apply fullscreen immediately without waiting for the webview.
            let _ = window.set_fullscreen(true);
            let _ = window.show();
            let _ = window.set_focus();
        }
        StartupMode::Manual => {
            let _ = window.show();
            let _ = window.set_focus();
        }
    }
}

/// Mobile has no tray, so show the initially hidden window right away.
#[cfg(not(desktop))]
pub fn apply_startup_mode(app: &AppHandle, _prefs: &StartupPrefs) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
    }
}

/// Register the desktop autostart plugin (must run before `autolaunch()` calls).
#[cfg(desktop)]
pub fn register_autostart(app: &AppHandle) -> Result<(), tauri::Error> {
    app.plugin(tauri_plugin_autostart::init(
        tauri_plugin_autostart::MacosLauncher::LaunchAgent,
        None,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_poisoned_lock_reads_as_tray_not_quit() {
        // Poison the lock in a spawned thread (`join` captures the panic).
        let lock = std::sync::Arc::new(std::sync::RwLock::new(false));
        let spawner = std::sync::Arc::clone(&lock);
        let handle = std::thread::spawn(move || {
            let _guard = spawner.write().expect("write");
            panic!("poison");
        });
        assert!(handle.join().is_err(), "the writer panicked");
        // Spawner dropped its clone on join, so `try_unwrap` succeeds.
        let behavior =
            CloseBehavior(std::sync::Arc::try_unwrap(lock).expect("sole owner of the lock"));
        assert!(behavior.hides_to_tray());
    }
}
