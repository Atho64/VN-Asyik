use super::*;
use crate::{
    ai::{
        conversation::{self, store as chat},
        profiles,
    },
    db::{Db, queries},
    state::AppState,
};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use vn_core::{
    chat::{ChatOptions, Persona},
    companion::{DialogueSource, Knowledge, Lifecycle, SessionOptions},
    model::{Game, GameStatus},
};

pub(super) async fn fixture() -> (AppState, SessionOptions) {
    fixture_with_probe(Arc::new(crate::platform::probe::UnsupportedProbe)).await
}

async fn fixture_with_probe(
    probe: Arc<dyn crate::platform::probe::SystemProbe>,
) -> (AppState, SessionOptions) {
    let state = AppState::for_companion_test(Db::open_memory().await.unwrap(), probe).await;
    let game = Game::new(
        state.device(),
        vn_core::Timestamp::now().unwrap(),
        "Synthetic VN",
        GameStatus::Playing,
    )
    .unwrap();
    queries::upsert_game(state.db().pool(), &game)
        .await
        .unwrap();
    configure(&state, "http://127.0.0.1:9").await;
    let options = SessionOptions {
        game_id: game.meta.id.to_string(),
        playthrough_label: "First reading".into(),
        knowledge: Knowledge::Observed,
        boundary: String::new(),
        source: DialogueSource::Manual,
        language: "en".into(),
        persona: Persona::Companion,
        profile: None,
    };
    (state, options)
}

pub(super) async fn configure(state: &AppState, url: &str) {
    // Unique credential slot: tests never read or transmit a real saved key.
    let id = uuid::Uuid::new_v4().to_string();
    profiles::store(
        state.db().pool(),
        &profiles::Profiles {
            active: Some(id.clone()),
            profiles: vec![profiles::Profile {
                id,
                name: "Test provider".into(),
                provider: crate::ai::chat::Provider::OpenAi,
                base_url: url.into(),
                model: "synthetic-model".into(),
            }],
        },
    )
    .await
    .unwrap();
}

pub(super) async fn start(state: &AppState, options: SessionOptions) -> store::Session {
    state
        .companion()
        .start(state, &uuid::Uuid::new_v4().to_string(), options)
        .await
        .unwrap()
        .session
        .unwrap()
}

async fn enable_luna(state: &AppState) {
    state
        .luna()
        .save(
            state.db().pool(),
            crate::tracking::luna::LunaSettings {
                enabled: true,
                base_url: "http://127.0.0.1:2333".into(),
            },
        )
        .await
        .unwrap();
    state.hook().set_connected(true).await;
}

#[tokio::test]
async fn beta_disable_revokes_work_and_preserves_the_session_and_luna_config() {
    let (state, options) = fixture().await;
    let session = start(&state, options).await;
    enable_luna(&state).await;
    assert!(crate::commands::prefs::require_beta(&state).await.is_err());
    queries::set_pref(state.db().pool(), "beta_features", "true")
        .await
        .unwrap();
    crate::commands::prefs::require_beta(&state).await.unwrap();
    let (lease, cancelled) = state.chat().enter("beta-test").unwrap();
    state.chat().cancel_all();
    assert!(*cancelled.borrow());
    drop(lease);

    let work = state
        .companion()
        .prepare(
            &state,
            &session.id,
            &uuid::Uuid::new_v4().to_string(),
            "A pending question",
        )
        .await
        .unwrap()
        .unwrap();
    queries::set_pref(state.db().pool(), "beta_features", "false")
        .await
        .unwrap();
    state.luna().apply_beta(state.db().pool(), false).await;
    state.companion().suspend_beta(&state).await.unwrap();
    let snapshot = state.companion().snapshot().unwrap();
    assert_eq!(snapshot.lifecycle, Lifecycle::Paused);
    assert_eq!(snapshot.source.line_count, 0);
    assert!(
        !state
            .companion()
            .lock()
            .unwrap()
            .machine
            .accepts(work.token)
    );
    assert_eq!(
        store::get(state.db().pool(), &session.id)
            .await
            .unwrap()
            .state,
        "paused"
    );
    assert_eq!(store::list(state.db().pool()).await.unwrap().len(), 1);
    assert!(!state.luna().settings().enabled);
    assert!(
        crate::tracking::luna::LunaMonitor::load(state.db().pool())
            .await
            .enabled
    );
    state.luna().apply_beta(state.db().pool(), true).await;
    assert!(state.luna().settings().enabled);
    assert_eq!(
        state.companion().snapshot().unwrap().lifecycle,
        Lifecycle::Paused
    );
}

#[tokio::test]
async fn migration_preserves_existing_chat_and_foreign_keys() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::raw_sql(include_str!("../db/migrations/0014_chat.sql"))
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO chat_threads VALUES('old','Existing chat',?,1,1)")
        .bind(r#"{"gameId":null,"spoilers":"none","boundary":"","allowOnline":false,"language":"en"}"#)
        .execute(&pool).await.unwrap();
    sqlx::raw_sql(include_str!("../db/migrations/0030_companion_sessions.sql"))
        .execute(&pool)
        .await
        .unwrap();
    let old = chat::get(&pool, "old").await.unwrap();
    assert_eq!(old.title, "Existing chat");
    assert_eq!(old.options.persona, Persona::Companion);
    assert_eq!(chat::list(&pool).await.unwrap().len(), 1);
    assert!(
        sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&pool)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn session_creation_is_idempotent_and_ordinary_chat_cannot_take_ownership() {
    let (state, options) = fixture().await;
    let session = start(&state, options.clone()).await;
    let repeated = state
        .companion()
        .start(&state, &session.id, options.clone())
        .await
        .unwrap();
    assert_eq!(repeated.session.unwrap().thread_id, session.thread_id);
    assert_eq!(store::list(state.db().pool()).await.unwrap().len(), 1);
    assert!(chat::list(state.db().pool()).await.unwrap().is_empty());
    assert_eq!(
        store::require_regular_chat(state.db().pool(), &session.thread_id)
            .await
            .unwrap_err(),
        "companion_owned"
    );
    let regular = chat::create(
        state.db().pool(),
        ChatOptions {
            language: "en".into(),
            ..ChatOptions::default()
        },
    )
    .await
    .unwrap();
    store::require_regular_chat(state.db().pool(), &regular.id)
        .await
        .unwrap();
    assert_eq!(chat::list(state.db().pool()).await.unwrap().len(), 1);
    assert!(
        state
            .companion()
            .start(&state, &uuid::Uuid::new_v4().to_string(), options)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn source_cursor_excludes_pre_session_paused_and_reconnected_dialogue() {
    let (state, mut options) = fixture().await;
    enable_luna(&state).await;
    state.hook().push_external("OLD GAME").await;
    options.source = DialogueSource::Luna;
    let session = start(&state, options).await;
    assert_eq!(state.companion().snapshot().unwrap().source.line_count, 0);
    state.hook().push_external("Current scene").await;
    state.companion().refresh(&state).await.unwrap();
    assert_eq!(
        state.companion().snapshot().unwrap().source.preview,
        ["Current scene"]
    );
    state
        .companion()
        .control(&state, &session.id, Action::Pause)
        .await
        .unwrap();
    state.hook().push_external("While paused").await;
    state
        .companion()
        .control(&state, &session.id, Action::Resume)
        .await
        .unwrap();
    state.companion().refresh(&state).await.unwrap();
    assert_eq!(state.companion().snapshot().unwrap().source.line_count, 0);
    state.hook().set_connected(false).await;
    state.hook().set_connected(true).await;
    state
        .hook()
        .push_external("Before reconnect confirmation")
        .await;
    state.companion().refresh(&state).await.unwrap();
    assert_eq!(
        state.companion().snapshot().unwrap().lifecycle,
        Lifecycle::Paused
    );
    state
        .companion()
        .control(&state, &session.id, Action::Resume)
        .await
        .unwrap();
    state.hook().push_external("After confirmation").await;
    state.companion().refresh(&state).await.unwrap();
    assert_eq!(
        state.companion().snapshot().unwrap().source.preview,
        ["After confirmation"]
    );
}

#[tokio::test]
async fn source_change_requires_a_new_context_and_an_earlier_save_gets_a_new_transcript() {
    let (state, mut options) = fixture().await;
    enable_luna(&state).await;
    options.source = DialogueSource::Luna;
    let session = start(&state, options.clone()).await;
    state
        .luna()
        .save(
            state.db().pool(),
            crate::tracking::luna::LunaSettings {
                enabled: true,
                base_url: "http://127.0.0.1:2334".into(),
            },
        )
        .await
        .unwrap();
    state.hook().set_connected(true).await;
    state.companion().refresh(&state).await.unwrap();
    assert_eq!(
        state
            .companion()
            .control(&state, &session.id, Action::Resume)
            .await
            .unwrap_err(),
        "source_changed"
    );
    state
        .companion()
        .control(&state, &session.id, Action::End)
        .await
        .unwrap();
    let replay = start(&state, options).await;
    assert_ne!(replay.playthrough_id, session.playthrough_id);
    assert_ne!(replay.thread_id, session.thread_id);
    assert!(
        store::messages(state.db().pool(), &replay.id)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn deleted_parent_ends_the_run_without_exposing_its_chat_as_regular() {
    let (state, options) = fixture().await;
    let session = start(&state, options).await;
    sqlx::query("UPDATE games SET deleted=1 WHERE id=?")
        .bind(&session.options.game_id)
        .execute(state.db().pool())
        .await
        .unwrap();
    state.companion().refresh(&state).await.unwrap();
    assert_eq!(
        state.companion().snapshot().unwrap().lifecycle,
        Lifecycle::Off
    );
    let histories = store::list(state.db().pool()).await.unwrap();
    assert_eq!(histories.len(), 1);
    assert_eq!(histories[0].id, session.id);
    assert!(histories[0].game_title.is_none());
    assert!(
        store::messages(state.db().pool(), &session.id)
            .await
            .is_ok()
    );
    assert_eq!(
        store::get(state.db().pool(), &session.id)
            .await
            .unwrap_err(),
        "context_game_missing"
    );
    assert!(
        store::require_regular_chat(state.db().pool(), &session.thread_id)
            .await
            .is_err()
    );
    state.companion().delete(&state, &session.id).await.unwrap();
    assert!(
        chat::get(state.db().pool(), &session.thread_id)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn a_crash_keeps_history_but_never_resumes_observation_or_generation() {
    let (state, options) = fixture().await;
    let session = start(&state, options).await;
    let thread = chat::get(state.db().pool(), &session.thread_id)
        .await
        .unwrap();
    chat::begin(state.db().pool(), &thread, "A private question", false)
        .await
        .unwrap();
    chat::recover(state.db().pool()).await.unwrap();
    store::recover(state.db().pool()).await.unwrap();
    assert_eq!(
        store::get(state.db().pool(), &session.id)
            .await
            .unwrap()
            .state,
        "ended"
    );
    assert_eq!(
        store::messages(state.db().pool(), &session.id)
            .await
            .unwrap()[1]
            .status,
        "error"
    );
    assert_eq!(
        CompanionRuntime::default().snapshot().unwrap().lifecycle,
        Lifecycle::Off
    );
}

/// A real HTTP/SSE exchange with an entirely local fake provider. It sends a
/// partial answer then waits for the test to permit completion. No paid calls.
pub(super) async fn provider(
    state: &AppState,
) -> (
    tokio::sync::oneshot::Receiver<String>,
    tokio::sync::oneshot::Sender<()>,
    tokio::task::JoinHandle<()>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    configure(
        state,
        &format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port()),
    )
    .await;
    let (seen_tx, seen_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        let mut buf = [0_u8; 4096];
        loop {
            let n = socket.read(&mut buf).await.unwrap();
            if n == 0 {
                return;
            }
            request.extend_from_slice(&buf[..n]);
            if let Some(end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&request[..end]).to_lowercase();
                assert!(
                    !headers.contains("authorization:"),
                    "test must not transmit a credential"
                );
                let len: usize = headers
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length:"))
                    .unwrap()
                    .trim()
                    .parse()
                    .unwrap();
                if request.len() >= end + 4 + len {
                    break;
                }
            }
        }
        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\ndata: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"A quiet scene.\"},\"finish_reason\":null}]}\n\n").await.unwrap();
        let _ = seen_tx.send(String::from_utf8(request).unwrap());
        let _ = release_rx.await;
        let _ = socket.write_all(b"data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\" We can discuss it.\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n").await;
    });
    (seen_rx, release_tx, task)
}

pub(super) async fn wait_for_partial(state: &AppState) {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            if state
                .companion()
                .snapshot()
                .unwrap()
                .live
                .is_some_and(|message| !message.content.is_empty())
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn no_subscribers_and_repeated_send_ids_do_not_cancel_or_duplicate_a_turn() {
    let (state, options) = fixture().await;
    let state = Arc::new(state);
    let session = start(&state, options).await;
    let (seen, release, server) = provider(&state).await;
    let request = uuid::Uuid::new_v4().to_string();
    let work = state
        .companion()
        .prepare(&state, &session.id, &request, "What is happening?")
        .await
        .unwrap()
        .unwrap();
    let other_request = uuid::Uuid::new_v4().to_string();
    assert_eq!(
        state
            .companion()
            .prepare(&state, &session.id, &other_request, "Another window")
            .await
            .unwrap_err(),
        "chat_busy"
    );
    assert_eq!(
        state.companion().snapshot().unwrap().request_id.as_deref(),
        Some(request.as_str())
    );
    assert!(
        state
            .companion()
            .prepare(&state, &session.id, &request, "What is happening?")
            .await
            .unwrap()
            .is_none()
    );
    let runner = {
        let state = state.clone();
        tokio::spawn(async move {
            state.companion().run(&state, work).await;
        })
    };
    seen.await.unwrap();
    wait_for_partial(&state).await;
    let snapshot = state.companion().snapshot().unwrap();
    assert!(snapshot.busy);
    assert_eq!(snapshot.request_id.as_deref(), Some(request.as_str()));
    assert_eq!(snapshot.live.unwrap().content, "A quiet scene.");
    // No notifier was ever attached. Taking snapshots has no ownership effect.
    release.send(()).unwrap();
    runner.await.unwrap();
    server.await.unwrap();
    let messages = store::messages(state.db().pool(), &session.id)
        .await
        .unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[1].status, "complete");
    assert_eq!(messages[1].content, "A quiet scene. We can discuss it.");
    assert!(
        state
            .companion()
            .prepare(&state, &session.id, &request, "What is happening?")
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn ending_a_stream_discards_late_text_and_keeps_cancelled_output_out_of_replay() {
    let (state, options) = fixture().await;
    let state = Arc::new(state);
    let session = start(&state, options).await;
    let (seen, release, server) = provider(&state).await;
    let work = state
        .companion()
        .prepare(
            &state,
            &session.id,
            &uuid::Uuid::new_v4().to_string(),
            "What is happening?",
        )
        .await
        .unwrap()
        .unwrap();
    let runner = {
        let state = state.clone();
        tokio::spawn(async move {
            state.companion().run(&state, work).await;
        })
    };
    seen.await.unwrap();
    wait_for_partial(&state).await;
    state
        .companion()
        .control(&state, &session.id, Action::End)
        .await
        .unwrap();
    release.send(()).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(10), runner)
        .await
        .unwrap()
        .unwrap();
    server.await.unwrap();
    let messages = store::messages(state.db().pool(), &session.id)
        .await
        .unwrap();
    assert_eq!(messages[1].status, "cancelled");
    assert!(!messages[1].content.contains("We can discuss"));
    let history =
        conversation::build_history(&session.options.chat_options(), &messages, &mut Vec::new());
    assert!(!history.iter().any(|message| message["role"] == "assistant"));
    assert!(!state.companion().snapshot().unwrap().busy);
}

#[tokio::test]
async fn connection_generation_blocks_callbacks_before_the_monitor_can_run() {
    let (state, mut options) = fixture().await;
    enable_luna(&state).await;
    options.source = DialogueSource::Luna;
    let session = start(&state, options).await;
    let work = state
        .companion()
        .prepare(
            &state,
            &session.id,
            &uuid::Uuid::new_v4().to_string(),
            "Question",
        )
        .await
        .unwrap()
        .unwrap();
    let sink = ReadingSink {
        memory_revision: state.companion().lock().unwrap().memory_revision,
        runtime: state.companion(),
        hook: state.hook(),
        token: work.token,
        source_generation: work.source_generation,
    };
    assert!(sink.is_current());
    state.hook().set_connected(false).await;
    assert!(!sink.is_current());
    assert!(
        sink.send(ChatEvent::Delta {
            text: "Late secret".into()
        })
        .is_err()
    );
    assert!(state.companion().snapshot().unwrap().live.is_none());
}

#[tokio::test]
async fn controls_revoke_delivery_before_waiting_for_a_database_commit() {
    for action in [Action::CancelTurn, Action::Pause, Action::End] {
        let (state, options) = fixture().await;
        let state = Arc::new(state);
        let session = start(&state, options).await;
        let work = state
            .companion()
            .prepare(
                &state,
                &session.id,
                &uuid::Uuid::new_v4().to_string(),
                "Question",
            )
            .await
            .unwrap()
            .unwrap();
        let sink = ReadingSink {
            memory_revision: state.companion().lock().unwrap().memory_revision,
            runtime: state.companion(),
            hook: state.hook(),
            token: work.token,
            source_generation: work.source_generation,
        };
        let gate = state.companion().mutation.lock().await;
        let control = {
            let state = state.clone();
            let id = session.id.clone();
            tokio::spawn(async move { state.companion().control(&state, &id, action).await })
        };
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while sink.is_current() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("database work must not delay turn invalidation");
        assert!(
            *state
                .companion()
                .lock()
                .unwrap()
                .cancel
                .as_ref()
                .unwrap()
                .borrow()
        );
        assert!(
            sink.send(ChatEvent::Delta {
                text: "Late text".into()
            })
            .is_err()
        );
        drop(gate);
        control.await.unwrap().unwrap();
    }
}

#[tokio::test]
async fn controls_for_an_old_session_cannot_interrupt_a_new_one() {
    let (state, options) = fixture().await;
    let old = start(&state, options.clone()).await;
    state
        .companion()
        .control(&state, &old.id, Action::End)
        .await
        .unwrap();
    let current = start(&state, options).await;
    let work = state
        .companion()
        .prepare(
            &state,
            &current.id,
            &uuid::Uuid::new_v4().to_string(),
            "Question",
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        state
            .companion()
            .control(&state, &old.id, Action::End)
            .await
            .unwrap_err(),
        "companion_missing"
    );
    assert!(
        state
            .companion()
            .lock()
            .unwrap()
            .machine
            .accepts(work.token)
    );
    assert!(
        !*state
            .companion()
            .lock()
            .unwrap()
            .cancel
            .as_ref()
            .unwrap()
            .borrow()
    );
}

#[tokio::test]
async fn companion_text_tools_never_fall_back_to_unbound_or_forbidden_dialogue() {
    use conversation::tools::{ReadingContext, ToolContext};
    let (state, options) = fixture().await;
    state
        .hook()
        .push_external("UNBOUND SECRET FROM A DIFFERENT GAME")
        .await;
    let chat_options = options.chat_options();
    let client = crate::vndb::http_client_builder().build().unwrap();
    let mut reading = ReadingContext {
        memory: memory::TurnMemory::default(),
        system_prompt: None,
        game_id: options.game_id,
        playthrough_id: uuid::Uuid::new_v4().to_string(),
        knowledge: Knowledge::Observed,
        source: DialogueSource::Luna,
        lines: Vec::new(),
    };
    for knowledge in [Knowledge::Observed, Knowledge::Metadata] {
        reading.knowledge = knowledge;
        let context = ToolContext {
            pool: state.db().pool(),
            vndb: state.vndb(),
            client: &client,
            options: &chat_options,
            hook: state.hook(),
            reading: Some(&reading),
        };
        let result = conversation::tools::run(&context, "recent_game_text", "{}", &mut Vec::new())
            .await
            .unwrap();
        assert_eq!(result["data"]["lines"], serde_json::json!([]));
        assert!(!result.to_string().contains("UNBOUND SECRET"));
        let digest = conversation::tools::run(&context, "script_digest", "{}", &mut Vec::new())
            .await
            .unwrap();
        assert!(digest["data"]["withheld"].is_string());
    }
}

#[tokio::test]
async fn companion_session_and_transcript_are_absent_from_library_exports_and_sync() {
    let (state, mut options) = fixture().await;
    options.playthrough_label = "LOCAL_ONLY_COMPANION_CONTEXT".into();
    let mut settings = vn_core::companion::profile::Preset::Companion
        .snapshot()
        .settings;
    settings.name = "LOCAL_ONLY_COMPANION_PROFILE".into();
    let profile = super::profiles::save(state.db().pool(), None, None, settings)
        .await
        .unwrap();
    options.profile = Some(profile.clone());
    let session = start(&state, options).await;
    let thread = chat::get(state.db().pool(), &session.thread_id)
        .await
        .unwrap();
    chat::begin(
        state.db().pool(),
        &thread,
        "LOCAL_ONLY_COMPANION_QUESTION",
        false,
    )
    .await
    .unwrap();
    let library = crate::commands::data::collect_library_file(
        state.db().pool(),
        state.device(),
        state.data_dir(),
    )
    .await
    .unwrap();
    let sync = crate::sync::collect_local(state.db().pool(), state.device())
        .await
        .unwrap();
    for payload in [
        serde_json::to_string(&library).unwrap(),
        serde_json::to_string(&sync).unwrap(),
    ] {
        assert!(!payload.contains("LOCAL_ONLY_COMPANION"));
        assert!(!payload.contains(&session.id));
        assert!(!payload.contains(&session.thread_id));
        assert!(!payload.contains(&profile.id));
    }
}

#[derive(Debug)]
struct ClosingProbe(std::sync::atomic::AtomicBool);

impl crate::platform::probe::SystemProbe for ClosingProbe {
    fn list_processes(
        &self,
    ) -> Result<Vec<crate::platform::probe::RunningProcess>, crate::platform::probe::ProbeError>
    {
        Ok(Vec::new())
    }
    fn focused_pid(&self) -> Result<Option<u32>, crate::platform::probe::ProbeError> {
        Ok(Some(42))
    }
    fn focused_window(
        &self,
    ) -> Result<Option<crate::platform::probe::FocusedWindow>, crate::platform::probe::ProbeError>
    {
        Ok(None)
    }
    fn presence(
        &self,
    ) -> Result<crate::platform::probe::Presence, crate::platform::probe::ProbeError> {
        Ok(crate::platform::probe::Presence { idle_for_ms: 0 })
    }
    fn is_running(&self, pid: u32) -> Result<bool, crate::platform::probe::ProbeError> {
        assert_eq!(pid, 42);
        Ok(self.0.load(std::sync::atomic::Ordering::Acquire))
    }
}

#[tokio::test]
async fn a_known_game_closing_pauses_the_companion_without_modifying_tracking() {
    let probe = Arc::new(ClosingProbe(std::sync::atomic::AtomicBool::new(true)));
    let (state, options) = fixture_with_probe(probe.clone()).await;
    start(&state, options).await;
    state.companion().lock().unwrap().process_id = Some(42);
    state.companion().refresh(&state).await.unwrap();
    assert_eq!(
        state.companion().snapshot().unwrap().lifecycle,
        Lifecycle::Active
    );
    probe.0.store(false, std::sync::atomic::Ordering::Release);
    state.companion().refresh(&state).await.unwrap();
    let snapshot = state.companion().snapshot().unwrap();
    assert_eq!(snapshot.lifecycle, Lifecycle::Paused);
    assert_eq!(
        snapshot.session.unwrap().reason.as_deref(),
        Some("game_closed")
    );
    assert_eq!(state.tracker().companion_target().await, None);
}

/// Reproducible native UI fixture. It cannot target the production identifier.
#[tokio::test]
#[ignore = "requires explicit VN_ASYIK_SMOKE_DB and VN_ASYIK_SMOKE_URL for an isolated native UI check"]
async fn seed_companion_ui_fixture() {
    let path = std::path::PathBuf::from(
        std::env::var("VN_ASYIK_SMOKE_DB").expect("set the isolated DB path"),
    );
    assert_eq!(path.file_name().unwrap(), "vn-tracker.sqlite");
    assert_eq!(
        path.parent().unwrap().file_name().unwrap(),
        "com.atho.vnasyik.companion-smoke"
    );
    let url = std::env::var("VN_ASYIK_SMOKE_URL").expect("set the local test server URL");
    assert!(url.starts_with("http://127.0.0.1:"));
    let state = AppState::for_companion_test(
        Db::open(&path).await.unwrap(),
        Arc::new(crate::platform::probe::UnsupportedProbe),
    )
    .await;
    let game = Game::new(
        state.device(),
        vn_core::Timestamp::now().unwrap(),
        "Companion Test VN",
        GameStatus::Playing,
    )
    .unwrap();
    queries::upsert_game(state.db().pool(), &game)
        .await
        .unwrap();
    configure(&state, &url).await;
    queries::set_pref(state.db().pool(), "language", "en")
        .await
        .unwrap();
    queries::set_pref(state.db().pool(), "theme", "dark")
        .await
        .unwrap();
    state
        .luna()
        .save(
            state.db().pool(),
            crate::tracking::luna::LunaSettings {
                enabled: true,
                base_url: url,
            },
        )
        .await
        .unwrap();
    state.db().close().await;
}
