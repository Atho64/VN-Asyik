//! Shift+S observes the focused game's keys without reserving uppercase S system-wide.

use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, VIRTUAL_KEY, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_S, VK_SHIFT,
};

use crate::state::AppState;

#[derive(Default)]
struct ShortcutPress {
    target: Option<(String, u32)>,
    s_down: bool,
}

impl ShortcutPress {
    fn observe(&mut self, target: Option<&(String, u32)>, s_down: bool, shift_only: bool) -> bool {
        if self.target.as_ref() != target {
            self.target = target.cloned();
            self.s_down = s_down;
            return false;
        }
        let pressed = target.is_some() && s_down && !self.s_down && shift_only;
        self.s_down = s_down;
        pressed
    }
}

#[allow(unsafe_code, reason = "GetAsyncKeyState is a read-only Win32 FFI call")]
fn key_down(key: VIRTUAL_KEY) -> bool {
    // SAFETY: this reads only the requested OS key state, without pointers or hooks.
    // Microsoft documents the sign bit as down; the unreliable low bit is ignored.
    unsafe { GetAsyncKeyState(i32::from(key.0)) < 0 }
}

pub(crate) fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut press = ShortcutPress::default();
        loop {
            let state = app.state::<AppState>();
            let target = state.tracker().companion_target().await.filter(|(_, pid)| {
                *pid != std::process::id()
                    && state.tracker().probe().focused_pid().ok().flatten() == Some(*pid)
            });
            let focused = target.is_some();
            let (s_down, shift_only) = if focused {
                (
                    key_down(VK_S),
                    key_down(VK_SHIFT)
                        && !key_down(VK_CONTROL)
                        && !key_down(VK_MENU)
                        && !key_down(VK_LWIN)
                        && !key_down(VK_RWIN),
                )
            } else {
                (false, false)
            };
            if press.observe(target.as_ref(), s_down, shift_only) {
                // Awaiting bounds captures to one at a time, including protected-window failures.
                let outcome = crate::commands::screenshots::screenshot_capture(state).await;
                let payload = match outcome {
                    Ok(card) => serde_json::json!({ "ok": true, "id": card.id }),
                    Err(error) => serde_json::json!({ "ok": false, "code": error.code }),
                };
                let _ = app.emit("vn:screenshot-captured", payload);
            }
            tokio::time::sleep(Duration::from_millis(if focused { 20 } else { 200 })).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::ShortcutPress;

    #[test]
    fn one_capture_per_s_press_and_only_with_shift() {
        let target = ("game".to_owned(), 41);
        let mut press = ShortcutPress::default();
        assert!(!press.observe(Some(&target), false, false));
        assert!(press.observe(Some(&target), true, true));
        assert!(!press.observe(Some(&target), true, true));
        assert!(!press.observe(Some(&target), false, true));
        assert!(press.observe(Some(&target), true, true));
        assert!(!press.observe(Some(&target), false, false));
        assert!(!press.observe(Some(&target), true, false));
        // Releasing Ctrl or adding Shift while S remains held is not a new press.
        assert!(!press.observe(Some(&target), true, true));
    }

    #[test]
    fn focus_return_or_replacement_does_not_capture_a_held_key() {
        let first = ("game".to_owned(), 41);
        let second = ("other-game".to_owned(), 42);
        let mut press = ShortcutPress::default();
        assert!(!press.observe(None, true, true));
        assert!(!press.observe(Some(&first), true, true));
        assert!(!press.observe(Some(&first), false, true));
        assert!(press.observe(Some(&first), true, true));
        assert!(!press.observe(None, false, false));
        assert!(!press.observe(Some(&first), true, true));
        assert!(!press.observe(Some(&second), true, true));
        assert!(!press.observe(Some(&second), false, true));
        assert!(press.observe(Some(&second), true, true));
    }
}
