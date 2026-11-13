//! Pure desktop window-positioning rules for launch placement.
//!
//! Coordinates are physical screen pixels matching Tauri's `PhysicalPosition<i32>`
//! and `Monitor` rectangles, kept pure here so visibility checks and sentinel
//! guards run as plain unit tests without spawning a window.

use serde::{Deserialize, Serialize};

/// Windows moves minimized top-level windows to `(-32000, -32000)`.
/// Reject anything at or below this threshold so minimized state never overwrites
/// a user's saved customized position or gets restored at startup.
pub const MINIMIZED_SENTINEL: i32 = -30_000;

/// Minimum horizontal pixels of the window that must remain visible on a monitor.
const MIN_HORIZONTAL_OVERLAP_PX: i64 = 120;

/// Small upward tolerance (px) for Windows DWM invisible drop-shadow borders when
/// a window is snapped near the top edge of a monitor.
const TOP_EDGE_TOLERANCE_PX: i64 = 16;

/// Minimum vertical clearance (px) required above a monitor's bottom edge so the
/// title bar remains reachable by mouse drag.
const MIN_TITLEBAR_CLEARANCE_PX: i64 = 32;

/// How desktop launch placement behaves when opening the main window.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowLaunchMode {
    /// Center the window on launch instead of letting the OS chase the cursor.
    #[default]
    Centered,
    /// Restore a user-configured / remembered physical window position if visible.
    Customized,
}

/// Physical top-left window coordinates `(x, y)`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowPosition {
    /// Horizontal offset relative to the virtual desktop origin.
    pub x: i32,
    /// Vertical offset relative to the virtual desktop origin.
    pub y: i32,
}

/// Persisted desktop window launch preference.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowLaunchPrefs {
    /// Whether to center or restore `(x, y)` on app launch.
    pub mode: WindowLaunchMode,
    /// Saved customized top-left X coordinate (physical pixels).
    pub x: Option<i32>,
    /// Saved customized top-left Y coordinate (physical pixels).
    pub y: Option<i32>,
}

/// Physical monitor work-area or display rectangle `(x, y, width, height)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MonitorRect {
    /// Monitor top-left X coordinate.
    pub x: i32,
    /// Monitor top-left Y coordinate.
    pub y: i32,
    /// Monitor width in physical pixels.
    pub width: u32,
    /// Monitor height in physical pixels.
    pub height: u32,
}

/// Whether `pos` places enough of a window of `window_size` safely inside any
/// currently connected monitor so the user can see and drag the title bar.
#[must_use]
pub fn is_valid_position(
    pos: WindowPosition,
    window_size: (u32, u32),
    monitors: &[MonitorRect],
) -> bool {
    if pos.x <= MINIMIZED_SENTINEL || pos.y <= MINIMIZED_SENTINEL {
        return false;
    }
    if window_size.0 == 0 || window_size.1 == 0 || monitors.is_empty() {
        return false;
    }

    let win_left = i64::from(pos.x);
    let win_width = i64::from(window_size.0);
    let win_right = win_left + win_width;
    let win_top = i64::from(pos.y);
    let required_overlap = win_width.min(MIN_HORIZONTAL_OVERLAP_PX);

    monitors.iter().any(|monitor| {
        if monitor.width == 0 || monitor.height == 0 {
            return false;
        }
        let mon_left = i64::from(monitor.x);
        let mon_right = mon_left + i64::from(monitor.width);
        let mon_top = i64::from(monitor.y);
        let mon_bottom = mon_top + i64::from(monitor.height);

        let overlap_left = win_left.max(mon_left);
        let overlap_right = win_right.min(mon_right);
        let overlap_w = overlap_right - overlap_left;

        let titlebar_reachable = win_top >= mon_top - TOP_EDGE_TOLERANCE_PX
            && win_top <= mon_bottom - MIN_TITLEBAR_CLEARANCE_PX;

        overlap_w >= required_overlap && titlebar_reachable
    })
}

/// Resolve an explicit launch position for `main` when in `Customized` mode.
///
/// Returns `None` when mode is `Centered`, coordinates are missing, or the saved
/// coordinates no longer sit on any available monitor (so the caller falls back
/// to `window.center()`).
#[must_use]
pub fn resolve_launch_position(
    prefs: &WindowLaunchPrefs,
    window_size: (u32, u32),
    monitors: &[MonitorRect],
) -> Option<WindowPosition> {
    if prefs.mode != WindowLaunchMode::Customized {
        return None;
    }
    let pos = WindowPosition {
        x: prefs.x?,
        y: prefs.y?,
    };
    is_valid_position(pos, window_size, monitors).then_some(pos)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PRIMARY: MonitorRect = MonitorRect {
        x: 0,
        y: 0,
        width: 1920,
        height: 1080,
    };

    const LEFT_SECONDARY: MonitorRect = MonitorRect {
        x: -1920,
        y: 0,
        width: 1920,
        height: 1080,
    };

    const WIN_SIZE: (u32, u32) = (1280, 840);

    #[test]
    fn centered_mode_always_returns_none_even_with_saved_coords() {
        let prefs = WindowLaunchPrefs {
            mode: WindowLaunchMode::Centered,
            x: Some(150),
            y: Some(120),
        };
        assert_eq!(resolve_launch_position(&prefs, WIN_SIZE, &[PRIMARY]), None);
    }

    #[test]
    fn customized_mode_returns_valid_primary_and_snapped_coords() {
        let normal = WindowLaunchPrefs {
            mode: WindowLaunchMode::Customized,
            x: Some(200),
            y: Some(100),
        };
        assert_eq!(
            resolve_launch_position(&normal, WIN_SIZE, &[PRIMARY]),
            Some(WindowPosition { x: 200, y: 100 })
        );

        // Windows DWM leaves an invisible -7px left shadow border on snapped windows.
        let snapped = WindowLaunchPrefs {
            mode: WindowLaunchMode::Customized,
            x: Some(-7),
            y: Some(0),
        };
        assert_eq!(
            resolve_launch_position(&snapped, WIN_SIZE, &[PRIMARY]),
            Some(WindowPosition { x: -7, y: 0 })
        );
    }

    #[test]
    fn customized_mode_handles_secondary_monitor_and_unplug_fallback() {
        let on_left = WindowLaunchPrefs {
            mode: WindowLaunchMode::Customized,
            x: Some(-1600),
            y: Some(120),
        };
        assert_eq!(
            resolve_launch_position(&on_left, WIN_SIZE, &[LEFT_SECONDARY, PRIMARY]),
            Some(WindowPosition { x: -1600, y: 120 })
        );
        // Once the secondary monitor is disconnected, fall back cleanly to center.
        assert_eq!(
            resolve_launch_position(&on_left, WIN_SIZE, &[PRIMARY]),
            None
        );
    }

    #[test]
    fn rejects_minimized_sentinel_and_unreachable_titlebars() {
        assert!(!is_valid_position(
            WindowPosition {
                x: -32_000,
                y: -32_000,
            },
            WIN_SIZE,
            &[PRIMARY],
        ));
        assert!(!is_valid_position(
            WindowPosition { x: 1880, y: 100 },
            WIN_SIZE,
            &[PRIMARY],
        ));
        assert!(!is_valid_position(
            WindowPosition { x: 200, y: 1065 },
            WIN_SIZE,
            &[PRIMARY],
        ));
        assert!(!is_valid_position(
            WindowPosition { x: 200, y: -40 },
            WIN_SIZE,
            &[PRIMARY],
        ));
        assert!(!is_valid_position(
            WindowPosition { x: 200, y: 100 },
            WIN_SIZE,
            &[],
        ));
    }
}
