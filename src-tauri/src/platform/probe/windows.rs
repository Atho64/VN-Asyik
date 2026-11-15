//! Windows tracker probe using Win32 (`windows` 0.62) and `PROCESS_QUERY_LIMITED_INFORMATION`.
//! Read-only process/window inspection only (no elevation, memory writes, or injection; AGENTS.md §8.3).

// Single module allowed `unsafe` for direct Win32 FFI wrappers; every block has a SAFETY note.
#![allow(unsafe_code)]

use windows::Win32::Foundation::{CloseHandle, FILETIME, HWND, LPARAM};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{
    GetProcessTimes, OpenProcess, PROCESS_NAME_FORMAT, PROCESS_QUERY_LIMITED_INFORMATION,
    QueryFullProcessImageNameW,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetDesktopWindow, GetForegroundWindow, GetShellWindow, GetWindowTextLengthW,
    GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible,
};
use windows::core::BOOL;

use super::{ExecutableWindow, FocusedWindow, Presence, ProbeError, RunningProcess, SystemProbe};

/// Probe backed by the Win32 API.
#[derive(Debug, Clone, Copy, Default)]
pub struct WindowsProbe;

impl WindowsProbe {
    /// Create the probe.
    ///
    /// # Errors
    /// Currently infallible; kept fallible so a future capability check can fail
    /// cleanly without changing every call site.
    pub fn new() -> Result<Self, ProbeError> {
        Ok(Self)
    }
}

impl SystemProbe for WindowsProbe {
    fn executable_windows(&self) -> Result<Vec<ExecutableWindow>, ProbeError> {
        unsafe extern "system" fn collect(hwnd: HWND, data: LPARAM) -> BOOL {
            // SAFETY: EnumWindows invokes this callback synchronously with the live Vec.
            let windows = unsafe { &mut *(data.0 as *mut Vec<ExecutableWindow>) };
            // SAFETY: these APIs only inspect OS metadata; the buffers are local.
            unsafe {
                if windows.len() >= 100
                    || hwnd == GetDesktopWindow()
                    || hwnd == GetShellWindow()
                    || !IsWindowVisible(hwnd).as_bool()
                {
                    return BOOL(1);
                }
                let mut pid = 0;
                if GetWindowThreadProcessId(hwnd, Some(&raw mut pid)) == 0
                    || pid == 0
                    || pid == std::process::id()
                {
                    return BOOL(1);
                }
                let mut caption = [0u16; 1024];
                let written = usize::try_from(GetWindowTextW(hwnd, &mut caption)).unwrap_or(0);
                let title = String::from_utf16_lossy(&caption[..written])
                    .trim()
                    .to_owned();
                if title.is_empty() {
                    return BOOL(1);
                }
                let metadata = process_window_metadata(pid);
                let created = metadata.as_ref().map_or(0, |(created, _)| *created);
                windows.push(ExecutableWindow {
                    key: format!("{:x}-{pid}-{created}", hwnd.0 as usize),
                    title,
                    exe_path: metadata.and_then(|(_, path)| path),
                });
            }
            BOOL(1)
        }
        let mut windows = Vec::new();
        // SAFETY: windows stays alive until enumeration and its callbacks return.
        unsafe { EnumWindows(Some(collect), LPARAM((&raw mut windows) as isize)) }
            .map_err(|err| ProbeError::Failed(err.to_string()))?;
        windows.sort_by(|a: &ExecutableWindow, b| a.title.cmp(&b.title).then(a.key.cmp(&b.key)));
        Ok(windows)
    }

    fn list_processes(&self) -> Result<Vec<RunningProcess>, ProbeError> {
        let focused = self.focused_pid().unwrap_or(None);
        let mut out = Vec::new();

        // SAFETY: the snapshot handle is owned and closed in this scope, and the
        // entry struct is initialised with its documented size before use.
        unsafe {
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)
                .map_err(|err| ProbeError::Failed(err.to_string()))?;

            let mut entry = PROCESSENTRY32W {
                dwSize: u32::try_from(std::mem::size_of::<PROCESSENTRY32W>()).unwrap_or(0),
                ..Default::default()
            };

            let mut has_entry = Process32FirstW(snapshot, std::ptr::from_mut(&mut entry)).is_ok();
            while has_entry {
                let name = wide_to_string(&entry.szExeFile);
                if !name.is_empty() {
                    out.push(RunningProcess {
                        pid: entry.th32ProcessID,
                        exe_name: name.to_lowercase(),
                        exe_path: process_path(entry.th32ProcessID),
                        focused: Some(entry.th32ProcessID) == focused,
                    });
                }
                has_entry = Process32NextW(snapshot, std::ptr::from_mut(&mut entry)).is_ok();
            }

            let _ = CloseHandle(snapshot);
        }

        Ok(out)
    }

    fn focused_window(&self) -> Result<Option<FocusedWindow>, ProbeError> {
        // SAFETY: every call takes no ownership; the buffer is a local and the
        // length passed is its real capacity.
        unsafe {
            let hwnd: HWND = GetForegroundWindow();
            if hwnd.0.is_null() {
                return Ok(None);
            }
            let mut pid = 0u32;
            let thread = GetWindowThreadProcessId(hwnd, Some(std::ptr::from_mut(&mut pid)));
            if thread == 0 || pid == 0 {
                return Ok(None);
            }

            // The caption is whatever the engine chose, including an empty one.
            let length = GetWindowTextLengthW(hwnd);
            let mut buffer = vec![0u16; usize::try_from(length).unwrap_or(0) + 1];
            let written = GetWindowTextW(hwnd, &mut buffer);
            let written = usize::try_from(written).unwrap_or(0).min(buffer.len());
            let title = String::from_utf16_lossy(&buffer[..written]);

            Ok(Some(FocusedWindow { pid, title }))
        }
    }

    fn focused_pid(&self) -> Result<Option<u32>, ProbeError> {
        // SAFETY: both calls take no ownership and return plain values.
        unsafe {
            let hwnd: HWND = GetForegroundWindow();
            if hwnd.0.is_null() {
                return Ok(None);
            }
            let mut pid = 0u32;
            let thread = GetWindowThreadProcessId(hwnd, Some(std::ptr::from_mut(&mut pid)));
            if thread == 0 || pid == 0 {
                return Ok(None);
            }
            Ok(Some(pid))
        }
    }

    fn presence(&self) -> Result<Presence, ProbeError> {
        // SAFETY: the struct is filled in by the OS and its size is set as the
        // API requires.
        unsafe {
            let mut info = LASTINPUTINFO {
                cbSize: u32::try_from(std::mem::size_of::<LASTINPUTINFO>()).unwrap_or(0),
                dwTime: 0,
            };
            if !GetLastInputInfo(std::ptr::from_mut(&mut info)).as_bool() {
                return Err(ProbeError::Failed(
                    "GetLastInputInfo returned false".to_owned(),
                ));
            }
            // `GetTickCount` shares the same millisecond time base as `dwTime`.
            let now = windows::Win32::System::SystemInformation::GetTickCount();
            // The counter wraps every ~49.7 days, so subtract with wrapping.
            let idle = now.wrapping_sub(info.dwTime);
            Ok(Presence {
                idle_for_ms: i64::from(idle),
            })
        }
    }

    fn is_running(&self, pid: u32) -> Result<bool, ProbeError> {
        if pid == 0 {
            return Ok(false);
        }
        // SAFETY: the handle is immediately closed; no operation is performed with
        // it beyond opening.
        unsafe {
            match OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
                Ok(handle) => {
                    let _ = CloseHandle(handle);
                    Ok(true)
                }
                Err(_) => Ok(false),
            }
        }
    }
}

fn process_window_metadata(pid: u32) -> Option<(u64, Option<String>)> {
    // SAFETY: the owned process handle is closed, and all FILETIME outputs are local.
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let (mut created, mut exit, mut kernel, mut user) = (
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
        );
        let result = GetProcessTimes(
            handle,
            &raw mut created,
            &raw mut exit,
            &raw mut kernel,
            &raw mut user,
        );
        // Read both identity and path through the same handle, even if the PID
        // is recycled while the picker is being assembled.
        let mut buffer = vec![0u16; 32_768];
        let mut len = u32::try_from(buffer.len()).unwrap_or(0);
        let path_result = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_FORMAT(0),
            windows::core::PWSTR(buffer.as_mut_ptr()),
            &raw mut len,
        );
        let path = path_result
            .ok()
            .map(|()| String::from_utf16_lossy(&buffer[..len as usize]));
        let _ = CloseHandle(handle);
        result.ok()?;
        Some((
            (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime),
            path,
        ))
    }
}

/// Read a process's full image path with minimal rights.
///
/// Returns `None` for protected or elevated processes rather than failing the
/// whole enumeration: one unreadable process must not hide the others.
fn process_path(pid: u32) -> Option<String> {
    // SAFETY: the buffer is sized before the call and `len` is updated by the API
    // to the number of characters actually written.
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buffer = [0u16; 1024];
        let mut len = u32::try_from(buffer.len()).unwrap_or(0);
        let result = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_FORMAT(0),
            windows::core::PWSTR(buffer.as_mut_ptr()),
            std::ptr::from_mut(&mut len),
        );
        let _ = CloseHandle(handle);
        if result.is_err() {
            return None;
        }
        Some(String::from_utf16_lossy(&buffer[..len as usize]))
    }
}

/// Convert a fixed-size UTF-16 field, stopping at the first NUL.
fn wide_to_string(buffer: &[u16]) -> String {
    let end = buffer
        .iter()
        .position(|value| *value == 0)
        .unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_strings_stop_at_the_nul_terminator() {
        let mut buffer = [0u16; 8];
        for (index, unit) in "ab".encode_utf16().enumerate() {
            buffer[index] = unit;
        }
        assert_eq!(wide_to_string(&buffer), "ab");
    }

    #[test]
    fn wide_strings_handle_a_full_buffer_without_terminator() {
        let buffer = ['é' as u16; 4];
        assert_eq!(wide_to_string(&buffer).chars().count(), 4);
    }

    #[test]
    fn the_probe_can_enumerate_real_processes() {
        let probe = WindowsProbe::new().expect("probe");
        let processes = probe.list_processes().expect("list");
        assert!(!processes.is_empty(), "a desktop always has processes");
        assert!(
            processes
                .iter()
                .any(|process| process.pid == std::process::id()),
            "the test process must appear in its own snapshot"
        );
    }

    #[test]
    fn presence_returns_a_plausible_idle_time() {
        let probe = WindowsProbe::new().expect("probe");
        let presence = probe.presence().expect("presence");
        assert!(presence.idle_for_ms >= 0);
        assert!(presence.idle_for_ms < 86_400_000);
    }

    #[test]
    fn the_test_process_is_running_and_a_bogus_pid_is_not() {
        let probe = WindowsProbe::new().expect("probe");
        assert!(probe.is_running(std::process::id()).expect("self"));
        assert!(!probe.is_running(0).expect("zero"));
        assert!(!probe.is_running(u32::MAX - 1).expect("bogus"));
    }

    #[test]
    fn focused_pid_is_readable() {
        let probe = WindowsProbe::new().expect("probe");
        let focused = probe.focused_pid().expect("focused");
        if let Some(pid) = focused {
            assert!(pid > 0);
        }
    }

    #[test]
    fn exe_names_from_the_snapshot_are_lowercase() {
        let probe = WindowsProbe::new().expect("probe");
        for process in probe.list_processes().expect("list").iter().take(20) {
            assert_eq!(process.exe_name, process.exe_name.to_lowercase());
        }
    }
}
