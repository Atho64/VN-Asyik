//! One-shot, reader-consented Android MediaProjection bridge.

use std::sync::OnceLock;

use serde::Deserialize;
use tauri::plugin::PluginHandle;

use crate::commands::{CommandError, CommandResult};

static HANDLE: OnceLock<PluginHandle<tauri::Wry>> = OnceLock::new();

pub fn init() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri::plugin::Builder::<tauri::Wry, ()>::new("android-screenshots")
        .setup(|_app, api| {
            let handle = api.register_android_plugin("com.atho.vnasyik", "ScreenshotPlugin")?;
            let _ = HANDLE.set(handle);
            Ok(())
        })
        .build()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureReply {
    pub data_url: Option<String>,
    pub error: Option<String>,
}

/// Waits for the notification's Capture or Cancel action. The native service
/// releases its projection after one shot, OS revocation, or a two-minute timeout.
pub async fn capture() -> CommandResult<CaptureReply> {
    let reply: CaptureReply = HANDLE
        .get()
        .ok_or_else(|| CommandError::new("screenshot_failed", "Screenshot plugin is not ready"))?
        .run_mobile_plugin_async("capture", ())
        .await
        .map_err(|_| CommandError::new("screenshot_failed", "Android capture could not start"))?;
    if let Some(code) = &reply.error {
        let code = match code.as_str() {
            "screenshot_cancelled" => "screenshot_cancelled",
            "screenshot_notifications" => "screenshot_notifications",
            "screenshot_busy" => "screenshot_busy",
            "screenshot_timeout" => "screenshot_timeout",
            "screenshot_encode" => "screenshot_encode",
            _ => "screenshot_failed",
        };
        return Err(CommandError::new(
            code,
            "Android screen capture ended without an image",
        ));
    }
    Ok(reply)
}
