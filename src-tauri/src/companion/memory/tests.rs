use super::*;
use crate::companion::tests::{fixture, start};

fn request(body: &str, revision: i64) -> Save {
    Save {
        expected_companion_id: None,
        id: None,
        expected_version: None,
        scope: Scope::Shared,
        class: Class::Preference,
        body: body.into(),
        pinned: false,
        session_id: None,
        message_id: None,
        request_id: uuid::Uuid::new_v4().to_string(),
        expected_revision: revision,
    }
}

#[tokio::test]
async fn defaults_and_constraints_preserve_local_history() {
    let (state, options) = fixture().await;
    let pool = state.db().pool();
    let session = start(&state, options).await;
    let settings = settings(pool).await.unwrap();
    assert!(!settings.retrieval_enabled && !settings.auto_summary);
    assert_eq!(settings.revision, 0);
    assert!(
        sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(pool)
            .await
            .unwrap()
            .is_empty()
    );
    let saved = state
        .companion()
        .memory_save(&state, request("Prefer short replies", 0))
        .await
        .unwrap();
    assert!(saved.origin_session.is_none());
    assert!(
        sqlx::query("UPDATE companion_memories SET scope='reading' WHERE id=?")
            .bind(saved.id)
            .execute(pool)
            .await
            .is_err()
    );
    assert!(
        crate::companion::store::get(pool, &session.id)
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn forget_blocks_replay_and_clears_evidence_but_keeps_chat_visible() {
    let (state, options) = fixture().await;
    let session = start(&state, options).await;
    let pool = state.db().pool();
    let thread = conversation_thread(pool, &session.thread_id).await;
    crate::ai::conversation::store::begin(pool, &thread, "old private story", false)
        .await
        .unwrap();
    let saved = state
        .companion()
        .memory_save(&state, request("private preference", 0))
        .await
        .unwrap();
    state
        .companion()
        .memory_forget(
            &state,
            Forget::Item {
                id: saved.id.clone(),
                version: 1,
            },
            1,
        )
        .await
        .unwrap();
    assert!(live(pool).await.unwrap().is_empty());
    assert!(sources(pool, &saved.id, 1).await.unwrap().is_empty());
    assert!(
        crate::ai::conversation::store::model_messages(pool, &session.thread_id)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        crate::ai::conversation::store::messages(pool, &session.thread_id)
            .await
            .unwrap()
            .iter()
            .any(|m| m.content.contains("old private story"))
    );
    let (body,evidence):(String,String) = sqlx::query_as("SELECT m.body,s.evidence FROM companion_memories m JOIN companion_memory_sources s ON s.memory_id=m.id WHERE m.id=?")
        .bind(saved.id).fetch_one(pool).await.unwrap();
    assert!(body.is_empty() && evidence.is_empty());
    crate::ai::conversation::store::begin(pool, &thread, "new context", false)
        .await
        .unwrap();
    let fresh = crate::ai::conversation::store::model_messages(pool, &session.thread_id)
        .await
        .unwrap();
    assert!(fresh.iter().any(|m| m.content == "new context"));
    assert!(!fresh.iter().any(|m| m.content == "old private story"));
}

async fn conversation_thread(pool: &SqlitePool, id: &str) -> crate::ai::conversation::ChatThread {
    crate::ai::conversation::store::get(pool, id).await.unwrap()
}

#[tokio::test]
async fn optimistic_edit_does_not_widen_scope_or_restore_forgotten_requests() {
    let (state, options) = fixture().await;
    let session = start(&state, options).await;
    let mut input = request("We met at the station", 0);
    input.scope = Scope::Reading;
    input.class = Class::Observation;
    input.session_id = Some(session.id.clone());
    let saved = state.companion().memory_save(&state, input).await.unwrap();
    let mut edit = request("We met at the river", 1);
    edit.id = Some(saved.id.clone());
    edit.expected_version = Some(1);
    let edited = state.companion().memory_save(&state, edit).await.unwrap();
    assert_eq!(edited.scope, Scope::Reading);
    assert_eq!(edited.knowledge, Knowledge::Observed);
    let mut outdated = request("old text", 2);
    outdated.id = Some(saved.id);
    outdated.expected_version = Some(1);
    assert_eq!(
        state
            .companion()
            .memory_save(&state, outdated)
            .await
            .unwrap_err(),
        "memory_stale"
    );
}

#[tokio::test]
async fn source_copy_survives_rollover_and_assistant_guesses_stay_inferred() {
    let (state, options) = fixture().await;
    let session = start(&state, options).await;
    let pool = state.db().pool();
    let thread = conversation_thread(pool, &session.thread_id).await;
    let answer = crate::ai::conversation::store::begin(pool, &thread, "An ambiguous scene", false)
        .await
        .unwrap();
    sqlx::query("UPDATE chat_messages SET content='Perhaps Aki wrote the letter',status='complete' WHERE id=?")
        .bind(&answer).execute(pool).await.unwrap();
    let mut save = request("Perhaps Aki wrote the letter", 0);
    save.scope = Scope::Reading;
    save.class = Class::Episode;
    save.session_id = Some(session.id.clone());
    save.message_id = Some(answer.clone());
    let memory = state.companion().memory_save(&state, save).await.unwrap();
    assert_eq!(memory.evidence, Evidence::Inference);
    for _ in 0..105 {
        crate::ai::conversation::store::begin(pool, &thread, "Next scene", false)
            .await
            .unwrap();
    }
    assert!(
        !crate::ai::conversation::store::messages(pool, &session.thread_id)
            .await
            .unwrap()
            .iter()
            .any(|m| m.id == answer)
    );
    assert_eq!(
        sources(pool, &memory.id, 1).await.unwrap()[0].evidence,
        "Perhaps Aki wrote the letter"
    );
    let mut edit = request(&memory.body, 1);
    edit.scope = Scope::Reading;
    edit.id = Some(memory.id.clone());
    edit.expected_version = Some(1);
    edit.pinned = true;
    let memory = state.companion().memory_save(&state, edit).await.unwrap();
    assert_eq!(memory.evidence, Evidence::Inference);
    assert_eq!(
        sources(pool, &memory.id, 2).await.unwrap()[0]
            .message_id
            .as_deref(),
        Some(answer.as_str())
    );
}

#[tokio::test]
async fn trimming_captures_evidence_atomically_and_restart_never_retries() {
    let (state, options) = fixture().await;
    let session = start(&state, options).await;
    let pool = state.db().pool();
    state
        .companion()
        .memory_settings(&state, false, true, 0)
        .await
        .unwrap();
    let thread = conversation_thread(pool, &session.thread_id).await;
    for i in 0..99 {
        let answer =
            crate::ai::conversation::store::begin(pool, &thread, &format!("Scene {i}"), false)
                .await
                .unwrap();
        sqlx::query("UPDATE chat_messages SET status='complete',content='The reader saw a letter' WHERE id=?").bind(answer).execute(pool).await.unwrap();
    }
    let config = crate::ai::profiles::resolve(pool).await.config;
    let memory = state
        .companion()
        .capture_memory(&state, &session, "Next", &config)
        .await
        .unwrap();
    let mut tx = pool.begin().await.unwrap();
    // A rollback undoes the input copy too, so no job claims trimmed evidence
    // from a conversation append that never committed.
    jobs::capture(&mut tx, memory.capture.as_ref().unwrap(), Some(4), true)
        .await
        .unwrap();
    tx.rollback().await.unwrap();
    assert!(jobs::list(pool).await.unwrap().is_empty());
    crate::ai::conversation::store::begin_scoped(pool, &thread, "Scene 100", false, Some(&memory))
        .await
        .unwrap();
    let jobs = jobs::list(pool).await.unwrap();
    assert_eq!(jobs.len(), 1);
    let (input,): (String,) = sqlx::query_as("SELECT input FROM companion_memory_jobs WHERE id=?")
        .bind(&jobs[0].id)
        .fetch_one(pool)
        .await
        .unwrap();
    assert!(input.contains("Scene 0"));
    recover(pool).await.unwrap();
    assert_eq!(jobs::list(pool).await.unwrap()[0].state, "paused");
    assert!(
        state
            .companion()
            .prepare_summary(&state, None)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn queued_recaps_are_idempotent_and_forget_clears_inputs() {
    let (state, options) = fixture().await;
    let session = start(&state, options).await;
    let pool = state.db().pool();
    let thread = conversation_thread(pool, &session.thread_id).await;
    crate::ai::conversation::store::begin(pool, &thread, "Sensitive observation", false)
        .await
        .unwrap();
    let first = state
        .companion()
        .suggest_recap(&state, &session.id)
        .await
        .unwrap();
    let second = state
        .companion()
        .suggest_recap(&state, &session.id)
        .await
        .unwrap();
    assert_eq!(first, second);
    state
        .companion()
        .memory_forget(&state, Forget::Everything, 0)
        .await
        .unwrap();
    let (input, output, prompt, status): (String, String, String, String) =
        sqlx::query_as("SELECT input,output,prompt,state FROM companion_memory_jobs WHERE id=?")
            .bind(first)
            .fetch_one(pool)
            .await
            .unwrap();
    assert!(input.is_empty() && output.is_empty() && prompt.is_empty());
    assert_eq!(status, "cancelled");
    assert_eq!(
        state
            .companion()
            .suggest_recap(&state, &session.id)
            .await
            .unwrap_err(),
        "memory_summary_gap"
    );
}

#[tokio::test]
async fn forget_revokes_before_the_gate_and_rejects_late_streamed_output() {
    use crate::{
        ai::conversation::{ChatEvent, EventSink},
        companion::{ReadingSink, tests::provider},
    };
    use std::sync::Arc;
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
            "Question",
        )
        .await
        .unwrap()
        .unwrap();
    let sink = ReadingSink {
        runtime: state.companion(),
        hook: state.hook(),
        token: work.token,
        source_generation: work.source_generation,
        memory_revision: 0,
    };
    let runner = {
        let state = state.clone();
        tokio::spawn(async move {
            state.companion().run(&state, work).await;
        })
    };
    seen.await.unwrap();
    let gate = state.companion().mutation.lock().await;
    let forgetting = {
        let state = state.clone();
        tokio::spawn(async move {
            state
                .companion()
                .memory_forget(&state, Forget::Everything, 0)
                .await
        })
    };
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while sink.is_current() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(
        sink.send(ChatEvent::Delta {
            text: "Late forgotten text".into()
        })
        .is_err()
    );
    drop(gate);
    forgetting.await.unwrap().unwrap();
    let _ = release.send(());
    runner.await.unwrap();
    server.await.unwrap();
    assert!(
        !crate::ai::conversation::store::model_messages(state.db().pool(), &session.thread_id)
            .await
            .unwrap()
            .iter()
            .any(|m| m.role == "assistant" && m.status == "complete")
    );
}

#[tokio::test]
async fn failed_correction_rolls_back_body_revision_sources_and_barriers() {
    let (state, options) = fixture().await;
    let session = start(&state, options).await;
    let pool = state.db().pool();
    let memory = state
        .companion()
        .memory_save(&state, request("old preference", 0))
        .await
        .unwrap();
    sqlx::raw_sql("CREATE TRIGGER reject_memory_update BEFORE UPDATE OF body ON companion_memories BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").execute(pool).await.unwrap();
    let mut edit = request("new preference", 1);
    edit.id = Some(memory.id.clone());
    edit.expected_version = Some(1);
    assert_eq!(
        state
            .companion()
            .memory_save(&state, edit)
            .await
            .unwrap_err(),
        "db_error"
    );
    assert_eq!(settings(pool).await.unwrap().revision, 1);
    assert_eq!(live(pool).await.unwrap()[0].body, "old preference");
    assert_eq!(
        sources(pool, &memory.id, 1).await.unwrap()[0].evidence,
        "old preference"
    );
    assert!(!view(pool, Some(&session.id)).await.unwrap().replay_excluded);
}

#[tokio::test]
async fn session_deletion_forgets_derived_memory_but_keeps_independent_preferences() {
    let (state, options) = fixture().await;
    let session = start(&state, options).await;
    state
        .companion()
        .memory_save(&state, request("short replies", 0))
        .await
        .unwrap();
    let mut story = request("A letter at the station", 1);
    story.scope = Scope::Reading;
    story.class = Class::Observation;
    story.session_id = Some(session.id.clone());
    state.companion().memory_save(&state, story).await.unwrap();
    state
        .companion()
        .control(&state, &session.id, crate::companion::Action::End)
        .await
        .unwrap();
    state.companion().delete(&state, &session.id).await.unwrap();
    let saved = live(state.db().pool()).await.unwrap();
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].body, "short replies");
}

#[tokio::test]
async fn byte_pin_and_marker_limits_leave_forget_available() {
    let (state, _) = fixture().await;
    let pool = state.db().pool();
    assert_eq!(
        state
            .companion()
            .memory_save(&state, request(&"語".repeat(683), 0))
            .await
            .unwrap_err(),
        "invalid_memory"
    );
    let memory = state
        .companion()
        .memory_save(&state, request("preference", 0))
        .await
        .unwrap();
    sqlx::query("WITH RECURSIVE nums(n) AS (SELECT 1 UNION ALL SELECT n+1 FROM nums WHERE n<9999) INSERT INTO companion_memories(id,version,class,scope,knowledge,body,evidence,created_at,updated_at,deleted) SELECT CAST(n AS TEXT),1,'preference','shared','metadata','','reader',1,1,1 FROM nums")
        .execute(pool).await.unwrap();
    assert_eq!(
        state
            .companion()
            .memory_save(&state, request("new", 1))
            .await
            .unwrap_err(),
        "memory_capacity"
    );
    state
        .companion()
        .memory_forget(
            &state,
            Forget::Item {
                id: memory.id,
                version: 1,
            },
            1,
        )
        .await
        .unwrap();
    assert!(live(pool).await.unwrap().is_empty());
}

#[test]
fn replay_budget_counts_cached_json_and_utf8() {
    use crate::ai::conversation::{ChatMessage, ChatTrace, bounded_history};
    let mut messages = Vec::new();
    for i in 0..20 {
        messages.push(ChatMessage {
            id: format!("u{i}"),
            role: "user".into(),
            content: "A question".into(),
            status: "complete".into(),
            error: None,
            tools: Vec::new(),
            sources: Vec::new(),
            tokens: None,
            created_at: 1,
        });
        messages.push(ChatMessage {
            id: format!("a{i}"),
            role: "assistant".into(),
            content: "An answer".into(),
            status: "complete".into(),
            error: None,
            tools: vec![ChatTrace {
                id: "t".into(),
                name: "script_digest".into(),
                arguments: "{}".into(),
                status: "complete".into(),
                result: Some(serde_json::json!({"text":"語".repeat(5000)})),
            }],
            sources: Vec::new(),
            tokens: None,
            created_at: 1,
        });
    }
    let history = bounded_history("system", &messages, &mut Vec::new(), 40_000);
    let bytes = history
        .iter()
        .skip(1)
        .map(|m| m["content"].as_str().unwrap().len())
        .sum::<usize>();
    assert!(bytes <= 40_000);
    assert!(history.len() < messages.len());
}

async fn recap_provider(
    state: &crate::state::AppState,
) -> (
    tokio::sync::oneshot::Receiver<String>,
    tokio::sync::oneshot::Sender<()>,
    tokio::task::JoinHandle<()>,
) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    crate::companion::tests::configure(
        state,
        &format!("http://{}", listener.local_addr().unwrap()),
    )
    .await;
    let (seen_tx, seen_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        let mut buffer = [0u8; 8192];
        loop {
            let count = socket.read(&mut buffer).await.unwrap();
            if count == 0 {
                return;
            }
            request.extend_from_slice(&buffer[..count]);
            if let Some(end) = request.windows(4).position(|b| b == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&request[..end]).to_lowercase();
                assert!(!headers.contains("authorization:"));
                let length: usize = headers
                    .lines()
                    .find_map(|l| l.strip_prefix("content-length:"))
                    .unwrap()
                    .trim()
                    .parse()
                    .unwrap();
                if request.len() >= end + 4 + length {
                    break;
                }
            }
        }
        let _ = seen_tx.send(String::from_utf8(request).unwrap());
        let _ = release_rx.await;
        let body = r#"{"choices":[{"message":{"content":"The reader saw a letter. Its author remains uncertain."},"finish_reason":"stop"}]}"#;
        let _=socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await;
    });
    (seen_rx, release_tx, server)
}

#[tokio::test]
async fn recap_uses_restricted_inputs_and_requires_review_before_retrieval() {
    use std::sync::Arc;
    let (state, mut options) = fixture().await;
    options.knowledge = Knowledge::All;
    let state = Arc::new(state);
    let session = start(&state, options).await;
    let pool = state.db().pool();
    let (seen, release, server) = recap_provider(&state).await;
    let thread = conversation_thread(pool, &session.thread_id).await;
    let answer =
        crate::ai::conversation::store::begin(pool, &thread, "The reader saw a letter", false)
            .await
            .unwrap();
    sqlx::query("UPDATE chat_messages SET status='complete',content='The author is uncertain',tools_json=? WHERE id=?")
        .bind(r#"[{"id":"t","name":"script_digest","arguments":"{}","status":"complete","result":{"text":"ALL_POLICY_EVIDENCE"}}]"#).bind(answer).execute(pool).await.unwrap();
    let id = state
        .companion()
        .suggest_recap(&state, &session.id)
        .await
        .unwrap();
    let work = state
        .companion()
        .prepare_summary(&state, Some(&id))
        .await
        .unwrap()
        .unwrap();
    let runner = {
        let state = state.clone();
        tokio::spawn(async move {
            state.companion().run_summary(&state, work).await;
        })
    };
    let request = seen.await.unwrap();
    let body: serde_json::Value =
        serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
    assert!(body.get("tools").is_none());
    assert!(request.contains("ALL_POLICY_EVIDENCE"));
    release.send(()).unwrap();
    runner.await.unwrap();
    server.await.unwrap();
    assert_eq!(jobs::list(pool).await.unwrap()[0].state, "review");
    assert!(live(pool).await.unwrap().is_empty());
    state
        .companion()
        .review_recap(&state, &id, Some("The author remains uncertain"), false, 0)
        .await
        .unwrap();
    let memories = live(pool).await.unwrap();
    assert_eq!(memories.len(), 1);
    assert_eq!(memories[0].knowledge, Knowledge::All);
    assert_eq!(memories[0].evidence, Evidence::Inference);
    sqlx::query("DELETE FROM chat_messages WHERE thread_id=?")
        .bind(&session.thread_id)
        .execute(pool)
        .await
        .unwrap();
    assert!(
        sources(pool, &memories[0].id, 1)
            .await
            .unwrap()
            .iter()
            .any(|s| s.evidence.contains("ALL_POLICY_EVIDENCE"))
    );
    let (input,): (String,) = sqlx::query_as("SELECT input FROM companion_memory_jobs WHERE id=?")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap();
    assert!(input.is_empty());
}

#[tokio::test]
async fn forgetting_a_running_recap_rejects_the_late_provider_result() {
    use std::sync::Arc;
    let (state, options) = fixture().await;
    let state = Arc::new(state);
    let session = start(&state, options).await;
    let (seen, release, server) = recap_provider(&state).await;
    let pool = state.db().pool();
    let thread = conversation_thread(pool, &session.thread_id).await;
    crate::ai::conversation::store::begin(pool, &thread, "Private story observation", false)
        .await
        .unwrap();
    let id = state
        .companion()
        .suggest_recap(&state, &session.id)
        .await
        .unwrap();
    let work = state
        .companion()
        .prepare_summary(&state, Some(&id))
        .await
        .unwrap()
        .unwrap();
    let runner = {
        let state = state.clone();
        tokio::spawn(async move {
            state.companion().run_summary(&state, work).await;
        })
    };
    seen.await.unwrap();
    let gate = state.companion().mutation.lock().await;
    let forgetting = {
        let state = state.clone();
        tokio::spawn(async move {
            state
                .companion()
                .memory_forget(&state, Forget::Everything, 0)
                .await
        })
    };
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            if state
                .companion()
                .lock()
                .unwrap()
                .summary
                .as_ref()
                .is_some_and(|w| *w.cancel.borrow())
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    drop(gate);
    forgetting.await.unwrap().unwrap();
    let _ = release.send(());
    runner.await.unwrap();
    server.await.unwrap();
    assert!(jobs::list(pool).await.unwrap().is_empty());
    assert!(live(pool).await.unwrap().is_empty());
    let (input, output): (String, String) =
        sqlx::query_as("SELECT input,output FROM companion_memory_jobs WHERE id=?")
            .bind(&id)
            .fetch_one(pool)
            .await
            .unwrap();
    assert!(input.is_empty() && output.is_empty());
    assert_eq!(
        state
            .companion()
            .review_recap(&state, &id, Some("late result"), false, 1)
            .await
            .unwrap_err(),
        "memory_stale"
    );
}

#[tokio::test]
async fn memory_and_jobs_never_enter_library_exports_or_sync() {
    let (state, options) = fixture().await;
    let session = start(&state, options).await;
    let pool = state.db().pool();
    let memory = state
        .companion()
        .memory_save(&state, request("LOCAL_ONLY_MEMORY_SENTINEL", 0))
        .await
        .unwrap();
    let thread = conversation_thread(pool, &session.thread_id).await;
    crate::ai::conversation::store::begin(pool, &thread, "LOCAL_ONLY_JOB_SENTINEL", false)
        .await
        .unwrap();
    let id = state
        .companion()
        .suggest_recap(&state, &session.id)
        .await
        .unwrap();
    let library =
        crate::commands::data::collect_library_file(pool, state.device(), state.data_dir())
            .await
            .unwrap();
    let sync = crate::sync::collect_local(pool, state.device())
        .await
        .unwrap();
    for payload in [
        serde_json::to_string(&library).unwrap(),
        serde_json::to_string(&sync).unwrap(),
    ] {
        assert!(!payload.contains("LOCAL_ONLY_"));
        assert!(!payload.contains(&memory.id));
        assert!(!payload.contains(&id));
    }
}

#[tokio::test]
async fn upgrading_schema31_preserves_history_and_foreign_keys() {
    let (state, options) = fixture().await;
    let session = start(&state, options).await;
    let pool = state.db().pool();
    let thread = conversation_thread(pool, &session.thread_id).await;
    crate::ai::conversation::store::begin(pool, &thread, "Existing history", false)
        .await
        .unwrap();
    sqlx::raw_sql("DROP TABLE companion_memory_sources; DROP TABLE companion_memories; DROP TABLE companion_memory_jobs; DROP TABLE companion_memory_state; DROP TABLE companion_replay_barriers; DELETE FROM _sqlx_migrations WHERE version=32;")
        .execute(pool).await.unwrap();
    assert!(
        sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(pool)
            .await
            .unwrap()
            .is_empty()
    );
    state.db().migrate().await.unwrap();
    assert!(
        sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(pool)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        crate::companion::store::messages(pool, &session.id)
            .await
            .unwrap()[0]
            .content,
        "Existing history"
    );
    assert!(!settings(pool).await.unwrap().retrieval_enabled);
}

#[tokio::test]
async fn identity_changes_cannot_retarget_an_open_memory_editor() {
    let (state, options) = fixture().await;
    let session = start(&state, options).await;
    let original = Context::new(&session.options, &session.playthrough_id).companion_id;
    state
        .companion()
        .apply_profile(
            &state,
            &session.id,
            vn_core::companion::profile::Preset::Analyst.snapshot(),
        )
        .await
        .unwrap();
    let mut save = request("Prefer short replies", 0);
    save.scope = Scope::Companion;
    save.session_id = Some(session.id);
    save.expected_companion_id = Some(original);
    assert_eq!(
        state
            .companion()
            .memory_save(&state, save)
            .await
            .unwrap_err(),
        "memory_stale"
    );
}

#[tokio::test]
async fn acknowledged_save_cannot_be_reused_for_another_identity() {
    let (state, options) = fixture().await;
    let session = start(&state, options).await;
    let mut save = request("Prefer short replies", 0);
    save.scope = Scope::Companion;
    save.session_id = Some(session.id.clone());
    save.expected_companion_id =
        Some(Context::new(&session.options, &session.playthrough_id).companion_id);
    let saved = state
        .companion()
        .memory_save(&state, save.clone())
        .await
        .unwrap();
    assert_eq!(
        state
            .companion()
            .memory_save(&state, save.clone())
            .await
            .unwrap()
            .id,
        saved.id
    );
    save.expected_companion_id = Some("different-profile".into());
    assert_eq!(
        state
            .companion()
            .memory_save(&state, save)
            .await
            .unwrap_err(),
        "memory_stale"
    );
}
