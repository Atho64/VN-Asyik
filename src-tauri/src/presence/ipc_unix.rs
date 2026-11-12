//! Discord Rich Presence over a Unix domain socket.
//!
//! The Windows client uses a named pipe; Discord's protocol is identical apart
//! from the transport, so the payload and the framing live in `ipc.rs` and only
//! the socket does here.
//!
//! Discord listens on `$XDG_RUNTIME_DIR/discord-ipc-0`, falling back to `/tmp`,
//! and increments the suffix when several clients are already attached. Which one
//! is free is not knowable in advance, so they are tried in order.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

use super::ipc::{
    MAX_FRAME, OP_CLOSE, OP_FRAME, OP_HANDSHAKE, OP_PING, OP_PONG, encode_frame,
    set_activity_command,
};
use super::{PresenceError, PresenceSink};
use vn_core::presence::PresenceActivity;

/// How long to wait for Discord's answer to a frame.
///
/// Discord answers the handshake immediately. Waiting forever on a socket that
/// was accepted but never speaks would stall the presence poll loop, which is the
/// one thing this client must never do.
const REPLY_TIMEOUT: Duration = Duration::from_secs(2);

/// The sockets Discord may be listening on, in the order to try them.
#[must_use]
pub fn socket_candidates() -> Vec<PathBuf> {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    (0..10)
        .map(|index| base.join(format!("discord-ipc-{index}")))
        .collect()
}

/// Read one frame, rejecting a length that could not be a real reply.
fn read_frame(stream: &mut UnixStream) -> std::io::Result<(u32, String)> {
    let mut header = [0u8; 8];
    stream.read_exact(&mut header)?;
    let op = u32::from_le_bytes([header[0], header[1], header[2], header[3]]);
    let length = u32::from_le_bytes([header[4], header[5], header[6], header[7]]);
    if length > MAX_FRAME {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "the reply is larger than any real presence payload",
        ));
    }
    let mut body = vec![0u8; length as usize];
    stream.read_exact(&mut body)?;
    Ok((op, String::from_utf8_lossy(&body).into_owned()))
}

#[derive(Debug, Default)]
pub struct UnixIpcSink {
    stream: Option<UnixStream>,
    connected_id: Option<String>,
}

impl UnixIpcSink {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn drop_stream(&mut self) {
        self.stream = None;
        self.connected_id = None;
    }

    /// Attach to the first socket that answers the handshake.
    fn connect(&mut self, client_id: &str) -> Result<(), PresenceError> {
        self.drop_stream();
        let handshake = serde_json::json!({ "v": 1, "client_id": client_id }).to_string();
        for path in socket_candidates() {
            let Ok(mut stream) = UnixStream::connect(&path) else {
                continue;
            };
            if stream.set_read_timeout(Some(REPLY_TIMEOUT)).is_err() {
                continue;
            }
            if stream
                .write_all(&encode_frame(OP_HANDSHAKE, &handshake))
                .is_err()
            {
                continue;
            }
            // Discord answers READY, or closes the socket for an unknown client
            // id. A read failure here means "not this socket", not "no Discord".
            match read_frame(&mut stream) {
                Ok((_, reply)) if reply.contains("READY") => {
                    self.stream = Some(stream);
                    self.connected_id = Some(client_id.to_owned());
                    return Ok(());
                }
                _ => continue,
            }
        }
        Err(PresenceError::DiscordNotRunning)
    }

    fn send(&mut self, command: serde_json::Value) -> Result<(), String> {
        let Some(ref mut stream) = self.stream else {
            return Err("not connected".to_owned());
        };
        stream
            .write_all(&encode_frame(OP_FRAME, &command.to_string()))
            .map_err(|err| err.to_string())?;
        // Drain the reply so the socket does not fill up. A ping is answered; a
        // command is acknowledged. Neither is worth failing the update over, but
        // a closed socket is, because the next write would fail anyway.
        match read_frame(stream) {
            Ok((OP_CLOSE, _)) => Err("Discord closed the connection".to_owned()),
            Ok((OP_PING, _)) => {
                let pong = encode_frame(OP_PONG, "{}");
                let _ = stream.write_all(&pong);
                Ok(())
            }
            Ok(_) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::TimedOut => Ok(()),
            Err(err) => Err(err.to_string()),
        }
    }
}

impl PresenceSink for UnixIpcSink {
    fn update(
        &mut self,
        client_id: &str,
        activity: &PresenceActivity,
    ) -> Result<(), PresenceError> {
        // A write failure on a live connection usually means Discord restarted,
        // so reconnect once and retry before reporting anything.
        if self.connected_id.as_deref() == Some(client_id)
            && self.stream.is_some()
            && self.send(set_activity_command(Some(activity))).is_ok()
        {
            return Ok(());
        }

        self.connect(client_id)?;
        if let Err(message) = self.send(set_activity_command(Some(activity))) {
            self.drop_stream();
            return Err(PresenceError::IpcError(message));
        }
        Ok(())
    }

    fn clear(&mut self) -> Result<(), PresenceError> {
        if self.stream.is_some() && self.send(set_activity_command(None)).is_err() {
            self.drop_stream();
        }
        Ok(())
    }

    fn shutdown(&mut self) -> Result<(), PresenceError> {
        self.drop_stream();
        Ok(())
    }

    fn is_connected(&self) -> bool {
        self.stream.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidates_start_at_zero_and_cover_the_usual_range() {
        let paths = socket_candidates();
        assert_eq!(paths.len(), 10);
        assert!(paths[0].ends_with("discord-ipc-0"));
        assert!(paths[9].ends_with("discord-ipc-9"));
    }
}
