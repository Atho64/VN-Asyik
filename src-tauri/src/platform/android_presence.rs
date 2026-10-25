//! Android Discord Gateway plugin bridge.

#[cfg(target_os = "android")]
mod platform_impl {
    use std::sync::OnceLock;
    use tauri::plugin::{Builder, PluginHandle};
    use vn_core::presence::PresenceActivity;

    static HANDLE: OnceLock<PluginHandle<tauri::Wry>> = OnceLock::new();

    pub fn init() -> tauri::plugin::TauriPlugin<tauri::Wry> {
        Builder::<tauri::Wry, ()>::new("discord-presence")
            .setup(|_app, api| {
                let handle =
                    api.register_android_plugin("app.vntracker.desktop", "DiscordGatewayPlugin")?;
                let _ = HANDLE.set(handle);
                Ok(())
            })
            .build()
    }

    fn handle() -> Result<&'static PluginHandle<tauri::Wry>, String> {
        HANDLE
            .get()
            .ok_or_else(|| "discord-presence plugin is not registered".to_owned())
    }

    pub fn update(token: &str, client_id: &str, activity: &PresenceActivity) -> Result<(), String> {
        // The card title. Falls back to the VN name only when the activity has no
        // name; `large_text` would be the cover's hover tooltip, not a title.
        let name = activity
            .name
            .as_deref()
            .filter(|n| !n.trim().is_empty())
            .or(activity.details.as_deref())
            .unwrap_or("VN Asyik");
        let payload = serde_json::json!({
            "token": token,
            "clientId": client_id,
            "name": name,
            "details": activity.details,
            "state": activity.state,
            "startTimestamp": activity.start_timestamp.map(|t| if t < 10_000_000_000 { t * 1000 } else { t }),
            "largeImage": activity.large_image,
            "largeText": activity.large_text,
            "smallImage": activity.small_image,
            "smallText": activity.small_text,
            "buttons": activity.buttons.iter().map(|b| &b.label).collect::<Vec<_>>(),
            "buttonUrls": activity.buttons.iter().map(|b| &b.url).collect::<Vec<_>>(),
        });

        handle()?
            .run_mobile_plugin::<()>("update", payload)
            .map_err(|e| e.to_string())
    }

    pub fn clear() -> Result<(), String> {
        handle()?
            .run_mobile_plugin::<()>("clear", ())
            .map_err(|e| e.to_string())
    }

    pub fn stop() -> Result<(), String> {
        handle()?
            .run_mobile_plugin::<()>("stop", ())
            .map_err(|e| e.to_string())
    }

    #[derive(serde::Deserialize)]
    struct StatusReply {
        status: String,
        message: Option<String>,
    }

    pub fn status() -> Result<crate::presence::PresenceStatus, String> {
        let reply = handle()?
            .run_mobile_plugin::<StatusReply>("status", ())
            .map_err(|e| e.to_string())?;
        match reply.status.as_str() {
            "connected" => Ok(crate::presence::PresenceStatus::Connected),
            "connecting" => Ok(crate::presence::PresenceStatus::Connecting),
            "error" => Ok(crate::presence::PresenceStatus::Error(
                reply.message.unwrap_or_else(|| "Unknown error".to_string()),
            )),
            _ => Ok(crate::presence::PresenceStatus::Disconnected),
        }
    }

    pub fn is_connected() -> bool {
        status().map_or(false, |st| st == crate::presence::PresenceStatus::Connected)
    }
}

#[cfg(target_os = "android")]
pub use platform_impl::*;

#[cfg(not(target_os = "android"))]
pub fn init() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri::plugin::Builder::<tauri::Wry, ()>::new("discord-presence").build()
}

#[cfg(not(target_os = "android"))]
pub fn update(
    _token: &str,
    _client_id: &str,
    _activity: &vn_core::presence::PresenceActivity,
) -> Result<(), String> {
    Err("Not supported on this platform".to_string())
}

#[cfg(not(target_os = "android"))]
pub fn clear() -> Result<(), String> {
    Ok(())
}

#[cfg(not(target_os = "android"))]
pub fn stop() -> Result<(), String> {
    Ok(())
}

#[cfg(not(target_os = "android"))]
pub fn is_connected() -> bool {
    false
}

#[cfg(not(target_os = "android"))]
pub fn status() -> Result<crate::presence::PresenceStatus, String> {
    Ok(crate::presence::PresenceStatus::UnsupportedPlatform)
}
