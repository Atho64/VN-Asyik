//! Desktop main-window launch placement and live position memory.
//!
//! Coordinates stay device-local (`local_prefs`) and are guarded against both
//! Windows minimized `-32000,-32000` sentinels and unplugged monitors via pure
//! checks in `vn_core::window`. Non-desktop targets compile only the tracker
//! type and report `"unsupported"` from commands.

use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use vn_core::window::{WindowLaunchMode, WindowLaunchPrefs};

#[cfg(desktop)]
use tauri::{AppHandle, Manager, PhysicalPosition};
#[cfg(desktop)]
use vn_core::window::{MonitorRect, WindowPosition, is_valid_position, resolve_launch_position};

/// Grace period after programmatic positioning so synthetic OS move events do
/// not feed back into customized coordinates.
#[cfg(desktop)]
const PROGRAMMATIC_MOVE_GRACE_MS: i64 = 600;

/// Debounce interval before writing dragged coordinates to `SQLite`.
#[cfg(desktop)]
const MOVE_DEBOUNCE_MS: u64 = 350;

#[derive(Debug, Clone, Serialize)]
pub struct WindowLaunchView {
    pub mode: WindowLaunchMode,
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub current_x: Option<i32>,
    pub current_y: Option<i32>,
    pub supported: bool,
}

#[derive(Debug, Default)]
struct TrackerState {
    mode: WindowLaunchMode,
    custom_x: Option<i32>,
    custom_y: Option<i32>,
    suppress_until_ms: i64,
    generation: u64,
}

/// Coordinates programmatic window moves, startup positioning, and debounced
/// persistence of user drags across `setup` and `on_window_event`.
#[derive(Debug, Default)]
pub struct WindowLaunchTracker {
    inner: Mutex<TrackerState>,
}

fn now_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |dur| i64::try_from(dur.as_millis()).unwrap_or(i64::MAX))
}

impl WindowLaunchTracker {
    pub fn sync_from_prefs(&self, prefs: &WindowLaunchPrefs) {
        if let Ok(mut guard) = self.inner.lock() {
            guard.mode = prefs.mode;
            guard.custom_x = prefs.x;
            guard.custom_y = prefs.y;
        }
    }

    #[must_use]
    pub fn snapshot(&self) -> WindowLaunchPrefs {
        self.inner
            .lock()
            .map(|guard| WindowLaunchPrefs {
                mode: guard.mode,
                x: guard.custom_x,
                y: guard.custom_y,
            })
            .unwrap_or_default()
    }

    /// Ignore OS move events for `millis` ms after programmatic repositioning.
    pub fn suppress_moves_for(&self, millis: i64) {
        if let Ok(mut guard) = self.inner.lock() {
            guard.suppress_until_ms = now_unix_ms().saturating_add(millis);
            guard.generation = guard.generation.wrapping_add(1);
        }
    }

    #[must_use]
    pub fn is_suppressed(&self) -> bool {
        self.inner
            .lock()
            .is_ok_and(|guard| now_unix_ms() < guard.suppress_until_ms)
    }

    #[cfg(desktop)]
    pub fn schedule_moved(&self) -> u64 {
        self.inner.lock().map_or(0, |mut guard| {
            guard.generation = guard.generation.wrapping_add(1);
            guard.generation
        })
    }

    #[cfg(desktop)]
    #[must_use]
    pub fn current_generation(&self) -> u64 {
        self.inner.lock().map_or(0, |guard| guard.generation)
    }
}

#[cfg(desktop)]
#[must_use]
pub fn monitor_rects(monitors: &[tauri::Monitor]) -> Vec<MonitorRect> {
    monitors
        .iter()
        .map(|monitor| {
            let pos = monitor.position();
            let size = monitor.size();
            MonitorRect {
                x: pos.x,
                y: pos.y,
                width: size.width,
                height: size.height,
            }
        })
        .collect()
}

#[cfg(desktop)]
pub fn apply_startup_position(
    app: &AppHandle,
    prefs: &WindowLaunchPrefs,
    tracker: &WindowLaunchTracker,
) {
    tracker.sync_from_prefs(prefs);
    let Some(window) = app.get_webview_window("main") else {
        return;
    };

    let size = window
        .outer_size()
        .map_or((1280, 840), |s| (s.width, s.height));
    let monitors = window
        .available_monitors()
        .map(|list| monitor_rects(&list))
        .unwrap_or_default();

    tracker.suppress_moves_for(PROGRAMMATIC_MOVE_GRACE_MS);
    match resolve_launch_position(prefs, size, &monitors) {
        Some(pos) => {
            let _ = window.set_position(PhysicalPosition::new(pos.x, pos.y));
        }
        None => {
            let _ = window.center();
        }
    }
}

/// Validate coordinates against active monitors and optionally reposition immediately.
#[cfg(desktop)]
pub fn apply_window_setting(
    window: &tauri::WebviewWindow,
    tracker: &WindowLaunchTracker,
    prefs: &WindowLaunchPrefs,
    apply_now: bool,
) -> Result<(), String> {
    if prefs.mode == WindowLaunchMode::Customized {
        let (Some(x), Some(y)) = (prefs.x, prefs.y) else {
            return Err("Custom coordinates require both X and Y.".into());
        };
        let size = window
            .outer_size()
            .map_or((1280, 840), |s| (s.width, s.height));
        let monitors = window
            .available_monitors()
            .map(|list| monitor_rects(&list))
            .unwrap_or_default();
        if !is_valid_position(WindowPosition { x, y }, size, &monitors) {
            return Err("Those coordinates lie outside your active displays.".into());
        }
    }

    tracker.sync_from_prefs(prefs);

    if apply_now {
        tracker.suppress_moves_for(PROGRAMMATIC_MOVE_GRACE_MS);
        match prefs.mode {
            WindowLaunchMode::Centered => {
                window.center().map_err(|err| err.to_string())?;
            }
            WindowLaunchMode::Customized => {
                if let (Some(x), Some(y)) = (prefs.x, prefs.y) {
                    window
                        .set_position(PhysicalPosition::new(x, y))
                        .map_err(|err| err.to_string())?;
                }
            }
        }
    }
    Ok(())
}

#[cfg(desktop)]
#[must_use]
pub fn live_position(window: &tauri::WebviewWindow) -> Option<(i32, i32)> {
    if window.is_minimized().unwrap_or(false) {
        return None;
    }
    let pos = window.outer_position().ok()?;
    (pos.x > vn_core::window::MINIMIZED_SENTINEL && pos.y > vn_core::window::MINIMIZED_SENTINEL)
        .then_some((pos.x, pos.y))
}

#[cfg(desktop)]
pub fn handle_window_moved(window: &tauri::Window, pos: &PhysicalPosition<i32>) {
    if window.label() != "main" {
        return;
    }
    if !window.is_visible().unwrap_or(false)
        || window.is_minimized().unwrap_or(false)
        || window.is_maximized().unwrap_or(false)
    {
        return;
    }

    let app = window.app_handle();
    let tracker = app.state::<WindowLaunchTracker>();
    if tracker.snapshot().mode != WindowLaunchMode::Customized || tracker.is_suppressed() {
        return;
    }

    let size = window
        .outer_size()
        .map_or((1280, 840), |s| (s.width, s.height));
    let monitors = window
        .available_monitors()
        .map(|list| monitor_rects(&list))
        .unwrap_or_default();
    if !is_valid_position(WindowPosition { x: pos.x, y: pos.y }, size, &monitors) {
        return;
    }

    let x = pos.x;
    let y = pos.y;
    let generation = tracker.schedule_moved();
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(MOVE_DEBOUNCE_MS)).await;
        let tracker = app.state::<WindowLaunchTracker>();
        if tracker.current_generation() != generation {
            return;
        }
        if tracker.snapshot().mode != WindowLaunchMode::Customized {
            return;
        }
        let next = WindowLaunchPrefs {
            mode: WindowLaunchMode::Customized,
            x: Some(x),
            y: Some(y),
        };
        let state = app.state::<crate::state::AppState>();
        if crate::db::queries::save_window_launch_prefs(state.db().pool(), &next)
            .await
            .is_ok()
        {
            tracker.sync_from_prefs(&next);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracker_syncs_and_bumps_generation_on_suppress_or_move() {
        let tracker = WindowLaunchTracker::default();
        assert_eq!(tracker.snapshot(), WindowLaunchPrefs::default());

        tracker.sync_from_prefs(&WindowLaunchPrefs {
            mode: WindowLaunchMode::Customized,
            x: Some(150),
            y: Some(250),
        });
        assert_eq!(tracker.snapshot().mode, WindowLaunchMode::Customized);
        assert_eq!(tracker.snapshot().x, Some(150));
        assert_eq!(tracker.snapshot().y, Some(250));

        assert!(!tracker.is_suppressed());
        tracker.suppress_moves_for(5_000);
        assert!(tracker.is_suppressed());
    }
}
