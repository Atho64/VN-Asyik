//! The live text ring the companion chat reads.
//!
//! Text arrives from one source: the `LunaTranslator` network service, whose
//! websocket streams every line the game displays. There is no in-process
//! hooking, no injected module and no named pipe — this file only holds a bounded
//! buffer of recent lines and the connection state the UI shows.
//!
//! Nothing reaches the game. This is a read-only consumer of a local websocket
//! the user chose to enable.

use std::collections::VecDeque;
use std::sync::Arc;

use serde::Serialize;
use tokio::sync::Mutex;

/// How many lines are kept for the live viewer and the chat tool.
const MAX_LINES: usize = 200;

/// Longest line accepted, in bytes.
const MAX_LINE_BYTES: usize = 16 * 1024;

/// One line of dialogue seen by the text source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapturedLine {
    pub text: String,
    /// Always 0 now that there is no engine thread to report. Kept so the wire
    /// shape the UI reads does not change.
    pub thread: u32,
    /// When it arrived, in ms since the epoch.
    pub at: i64,
}

/// State of the live text source, for the UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HookStatus {
    /// Whether the `LunaTranslator` service is connected right now.
    pub connected: bool,
    /// Always `None`: there is no hooked process.
    pub pid: Option<u32>,
    /// Always `None`: there is no hook address.
    pub address: Option<String>,
    /// Last connection error reported by the websocket client, if any.
    pub error: Option<String>,
    /// Number of lines captured since the app started.
    pub line_count: usize,
}

/// Shared live-text state.
#[derive(Debug, Default)]
pub struct HookHost {
    inner: Mutex<HookInner>,
}

#[derive(Debug, Default)]
struct HookInner {
    lines: VecDeque<CapturedLine>,
    error: Option<String>,
    connected: bool,
    total_lines: usize,
}

impl HookHost {
    /// Current status.
    pub async fn status(&self) -> HookStatus {
        let inner = self.inner.lock().await;
        HookStatus {
            connected: inner.connected,
            pid: None,
            address: None,
            error: inner.error.clone(),
            line_count: inner.total_lines,
        }
    }

    /// The captured lines, oldest first.
    pub async fn lines(&self) -> Vec<CapturedLine> {
        let inner = self.inner.lock().await;
        inner.lines.iter().cloned().collect()
    }

    /// Forget everything, for a clean disconnect.
    pub async fn reset(&self) {
        let mut inner = self.inner.lock().await;
        inner.lines.clear();
        inner.error = None;
        inner.connected = false;
        inner.total_lines = 0;
    }

    /// Record a line captured by the `LunaTranslator` network service.
    ///
    /// Oversized and empty lines are dropped rather than stored, so a misbehaving
    /// source cannot grow the ring past `MAX_LINE_BYTES` per entry.
    pub async fn push_external(&self, text: &str) {
        if text.is_empty() || text.len() > MAX_LINE_BYTES {
            return;
        }
        self.push_line(CapturedLine {
            text: text.to_owned(),
            thread: 0,
            at: now_ms(),
        })
        .await;
    }

    async fn push_line(&self, line: CapturedLine) {
        let mut inner = self.inner.lock().await;
        inner.total_lines += 1;
        if inner.lines.len() >= MAX_LINES {
            inner.lines.pop_front();
        }
        inner.lines.push_back(line);
    }

    /// Record a connection error for the UI.
    pub async fn push_error(&self, message: String) {
        let mut inner = self.inner.lock().await;
        inner.error = Some(message);
    }

    /// Mark the source connected or disconnected.
    pub async fn set_connected(&self, connected: bool) {
        let mut inner = self.inner.lock().await;
        inner.connected = connected;
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// Kept so callers that hold an `Arc<HookHost>` do not need a second type.
pub type SharedHost = Arc<HookHost>;

#[cfg(test)]
mod tests {
    use super::*;

    fn host() -> HookHost {
        HookHost::default()
    }

    #[tokio::test]
    async fn a_pushed_line_is_captured() {
        let host = host();
        host.push_external("こんにちは").await;
        let lines = host.lines().await;
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "こんにちは");
        assert_eq!(lines[0].thread, 0);
    }

    #[tokio::test]
    async fn an_empty_line_is_ignored() {
        let host = host();
        host.push_external("").await;
        assert!(host.lines().await.is_empty());
    }

    #[tokio::test]
    async fn lines_are_capped_and_the_oldest_are_dropped() {
        let host = host();
        for index in 0..(MAX_LINES + 10) {
            host.push_external(&format!("line {index}")).await;
        }
        let lines = host.lines().await;
        assert_eq!(lines.len(), MAX_LINES);
        // The newest line survives.
        assert_eq!(
            lines.last().expect("non-empty").text,
            format!("line {}", MAX_LINES + 9)
        );
    }

    #[tokio::test]
    async fn the_total_count_keeps_rising_past_the_cap() {
        let host = host();
        for index in 0..(MAX_LINES + 5) {
            host.push_external(&format!("l{index}")).await;
        }
        assert_eq!(host.status().await.line_count, MAX_LINES + 5);
    }

    #[tokio::test]
    async fn an_oversized_line_is_dropped() {
        let host = host();
        let huge = "x".repeat(MAX_LINE_BYTES + 1);
        host.push_external(&huge).await;
        assert!(host.lines().await.is_empty());
    }

    #[tokio::test]
    async fn there_is_never_a_hooked_process() {
        // The hook is gone; the wire shape keeps `pid`/`address` so the UI does
        // not have to special-case their absence.
        let host = host();
        host.set_connected(true).await;
        let status = host.status().await;
        assert_eq!(status.pid, None);
        assert_eq!(status.address, None);
    }

    #[tokio::test]
    async fn an_error_is_recorded() {
        let host = host();
        host.push_error("connection refused".to_owned()).await;
        assert_eq!(
            host.status().await.error.as_deref(),
            Some("connection refused")
        );
    }

    #[tokio::test]
    async fn reset_clears_everything() {
        let host = host();
        host.push_external("a").await;
        host.push_error("e".to_owned()).await;
        host.set_connected(true).await;
        host.reset().await;
        let status = host.status().await;
        assert_eq!(status.line_count, 0);
        assert!(!status.connected);
        assert!(host.lines().await.is_empty());
    }
}
