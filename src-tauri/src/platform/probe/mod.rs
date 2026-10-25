//! Platform abstraction for playtime tracking.
//!
//! The tracker core is pure; this module is the only place that talks to the OS.
//! Each platform gets its own implementation behind [`SystemProbe`], so the
//! tracking loop, persistence and UI are written once and the unsupported
//! platforms return an honest [`ProbeError::Unsupported`] instead of failing to
//! compile.

use serde::{Deserialize, Serialize};

#[cfg(windows)]
pub mod windows;

/// A running process that could be a game.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunningProcess {
    /// Process id.
    pub pid: u32,
    /// Lowercased file name, e.g. `siglusengine.exe`.
    pub exe_name: String,
    /// Full path, when the OS let us read it.
    pub exe_path: Option<String>,
    /// Whether this process currently owns the foreground window.
    pub focused: bool,
}

/// What the OS reports about user presence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Presence {
    /// Milliseconds since the last keyboard or mouse input.
    pub idle_for_ms: i64,
}

/// The foreground window's process and caption.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FocusedWindow {
    /// Process that owns the window.
    pub pid: u32,
    /// Window caption. May be empty; the engine controls it.
    pub title: String,
}

/// Failures from the platform layer.
#[derive(Debug, thiserror::Error)]
pub enum ProbeError {
    /// This platform has no tracker implementation in this build.
    #[error("playtime tracking is not supported on this platform yet")]
    Unsupported,
    /// The OS refused an operation, usually a permissions or elevation issue.
    #[error("the operating system refused the request: {0}")]
    Denied(String),
    /// Something unexpected happened while enumerating or reading state.
    #[error("could not read process state: {0}")]
    Failed(String),
}

/// The OS surface the tracker needs.
///
/// Deliberately four methods: anything more would be a capability the platform
/// cannot honestly provide on every target.
pub trait SystemProbe: Send + Sync + std::fmt::Debug {
    /// Every running process with a readable name, marking the focused one.
    ///
    /// # Errors
    /// Returns [`ProbeError::Unsupported`] on platforms without a probe.
    fn list_processes(&self) -> Result<Vec<RunningProcess>, ProbeError>;

    /// The process that currently owns the foreground window, if any.
    ///
    /// # Errors
    /// Returns [`ProbeError::Unsupported`] on platforms without a probe.
    fn focused_pid(&self) -> Result<Option<u32>, ProbeError>;

    /// The caption of the foreground window, together with its process id.
    ///
    /// Returned together because the title is only meaningful for the window that
    /// owns it: reading a caption and a pid in two separate calls can describe two
    /// different windows if focus changes between them.
    ///
    /// # Errors
    /// Returns [`ProbeError::Unsupported`] on platforms without a probe.
    fn focused_window(&self) -> Result<Option<FocusedWindow>, ProbeError>;

    /// How long the user has been without input.
    ///
    /// # Errors
    /// Returns [`ProbeError::Unsupported`] on platforms without a probe.
    fn presence(&self) -> Result<Presence, ProbeError>;

    /// Whether a process with this id is still alive.
    fn is_running(&self, pid: u32) -> Result<bool, ProbeError>;
}

/// Probe used on platforms without tracking support.
///
/// Keeps the whole app compiling and lets the UI say "not available here" with a
/// reason, instead of the feature silently not existing.
#[derive(Debug, Clone, Copy, Default)]
pub struct UnsupportedProbe;

impl SystemProbe for UnsupportedProbe {
    fn list_processes(&self) -> Result<Vec<RunningProcess>, ProbeError> {
        Err(ProbeError::Unsupported)
    }

    fn focused_pid(&self) -> Result<Option<u32>, ProbeError> {
        Err(ProbeError::Unsupported)
    }

    fn focused_window(&self) -> Result<Option<FocusedWindow>, ProbeError> {
        Err(ProbeError::Unsupported)
    }

    fn presence(&self) -> Result<Presence, ProbeError> {
        Err(ProbeError::Unsupported)
    }

    fn is_running(&self, _pid: u32) -> Result<bool, ProbeError> {
        Err(ProbeError::Unsupported)
    }
}

/// The probe for the current build.
///
/// # Errors
/// Returns [`ProbeError::Unsupported`] when this platform has no implementation.
pub fn probe() -> Result<Box<dyn SystemProbe>, ProbeError> {
    #[cfg(windows)]
    {
        Ok(Box::new(windows::WindowsProbe::new()?))
    }
    #[cfg(not(windows))]
    {
        Err(ProbeError::Unsupported)
    }
}

/// Normalise a path for comparison: lowercase, forward slashes.
///
/// Windows paths arrive with either separator depending on the API used, and
/// comparing them raw means a game is matched once and not the next time.
#[must_use]
pub fn normalize_path(path: &str) -> String {
    path.replace('\\', "/").to_lowercase()
}

/// The file name from a path, lowercased.
#[must_use]
pub fn exe_name_of(path: &str) -> String {
    normalize_path(path)
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_normalise_separators_and_case() {
        assert_eq!(
            normalize_path(r"C:\Games\SiglusEngine.exe"),
            "c:/games/siglusengine.exe"
        );
        assert_eq!(
            normalize_path("C:/Games/SiglusEngine.exe"),
            "c:/games/siglusengine.exe"
        );
    }

    #[test]
    fn exe_names_are_extracted_from_either_separator() {
        assert_eq!(
            exe_name_of(r"C:\Games\SiglusEngine.exe"),
            "siglusengine.exe"
        );
        assert_eq!(exe_name_of("C:/Games/KrkrZ.exe"), "krkrz.exe");
        assert_eq!(exe_name_of("game.exe"), "game.exe");
        assert_eq!(exe_name_of(""), "");
    }

    #[test]
    fn unsupported_probe_reports_unsupported_everywhere() {
        let probe = UnsupportedProbe;
        assert!(matches!(
            probe.list_processes(),
            Err(ProbeError::Unsupported)
        ));
        assert!(matches!(probe.focused_pid(), Err(ProbeError::Unsupported)));
        assert!(matches!(probe.presence(), Err(ProbeError::Unsupported)));
        assert!(matches!(probe.is_running(1), Err(ProbeError::Unsupported)));
    }
}
