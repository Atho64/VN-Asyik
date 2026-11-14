use super::{Forget, Save, View, settings};
use crate::{companion::CompanionRuntime, state::AppState};
use vn_core::companion::memory::{Context, Memory, retrieve};

#[derive(Debug, Clone, Default)]
pub struct TurnMemory {
    pub context: Option<Context>,
    pub revision: i64,
    pub records: Vec<Memory>,
    pub capture: Option<super::jobs::Capture>,
}

impl CompanionRuntime {
    /// Synchronous revocation precedes the gate, including when an older
    /// provider result is waiting for its final `SQLite` commit.
    pub(crate) fn revoke_memory(&self) -> Result<(), String> {
        let mut inner = self.lock()?;
        inner.machine.cancel_turn();
        inner.revoke();
        self.changed(&mut inner);
        Ok(())
    }

    pub(crate) async fn memory_changed(
        &self,
        state: &AppState,
        reset_cursor: bool,
    ) -> Result<(), String> {
        let current = settings(state.db().pool()).await?;
        let source = state.hook().snapshot().await;
        let selected = self.lock()?.session.clone();
        let excluded = if let Some(session) = &selected {
            let (excluded,): (bool,) = sqlx::query_as(
                "SELECT EXISTS(SELECT 1 FROM companion_replay_barriers WHERE thread_id=?)",
            )
            .bind(&session.thread_id)
            .fetch_one(state.db().pool())
            .await
            .map_err(|_| "db_error")?;
            excluded
        } else {
            false
        };
        let mut inner = self.lock()?;
        if let Some(session) = &mut inner.session {
            session.replay_excluded = excluded;
        }
        inner.memory_revision = current.revision;
        if reset_cursor && inner.machine.source_cursor.is_some() {
            inner.machine.source_cursor = Some(source.cursor);
            inner.source.preview.clear();
            inner.source.line_count = 0;
            inner.source.last_line_at = None;
        }
        self.changed(&mut inner);
        Ok(())
    }

    pub async fn memory_save(&self, state: &AppState, request: Save) -> Result<Memory, String> {
        self.revoke_memory()?;
        let _gate = self.mutation.lock().await;
        super::expire(state.db().pool()).await?;
        let session = match request.session_id.as_deref() {
            Some(id) => Some(crate::companion::store::get(state.db().pool(), id).await?),
            None => None,
        };
        let result = super::save(state.db().pool(), &request, session.as_ref()).await;
        let reset_cursor = result.as_ref().is_ok_and(|(_, reset)| *reset);
        self.memory_changed(state, reset_cursor).await?;
        result.map(|(memory, _)| memory)
    }

    pub async fn memory_forget(
        &self,
        state: &AppState,
        target: Forget,
        expected_revision: i64,
    ) -> Result<(), String> {
        self.revoke_memory()?;
        let _gate = self.mutation.lock().await;
        let mut tx = state.db().pool().begin().await.map_err(|_| "db_error")?;
        super::check_revision(&mut tx, expected_revision).await?;
        super::forget_tx(&mut tx, &target).await?;
        tx.commit().await.map_err(|_| "db_error")?;
        self.memory_changed(state, true).await
    }

    pub async fn memory_settings(
        &self,
        state: &AppState,
        retrieval: bool,
        automatic: bool,
        expected_revision: i64,
    ) -> Result<(), String> {
        self.revoke_memory()?;
        let _gate = self.mutation.lock().await;
        let mut tx = state.db().pool().begin().await.map_err(|_| "db_error")?;
        super::check_revision(&mut tx, expected_revision).await?;
        sqlx::query("UPDATE companion_memory_state SET retrieval_enabled=?,auto_summary=?,revision=revision+1,gap=NULL WHERE id=1")
            .bind(retrieval).bind(automatic).execute(&mut *tx).await.map_err(|_| "db_error")?;
        super::cancel_jobs(&mut tx).await?;
        tx.commit().await.map_err(|_| "db_error")?;
        self.memory_changed(state, false).await
    }

    pub async fn memory_view(
        &self,
        state: &AppState,
        session: Option<&str>,
    ) -> Result<View, String> {
        let _gate = self.mutation.lock().await;
        super::expire(state.db().pool()).await?;
        super::view(state.db().pool(), session).await
    }

    pub(crate) async fn capture_memory(
        &self,
        state: &AppState,
        session: &crate::companion::store::Session,
        query: &str,
        config: &crate::ai::chat::ChatConfig,
    ) -> Result<TurnMemory, String> {
        let settings = settings(state.db().pool()).await?;
        let context = Context::new(&session.options, &session.playthrough_id);
        let records = if settings.retrieval_enabled {
            retrieve(
                &super::live(state.db().pool()).await?,
                &context,
                query,
                super::now(),
            )
        } else {
            Vec::new()
        };
        Ok(TurnMemory {
            context: Some(context),
            revision: settings.revision,
            records,
            capture: settings
                .auto_summary
                .then(|| super::jobs::Capture::new(session, settings.revision, config)),
        })
    }
}
