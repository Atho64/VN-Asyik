//! Platform capability reporting.
//!
//! The UI shows this matrix honestly (AGENTS.md §2 rule 4, §9): a capability is
//! reported as unsupported rather than faked.

use serde::Serialize;

pub mod android;
pub mod android_presence;
#[cfg(windows)]
pub mod context_menu;
pub mod probe;
pub mod storage;

/// The tracking probe for this platform.
///
/// # Errors
/// Returns `ProbeError::Unsupported` where tracking cannot work.
pub fn tracking_probe() -> Result<Box<dyn probe::SystemProbe>, probe::ProbeError> {
    probe::probe()
}

/// How well a capability works on the current platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Support {
    /// Fully implemented and tested.
    Supported,
    /// Implemented but unverified or limited.
    Experimental,
    /// Cannot work here.
    Unavailable,
}

/// Capability matrix for the running platform.
#[derive(Debug, Clone, Serialize)]
pub struct PlatformInfo {
    /// `windows` | `macos` | `linux` | `android` | `ios` | `unknown`.
    pub os: &'static str,
    /// `desktop` or `mobile`.
    pub kind: &'static str,
    /// Whether the product targets this platform officially.
    pub first_class: bool,
    /// Desktop process/window playtime tracking.
    pub process_tracking: Support,
    /// Per-chapter detection through engine adapters.
    pub chapter_detection: Support,
    /// Discord Rich Presence.
    pub discord_presence: Support,
    /// Cloud sync (planned, Phase 5).
    pub cloud_sync: Support,
}

/// Describe the current platform.
#[must_use]
pub const fn info() -> PlatformInfo {
    if cfg!(target_os = "windows") {
        PlatformInfo {
            os: "windows",
            kind: "desktop",
            first_class: true,
            process_tracking: Support::Experimental,
            chapter_detection: Support::Experimental,
            discord_presence: Support::Supported,
            cloud_sync: Support::Unavailable,
        }
    } else if cfg!(target_os = "macos") {
        PlatformInfo {
            os: "macos",
            kind: "desktop",
            first_class: false,
            process_tracking: Support::Unavailable,
            chapter_detection: Support::Unavailable,
            discord_presence: Support::Unavailable,
            cloud_sync: Support::Unavailable,
        }
    } else if cfg!(target_os = "linux") {
        PlatformInfo {
            os: "linux",
            kind: "desktop",
            first_class: false,
            // No probe is implemented for Linux; `probe()` returns Unsupported.
            // Claiming Experimental here would promise something that is not there.
            process_tracking: Support::Unavailable,
            chapter_detection: Support::Unavailable,
            discord_presence: Support::Unavailable,
            cloud_sync: Support::Unavailable,
        }
    } else if cfg!(target_os = "android") {
        PlatformInfo {
            os: "android",
            kind: "mobile",
            first_class: true,
            // Playtime comes from the usage event log. Device-tested in Phase 8,
            // but shared-package attribution and OEM event retention remain
            // limited, so the capability stays Experimental.
            process_tracking: Support::Experimental,
            chapter_detection: Support::Unavailable,
            discord_presence: Support::Supported,
            cloud_sync: Support::Unavailable,
        }
    } else {
        PlatformInfo {
            os: "unknown",
            kind: "unknown",
            first_class: false,
            process_tracking: Support::Unavailable,
            chapter_detection: Support::Unavailable,
            discord_presence: Support::Unavailable,
            cloud_sync: Support::Unavailable,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exactly_one_platform_is_described() {
        let described = [
            cfg!(target_os = "windows"),
            cfg!(target_os = "macos"),
            cfg!(target_os = "linux"),
            cfg!(target_os = "android"),
        ]
        .iter()
        .filter(|&&x| x)
        .count();
        assert!(
            described <= 1,
            "cfg targets are mutually exclusive; found {described}"
        );
    }

    #[test]
    fn reports_the_running_os() {
        let info = info();
        assert!(matches!(
            info.os,
            "windows" | "macos" | "linux" | "android" | "unknown"
        ));
        assert!(matches!(info.kind, "desktop" | "mobile" | "unknown"));
    }

    #[test]
    fn windows_is_first_class_and_honest_about_chapters() {
        if cfg!(target_os = "windows") {
            let info = info();
            assert!(info.first_class);
            // Phase 1 has no adapter implemented yet: never claim "supported".
            assert_eq!(info.chapter_detection, Support::Experimental);
        }
    }

    #[test]
    fn discord_presence_matches_the_implemented_sink() {
        let expected = if cfg!(target_os = "windows") {
            Support::Supported
        } else {
            Support::Unavailable
        };
        assert_eq!(info().discord_presence, expected);
    }

    #[test]
    fn android_reports_limited_package_tracking() {
        if cfg!(target_os = "android") {
            let info = info();
            assert_eq!(info.process_tracking, Support::Experimental);
        }
    }
}
