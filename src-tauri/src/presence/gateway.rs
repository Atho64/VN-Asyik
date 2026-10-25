use super::{PresenceError, PresenceSink};
use vn_core::presence::PresenceActivity;

#[derive(Debug, Default)]
pub struct AndroidGatewaySink;

impl AndroidGatewaySink {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl PresenceSink for AndroidGatewaySink {
    fn update(
        &mut self,
        client_id: &str,
        activity: &PresenceActivity,
    ) -> Result<(), PresenceError> {
        let token = crate::secrets::load_discord_token().map_err(|_| {
            PresenceError::IpcError(
                "No Discord token configured. Please add your token in Settings.".to_string(),
            )
        })?;

        crate::platform::android_presence::update(token.expose(), client_id, activity)
            .map_err(PresenceError::IpcError)
    }

    fn clear(&mut self) -> Result<(), PresenceError> {
        crate::platform::android_presence::clear().map_err(PresenceError::IpcError)
    }

    fn shutdown(&mut self) -> Result<(), PresenceError> {
        crate::platform::android_presence::stop().map_err(PresenceError::IpcError)
    }

    fn is_connected(&self) -> bool {
        crate::platform::android_presence::is_connected()
    }
}
