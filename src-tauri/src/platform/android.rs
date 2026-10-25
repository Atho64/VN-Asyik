//! Android package launcher and foreground usage event bridge.
//!
//! Native code removes unrelated package/activity identities before returning
//! events. Only selected packages and an anonymous app-switch marker cross IPC.

use serde::{Deserialize, Serialize};
#[cfg(target_os = "android")]
use tauri::Runtime;

use vn_core::usage::{UsageEvent, UsageEventKind, UsageInterval};

/// Native self-resume events use this marker to end explicit attribution.
pub const SELF_PACKAGE_MARKER: &str = "vn-asyik:self";

/// What the Kotlin plugin reports about usage access.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageAccess {
    /// Whether the user has granted usage access in system Settings.
    pub granted: bool,
    /// The Settings action to open, so the UI can send the user there.
    pub settings_action: String,
}

/// One raw event from the platform.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawUsageEvent {
    pub package: Option<String>,
    #[serde(default)]
    pub activity: Option<String>,
    /// Raw `UsageEvents.Event` type code.
    pub kind: i32,
    pub at_ms: i64,
}

impl RawUsageEvent {
    /// Reduce to the portable event the interval builder understands.
    pub fn to_usage_event(&self) -> UsageEvent {
        UsageEvent {
            package: self.package.clone(),
            activity: self.activity.clone(),
            kind: UsageEventKind::from_code(self.kind),
            at_ms: self.at_ms,
        }
    }
}

/// Convert a platform event list into playtime intervals.
pub fn intervals_from_raw(
    packages: &[String],
    raw: &[RawUsageEvent],
    end_cap_ms: i64,
) -> Vec<UsageInterval> {
    let events: Vec<UsageEvent> = raw.iter().map(RawUsageEvent::to_usage_event).collect();
    vn_core::usage::intervals_for(packages, &events, end_cap_ms)
}

/// Playtime tracking and runner launching on Android.
#[derive(Debug, Clone, Copy, Default)]
pub struct AndroidUsage;

/// An installed app with a launcher activity, including runner forks.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AndroidApp {
    pub package: String,
    pub label: String,
}

/// Why an Android usage call failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AndroidUsageError {
    /// Not running on Android.
    Unsupported,
    /// The user has not granted usage access in Settings.
    NotGranted,
    /// The plugin could not be reached or the device was locked.
    Unavailable(String),
}

impl std::fmt::Display for AndroidUsageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsupported => write!(f, "Android usage tracking is not available here"),
            Self::NotGranted => write!(f, "Usage access has not been granted"),
            Self::Unavailable(reason) => write!(f, "Usage data unavailable: {reason}"),
        }
    }
}

impl std::error::Error for AndroidUsageError {}

/// The Android usage-stats plugin, or an inert one elsewhere.
#[cfg(not(target_os = "android"))]
pub fn init() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri::plugin::Builder::<tauri::Wry, ()>::new("usage-stats").build()
}

#[cfg(target_os = "android")]
pub use platform_impl::init;

#[cfg(target_os = "android")]
mod platform_impl {
    use super::{AndroidApp, AndroidUsageError, RawUsageEvent, UsageAccess};
    use std::sync::OnceLock;
    use tauri::plugin::{Builder, PluginHandle};

    static HANDLE: OnceLock<PluginHandle<tauri::Wry>> = OnceLock::new();

    pub fn init() -> tauri::plugin::TauriPlugin<tauri::Wry> {
        Builder::<tauri::Wry, ()>::new("usage-stats")
            .setup(|_app, api| {
                let handle =
                    api.register_android_plugin("app.vntracker.desktop", "UsageStatsPlugin")?;
                let _ = HANDLE.set(handle);
                Ok(())
            })
            .build()
    }

    pub fn open_settings() -> Result<(), AndroidUsageError> {
        handle()?
            .run_mobile_plugin::<()>("openSettings", ())
            .map_err(|err| AndroidUsageError::Unavailable(err.to_string()))
    }

    fn handle() -> Result<&'static PluginHandle<tauri::Wry>, AndroidUsageError> {
        HANDLE.get().ok_or_else(|| {
            AndroidUsageError::Unavailable("usage-stats plugin is not registered".to_owned())
        })
    }

    pub fn status() -> Result<UsageAccess, AndroidUsageError> {
        handle()?
            .run_mobile_plugin::<UsageAccess>("status", ())
            .map_err(|err| AndroidUsageError::Unavailable(err.to_string()))
    }

    pub async fn access() -> Result<UsageAccess, AndroidUsageError> {
        handle()?
            .run_mobile_plugin_async("status", ())
            .await
            .map_err(|err| AndroidUsageError::Unavailable(err.to_string()))
    }

    pub async fn installed_apps() -> Result<Vec<AndroidApp>, AndroidUsageError> {
        handle()?
            .run_mobile_plugin_async("installedApps", ())
            .await
            .map_err(|err| AndroidUsageError::Unavailable(err.to_string()))
    }

    pub async fn launch_package(package: &str) -> Result<(), AndroidUsageError> {
        handle()?
            .run_mobile_plugin_async("launchPackage", serde_json::json!({"package": package}))
            .await
            .map_err(|err| {
                if err.to_string().contains("usage access not granted") {
                    AndroidUsageError::NotGranted
                } else {
                    AndroidUsageError::Unavailable(err.to_string())
                }
            })
    }

    pub async fn query(
        packages: &[String],
        since_ms: i64,
        until_ms: i64,
    ) -> Result<Vec<RawUsageEvent>, AndroidUsageError> {
        handle()?
            .run_mobile_plugin_async(
                "queryEvents",
                serde_json::json!({
                    "packages": packages, "sinceMs": since_ms, "untilMs": until_ms,
                }),
            )
            .await
            .map_err(|err| {
                if err.to_string().contains("usage access not granted") {
                    AndroidUsageError::NotGranted
                } else {
                    AndroidUsageError::Unavailable(err.to_string())
                }
            })
    }

    pub fn events(
        packages: &[String],
        since_ms: i64,
        until_ms: i64,
    ) -> Result<Vec<RawUsageEvent>, AndroidUsageError> {
        let payload = serde_json::json!({
            "packages": packages,
            "sinceMs": since_ms,
            "untilMs": until_ms,
        });
        match handle()?.run_mobile_plugin::<Vec<RawUsageEvent>>("queryEvents", payload) {
            Ok(events) => Ok(events),
            Err(err) => {
                let text = err.to_string();
                if text.contains("usage access not granted") {
                    Err(AndroidUsageError::NotGranted)
                } else {
                    Err(AndroidUsageError::Unavailable(text))
                }
            }
        }
    }
}

#[cfg(target_os = "android")]
impl AndroidUsage {
    pub async fn installed_apps() -> Result<Vec<AndroidApp>, AndroidUsageError> {
        platform_impl::installed_apps().await
    }

    pub async fn launch_package(package: &str) -> Result<(), AndroidUsageError> {
        platform_impl::launch_package(package).await
    }

    pub async fn access() -> Result<UsageAccess, AndroidUsageError> {
        platform_impl::access().await
    }

    pub async fn query(
        packages: &[String],
        since_ms: i64,
        until_ms: i64,
    ) -> Result<Vec<RawUsageEvent>, AndroidUsageError> {
        platform_impl::query(packages, since_ms, until_ms).await
    }

    pub fn open_settings() -> Result<(), AndroidUsageError> {
        platform_impl::open_settings()
    }

    pub fn status<R: Runtime>(
        _app: &tauri::AppHandle<R>,
    ) -> Result<UsageAccess, AndroidUsageError> {
        platform_impl::status()
    }

    pub fn events<R: Runtime>(
        _app: &tauri::AppHandle<R>,
        packages: &[String],
        since_ms: i64,
        until_ms: i64,
    ) -> Result<Vec<RawUsageEvent>, AndroidUsageError> {
        platform_impl::events(packages, since_ms, until_ms)
    }
}

#[cfg(not(target_os = "android"))]
#[allow(
    clippy::unused_async,
    reason = "same async interface as the Android implementation"
)]
impl AndroidUsage {
    pub async fn installed_apps() -> Result<Vec<AndroidApp>, AndroidUsageError> {
        Err(AndroidUsageError::Unsupported)
    }

    pub async fn launch_package(_package: &str) -> Result<(), AndroidUsageError> {
        Err(AndroidUsageError::Unsupported)
    }

    pub async fn access() -> Result<UsageAccess, AndroidUsageError> {
        Err(AndroidUsageError::Unsupported)
    }

    pub async fn query(
        _packages: &[String],
        _since_ms: i64,
        _until_ms: i64,
    ) -> Result<Vec<RawUsageEvent>, AndroidUsageError> {
        Err(AndroidUsageError::Unsupported)
    }

    pub const fn open_settings() -> Result<(), AndroidUsageError> {
        Err(AndroidUsageError::Unsupported)
    }

    pub const fn status<R: tauri::Runtime>(
        _app: &tauri::AppHandle<R>,
    ) -> Result<UsageAccess, AndroidUsageError> {
        Err(AndroidUsageError::Unsupported)
    }

    pub const fn events<R: tauri::Runtime>(
        _app: &tauri::AppHandle<R>,
        _packages: &[String],
        _since_ms: i64,
        _until_ms: i64,
    ) -> Result<Vec<RawUsageEvent>, AndroidUsageError> {
        Err(AndroidUsageError::Unsupported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_event_codes_map_through_to_portable_kinds() {
        let resumed = RawUsageEvent {
            package: Some("com.example.vn".to_owned()),
            activity: None,
            kind: 1,
            at_ms: 0,
        };
        assert_eq!(
            resumed.to_usage_event().kind,
            UsageEventKind::Resumed,
            "ACTIVITY_RESUMED is 1",
        );
    }

    #[test]
    fn intervals_are_built_from_recorded_events() {
        let packages = vec!["com.example.vn".to_owned()];
        let raw = vec![
            RawUsageEvent {
                package: Some("com.example.vn".to_owned()),
                activity: None,
                kind: 1,
                at_ms: 1_000,
            },
            RawUsageEvent {
                package: Some("com.example.vn".to_owned()),
                activity: None,
                kind: 2,
                at_ms: 61_000,
            },
        ];
        let intervals = intervals_from_raw(&packages, &raw, 100_000);
        assert_eq!(intervals.len(), 1);
        assert_eq!(intervals[0].duration_ms(), 60_000);
    }

    #[test]
    fn another_packages_events_are_dropped_before_interval_building() {
        let packages = vec!["com.example.vn".to_owned()];
        let raw = vec![RawUsageEvent {
            package: Some("com.android.chrome".to_owned()),
            activity: None,
            kind: 1,
            at_ms: 1_000,
        }];
        assert!(intervals_from_raw(&packages, &raw, 100_000).is_empty());
    }

    #[test]
    fn the_plugin_payloads_deserialize_from_the_keys_kotlin_actually_sends() {
        let access: UsageAccess = serde_json::from_str(
            r#"{"granted":true,"settingsAction":"android.settings.USAGE_ACCESS_SETTINGS"}"#,
        )
        .expect("status payload");
        assert!(access.granted);
        assert_eq!(
            access.settings_action,
            "android.settings.USAGE_ACCESS_SETTINGS"
        );

        let events: Vec<RawUsageEvent> =
            serde_json::from_str(r#"[{"package":"com.example.vn","kind":1,"atMs":1700000000000}]"#)
                .expect("events payload");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].package.as_deref(), Some("com.example.vn"));
        assert_eq!(events[0].kind, 1);
        assert_eq!(events[0].at_ms, 1_700_000_000_000);

        let screen: RawUsageEvent = serde_json::from_str(
            r#"{"package":null,"activity":null,"kind":16,"atMs":1700000000010}"#,
        )
        .expect("device event payload");
        assert_eq!(
            screen.to_usage_event().kind,
            UsageEventKind::ScreenNonInteractive
        );
        assert!(screen.to_usage_event().package.is_none());
    }

    #[test]
    fn unsupported_is_reported_rather_than_silently_returning_nothing() {
        let error = unsupported_error();
        assert_eq!(error, AndroidUsageError::Unsupported);
        assert!(
            error.to_string().contains("not available"),
            "the message must tell the user why, got {error}",
        );
    }

    const fn unsupported_error() -> AndroidUsageError {
        AndroidUsageError::Unsupported
    }
}
