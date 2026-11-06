//! Synthetic fixtures only; no real windows, game data or paid providers.
use super::*;
use crate::{
    ai::{
        chat::Provider,
        conversation::{self, store as chat},
        profiles,
    },
    companion::{
        memory,
        tests::{configure, fixture, start},
    },
    state::AppState,
};
use serde_json::{Value, json};
use sqlx::Row;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::oneshot,
};

async fn server(answer: Value) -> (String, oneshot::Receiver<Value>, oneshot::Sender<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (sent, received) = oneshot::channel();
    let (release, released) = oneshot::channel();
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut bytes = Vec::new();
        let mut buf = [0; 4096];
        let body = loop {
            let n = socket.read(&mut buf).await.unwrap();
            if n == 0 {
                return;
            }
            bytes.extend_from_slice(&buf[..n]);
            if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&bytes[..end]);
                let len = headers
                    .lines()
                    .find_map(|l| {
                        l.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .and_then(|s| s.trim().parse::<usize>().ok())
                    })
                    .unwrap();
                if bytes.len() >= end + 4 + len {
                    break serde_json::from_slice(&bytes[end + 4..end + 4 + len]).unwrap();
                }
            }
        };
        let _ = sent.send(body);
        let _ = released.await;
        let body = answer.to_string();
        let reply = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        let _ = socket.write_all(reply.as_bytes()).await;
    });
    (url, received, release)
}
fn response(text: &str) -> Value {
    json!({"choices":[{"finish_reason":"stop","message":{"content":text}}]})
}
fn target() -> windows::Target {
    #[cfg(windows)]
    {
        windows::Target {
            handle: 0,
            pid: 0,
            created: 0,
            title: "Synthetic window".into(),
            synthetic: true,
        }
    }
    #[cfg(not(windows))]
    {
        windows::Target {
            title: "Synthetic window".into(),
            synthetic: true,
        }
    }
}
async fn bind(state: &AppState, session: &crate::companion::store::Session) -> String {
    let identity = Identity::of(&profiles::resolve(state.db().pool()).await);
    let frame = Arc::new(encode(32, 24, vec![255; 32 * 24 * 4], Crop::default()).unwrap());
    let preview = uuid::Uuid::new_v4().to_string();
    let mut scenes = Scenes::default();
    scenes.observe(&frame.signature, 0, 0);
    let mut inner = state.companion().lock().unwrap();
    inner.vision.origin = std::time::Instant::now()
        .checked_sub(std::time::Duration::from_secs(3))
        .unwrap();
    inner.vision.accepted = Some(identity.clone());
    inner.vision.budget_session = session.id.clone();
    inner.vision.binding = Some(Binding {
        session: session.id.clone(),
        playthrough: session.playthrough_id.clone(),
        identity,
        target: target(),
        crop: Crop::default(),
        feed: Feed {
            stop: Arc::new(AtomicBool::new(false)),
            state: Arc::new(Mutex::new(FeedState {
                latest: Some(frame.clone()),
                error: None,
            })),
        },
        preview: Some((preview.clone(), frame, std::time::Instant::now())),
        armed: true,
        quiet: false,
        focused: true,
        scenes,
    });
    preview
}

#[test]
fn crop_downscale_and_no_upscale_are_real_encoded_dimensions() {
    let frame = encode(32, 24, vec![255; 32 * 24 * 4], Crop::default()).unwrap();
    assert_eq!((frame.width, frame.height), (32, 24));
    let cropped = encode(
        32,
        24,
        vec![255; 32 * 24 * 4],
        Crop {
            x: 500,
            y: 0,
            width: 500,
            height: 1000,
        },
    )
    .unwrap();
    assert_eq!((cropped.width, cropped.height), (16, 24));
    let large = encode(1600, 900, vec![255; 1600 * 900 * 4], Crop::default()).unwrap();
    assert_eq!((large.width, large.height), (1280, 720));
    assert!(!format!("{frame:?}").contains("base64"));
}

#[tokio::test]
async fn manual_image_and_provenance_are_scoped_and_pixels_never_persist() {
    let (state, options) = fixture().await;
    let (url, received, release) = server(response("A bright scene.")).await;
    configure(&state, &url).await;
    let session = start(&state, options).await;
    let preview = bind(&state, &session).await;
    let work = state
        .companion()
        .prepare_visual(
            &state,
            &session.id,
            Some(&preview),
            "What is visible?",
            false,
        )
        .await
        .unwrap()
        .unwrap();
    let state = Arc::new(state);
    let worker = state.clone();
    let task = tokio::spawn(async move {
        worker.companion().run_visual(&worker, work).await;
    });
    let body = received.await.unwrap();
    assert!(body.get("tools").is_none());
    assert_eq!(body["max_tokens"], 1200);
    assert!(
        body["messages"].as_array().unwrap().last().unwrap()["content"][1]["image_url"]["url"]
            .as_str()
            .unwrap()
            .starts_with("data:image/jpeg;base64,")
    );
    release.send(()).unwrap();
    task.await.unwrap();
    let messages = chat::messages(state.db().pool(), &session.thread_id)
        .await
        .unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].content, "What is visible?");
    assert_eq!(messages[1].status, "complete");
    let stored = serde_json::to_string(&messages).unwrap();
    assert!(!stored.contains("base64"));
    assert!(!stored.contains("Synthetic window"));
    let (origin,w,h):(String,i64,i64)=sqlx::query_as("SELECT v.origin,v.width,v.height FROM companion_visual_turns v JOIN chat_messages m ON m.id=v.message_id JOIN companion_sessions s ON s.id=v.session_id WHERE s.id=?").bind(&session.id).fetch_one(state.db().pool()).await.unwrap();
    assert_eq!((origin.as_str(), w, h), ("manual_image", 32, 24));
    assert!(
        sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(state.db().pool())
            .await
            .unwrap()
            .is_empty()
    );
    sqlx::query("DELETE FROM chat_messages WHERE id=?")
        .bind(&messages[1].id)
        .execute(state.db().pool())
        .await
        .unwrap();
    let (remaining,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM companion_visual_turns")
        .fetch_one(state.db().pool())
        .await
        .unwrap();
    assert_eq!(remaining, 0);
}

#[tokio::test]
async fn automatic_origin_silence_invalid_response_and_limits() {
    for (answer, visible) in [
        (r#"{"react":true,"text":"A quiet moment."}"#, true),
        (r#"{"react":false,"text":""}"#, false),
        ("invalid envelope", false),
    ] {
        let (state, options) = fixture().await;
        let (url, received, release) = server(response(answer)).await;
        configure(&state, &url).await;
        let session = start(&state, options).await;
        arm(&state, &session).await;
        let work = state
            .companion()
            .prepare_visual(&state, &session.id, None, "", true)
            .await
            .unwrap()
            .unwrap();
        assert!(
            state
                .companion()
                .prepare_visual(&state, &session.id, None, "", true)
                .await
                .unwrap()
                .is_none()
        );
        let state = Arc::new(state);
        let worker = state.clone();
        let task = tokio::spawn(async move {
            worker.companion().run_visual(&worker, work).await;
        });
        let body = received.await.unwrap();
        assert_eq!(body["max_tokens"], 400);
        assert!(body.get("tools").is_none());
        assert!(state.companion().snapshot().unwrap().live.is_none());
        release.send(()).unwrap();
        task.await.unwrap();
        let messages = crate::companion::store::messages(state.db().pool(), &session.id)
            .await
            .unwrap();
        assert_eq!(!messages.is_empty(), visible);
        assert!(messages.iter().all(|m| m.role == "assistant"));
        assert_eq!(state.companion().snapshot().unwrap().vision.total, 1);
        if visible {
            let replay = serde_json::to_string(&conversation::bounded_history(
                "",
                &messages,
                &mut Vec::new(),
                48000,
            ))
            .unwrap();
            assert!(replay.contains("NOT a reader statement"));
            assert!(replay.contains("visual inference"));
            assert!(!replay.contains("base64"));
        }
    }
}

#[tokio::test]
async fn newer_scene_forget_stop_and_provider_change_suppress_late_output() {
    for action in ["scene", "forget", "stop", "provider"] {
        let (state, options) = fixture().await;
        let (url, received, release) =
            server(response(r#"{"react":true,"text":"Must not arrive."}"#)).await;
        configure(&state, &url).await;
        let session = start(&state, options).await;
        arm(&state, &session).await;
        let work = state
            .companion()
            .prepare_visual(&state, &session.id, None, "", true)
            .await
            .unwrap()
            .unwrap();
        let state = Arc::new(state);
        let worker = state.clone();
        let task = tokio::spawn(async move {
            worker.companion().run_visual(&worker, work).await;
        });
        received.await.unwrap();
        match action {
            "scene" => {
                let mut inner = state.companion().lock().unwrap();
                let b = inner.vision.binding.as_mut().unwrap();
                b.feed.state.lock().unwrap().latest = Some(Arc::new(
                    encode(32, 24, vec![0; 32 * 24 * 4], Crop::default()).unwrap(),
                ));
            }
            "forget" => {
                let rev = memory::settings(state.db().pool()).await.unwrap().revision;
                state
                    .companion()
                    .memory_forget(&state, memory::Forget::Everything, rev)
                    .await
                    .unwrap();
            }
            "stop" => {
                state
                    .companion()
                    .vision_control(&state, &session.id, "stop", 0, None)
                    .await
                    .unwrap();
            }
            _ => {
                state.companion().invalidate_vision().unwrap();
                configure(&state, "http://127.0.0.1:9").await;
            }
        }
        let _ = release.send(());
        task.await.unwrap();
        assert!(
            state.companion().snapshot().unwrap().live.is_none(),
            "{action}"
        );
        assert!(
            crate::companion::store::messages(state.db().pool(), &session.id)
                .await
                .unwrap()
                .is_empty(),
            "{action}"
        );
        assert_eq!(state.companion().snapshot().unwrap().vision.total, 1);
        if action != "scene" {
            assert!(state.companion().lock().unwrap().vision.binding.is_none());
        }
    }
}

#[tokio::test]
async fn reader_question_preempts_automatic_work_without_losing_watch_consent() {
    let (state, options) = fixture().await;
    let (url, received, release) = server(response(r#"{"react":true,"text":"Late"}"#)).await;
    configure(&state, &url).await;
    let session = start(&state, options).await;
    arm(&state, &session).await;
    let work = state
        .companion()
        .prepare_visual(&state, &session.id, None, "", true)
        .await
        .unwrap()
        .unwrap();
    let state = Arc::new(state);
    let worker = state.clone();
    let task = tokio::spawn(async move {
        worker.companion().run_visual(&worker, work).await;
    });
    received.await.unwrap();
    let manual = state
        .companion()
        .prepare(
            &state,
            &session.id,
            &uuid::Uuid::new_v4().to_string(),
            "My question first",
        )
        .await
        .unwrap();
    assert!(manual.is_some());
    assert!(state.companion().snapshot().unwrap().vision.watching);
    let _ = release.send(());
    task.await.unwrap();
}

#[tokio::test]
async fn unknown_image_support_and_expired_preview_do_not_dispatch() {
    let (state, options) = fixture().await;
    let session = start(&state, options).await;
    let preview = bind(&state, &session).await;
    state.companion().lock().unwrap().vision.accepted = None;
    assert!(
        matches!(state.companion().prepare_visual(&state,&session.id,Some(&preview),"Look",false).await,Err(e) if e=="vision_test_required")
    );
    bind(&state, &session).await;
    assert!(
        matches!(state.companion().prepare_visual(&state,&session.id,Some("stale"),"Look",false).await,Err(e) if e=="vision_stale_preview")
    );
    assert_eq!(state.companion().snapshot().unwrap().vision.total, 0);
}

#[tokio::test]
async fn anthropic_text_images_and_tools_preserve_their_protocol_shapes() {
    let frame = encode(8, 8, vec![255; 8 * 8 * 4], Crop::default()).unwrap();
    let history = vec![
        json!({"role":"system","content":"system"}),
        json!({"role":"user","content":crate::ai::vision::content(&[crate::ai::vision::Part::Text("Look"),crate::ai::vision::Part::Image(&frame.image)])}),
        json!({"role":"assistant","content":"Checking","tool_calls":[{"id":"a","function":{"name":"get_game","arguments":"{}"}}]}),
        json!({"role":"tool","tool_call_id":"a","content":"{}"}),
    ];
    let (system, turns) = conversation::anthropic::convert(&history).unwrap();
    assert_eq!(system, "system");
    assert_eq!(turns[0]["content"][0]["text"], "Look");
    assert_eq!(turns[0]["content"][1]["source"]["media_type"], "image/jpeg");
    assert_eq!(turns[1]["content"][1]["type"], "tool_use");
    assert_eq!(turns[2]["content"][0]["type"], "tool_result");
    let (url, received, release) = server(
        json!({"stop_reason":"end_turn","content":[{"type":"text","text":"Image accepted"}]}),
    )
    .await;
    let config = ChatConfig {
        provider: Provider::Anthropic,
        base_url: url,
        model: "fixture".into(),
    };
    let task =
        tokio::spawn(
            async move { crate::ai::vision::complete(&config, None, &history[..2], 40).await },
        );
    let body = received.await.unwrap();
    assert!(body.get("tools").is_none());
    assert_eq!(body["messages"][0]["content"][1]["type"], "image");
    release.send(()).unwrap();
    assert_eq!(task.await.unwrap().unwrap(), "Image accepted");
    for url in [
        "https://remote/image.jpg",
        "data:image/jpeg;base64,bm90LWltYWdl",
        "data:image/png;base64,invalid",
    ] {
        assert!(
            crate::ai::vision::anthropic_content(
                &json!([{"type":"image_url","image_url":{"url":url}}])
            )
            .is_err()
        );
    }
}

async fn arm(state: &AppState, session: &crate::companion::store::Session) {
    bind(state, session).await;
    state
        .companion()
        .lock()
        .unwrap()
        .vision
        .binding
        .as_mut()
        .unwrap()
        .preview = None;
}

#[tokio::test]
async fn preview_dismissal_releases_watch_without_accepting_a_stale_token() {
    let (state, options) = fixture().await;
    configure(&state, "http://127.0.0.1:9").await;
    let session = start(&state, options).await;
    let token = bind(&state, &session).await;
    let generation = state.companion().snapshot().unwrap().vision.generation;
    for stale in [Some("another-preview"), None] {
        state
            .companion()
            .vision_control(&state, &session.id, "dismiss", generation, stale)
            .await
            .unwrap();
        assert!(
            state
                .companion()
                .prepare_visual(&state, &session.id, None, "", true)
                .await
                .unwrap()
                .is_none()
        );
    }
    state
        .companion()
        .vision_control(&state, &session.id, "dismiss", generation, Some(&token))
        .await
        .unwrap();
    assert!(
        state
            .companion()
            .prepare_visual(&state, &session.id, None, "", true)
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn provenance_migration_preserves_old_history_and_excludes_image_storage() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    for migration in [
        include_str!("../../db/migrations/0014_chat.sql"),
        include_str!("../../db/migrations/0030_companion_sessions.sql"),
        include_str!("../../db/migrations/0031_companion_profiles.sql"),
        include_str!("../../db/migrations/0032_companion_memory.sql"),
    ] {
        sqlx::raw_sql(migration).execute(&pool).await.unwrap();
    }
    sqlx::query("INSERT INTO chat_threads VALUES('old','Old conversation','{}',1,1)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO chat_messages(id,thread_id,position,role,content,status,created_at) VALUES('old-message','old',0,'user','Keep this history','complete',1)").execute(&pool).await.unwrap();
    sqlx::raw_sql(include_str!(
        "../../db/migrations/0033_companion_visual_turns.sql"
    ))
    .execute(&pool)
    .await
    .unwrap();
    let (text,): (String,) =
        sqlx::query_as("SELECT content FROM chat_messages WHERE id='old-message'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(text, "Keep this history");
    assert!(
        sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&pool)
            .await
            .unwrap()
            .is_empty()
    );
    let columns = sqlx::query("PRAGMA table_info(companion_visual_turns)")
        .fetch_all(&pool)
        .await
        .unwrap();
    for c in columns {
        let name: String = c.get("name");
        assert!(
            ![
                "pixels",
                "image",
                "base64",
                "window_title",
                "handle",
                "hash",
                "path"
            ]
            .contains(&name.as_str())
        );
    }
}
