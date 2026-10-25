use super::{PresenceError, PresenceSink};
use vn_core::presence::PresenceActivity;

#[cfg(windows)]
use discord_rich_presence::{
    DiscordIpc, DiscordIpcClient,
    activity::{Activity, Assets, Button, Timestamps},
};

#[cfg(windows)]
pub struct IpcSink {
    client: Option<DiscordIpcClient>,
    connected_id: Option<String>,
}

#[cfg(windows)]
impl std::fmt::Debug for IpcSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IpcSink")
            .field("connected", &self.client.is_some())
            .field("connected_id", &self.connected_id)
            .finish()
    }
}

#[cfg(windows)]
impl IpcSink {
    #[must_use]
    pub fn new() -> Self {
        Self {
            client: None,
            connected_id: None,
        }
    }
}

#[cfg(windows)]
impl Default for IpcSink {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(windows)]
impl IpcSink {
    fn drop_client(&mut self) {
        if let Some(mut old) = self.client.take() {
            let _ = old.close();
        }
        self.connected_id = None;
    }

    fn connect(&mut self, client_id: &str) -> Result<(), PresenceError> {
        self.drop_client();
        let mut client = DiscordIpcClient::new(client_id);
        client
            .connect()
            .map_err(|_| PresenceError::DiscordNotRunning)?;
        self.client = Some(client);
        self.connected_id = Some(client_id.to_string());
        Ok(())
    }
}

/// Translate our activity into the crate's builder and send it.
#[cfg(windows)]
fn send_activity(client: &mut DiscordIpcClient, activity: &PresenceActivity) -> Result<(), String> {
    let mut drp_activity = Activity::new();
    // Set first: this is the card's title. Without it Discord titles the card with
    // the Discord application name instead of the novel being read.
    if let Some(ref name) = activity.name {
        drp_activity = drp_activity.name(name.as_str());
    }
    if let Some(ref details) = activity.details {
        drp_activity = drp_activity.details(details.as_str());
    }
    if let Some(ref state) = activity.state {
        drp_activity = drp_activity.state(state.as_str());
    }
    if let Some(start_epoch) = activity.start_timestamp {
        drp_activity = drp_activity.timestamps(Timestamps::new().start(start_epoch));
    }

    let mut assets = Assets::new();
    let mut has_assets = false;
    if let Some(ref large_img) = activity.large_image {
        assets = assets.large_image(large_img.as_str());
        has_assets = true;
    }
    if let Some(ref large_txt) = activity.large_text {
        assets = assets.large_text(large_txt.as_str());
        has_assets = true;
    }
    if has_assets {
        drp_activity = drp_activity.assets(assets);
    }

    if !activity.buttons.is_empty() {
        let mut buttons = Vec::with_capacity(activity.buttons.len().min(2));
        for b in activity.buttons.iter().take(2) {
            buttons.push(Button::new(b.label.as_str(), b.url.as_str()));
        }
        drp_activity = drp_activity.buttons(buttons);
    }

    client.set_activity(drp_activity).map_err(|e| e.to_string())
}

#[cfg(windows)]
impl PresenceSink for IpcSink {
    fn update(
        &mut self,
        client_id: &str,
        activity: &PresenceActivity,
    ) -> Result<(), PresenceError> {
        // Try the live connection first. A write failure on it usually means
        // Discord was restarted since the last update, so reconnect once and retry
        // before reporting anything.
        if self.connected_id.as_deref() == Some(client_id)
            && let Some(ref mut client) = self.client
            && send_activity(client, activity).is_ok()
        {
            return Ok(());
        }

        // Failing to connect is "Discord is not running". Failing to send on a
        // fresh connection is a different problem (Discord refused the payload),
        // and is reported as what it is instead of being blamed on Discord not
        // running.
        self.connect(client_id)?;
        let Some(ref mut client) = self.client else {
            return Err(PresenceError::DiscordNotRunning);
        };
        if let Err(message) = send_activity(client, activity) {
            self.drop_client();
            return Err(PresenceError::IpcError(message));
        }
        Ok(())
    }

    fn clear(&mut self) -> Result<(), PresenceError> {
        if let Some(ref mut client) = self.client
            && client.clear_activity().is_err()
        {
            let _ = client.close();
            self.client = None;
            self.connected_id = None;
        }
        Ok(())
    }

    fn shutdown(&mut self) -> Result<(), PresenceError> {
        if let Some(mut client) = self.client.take() {
            let _ = client.close();
            self.connected_id = None;
        }
        Ok(())
    }

    fn is_connected(&self) -> bool {
        self.client.is_some()
    }
}

#[cfg(not(windows))]
#[derive(Debug, Default)]
pub struct IpcSink;

#[cfg(not(windows))]
impl IpcSink {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

#[cfg(not(windows))]
impl PresenceSink for IpcSink {
    fn update(
        &mut self,
        _client_id: &str,
        _activity: &PresenceActivity,
    ) -> Result<(), PresenceError> {
        Err(PresenceError::UnsupportedPlatform)
    }

    fn clear(&mut self) -> Result<(), PresenceError> {
        Ok(())
    }

    fn shutdown(&mut self) -> Result<(), PresenceError> {
        Ok(())
    }

    fn is_connected(&self) -> bool {
        false
    }
}
