use serde::Deserialize;
use vn_core::companion::{
    DialogueSource, Lifecycle, SessionOptions, SourceCursor, TurnToken, eligible_lines,
};

use super::{CompanionRuntime, ReadingSink, Snapshot, SourceView, store};
use crate::{
    ai::conversation::{self, EventSink, Generation, store as chat, tools::ReadingContext},
    state::AppState,
    tracking::hook::SourceSnapshot,
};

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Pause,
    Resume,
    End,
    CancelTurn,
}

#[derive(Debug)]
pub(crate) struct Work {
    session: store::Session,
    pub(super) token: TurnToken,
    text: String,
    cancelled: tokio::sync::watch::Receiver<bool>,
    pub(super) source_generation: Option<u64>,
    reading: ReadingContext,
    provider: crate::ai::profiles::Resolved,
}

async fn binding(
    state: &AppState,
    options: &SessionOptions,
) -> Result<(Option<String>, Option<SourceCursor>), String> {
    if options.source == DialogueSource::Manual {
        return Ok((None, None));
    }
    let settings = state.luna().settings();
    if !settings.enabled {
        return Err("source_unavailable".into());
    }
    let source = state.hook().snapshot().await;
    if !source.connected {
        return Err("source_unavailable".into());
    }
    Ok((Some(settings.text_socket_url()?), Some(source.cursor)))
}

fn source_view(source: &SourceSnapshot, after: Option<SourceCursor>, now: i64) -> SourceView {
    let Some(after) = after else {
        return SourceView {
            state: "manual".into(),
            ..SourceView::default()
        };
    };
    if !source.connected || source.cursor.generation != after.generation {
        return SourceView {
            state: "disconnected".into(),
            ..SourceView::default()
        };
    }
    let lines = eligible_lines(&source.lines, after, source.cursor, now);
    let latest = source.lines.iter().rev().find(|line| {
        line.cursor.generation == after.generation && line.cursor.sequence > after.sequence
    });
    SourceView {
        state: if !lines.is_empty() {
            "receiving"
        } else if latest.is_some() {
            "stale"
        } else {
            "waiting"
        }
        .into(),
        last_line_at: latest.map(|line| line.at),
        line_count: lines.len(),
        preview: lines
            .iter()
            .rev()
            .take(3)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .map(|line| line.text.chars().take(400).collect())
            .collect(),
    }
}

impl CompanionRuntime {
    /// Disabling Beta revokes delivery immediately, then waits for any in-flight
    /// session creation before persisting its paused state. No history is removed.
    pub async fn suspend_beta(&self, state: &AppState) -> Result<(), String> {
        {
            let mut inner = self.lock()?;
            inner.machine.cancel_turn();
            inner.revoke();
            if inner.machine.lifecycle == Lifecycle::Active {
                inner.machine.pause()?;
                inner.source = SourceView::default();
            }
            self.changed(&mut inner);
        }
        let _gate = self.mutation.lock().await;
        let session = {
            let inner = self.lock()?;
            matches!(
                inner.machine.lifecycle,
                Lifecycle::Active | Lifecycle::Paused
            )
            .then(|| inner.session.clone())
            .flatten()
        };
        if let Some(session) = session {
            self.pause_locked(state, session, "beta_disabled").await?;
        }
        Ok(())
    }

    pub async fn apply_profile(
        &self,
        state: &AppState,
        id: &str,
        profile: vn_core::companion::profile::ProfileSnapshot,
    ) -> Result<Snapshot, String> {
        profile.validate()?;
        let epoch = {
            let mut inner = self.lock()?;
            inner.require_session(id)?;
            if !matches!(
                inner.machine.lifecycle,
                Lifecycle::Active | Lifecycle::Paused
            ) {
                return Err("invalid_session_state".into());
            }
            inner.machine.cancel_turn();
            inner.revoke();
            self.changed(&mut inner);
            inner.machine.epoch
        };
        let _gate = self.mutation.lock().await;
        let previous = store::get(state.db().pool(), id).await?;
        if self.lock()?.machine.epoch != epoch {
            return Err("cancelled".into());
        }
        let mut updated = previous.clone();
        updated.options.profile = Some(profile);
        store::set_options(state.db().pool(), &updated).await?;
        {
            let mut inner = self.lock()?;
            if inner.machine.epoch == epoch {
                // Replace only the profile: never restore stale lifecycle/source state.
                inner.session.as_mut().ok_or("companion_missing")?.options = updated.options;
                inner.error = None;
                self.changed(&mut inner);
                drop(inner);
                return self.snapshot();
            }
        }
        // A newer control arrived during commit. The gate keeps new work out
        // until both persisted option copies have been restored.
        store::set_options(state.db().pool(), &previous).await?;
        Err("cancelled".into())
    }

    pub async fn start(
        &self,
        state: &AppState,
        id: &str,
        options: SessionOptions,
    ) -> Result<Snapshot, String> {
        options.validate()?;
        let _gate = self.mutation.lock().await;
        {
            let inner = self.lock()?;
            if let Some(session) = &inner.session
                && session.id == id
                && session.options == options
                && inner.machine.lifecycle != Lifecycle::Off
            {
                drop(inner);
                return self.snapshot();
            }
            if inner.worker.is_some() {
                return Err("chat_busy".into());
            }
        }
        let (url, cursor) = binding(state, &options).await?;
        {
            let mut inner = self.lock()?;
            inner.machine.starting()?;
            inner.error = None;
            self.changed(&mut inner);
        }
        let result = store::create(state.db().pool(), id, options, url).await;
        let session = match result {
            Ok(session) => session,
            Err(code) => {
                let mut inner = self.lock()?;
                inner.machine.off();
                inner.error = Some(code.clone());
                self.changed(&mut inner);
                return Err(code);
            }
        };
        let source = state.hook().snapshot().await;
        let target = state.tracker().companion_target().await;
        {
            let mut inner = self.lock()?;
            inner.process_id = target
                .filter(|(game, _)| game == &session.options.game_id)
                .map(|(_, pid)| pid);
            inner.machine.activate(cursor)?;
            inner.session = Some(session);
            inner.live = None;
            inner.requests.clear();
            inner.request_id = None;
            inner.source = source_view(&source, cursor, chat::now());
            self.changed(&mut inner);
        }
        self.refresh_locked(state).await?;
        self.snapshot()
    }

    pub async fn control(
        &self,
        state: &AppState,
        id: &str,
        action: Action,
    ) -> Result<Snapshot, String> {
        // Revoke delivery before waiting for database work. A final transcript
        // commit may be holding the gate, but it must not delay Stop or Pause.
        if !matches!(action, Action::Resume) {
            let mut inner = self.lock()?;
            inner.require_session(id)?;
            if inner.machine.lifecycle == Lifecycle::Starting {
                return Err("invalid_session_state".into());
            }
            match action {
                Action::Pause => {
                    inner.machine.pause()?;
                    inner.source = SourceView {
                        state: "paused".into(),
                        ..SourceView::default()
                    };
                }
                Action::End if inner.machine.lifecycle != Lifecycle::Off => {
                    inner.machine.ending();
                    inner.source = SourceView::default();
                }
                Action::CancelTurn if inner.machine.lifecycle == Lifecycle::Active => {
                    inner.machine.cancel_turn();
                }
                _ => (),
            }
            inner.revoke();
            self.changed(&mut inner);
        }
        if matches!(action, Action::CancelTurn) {
            return self.snapshot();
        }
        let _gate = self.mutation.lock().await;
        let session = self.lock()?.require_session(id)?;
        match action {
            Action::Pause | Action::End => {
                // A newer End may have superseded a Pause while it waited.
                // Persist the current lifecycle; never revive the older one.
                let lifecycle = self.lock()?.machine.lifecycle;
                match lifecycle {
                    Lifecycle::Paused => self.pause_locked(state, session, "user").await?,
                    Lifecycle::Ending => self.end_locked(state, session, "ended_by_user").await?,
                    Lifecycle::Off => (),
                    _ => return Err("invalid_session_state".into()),
                }
            }
            Action::CancelTurn => unreachable!(),
            Action::Resume => {
                let epoch = {
                    let inner = self.lock()?;
                    if inner.machine.lifecycle != Lifecycle::Paused {
                        return Err("invalid_session_state".into());
                    }
                    inner.machine.epoch
                };
                let mut session = store::get(state.db().pool(), id).await?;
                let (url, cursor) = binding(state, &session.options).await?;
                if url != session.source_url {
                    return Err("source_changed".into());
                }
                store::set_state(state.db().pool(), &mut session, "active", None).await?;
                let target = state.tracker().companion_target().await;
                let mut inner = self.lock()?;
                if inner.machine.epoch != epoch {
                    return Err("cancelled".into());
                }
                inner.machine.activate(cursor)?;
                inner.process_id = target
                    .filter(|(game, _)| game == &session.options.game_id)
                    .map(|(_, pid)| pid);
                inner.session = Some(session);
                inner.error = None;
                inner.source = SourceView {
                    state: if cursor.is_some() {
                        "waiting"
                    } else {
                        "manual"
                    }
                    .into(),
                    ..SourceView::default()
                };
                self.changed(&mut inner);
            }
        }
        self.snapshot()
    }

    async fn pause_locked(
        &self,
        state: &AppState,
        mut session: store::Session,
        reason: &str,
    ) -> Result<(), String> {
        {
            let mut inner = self.lock()?;
            inner.machine.pause()?;
            inner.revoke();
            inner.source = SourceView {
                state: "paused".into(),
                ..SourceView::default()
            };
            session.state = "paused".into();
            session.reason = Some(reason.into());
            inner.session = Some(session.clone());
            self.changed(&mut inner);
        }
        let result =
            store::set_state(state.db().pool(), &mut session, "paused", Some(reason)).await;
        let mut inner = self.lock()?;
        inner.session = Some(session);
        if let Err(code) = &result {
            inner.error = Some(code.clone());
        }
        self.changed(&mut inner);
        result
    }

    async fn end_locked(
        &self,
        state: &AppState,
        mut session: store::Session,
        reason: &str,
    ) -> Result<(), String> {
        if self.lock()?.machine.lifecycle == Lifecycle::Off {
            return Ok(());
        }
        {
            let mut inner = self.lock()?;
            inner.machine.ending();
            inner.revoke();
            self.changed(&mut inner);
        }
        let result = store::set_state(state.db().pool(), &mut session, "ended", Some(reason)).await;
        let mut inner = self.lock()?;
        inner.machine.off();
        inner.session = Some(session);
        inner.source = SourceView {
            state: "off".into(),
            ..SourceView::default()
        };
        if let Err(code) = &result {
            inner.error = Some(code.clone());
        }
        self.changed(&mut inner);
        result
    }

    pub async fn shutdown(&self, state: &AppState) -> Result<(), String> {
        {
            let mut inner = self.lock()?;
            if matches!(
                inner.machine.lifecycle,
                Lifecycle::Active | Lifecycle::Paused
            ) {
                inner.machine.ending();
                inner.revoke();
                self.changed(&mut inner);
            }
        }
        let _gate = self.mutation.lock().await;
        let session = self.lock()?.session.clone();
        if let Some(session) = session {
            self.end_locked(state, session, "shutdown").await?;
        }
        Ok(())
    }

    /// Called by a native timer, also before every dispatch and final commit.
    pub async fn refresh(&self, state: &AppState) -> Result<(), String> {
        if self.lock()?.machine.lifecycle != Lifecycle::Active {
            return Ok(());
        }
        let _gate = self.mutation.lock().await;
        self.refresh_locked(state).await
    }

    pub(super) async fn refresh_locked(&self, state: &AppState) -> Result<(), String> {
        let (session, cursor, pid, epoch) = {
            let inner = self.lock()?;
            if inner.machine.lifecycle != Lifecycle::Active {
                return Ok(());
            }
            (
                inner.session.clone().ok_or("companion_missing")?,
                inner.machine.source_cursor,
                inner.process_id,
                inner.machine.epoch,
            )
        };
        if let Err(code) = store::get(state.db().pool(), &session.id).await {
            return if code == "context_game_missing" {
                self.end_locked(state, session, "game_missing").await
            } else {
                self.pause_locked(state, session, "db_error").await
            };
        }
        let target = state.tracker().companion_target().await;
        if session.options.source == DialogueSource::Luna
            && target
                .as_ref()
                .is_some_and(|(game, _)| game != &session.options.game_id)
        {
            return self.pause_locked(state, session, "game_changed").await;
        }
        if let Some(pid) = pid {
            match state.tracker().probe().is_running(pid) {
                Ok(true) => (),
                Ok(false) => return self.pause_locked(state, session, "game_closed").await,
                Err(_) => return self.pause_locked(state, session, "game_unavailable").await,
            }
        }
        let source = state.hook().snapshot().await;
        if let Some(cursor) = cursor {
            let settings = state.luna().settings();
            if settings.text_socket_url().ok() != session.source_url {
                return self.pause_locked(state, session, "source_changed").await;
            }
            if !settings.enabled
                || !source.connected
                || source.cursor.generation != cursor.generation
            {
                return self.pause_locked(state, session, "source_lost").await;
            }
            // Windows reports no foreground PID on a locked/unavailable desktop.
            // This uses existing OS metadata, without changing playtime tracking.
            if cfg!(windows)
                && state
                    .tracker()
                    .probe()
                    .focused_pid()
                    .is_ok_and(|pid| pid.is_none())
            {
                return self.pause_locked(state, session, "game_unavailable").await;
            }
        }
        let view = source_view(&source, cursor, chat::now());
        let mut inner = self.lock()?;
        if inner.machine.epoch != epoch || inner.machine.lifecycle != Lifecycle::Active {
            return Ok(());
        }
        let process_id = pid.or_else(|| {
            target
                .filter(|(game, _)| game == &session.options.game_id)
                .map(|(_, pid)| pid)
        });
        if inner.source != view || inner.process_id != process_id {
            inner.source = view;
            inner.process_id = process_id;
            self.changed(&mut inner);
        }
        Ok(())
    }

    pub(crate) async fn prepare(
        &self,
        state: &AppState,
        id: &str,
        request_id: &str,
        text: &str,
    ) -> Result<Option<Work>, String> {
        uuid::Uuid::parse_str(request_id).map_err(|_| "invalid_request")?;
        vn_core::chat::validate_message(text)?;
        self.preempt_visual().await?;
        // Foreground questions preempt summary delivery immediately.
        if let Some(summary) = &self.lock()?.summary {
            summary.cancel.send_replace(true);
        }
        let _gate = self.mutation.lock().await;
        let epoch = self.lock()?.machine.epoch;
        self.refresh_locked(state).await?;
        // Pin the selected endpoint/model/key for the whole turn, before
        // acknowledging dispatch. Changing settings cannot redirect tool rounds.
        let provider = crate::ai::profiles::resolve(state.db().pool()).await;
        if provider.key.is_none()
            && !crate::commands::ai::is_local_endpoint(&provider.config.base_url)
        {
            return Err("chat_key_missing".into());
        }
        let source = state.hook().snapshot().await;
        let session_for_memory = self.lock()?.require_session(id)?;
        let memory = self
            .capture_memory(state, &session_for_memory, text, &provider.config)
            .await?;
        let mut inner = self.lock()?;
        let session = inner.require_session(id)?;
        if inner.machine.epoch != epoch {
            return Err("cancelled".into());
        }
        if inner.requests.iter().any(|seen| seen == request_id) {
            return Ok(None);
        }
        if inner.worker.is_some() || inner.vision.probing {
            return Err("chat_busy".into());
        }
        let token = inner.machine.begin_turn()?;
        let cursor = inner.machine.source_cursor;
        let reading = ReadingContext {
            memory,
            system_prompt: Some(
                session
                    .options
                    .prompt(vn_core::companion::profile::PromptMode::Answer),
            ),
            game_id: session.options.game_id.clone(),
            playthrough_id: session.playthrough_id.clone(),
            knowledge: session.options.knowledge,
            source: session.options.source,
            lines: cursor.map_or_else(Vec::new, |after| {
                eligible_lines(&source.lines, after, source.cursor, chat::now())
            }),
        };
        let (cancel, cancelled) = tokio::sync::watch::channel(false);
        inner.memory_revision = reading.memory.revision;
        inner.cancel = Some(cancel);
        inner.worker = Some(token);
        inner.requests.push_back(request_id.into());
        if inner.requests.len() > 200 {
            inner.requests.pop_front();
        }
        inner.error = None;
        inner.live = None;
        inner.request_id = Some(request_id.into());
        self.changed(&mut inner);
        Ok(Some(Work {
            session,
            token,
            text: text.into(),
            cancelled,
            reading,
            provider,
            source_generation: cursor.map(|cursor| cursor.generation),
        }))
    }

    pub(crate) async fn run(&self, state: &AppState, mut work: Work) {
        let sink = ReadingSink {
            memory_revision: work.reading.memory.revision,
            runtime: self,
            hook: state.hook(),
            token: work.token,
            source_generation: work.source_generation,
        };
        let result = match state.chat().enter(&work.session.thread_id) {
            Ok((_lease, _)) => {
                conversation::generate(
                    state,
                    Generation {
                        id: &work.session.thread_id,
                        text: &work.text,
                        retry: false,
                        output: &sink,
                        cancelled: &mut work.cancelled,
                        reading: Some(&work.reading),
                        provider: Some(work.provider),
                    },
                )
                .await
            }
            Err(code) => Err(code),
        };
        let _gate = self.mutation.lock().await;
        let _ = self.refresh_locked(state).await;
        let mut error = None;
        let message = match result {
            Ok(mut message) => {
                if !sink.is_current() {
                    message.status = "cancelled".into();
                    message.error = Some("cancelled".into());
                }
                if let Err(code) = chat::save(state.db().pool(), &message).await {
                    error = Some(code);
                }
                // Source loss may race the database await. No newer turn can
                // replay the answer while this commit gate is held.
                if message.status == "complete" && !sink.is_current() {
                    message.status = "cancelled".into();
                    message.error = Some("cancelled".into());
                    if let Err(code) = chat::save(state.db().pool(), &message).await {
                        error = Some(code);
                    }
                }
                if error.is_none() {
                    error.clone_from(&message.error);
                }
                if error.as_deref() == Some("db_error") {
                    message.status = "error".into();
                    message.error.clone_from(&error);
                }
                Some(message)
            }
            Err(code) => {
                error = Some(code);
                None
            }
        };
        if let Ok(mut inner) = self.lock()
            && inner.worker == Some(work.token)
        {
            inner.worker = None;
            inner.cancel = None;
            inner.machine.finish_turn(work.token);
            inner.live = message;
            inner.error = error.filter(|code| code != "cancelled");
            self.changed(&mut inner);
        }
    }

    pub async fn delete(&self, state: &AppState, id: &str) -> Result<(), String> {
        self.revoke_memory()?;
        let _gate = self.mutation.lock().await;
        {
            let inner = self.lock()?;
            if inner
                .session
                .as_ref()
                .is_some_and(|session| session.id == id)
                && (inner.machine.lifecycle != Lifecycle::Off || inner.worker.is_some())
            {
                return Err("companion_active".into());
            }
        }
        store::delete(state.db().pool(), id).await?;
        self.memory_changed(state, true).await?;
        let mut inner = self.lock()?;
        if inner
            .session
            .as_ref()
            .is_some_and(|session| session.id == id)
        {
            inner.session = None;
            inner.live = None;
            inner.request_id = None;
        }
        self.changed(&mut inner);
        Ok(())
    }
}
