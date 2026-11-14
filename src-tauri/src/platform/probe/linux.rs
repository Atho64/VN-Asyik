//! Linux process tracking over X11/EWMH (`/proc`, `_NET_ACTIVE_WINDOW`,
//! `_NET_WM_PID`, `_NET_WM_NAME`/`WM_NAME`, and `XScreenSaverQueryInfo`).
//!
//! Pure Wayland clients cannot inspect other windows, so native Wayland windows
//! remain invisible (`Experimental` capability); `XWayland` windows are tracked via X11.

use std::sync::Mutex;

use x11rb::connection::Connection;
use x11rb::protocol::screensaver::ConnectionExt as ScreensaverExt;
use x11rb::protocol::xproto::{Atom, AtomEnum, ConnectionExt as _, Window};
use x11rb::rust_connection::RustConnection;

use super::{FocusedWindow, Presence, ProbeError, RunningProcess, SystemProbe};

/// Atoms this probe asks for, resolved once at connect time.
struct Atoms {
    active_window: Atom,
    wm_pid: Atom,
    wm_name: Atom,
    utf8_string: Atom,
}

/// The Linux probe. Holds one X connection for the life of the service.
///
/// `RustConnection` is not `Sync`, so it sits behind a mutex: the tracker polls
/// from one async task, but `SystemProbe` requires `Send + Sync` and a future
/// caller must not be able to interleave two requests on one socket.
pub struct LinuxProbe {
    connection: Mutex<RustConnection>,
    root: Window,
    atoms: Atoms,
}

impl std::fmt::Debug for LinuxProbe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LinuxProbe").finish_non_exhaustive()
    }
}

impl LinuxProbe {
    /// Connect to the display named by `DISPLAY`.
    ///
    /// # Errors
    /// [`ProbeError::NoDisplay`] when there is no reachable X server, which is
    /// the honest answer on a pure Wayland session with no `XWayland`.
    pub fn new() -> Result<Self, ProbeError> {
        let (connection, screen) = RustConnection::connect(None).map_err(|err| {
            ProbeError::NoDisplay(format!(
                "no X11 display is reachable ({err}). On Wayland this needs XWayland, \
                 which most visual novels already run through"
            ))
        })?;
        let root = connection
            .setup()
            .roots
            .get(screen)
            .map(|screen| screen.root)
            .ok_or_else(|| ProbeError::NoDisplay("the X display has no root window".to_owned()))?;

        let intern = |name: &str| -> Result<Atom, ProbeError> {
            connection
                .intern_atom(false, name.as_bytes())
                .map_err(|err| ProbeError::Failed(err.to_string()))?
                .reply()
                .map(|reply| reply.atom)
                .map_err(|err| ProbeError::Failed(err.to_string()))
        };

        let atoms = Atoms {
            active_window: intern("_NET_ACTIVE_WINDOW")?,
            wm_pid: intern("_NET_WM_PID")?,
            wm_name: intern("_NET_WM_NAME")?,
            utf8_string: intern("UTF8_STRING")?,
        };

        Ok(Self {
            connection: Mutex::new(connection),
            root,
            atoms,
        })
    }

    /// The window the window manager says is active.
    fn active_window(&self, connection: &RustConnection) -> Result<Option<Window>, ProbeError> {
        let reply = connection
            .get_property(
                false,
                self.root,
                self.atoms.active_window,
                AtomEnum::WINDOW,
                0,
                1,
            )
            .map_err(|err| ProbeError::Failed(err.to_string()))?
            .reply()
            .map_err(|err| ProbeError::Failed(err.to_string()))?;
        // A window manager that does not implement EWMH leaves this unset, which
        // is a real state (no focus) rather than an error.
        Ok(reply
            .value32()
            .and_then(|mut values| values.next())
            .filter(|window| *window != 0))
    }

    /// The process that owns `window`, from `_NET_WM_PID`.
    fn window_pid(
        &self,
        connection: &RustConnection,
        window: Window,
    ) -> Result<Option<u32>, ProbeError> {
        let reply = connection
            .get_property(false, window, self.atoms.wm_pid, AtomEnum::CARDINAL, 0, 1)
            .map_err(|err| ProbeError::Failed(err.to_string()))?
            .reply()
            .map_err(|err| ProbeError::Failed(err.to_string()))?;
        Ok(reply.value32().and_then(|mut values| values.next()))
    }

    /// Reads window caption from UTF-8 `_NET_WM_NAME`, falling back to Latin-1 `WM_NAME`.
    /// UTF-8 is preferred so Japanese chapter titles are not mangled.
    fn window_title(
        &self,
        connection: &RustConnection,
        window: Window,
    ) -> Result<String, ProbeError> {
        let utf8 = connection
            .get_property(
                false,
                window,
                self.atoms.wm_name,
                self.atoms.utf8_string,
                0,
                u32::MAX,
            )
            .map_err(|err| ProbeError::Failed(err.to_string()))?
            .reply()
            .map_err(|err| ProbeError::Failed(err.to_string()))?;
        if !utf8.value.is_empty()
            && let Ok(text) = String::from_utf8(utf8.value)
        {
            return Ok(text);
        }

        let legacy = connection
            .get_property(
                false,
                window,
                AtomEnum::WM_NAME,
                AtomEnum::STRING,
                0,
                u32::MAX,
            )
            .map_err(|err| ProbeError::Failed(err.to_string()))?
            .reply()
            .map_err(|err| ProbeError::Failed(err.to_string()))?;
        // Latin-1 is a byte-per-character encoding, so this is lossless for it.
        Ok(legacy.value.iter().map(|byte| char::from(*byte)).collect())
    }
}

impl SystemProbe for LinuxProbe {
    fn list_processes(&self) -> Result<Vec<RunningProcess>, ProbeError> {
        let focused = self.focused_pid().unwrap_or(None);
        let mut processes = Vec::new();
        let entries =
            std::fs::read_dir("/proc").map_err(|err| ProbeError::Failed(err.to_string()))?;
        for entry in entries.flatten() {
            let Some(pid) = entry
                .file_name()
                .to_str()
                .and_then(|name| name.parse::<u32>().ok())
            else {
                continue;
            };
            // `/proc/<pid>/exe` is a symlink to the binary. It is unreadable for
            // other users' processes, which is not an error: those cannot be the
            // reader's game either.
            let Ok(path) = std::fs::read_link(format!("/proc/{pid}/exe")) else {
                continue;
            };
            let Some(exe_name) = path
                .file_name()
                .and_then(|name| name.to_str())
                .map(super::normalize_path)
            else {
                continue;
            };
            processes.push(RunningProcess {
                pid,
                exe_name,
                exe_path: Some(path.to_string_lossy().into_owned()),
                focused: focused == Some(pid),
            });
        }
        Ok(processes)
    }

    fn focused_pid(&self) -> Result<Option<u32>, ProbeError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| ProbeError::Failed("the X connection lock was poisoned".to_owned()))?;
        let Some(window) = self.active_window(&connection)? else {
            return Ok(None);
        };
        self.window_pid(&connection, window)
    }

    fn focused_window(&self) -> Result<Option<FocusedWindow>, ProbeError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| ProbeError::Failed("the X connection lock was poisoned".to_owned()))?;
        let Some(window) = self.active_window(&connection)? else {
            return Ok(None);
        };
        // Read the pid first: a window that belongs to no process is not the
        // game, and its caption is not worth reporting.
        let Some(pid) = self.window_pid(&connection, window)? else {
            return Ok(None);
        };
        Ok(Some(FocusedWindow {
            pid,
            title: self.window_title(&connection, window)?,
        }))
    }

    fn presence(&self) -> Result<Presence, ProbeError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| ProbeError::Failed("the X connection lock was poisoned".to_owned()))?;
        let info = connection
            .screensaver_query_info(self.root)
            .map_err(|err| ProbeError::Failed(err.to_string()))?
            .reply()
            .map_err(|err| ProbeError::Failed(err.to_string()))?;
        Ok(Presence {
            idle_for_ms: i64::from(info.ms_since_user_input),
        })
    }

    fn is_running(&self, pid: u32) -> Result<bool, ProbeError> {
        Ok(std::path::Path::new(&format!("/proc/{pid}")).exists())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The probe must refuse to exist without a display rather than claim
    /// tracking works. A Wayland session with no `XWayland` is exactly that case.
    #[test]
    fn a_missing_display_is_reported_as_such() {
        // Only meaningful when the test host has no DISPLAY (CI, headless).
        if std::env::var_os("DISPLAY").is_some() {
            return;
        }
        match LinuxProbe::new() {
            Err(ProbeError::NoDisplay(message)) => {
                assert!(
                    message.contains("X11"),
                    "the reason should say what is missing"
                );
            }
            Err(other) => panic!("expected NoDisplay, got {other:?}"),
            Ok(_) => panic!("a probe must not connect without a display"),
        }
    }

    /// `/proc` scanning is the part that can be tested anywhere.
    #[test]
    fn the_process_list_includes_this_test_binary() {
        let Ok(probe) = LinuxProbe::new() else {
            return;
        };
        let processes = probe.list_processes().expect("list processes");
        assert!(
            processes
                .iter()
                .any(|process| process.pid == std::process::id()),
            "the running test process should be listed"
        );
        assert!(
            processes
                .iter()
                .all(|process| !process.exe_name.contains('/')),
            "exe_name is a file name, not a path"
        );
    }
}
