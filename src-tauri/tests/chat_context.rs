//! Follow-up context, citation remapping and interrupted-answer persistence.
//!
//! These cover the failure modes that only appear on the *second* turn: a
//! replayed [S1] that now points at a different page, retrieved evidence being
//! dropped on the next question, and a stopped/truncated answer being replayed
//! to the model as if it were fact.

use serde_json::Value;
use vn_core::chat::ChatOptions;
use vn_tracker_lib::ai::conversation::{ChatMessage, Source, build_history, store};
use vn_tracker_lib::db::Db;

fn options() -> ChatOptions {
    ChatOptions {
        language: "en".into(),
        ..ChatOptions::default()
    }
}

fn message(role: &str, content: &str, status: &str) -> ChatMessage {
    ChatMessage {
        id: uuid::Uuid::new_v4().to_string(),
        role: role.into(),
        content: content.into(),
        status: status.into(),
        error: None,
        tools: vec![],
        sources: vec![],
        tokens: None,
        created_at: store::now(),
    }
}

fn source(id: &str, title: &str) -> Source {
    Source {
        id: id.into(),
        title: title.into(),
        url: Some(format!("https://vndb.org/{title}")),
        game_id: None,
    }
}

fn last_user_content(history: &[Value]) -> &str {
    history
        .iter()
        .rev()
        .find(|m| m["role"] == "user")
        .and_then(|m| m["content"].as_str())
        .expect("a user turn")
}

#[test]
fn a_replayed_citation_never_keeps_the_id_it_was_quoted_under() {
    let mut first = message("assistant", "CLANNAD is a [S1] nakige.", "complete");
    first.sources = vec![source("S1", "v11")];
    let turns = vec![
        message("user", "What is CLANNAD?", "complete"),
        first,
        message("user", "And its sequel?", "complete"),
    ];
    // The answer being written already retrieved one newer source.
    let mut sources = vec![source("S1", "v999")];
    let history = build_history(&options(), &turns, &mut sources);

    // Old [S1] is reassigned to S2 so it cannot be confused with the new S1.
    let assistant = history
        .iter()
        .find(|m| m["role"] == "assistant")
        .expect("assistant turn");
    assert_eq!(assistant["content"], "CLANNAD is a [S2] nakige.");
    assert_eq!(sources.len(), 2);
    assert_eq!(sources[1].id, "S2");
    assert_eq!(sources[1].title, "v11");
}

#[test]
fn cached_evidence_travels_with_the_turn_that_produced_it() {
    let mut answered = message("assistant", "You played 12 hours [S1].", "complete");
    answered.sources = vec![source("S1", "v11")];
    let mut trace = vn_tracker_lib::ai::conversation::ChatTrace {
        id: "call_1".into(),
        name: "library_stats".into(),
        arguments: "{}".into(),
        status: "complete".into(),
        result: Some(serde_json::json!({
            "source": "S1",
            "data": {"playtime_ms": 43_200_000_i64}
        })),
    };
    trace.status = "complete".into();
    answered.tools = vec![trace];
    let turns = vec![
        message("user", "How long have I played?", "complete"),
        answered,
    ];

    let mut sources = vec![];
    let history = build_history(&options(), &turns, &mut sources);
    let assistant = history
        .iter()
        .find(|m| m["role"] == "assistant")
        .expect("assistant turn");
    let content = assistant["content"].as_str().unwrap();
    assert!(content.contains("Previously retrieved evidence"));
    assert!(content.contains("43200000"));
    // The remapped evidence keeps its own citation id, so [S1] still resolves.
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].id, "S1");
}

#[test]
fn failed_and_stopped_answers_are_not_replayed_as_knowledge() {
    for status in ["error", "cancelled", "running"] {
        let turns = vec![
            message("user", "Explain the ending.", "complete"),
            message("assistant", "The killer is definitely the teacher.", status),
            message("user", "Say that again?", "complete"),
        ];
        let mut sources = vec![];
        let history = build_history(&options(), &turns, &mut sources);
        let leaked = history.iter().any(|m| {
            m["role"] == "assistant"
                && m["content"].as_str() == Some("The killer is definitely the teacher.")
        });
        assert!(!leaked, "{status} answers must not re-enter the context");
        assert_eq!(last_user_content(&history), "Say that again?");
    }
}

#[test]
fn context_starts_at_a_user_turn_even_when_the_budget_cuts_mid_exchange() {
    let mut turns = vec![message("user", "the first question", "complete")];
    for index in 0..40 {
        turns.push(message("assistant", &format!("answer {index}"), "complete"));
        turns.push(message("user", &format!("question {index}"), "complete"));
    }
    turns.push(message(
        "assistant",
        "x".repeat(40_000).as_str(),
        "complete",
    ));
    let mut sources = vec![];
    let history = build_history(&options(), &turns, &mut sources);
    assert_eq!(history[0]["role"], "system");
    // Replayed turns alternate user/assistant and start with a question: the
    // oldest kept turn is a continuation, never a dangling answer. The oversized
    // last turn is not the one being trimmed, so it survives.
    let bodies: Vec<&Value> = history.iter().skip(1).collect();
    assert!(bodies.len() >= 2);
    assert_eq!(bodies[0]["role"], "user");
    for pair in bodies.windows(2) {
        assert_ne!(pair[0]["role"], pair[1]["role"], "roles must alternate");
    }
    assert!(bodies.windows(2).any(|pair| pair[0]["role"] == "user"));
}

#[tokio::test]
async fn a_stopped_answer_is_persisted_as_incomplete_not_complete() {
    let db = Db::open_memory().await.expect("in-memory db");
    let thread = store::create(db.pool(), options()).await.expect("thread");
    let id = store::begin(db.pool(), &thread, "Tell me about the route", false)
        .await
        .expect("assistant row");

    // The user presses Stop after the first streamed sentence.
    let mut stopped = message("assistant", "She heads to the festival", "cancelled");
    stopped.id = id.clone();
    stopped.error = Some("cancelled".into());
    store::save(db.pool(), &stopped).await.expect("save");

    let saved = store::messages(db.pool(), &thread.id)
        .await
        .expect("history");
    let assistant = saved.iter().find(|m| m.role == "assistant").unwrap();
    assert_eq!(assistant.status, "cancelled");
    assert_eq!(assistant.error.as_deref(), Some("cancelled"));

    // A crash mid-stream must not leave a permanently running answer.
    let second = store::begin(db.pool(), &thread, "And after that?", false)
        .await
        .expect("second assistant row");
    store::recover(db.pool()).await.expect("recover");
    let saved = store::messages(db.pool(), &thread.id)
        .await
        .expect("history");
    let orphan = saved.iter().find(|m| m.id == second).unwrap();
    assert_eq!(orphan.status, "error");
    assert_eq!(orphan.error.as_deref(), Some("interrupted"));
}

#[tokio::test]
async fn a_failed_answer_can_be_retried_without_duplicating_the_question() {
    let db = Db::open_memory().await.expect("in-memory db");
    let thread = store::create(db.pool(), options()).await.expect("thread");
    let first = store::begin(db.pool(), &thread, "Which VN is unfinished?", false)
        .await
        .expect("assistant row");
    let mut failed = message("assistant", "", "error");
    failed.id = first;
    failed.error = Some("provider_offline".into());
    store::save(db.pool(), &failed).await.expect("save");

    let retried = store::begin(db.pool(), &thread, "", true)
        .await
        .expect("retry row");
    let saved = store::messages(db.pool(), &thread.id)
        .await
        .expect("history");
    assert_eq!(saved.len(), 2, "one question and one retried answer");
    assert_eq!(saved[0].role, "user");
    assert_eq!(saved[0].content, "Which VN is unfinished?");
    assert_eq!(saved[1].id, retried);

    // Retrying a completed answer would append a second reply to the same question.
    let mut complete = message("assistant", "Two games.", "complete");
    complete.id = retried;
    store::save(db.pool(), &complete).await.expect("save");
    assert!(store::begin(db.pool(), &thread, "", true).await.is_err());
}
