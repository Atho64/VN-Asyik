use super::{PresenceError, PresenceSink};
use vn_core::presence::PresenceActivity;

#[cfg(windows)]
use discord_rich_presence::{
    DiscordIpc, DiscordIpcClient,
    activity::{Activity, Assets, Button, Timestamps},
};

/// Opcode for the handshake frame.
// Used by the Unix client and by the protocol tests. Dead on Windows, where the
// `discord-rich-presence` crate does the framing and the payload building.
#[cfg_attr(not(unix), allow(dead_code))]
pub(crate) const OP_HANDSHAKE: u32 = 0;
/// Opcode for a command frame.
// Used by the Unix client and by the protocol tests. Dead on Windows, where the
// `discord-rich-presence` crate does the framing and the payload building.
#[cfg_attr(not(unix), allow(dead_code))]
pub(crate) const OP_FRAME: u32 = 1;
/// Opcode Discord sends to close the socket.
// Used by the Unix client and by the protocol tests. Dead on Windows, where the
// `discord-rich-presence` crate does the framing and the payload building.
#[cfg_attr(not(unix), allow(dead_code))]
pub(crate) const OP_CLOSE: u32 = 2;
/// Opcode for a ping.
// Used by the Unix client and by the protocol tests. Dead on Windows, where the
// `discord-rich-presence` crate does the framing and the payload building.
#[cfg_attr(not(unix), allow(dead_code))]
pub(crate) const OP_PING: u32 = 3;
/// Opcode for a pong.
// Used by the Unix client and by the protocol tests. Dead on Windows, where the
// `discord-rich-presence` crate does the framing and the payload building.
#[cfg_attr(not(unix), allow(dead_code))]
pub(crate) const OP_PONG: u32 = 4;

/// Largest frame accepted from Discord, in bytes.
///
/// A reply is a small JSON object; anything near this is a broken or hostile
/// peer, and allocating on its say-so is how a socket turns into a memory bug.
// Used by the Unix client and by the protocol tests. Dead on Windows, where the
// `discord-rich-presence` crate does the framing and the payload building.
#[cfg_attr(not(unix), allow(dead_code))]
pub(crate) const MAX_FRAME: u32 = 1 << 20;

/// The `activity` object Discord expects inside `SET_ACTIVITY`.
///
/// Shared by the Windows named-pipe client and the Unix socket client: the
/// payload shape is the protocol, not a platform detail, and two copies would
/// drift the moment a field is added.
// Used by the Unix client and by the protocol tests. Dead on Windows, where the
// `discord-rich-presence` crate does the framing and the payload building.
#[cfg_attr(not(unix), allow(dead_code))]
pub(crate) fn activity_payload(activity: &PresenceActivity) -> serde_json::Value {
    let mut body = serde_json::Map::new();
    if let Some(ref name) = activity.name {
        body.insert("name".to_owned(), name.clone().into());
    }
    if let Some(ref details) = activity.details {
        body.insert("details".to_owned(), details.clone().into());
    }
    if let Some(ref state) = activity.state {
        body.insert("state".to_owned(), state.clone().into());
    }
    if let Some(start) = activity.start_timestamp {
        body.insert(
            "timestamps".to_owned(),
            serde_json::json!({ "start": start }),
        );
    }
    let mut assets = serde_json::Map::new();
    if let Some(ref image) = activity.large_image {
        assets.insert("large_image".to_owned(), image.clone().into());
    }
    if let Some(ref text) = activity.large_text {
        assets.insert("large_text".to_owned(), text.clone().into());
    }
    if !assets.is_empty() {
        body.insert("assets".to_owned(), assets.into());
    }
    // Discord renders at most two, and ignores the rest.
    let buttons: Vec<serde_json::Value> = activity
        .buttons
        .iter()
        .take(2)
        .map(|button| serde_json::json!({ "label": button.label, "url": button.url }))
        .collect();
    if !buttons.is_empty() {
        body.insert("buttons".to_owned(), buttons.into());
    }
    body.into()
}

/// A `SET_ACTIVITY` command, or the clear form when `activity` is absent.
// Used by the Unix client and by the protocol tests. Dead on Windows, where the
// `discord-rich-presence` crate does the framing and the payload building.
#[cfg_attr(not(unix), allow(dead_code))]
pub(crate) fn set_activity_command(activity: Option<&PresenceActivity>) -> serde_json::Value {
    serde_json::json!({
        "cmd": "SET_ACTIVITY",
        "args": {
            "pid": std::process::id(),
            "activity": activity.map(activity_payload),
        },
        "nonce": uuid::Uuid::new_v4().to_string(),
    })
}

/// One Discord IPC frame: an 8-byte little-endian header and a JSON body.
// Used by the Unix client and by the protocol tests. Dead on Windows, where the
// `discord-rich-presence` crate does the framing and the payload building.
#[cfg_attr(not(unix), allow(dead_code))]
pub(crate) fn encode_frame(op: u32, payload: &str) -> Vec<u8> {
    let bytes = payload.as_bytes();
    // The length field is a u32 by protocol. A payload that cannot fit is a bug
    // in the caller, not a frame worth truncating into something Discord would
    // misread, so it is clamped and the peer sees a short body.
    let length = u32::try_from(bytes.len()).unwrap_or(u32::MAX);
    let mut frame = Vec::with_capacity(8 + bytes.len());
    frame.extend_from_slice(&op.to_le_bytes());
    frame.extend_from_slice(&length.to_le_bytes());
    frame.extend_from_slice(bytes);
    frame
}

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

#[cfg(not(any(windows, unix)))]
#[derive(Debug, Default)]
pub struct IpcSink;

#[cfg(not(any(windows, unix)))]
impl IpcSink {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

#[cfg(not(any(windows, unix)))]
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

#[cfg(test)]
mod protocol_tests {
    use super::*;

    fn activity() -> PresenceActivity {
        PresenceActivity {
            name: Some("Saku Saku".to_owned()),
            details: Some("Chapter 3".to_owned()),
            state: None,
            large_image: Some("https://t.vndb.org/cv/01/1.jpg".to_owned()),
            large_text: Some("Saku Saku".to_owned()),
            start_timestamp: Some(1_700_000_000),
            buttons: vec![
                vn_core::presence::PresenceButton {
                    label: "VNDB".to_owned(),
                    url: "https://vndb.org/v1".to_owned(),
                },
                vn_core::presence::PresenceButton {
                    label: "Two".to_owned(),
                    url: "https://example.com".to_owned(),
                },
                vn_core::presence::PresenceButton {
                    label: "Three".to_owned(),
                    url: "https://example.com/3".to_owned(),
                },
            ],
            small_image: None,
            small_text: None,
        }
    }

    /// The framing is the protocol: a wrong header makes Discord ignore every
    /// frame, which looks exactly like "Discord is not running".
    #[test]
    fn a_frame_is_an_eight_byte_header_then_the_body() {
        let frame = encode_frame(OP_FRAME, "{\"a\":1}");
        assert_eq!(&frame[0..4], &1u32.to_le_bytes());
        assert_eq!(&frame[4..8], &7u32.to_le_bytes());
        assert_eq!(&frame[8..], b"{\"a\":1}");
    }

    #[test]
    fn a_clear_command_carries_no_activity() {
        let command = set_activity_command(None);
        assert_eq!(command["cmd"], "SET_ACTIVITY");
        assert!(command["args"]["activity"].is_null());
        assert!(command["args"]["pid"].as_u64().is_some());
    }

    /// Discord renders at most two buttons and rejects more.
    #[test]
    fn only_two_buttons_are_sent() {
        let payload = activity_payload(&activity());
        assert_eq!(payload["buttons"].as_array().map(Vec::len), Some(2));
    }

    #[test]
    fn the_payload_carries_the_fields_discord_draws() {
        let payload = activity_payload(&activity());
        assert_eq!(payload["name"], "Saku Saku");
        assert_eq!(payload["details"], "Chapter 3");
        assert_eq!(payload["timestamps"]["start"], 1_700_000_000);
        assert_eq!(payload["assets"]["large_text"], "Saku Saku");
        // Absent fields are omitted rather than sent as null, which Discord
        // renders as an empty line.
        assert!(payload.get("state").is_none());
    }
}
