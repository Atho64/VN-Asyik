use super::{profiles, tests::*, *};
use std::sync::Arc;
use vn_core::companion::profile::Preset;

#[tokio::test]
async fn profile_versions_conflicts_and_deletion_preserve_session_snapshots() {
    let (state, mut options) = fixture().await;
    let pool = state.db().pool();
    let first = profiles::save(pool, None, None, Preset::Kawaii.snapshot().settings)
        .await
        .unwrap();
    options.profile = Some(first.clone());
    let session = start(&state, options).await;
    let mut edited = first.settings.clone();
    edited.name = "Edited identity".into();
    let second = profiles::save(pool, Some(&first.id), Some(1), edited.clone())
        .await
        .unwrap();
    assert_eq!(second.version, 2);
    assert_eq!(
        profiles::save(pool, Some(&first.id), Some(1), edited)
            .await
            .unwrap_err(),
        "profile_conflict"
    );
    assert_eq!(
        profiles::delete(pool, &first.id, 1).await.unwrap_err(),
        "profile_conflict"
    );
    assert_eq!(
        state
            .companion()
            .snapshot()
            .unwrap()
            .session
            .unwrap()
            .options
            .profile,
        Some(first.clone())
    );
    profiles::delete(pool, &first.id, 2).await.unwrap();
    state
        .companion()
        .control(&state, &session.id, Action::End)
        .await
        .unwrap();
    assert_eq!(
        store::list(pool).await.unwrap()[0].options.profile,
        Some(first)
    );
    let catalogue = profiles::list(pool).await.unwrap();
    assert_eq!(catalogue.presets.len(), 12);
    assert!(catalogue.profiles.is_empty());
}

#[tokio::test]
async fn profiles_are_bounded_and_builtins_cannot_be_overwritten() {
    let (state, _) = fixture().await;
    let pool = state.db().pool();
    let preset = Preset::Companion.snapshot();
    assert_eq!(
        profiles::save(pool, Some(&preset.id), Some(1), preset.settings.clone())
            .await
            .unwrap_err(),
        "profile_conflict"
    );
    let mut invalid = preset.settings.clone();
    invalid.warmth = 3;
    assert_eq!(
        profiles::save(pool, None, None, invalid).await.unwrap_err(),
        "invalid_profile"
    );
    for _ in 0..profiles::MAX_PROFILES {
        profiles::save(pool, None, None, preset.settings.clone())
            .await
            .unwrap();
    }
    assert_eq!(
        profiles::save(pool, None, None, preset.settings)
            .await
            .unwrap_err(),
        "profile_limit"
    );
}

#[tokio::test]
async fn profile_migration_preserves_legacy_session_options_and_foreign_keys() {
    let (state, options) = fixture().await;
    let session = start(&state, options.clone()).await;
    let pool = state.db().pool();
    // Replay the additive SQL over an existing 20A session in this memory-only fixture.
    sqlx::query("DROP TABLE companion_profiles")
        .execute(pool)
        .await
        .unwrap();
    let before: (String,) =
        sqlx::query_as("SELECT options_json FROM companion_sessions WHERE id=?")
            .bind(&session.id)
            .fetch_one(pool)
            .await
            .unwrap();
    assert!(!before.0.contains("profile"));
    assert!(
        sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(pool)
            .await
            .unwrap()
            .is_empty()
    );
    sqlx::raw_sql(include_str!("../db/migrations/0031_companion_profiles.sql"))
        .execute(pool)
        .await
        .unwrap();
    let after: (String,) = sqlx::query_as("SELECT options_json FROM companion_sessions WHERE id=?")
        .bind(&session.id)
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(before, after);
    assert_eq!(
        store::get(pool, &session.id).await.unwrap().options,
        options
    );
    assert!(
        sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(pool)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(profiles::list(pool).await.unwrap().profiles.is_empty());
}

#[tokio::test]
async fn applying_a_profile_revokes_before_the_gate_and_cannot_undo_a_newer_pause() {
    for pause in [false, true] {
        let (state, options) = fixture().await;
        let state = Arc::new(state);
        let session = start(&state, options.clone()).await;
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
        let apply = {
            let state = state.clone();
            let id = session.id.clone();
            tokio::spawn(async move {
                state
                    .companion()
                    .apply_profile(&state, &id, Preset::WryRival.snapshot())
                    .await
            })
        };
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while sink.is_current() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("apply must invalidate delivery before awaiting the gate");
        assert!(
            sink.send(ChatEvent::Delta {
                text: "Stale personality".into()
            })
            .is_err()
        );
        let newer = if pause {
            let state = state.clone();
            let id = session.id.clone();
            Some(tokio::spawn(async move {
                state.companion().control(&state, &id, Action::Pause).await
            }))
        } else {
            None
        };
        if pause {
            tokio::time::timeout(std::time::Duration::from_secs(2), async {
                while state.companion().snapshot().unwrap().lifecycle
                    != vn_core::companion::Lifecycle::Paused
                {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
        }
        drop(gate);
        if let Some(newer) = newer {
            assert_eq!(apply.await.unwrap().unwrap_err(), "cancelled");
            newer.await.unwrap().unwrap();
            assert_eq!(
                store::get(state.db().pool(), &session.id)
                    .await
                    .unwrap()
                    .options,
                options
            );
        } else {
            let updated = apply.await.unwrap().unwrap().session.unwrap();
            assert_eq!(updated.options.profile, Some(Preset::WryRival.snapshot()));
            assert_eq!(updated.playthrough_id, session.playthrough_id);
            assert_eq!(updated.options.knowledge, options.knowledge);
            assert_eq!(updated.options.source, options.source);
        }
    }
}

#[tokio::test]
async fn applying_during_stream_keeps_each_prompt_pinned_and_excludes_cancelled_replay() {
    let (state, mut options) = fixture().await;
    options.profile = Some(Preset::QuietListener.snapshot());
    let state = Arc::new(state);
    let session = start(&state, options).await;
    let (seen, release, server) = provider(&state).await;
    let work = state
        .companion()
        .prepare(
            &state,
            &session.id,
            &uuid::Uuid::new_v4().to_string(),
            "Describe this scene",
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
    let request = seen.await.unwrap();
    assert!(request.contains("quiet listener"));
    assert!(!request.contains("friendly rival"));
    wait_for_partial(&state).await;
    state
        .companion()
        .apply_profile(&state, &session.id, Preset::WryRival.snapshot())
        .await
        .unwrap();
    let _ = release.send(());
    runner.await.unwrap();
    server.await.unwrap();
    let saved = store::messages(state.db().pool(), &session.id)
        .await
        .unwrap();
    let replay = crate::ai::conversation::build_history(
        &session.options.chat_options(),
        &saved,
        &mut Vec::new(),
    );
    assert!(
        !serde_json::to_string(&replay)
            .unwrap()
            .contains("A quiet scene.")
    );
    let (seen, release, server) = provider(&state).await;
    let work = state
        .companion()
        .prepare(
            &state,
            &session.id,
            &uuid::Uuid::new_v4().to_string(),
            "Try again",
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
    let request = seen.await.unwrap();
    assert!(request.contains("friendly rival"));
    assert!(!request.contains("quiet listener"));
    assert!(!request.contains("A quiet scene."));
    assert!(request.contains("Only passages supplied or observed"));
    assert!(!request.contains("web_search"));
    release.send(()).unwrap();
    runner.await.unwrap();
    server.await.unwrap();
}
