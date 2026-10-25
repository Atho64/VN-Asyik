//! `LunaTranslator` network-service text source.
//!
//! The user runs `LunaTranslator` with its 网络服务 (network service) enabled and
//! points VN Asyik at the local HTTP origin (default port 2333; the owner uses
//! 2334). Verified 2026-10-24 from the official docs
//! (docs.lunatranslator.org/en/apiservice.html) and the GPL-3.0 source
//! (`network/server/tcpservice.py` `WSHandler.send_text`,
//! `textio/textoutput/websocket.py` `dispatch`): the endpoint
//! `/api/ws/text/origin` streams every extracted original text as a plain
//! WebSocket text frame — the raw string, no JSON wrapper. Frames are fed into
//! the shared [`HookHost`] ring, so the companion chat tool and the live-text
//! UI see Luna's text exactly like our own injected hook's output.
//!
//! Plain HTTP/WebSocket means this works on every platform, Android included —
//! unlike the Windows named pipe.

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::sync::Arc;
use tokio::sync::{Mutex, watch};
use tokio_stream::StreamExt;

use super::hook::HookHost;

/// Default the docs do not state; confirmed by the project owner (whose own
/// instance runs on 2334).
pub const DEFAULT_BASE_URL: &str = "http://127.0.0.1:2333";
/// `sync_state` key holding the settings JSON blob.
pub const SETTINGS_KEY: &str = "luna.service";
/// `LunaTranslator`'s websocket endpoint for extracted original text.
const ORIGIN_TEXT_PATH: &str = "/api/ws/text/origin";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LunaSettings {
    pub enabled: bool,
    pub base_url: String,
}

impl Default for LunaSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            base_url: DEFAULT_BASE_URL.to_owned(),
        }
    }
}

impl LunaSettings {
    /// Loopback-only origin with an optional port and nothing else. Any path,
    /// query, credentials or remote host is rejected: those are typos or
    /// attempts to point a "local reader" at the network (AGENTS.md §2.6).
    ///
    /// # Errors
    /// `invalid_base_url` with a reason.
    pub fn validate(&self) -> Result<(), String> {
        let url = url::Url::parse(self.base_url.trim())
            .map_err(|_| "invalid_base_url: not a URL".to_owned())?;
        if !matches!(url.scheme(), "http" | "https" | "ws" | "wss") {
            return Err("invalid_base_url: scheme must be http(s) or ws(s)".to_owned());
        }
        let loopback = match url.host() {
            Some(url::Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
            Some(url::Host::Ipv4(address)) => address.is_loopback(),
            Some(url::Host::Ipv6(address)) => address.is_loopback(),
            _ => false,
        };
        if !loopback {
            return Err("invalid_base_url: host must be localhost".to_owned());
        }
        if !matches!(url.path(), "" | "/") {
            return Err("invalid_base_url: no path allowed".to_owned());
        }
        if !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err("invalid_base_url: origin only, no credentials/query".to_owned());
        }
        Ok(())
    }

    /// The websocket URL of the original-text stream (`http` → `ws`).
    ///
    /// # Errors
    /// Propagates [`Self::validate`]; never invents a URL for an invalid base.
    pub fn text_socket_url(&self) -> Result<String, String> {
        self.validate()?;
        let mut url = url::Url::parse(self.base_url.trim())
            .map_err(|_| "invalid_base_url: not a URL".to_owned())?;
        let scheme = match url.scheme() {
            "http" => "ws",
            "https" => "wss",
            other => other,
        }
        .to_owned();
        // http→ws and https→wss stay within "special" schemes, so this cannot
        // fail on input that passed validation.
        let _ = url.set_scheme(&scheme);
        url.set_path(ORIGIN_TEXT_PATH);
        Ok(url.to_string())
    }
}

/// Runtime state for the UI: is the service configured, connected, and when
/// did text last arrive. `enabled` mirrors the settings, not a socket state.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LunaStatus {
    pub enabled: bool,
    pub connected: bool,
    pub url: Option<String>,
    pub last_line_at: Option<i64>,
    pub last_error: Option<String>,
}

/// Owns the connection lifecycle: reconnects with capped backoff, drops
/// everything cleanly when disabled, and pushes frames into the hook ring.
#[derive(Debug)]
pub struct LunaMonitor {
    host: Arc<HookHost>,
    settings_tx: watch::Sender<LunaSettings>,
    status: Mutex<LunaStatus>,
}

impl LunaMonitor {
    #[must_use]
    pub fn new(host: Arc<HookHost>, settings: LunaSettings) -> Self {
        Self {
            host,
            settings_tx: watch::Sender::new(settings),
            status: Mutex::new(LunaStatus::default()),
        }
    }

    /// Load settings from the local `sync_state` blob, defaulting when absent
    /// or unreadable (same tolerance as `chat.config`).
    #[must_use]
    pub async fn load(pool: &SqlitePool) -> LunaSettings {
        let found: Result<Option<(String,)>, _> =
            sqlx::query_as("SELECT value FROM sync_state WHERE key = ?")
                .bind(SETTINGS_KEY)
                .fetch_optional(pool)
                .await;
        match found {
            Ok(Some((json,))) => serde_json::from_str(&json).unwrap_or_default(),
            _ => LunaSettings::default(),
        }
    }

    /// Validate, persist, and apply settings; the supervise task picks the
    /// change up immediately through the watch channel.
    ///
    /// # Errors
    /// `invalid_base_url` or `db_error`.
    pub async fn save(
        &self,
        pool: &SqlitePool,
        settings: LunaSettings,
    ) -> Result<LunaSettings, String> {
        settings.validate()?;
        let json = serde_json::to_string(&settings).map_err(|_| "invalid_settings".to_owned())?;
        sqlx::query("INSERT INTO sync_state (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value")
            .bind(SETTINGS_KEY)
            .bind(&json)
            .execute(pool)
            .await
            .map_err(|_| "db_error".to_owned())?;
        self.settings_tx.send_if_modified(|current| {
            if *current == settings {
                false
            } else {
                *current = settings.clone();
                true
            }
        });
        Ok(settings)
    }

    #[must_use]
    pub fn settings(&self) -> LunaSettings {
        self.settings_tx.borrow().clone()
    }

    pub async fn status(&self) -> LunaStatus {
        let mut status = self.status.lock().await.clone();
        status.enabled = self.settings_tx.borrow().enabled;
        status
    }

    async fn set_status(&self, change: impl FnOnce(&mut LunaStatus)) {
        let mut status = self.status.lock().await;
        change(&mut status);
    }

    /// Reconnect loop. One instance is spawned at boot and runs forever;
    /// settings changes arrive over the watch channel and take effect without
    /// a restart, including disabling (which closes any live socket).
    pub async fn supervise(self: Arc<Self>) {
        let mut settings_rx = self.settings_tx.subscribe();
        let mut backoff = std::time::Duration::from_secs(1);
        loop {
            let settings = settings_rx.borrow_and_update().clone();
            let Ok(url) = settings.text_socket_url() else {
                // Invalid stored settings are surfaced by the status command;
                // the loop idles until they are fixed.
                self.set_status(|s| {
                    s.connected = false;
                    s.url = None;
                    s.last_error = Some("invalid_base_url".to_owned());
                })
                .await;
                if settings_rx.changed().await.is_err() {
                    return;
                }
                continue;
            };
            if !settings.enabled {
                self.set_status(|s| {
                    s.connected = false;
                    s.url = None;
                    s.last_error = None;
                })
                .await;
                if settings_rx.changed().await.is_err() {
                    return;
                }
                continue;
            }
            let url_for_status = url.clone();
            self.set_status(move |s| {
                s.url = Some(url_for_status);
                s.last_error = None;
            })
            .await;
            tokio::select! {
                changed = settings_rx.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    // A settings change is not a failure; retry immediately
                    // with the new configuration.
                    backoff = std::time::Duration::from_secs(1);
                    continue;
                }
                result = self.listen_once(&url) => {
                    self.set_status(|s| s.connected = false).await;
                    match result {
                        Err(detail) => {
                            self.set_status(move |s| s.last_error = Some(detail)).await;
                        }
                        // A session that ended cleanly ran for a while; the
                        // next attempt should not inherit its failures' wait.
                        Ok(()) => backoff = std::time::Duration::from_secs(1),
                    }
                }
            }
            tokio::time::sleep(backoff).await;
            backoff = std::cmp::min(backoff * 2, std::time::Duration::from_secs(30));
        }
    }

    /// One connection: stream frames until the server closes or errors. The
    /// caller cancels this future on settings changes.
    async fn listen_once(&self, url: &str) -> Result<(), String> {
        let (mut socket, _response) = tokio_tungstenite::connect_async(url)
            .await
            .map_err(|err| format!("connect: {err}"))?;
        self.set_status(|s| {
            s.connected = true;
            s.last_error = None;
        })
        .await;
        while let Some(message) = socket.next().await {
            match message {
                Ok(tokio_tungstenite::tungstenite::Message::Text(text)) => {
                    let text = text.as_str();
                    if !text.trim().is_empty() {
                        self.host.push_external(text).await;
                        let at = now_ms();
                        self.set_status(move |s| s.last_line_at = Some(at)).await;
                    }
                }
                // A Close ends the stream cleanly; other frame kinds are
                // ignored so a newer server cannot break this client.
                Ok(tokio_tungstenite::tungstenite::Message::Close(_)) => return Ok(()),
                Ok(_) => {}
                Err(err) => return Err(format!("stream: {err}")),
            }
        }
        Ok(())
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// Result of a one-shot reachability probe, for the settings UI.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LunaProbe {
    pub ok: bool,
    pub detail: String,
}

/// GET the service root (`/`, the navigation page per the docs). Any HTTP
/// answer proves a listener — including error statuses. Validation runs first
/// so a probe can never be aimed off this machine.
#[must_use]
pub async fn probe(settings: &LunaSettings) -> LunaProbe {
    if let Err(reason) = settings.validate() {
        return LunaProbe {
            ok: false,
            detail: reason,
        };
    }
    let Ok(client) = crate::vndb::http_client_builder()
        .connect_timeout(std::time::Duration::from_secs(2))
        .timeout(std::time::Duration::from_secs(3))
        .redirect(reqwest::redirect::Policy::none())
        .build()
    else {
        return LunaProbe {
            ok: false,
            detail: "could not build an HTTP client".to_owned(),
        };
    };
    let url = format!("{}/", settings.base_url.trim().trim_end_matches('/'));
    match client.get(&url).send().await {
        Ok(response) => LunaProbe {
            ok: true,
            detail: format!("HTTP {}", response.status().as_u16()),
        },
        Err(err) => LunaProbe {
            ok: false,
            detail: err.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_owner_ports_and_schemes_validate() {
        for base in [
            "http://127.0.0.1:2333",
            "http://127.0.0.1:2334",
            "http://localhost:2334",
            "http://[::1]:2333",
            "https://127.0.0.1:8443",
            "ws://127.0.0.1:2334/",
        ] {
            let settings = LunaSettings {
                enabled: true,
                base_url: base.to_owned(),
            };
            settings
                .validate()
                .unwrap_or_else(|err| panic!("{base}: {err}"));
        }
    }

    #[test]
    fn anything_not_loopback_origin_is_rejected() {
        for base in [
            "http://example.com:2333",
            "https://luna.example.net",
            "ftp://127.0.0.1:2333",
            "http://127.0.0.1:2333/path",
            "http://user:pw@127.0.0.1:2333",
            "http://127.0.0.1:2333?x=1",
            "not a url",
            "",
        ] {
            let settings = LunaSettings {
                enabled: true,
                base_url: base.to_owned(),
            };
            assert!(
                settings.validate().is_err(),
                "{base} must not validate as a Luna base URL"
            );
        }
    }

    #[test]
    fn the_socket_url_swaps_the_scheme_and_appends_the_path() {
        let settings = |base: &str| LunaSettings {
            enabled: true,
            base_url: base.to_owned(),
        };
        assert_eq!(
            settings("http://127.0.0.1:2334").text_socket_url().unwrap(),
            "ws://127.0.0.1:2334/api/ws/text/origin"
        );
        assert_eq!(
            settings("https://127.0.0.1:2333")
                .text_socket_url()
                .unwrap(),
            "wss://127.0.0.1:2333/api/ws/text/origin"
        );
        assert_eq!(
            settings("ws://localhost:9999").text_socket_url().unwrap(),
            "ws://localhost:9999/api/ws/text/origin"
        );
        // An invalid base never invents a URL.
        assert!(settings("http://example.com").text_socket_url().is_err());
    }

    /// A real websocket round trip against an in-test `LunaTranslator` stand-in:
    /// frames arrive as plain text (verified from `LunaTranslator` source, no
    /// JSON wrapper) and land in the shared hook ring.
    #[tokio::test]
    async fn frames_stream_into_the_hook_ring() {
        use futures_util::SinkExt as _;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            socket
                .send(tokio_tungstenite::tungstenite::Message::Text(
                    "こんにちは".into(),
                ))
                .await
                .unwrap();
            socket
                .send(tokio_tungstenite::tungstenite::Message::Text("   ".into()))
                .await
                .unwrap();
            socket
                .send(tokio_tungstenite::tungstenite::Message::Close(None))
                .await
                .unwrap();
        });
        let host = Arc::new(HookHost::default());
        let monitor = LunaMonitor::new(
            host.clone(),
            LunaSettings {
                enabled: true,
                base_url: format!("http://127.0.0.1:{port}"),
            },
        );
        let url = monitor.settings().text_socket_url().unwrap();
        monitor.listen_once(&url).await.expect("clean close");
        server.await.unwrap();
        let lines = host.lines().await;
        assert_eq!(lines.len(), 1, "whitespace-only frames are dropped");
        assert_eq!(lines[0].text, "こんにちは");
        assert_eq!(lines[0].thread, 0);
        let status = monitor.status().await;
        assert!(status.connected, "connected until the server closed");
        assert!(status.last_line_at.is_some());
    }
}
