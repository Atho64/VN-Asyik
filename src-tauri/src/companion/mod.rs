//! A native, subscriber-independent reading companion. Questions use the selected
//! provider; automatic reactions and recap suggestions require separate opt-ins.
mod service;
pub use service::Action;
pub mod memory;
#[cfg(test)]
mod profile_tests;
pub mod profiles;
pub mod store;
#[cfg(test)]
mod tests;
pub mod vision;
pub mod window;

use serde::Serialize;
use std::{
    collections::VecDeque,
    sync::{Mutex, MutexGuard},
};
use tokio::sync::watch;
use vn_core::companion::{Lifecycle, SessionMachine, TurnToken};

use crate::{
    ai::conversation::{ChatEvent, ChatMessage, EventSink},
    tracking::hook::HookHost,
};

pub const EVENT: &str = "companion://changed";

/// Events are small invalidations. Subscribers obtain an ordered snapshot and
/// never replay a send command to catch up with missed deltas.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Notice {
    pub runtime_id: String,
    pub sequence: u64,
    pub session_id: Option<String>,
    pub epoch: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceView {
    pub state: String,
    pub last_line_at: Option<i64>,
    pub line_count: usize,
    pub preview: Vec<String>,
}

impl Default for SourceView {
    fn default() -> Self {
        Self {
            state: "off".into(),
            last_line_at: None,
            line_count: 0,
            preview: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Availability {
    Available,
    Unsupported,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    pub overlay: Availability,
    pub text: Availability,
    pub luna: Availability,
    pub background_runtime: Availability,
    pub process_lifecycle: Availability,
    pub vision: Availability,
    pub speech: Availability,
    pub durable_memory: Availability,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub request_id: Option<String>,
    pub runtime_id: String,
    pub sequence: u64,
    pub epoch: u64,
    pub lifecycle: Lifecycle,
    pub session: Option<store::Session>,
    pub busy: bool,
    pub live: Option<ChatMessage>,
    pub source: SourceView,
    pub error: Option<String>,
    pub process_id: Option<u32>,
    pub capabilities: Capabilities,
    pub vision: vision::View,
}

#[derive(Debug, Default)]
struct Inner {
    vision: vision::State,
    memory_revision: i64,
    summary: Option<memory::jobs::Worker>,
    request_id: Option<String>,
    machine: SessionMachine,
    session: Option<store::Session>,
    source: SourceView,
    live: Option<ChatMessage>,
    error: Option<String>,
    sequence: u64,
    worker: Option<TurnToken>,
    cancel: Option<watch::Sender<bool>>,
    requests: VecDeque<String>,
    process_id: Option<u32>,
}

impl Inner {
    fn revoke(&mut self) {
        self.vision.stop();
        if let Some(summary) = &self.summary {
            summary.cancel.send_replace(true);
        }
        if let Some(cancel) = &self.cancel {
            cancel.send_replace(true);
        }
        if let Some(live) = &mut self.live
            && live.status == "running"
        {
            live.status = "cancelled".into();
            live.error = Some("cancelled".into());
            for trace in &mut live.tools {
                if trace.status == "running" {
                    trace.status = "cancelled".into();
                }
            }
        }
    }

    fn require_session(&self, id: &str) -> Result<store::Session, String> {
        self.session
            .as_ref()
            .filter(|session| session.id == id)
            .cloned()
            .ok_or_else(|| "companion_missing".into())
    }
}

type Notifier = Box<dyn Fn(Notice) + Send + Sync>;

pub struct CompanionRuntime {
    runtime_id: String,
    inner: Mutex<Inner>,
    /// Async database commits and lifecycle changes share this gate. Never hold
    /// the synchronous snapshot lock across await.
    mutation: tokio::sync::Mutex<()>,
    notifier: Mutex<Option<Notifier>>,
}

impl std::fmt::Debug for CompanionRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompanionRuntime").finish_non_exhaustive()
    }
}

impl Default for CompanionRuntime {
    fn default() -> Self {
        Self {
            runtime_id: uuid::Uuid::new_v4().to_string(),
            inner: Mutex::new(Inner::default()),
            mutation: tokio::sync::Mutex::new(()),
            notifier: Mutex::new(None),
        }
    }
}

impl CompanionRuntime {
    fn lock(&self) -> Result<MutexGuard<'_, Inner>, String> {
        self.inner.lock().map_err(|_| "runtime_error".into())
    }

    pub fn set_notifier(&self, notify: impl Fn(Notice) + Send + Sync + 'static) {
        if let Ok(mut notifier) = self.notifier.lock() {
            *notifier = Some(Box::new(notify));
        }
    }

    fn changed(&self, inner: &mut Inner) {
        inner.sequence += 1;
        if let Ok(notifier) = self.notifier.lock()
            && let Some(notify) = notifier.as_ref()
        {
            notify(Notice {
                runtime_id: self.runtime_id.clone(),
                sequence: inner.sequence,
                session_id: inner.session.as_ref().map(|session| session.id.clone()),
                epoch: inner.machine.epoch,
            });
        }
    }

    pub fn snapshot(&self) -> Result<Snapshot, String> {
        let inner = self.lock()?;
        Ok(Snapshot {
            request_id: inner.request_id.clone(),
            runtime_id: self.runtime_id.clone(),
            sequence: inner.sequence,
            epoch: inner.machine.epoch,
            lifecycle: inner.machine.lifecycle,
            session: inner.session.clone(),
            busy: inner.worker.is_some(),
            live: inner.live.clone(),
            source: inner.source.clone(),
            error: inner.error.clone(),
            process_id: inner.process_id,
            vision: inner.vision.view(),
            capabilities: Capabilities {
                overlay: if cfg!(desktop) {
                    Availability::Available
                } else {
                    Availability::Unsupported
                },
                text: Availability::Available,
                luna: Availability::Available,
                background_runtime: if cfg!(target_os = "android") {
                    Availability::Unsupported
                } else {
                    Availability::Available
                },
                process_lifecycle: if cfg!(windows) {
                    Availability::Available
                } else {
                    Availability::Unsupported
                },
                vision: if cfg!(windows) {
                    Availability::Available
                } else {
                    Availability::Unsupported
                },
                speech: Availability::Unsupported,
                durable_memory: Availability::Available,
            },
        })
    }
}

struct ReadingSink<'a> {
    memory_revision: i64,
    runtime: &'a CompanionRuntime,
    hook: &'a HookHost,
    token: TurnToken,
    source_generation: Option<u64>,
}

impl ReadingSink<'_> {
    fn source_current(&self) -> bool {
        self.source_generation
            .is_none_or(|generation| generation == self.hook.generation())
    }
}

impl EventSink for ReadingSink<'_> {
    fn is_current(&self) -> bool {
        self.source_current()
            && self.runtime.lock().is_ok_and(|inner| {
                inner.machine.accepts(self.token) && inner.memory_revision == self.memory_revision
            })
    }

    fn send(&self, event: ChatEvent) -> Result<(), String> {
        let mut inner = self.runtime.lock()?;
        if !inner.machine.accepts(self.token)
            || !self.source_current()
            || inner.memory_revision != self.memory_revision
        {
            return Err("cancelled".into());
        }
        match event {
            ChatEvent::Started { message } => inner.live = Some(message),
            event => {
                let live = inner.live.as_mut().ok_or("runtime_error")?;
                match event {
                    ChatEvent::Delta { text } => live.content.push_str(&text),
                    ChatEvent::Sources { sources } => live.sources = sources,
                    ChatEvent::Tool { trace } => {
                        if let Some(existing) = live
                            .tools
                            .iter_mut()
                            .find(|old| old.id == trace.id && old.status == "running")
                        {
                            *existing = trace;
                        } else {
                            live.tools.push(trace);
                        }
                    }
                    ChatEvent::Started { .. } => unreachable!(),
                }
            }
        }
        self.runtime.changed(&mut inner);
        Ok(())
    }
}
