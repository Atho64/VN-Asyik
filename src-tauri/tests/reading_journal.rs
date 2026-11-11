//! Real `SQLite` coverage for journal persistence, provenance and portable data.

use vn_core::model::{Chapter, Game, GameStatus, PlaySession, Route, SessionSource};
use vn_core::reading::ReadingEntry;
use vn_core::{DeviceId, EntityId, Timestamp};
use vn_tracker_lib::commands::{backup, data};
use vn_tracker_lib::db::{Db, queries, reading};
use vn_tracker_lib::sync;

fn at(ms: i64) -> Timestamp {
    Timestamp::from_millis(ms)
}

async fn fixture() -> (Db, DeviceId, Game, PlaySession) {
    let db = Db::open_memory().await.unwrap();
    let device = DeviceId::parse("journal-device").unwrap();
    let game = Game::new(&device, at(1), "A long story", GameStatus::Playing).unwrap();
    queries::upsert_game(db.pool(), &game).await.unwrap();
    let session = PlaySession::new(
        &device,
        at(2000),
        game.meta.id,
        at(1000),
        at(2000),
        1000,
        SessionSource::Manual,
    )
    .unwrap();
    queries::insert_session(db.pool(), &session).await.unwrap();
    (db, device, game, session)
}

fn input(game: EntityId, session: Option<EntityId>) -> reading::SaveReadingInput {
    reading::SaveReadingInput {
        id: None,
        expected_updated_at: None,
        game_id: game,
        session_id: session,
        note: "  Before the choice\nAt the station  ".to_owned(),
        save_slot: " 12 ".to_owned(),
        route_name: "Shizuku".to_owned(),
        chapter_name: "August 7".to_owned(),
        photo_image_id: String::new(),
    }
}

#[tokio::test]
async fn session_previews_select_live_notes_and_follow_edits_and_deletes() {
    let (db, device, game, session) = fixture().await;
    let mut entries = Vec::new();
    for (rank, text) in [
        (1, "Older note".to_owned()),
        (2, "続".repeat(300)),
        (3, String::new()),
    ] {
        let mut draft = input(game.meta.id, Some(session.meta.id));
        draft.note = text;
        let entry = reading::save(db.pool(), &device, draft).await.unwrap();
        sqlx::query("UPDATE reading_entries SET created_at = ? WHERE id = ?")
            .bind(rank)
            .bind(entry.meta.id.to_string())
            .execute(db.pool())
            .await
            .unwrap();
        entries.push(entry);
    }
    // An unlinked game note and a malformed cross-game reference must not leak.
    reading::save(db.pool(), &device, input(game.meta.id, None))
        .await
        .unwrap();
    let other = Game::new(&device, at(1), "Other", GameStatus::Playing).unwrap();
    queries::upsert_game(db.pool(), &other).await.unwrap();
    let foreign = reading::save(db.pool(), &device, input(other.meta.id, None))
        .await
        .unwrap();
    sqlx::query("UPDATE reading_entries SET session_id = ? WHERE id = ?")
        .bind(session.meta.id.to_string())
        .bind(foreign.meta.id.to_string())
        .execute(db.pool())
        .await
        .unwrap();

    let page = queries::game_sessions_page(db.pool(), game.meta.id, 50)
        .await
        .unwrap();
    assert_eq!(page.note_previews.len(), 1);
    let key = session.meta.id.to_string();
    assert_eq!(page.note_previews[&key], format!("{}…", "続".repeat(240)));
    let mut draft = input(game.meta.id, Some(session.meta.id));
    draft.id = Some(entries[1].meta.id);
    draft.expected_updated_at = Some(entries[1].meta.updated_at.as_millis());
    draft.note = "Edited note".to_owned();
    let edited = reading::save(db.pool(), &device, draft).await.unwrap();
    let page = queries::game_sessions_page(db.pool(), game.meta.id, 50)
        .await
        .unwrap();
    assert_eq!(page.note_previews[&key], "Edited note");
    reading::delete(
        db.pool(),
        &device,
        game.meta.id,
        edited.meta.id,
        edited.meta.updated_at.as_millis(),
    )
    .await
    .unwrap();
    let page = queries::game_sessions_page(db.pool(), game.meta.id, 50)
        .await
        .unwrap();
    assert_eq!(page.note_previews[&key], "Older note");
    reading::delete(
        db.pool(),
        &device,
        game.meta.id,
        entries[0].meta.id,
        entries[0].meta.updated_at.as_millis(),
    )
    .await
    .unwrap();
    let page = queries::game_sessions_page(db.pool(), game.meta.id, 50)
        .await
        .unwrap();
    assert!(page.note_previews.is_empty());
    assert_eq!(page.counted_ms, 1000);
    assert_eq!(page.sessions, vec![session]);
}

#[tokio::test]
async fn session_previews_are_bounded_to_the_visible_page_and_live_parents() {
    use vn_tracker_lib::commands::library::SessionCursor;
    let (db, device, game, session) = fixture().await;
    let mut newer = session.clone();
    newer.meta.id = EntityId::new();
    newer.started_at = at(3000);
    newer.ended_at = at(4000);
    queries::insert_session(db.pool(), &newer).await.unwrap();
    for target in [&session, &newer] {
        reading::save(
            db.pool(),
            &device,
            input(game.meta.id, Some(target.meta.id)),
        )
        .await
        .unwrap();
    }
    let page = queries::game_sessions_page(db.pool(), game.meta.id, 1)
        .await
        .unwrap();
    assert_eq!(page.note_previews.len(), 1);
    assert!(page.note_previews.contains_key(&newer.meta.id.to_string()));
    let page = queries::game_sessions_before(
        db.pool(),
        game.meta.id,
        1,
        Some(SessionCursor {
            started_at: newer.started_at.as_millis(),
            id: newer.meta.id,
        }),
    )
    .await
    .unwrap();
    assert_eq!(page.note_previews.len(), 1);
    assert!(
        page.note_previews
            .contains_key(&session.meta.id.to_string())
    );
    sqlx::query("UPDATE play_sessions SET deleted = 1 WHERE id = ?")
        .bind(newer.meta.id.to_string())
        .execute(db.pool())
        .await
        .unwrap();
    let page = queries::game_sessions_page(db.pool(), game.meta.id, 50)
        .await
        .unwrap();
    assert!(!page.note_previews.contains_key(&newer.meta.id.to_string()));
    sqlx::query("UPDATE games SET deleted = 1 WHERE id = ?")
        .bind(game.meta.id.to_string())
        .execute(db.pool())
        .await
        .unwrap();
    assert!(
        queries::game_sessions_page(db.pool(), game.meta.id, 50)
            .await
            .unwrap()
            .note_previews
            .is_empty()
    );
}

#[tokio::test]
async fn selected_session_has_its_own_attribution_and_all_its_checkpoints() {
    let (db, device, game, old) = fixture().await;
    let route = Route::new(&device, at(1), game.meta.id, "Old route").unwrap();
    queries::upsert_route(db.pool(), &route).await.unwrap();
    let mut chapter = Chapter::new(&device, at(1), game.meta.id, "Old chapter", 0).unwrap();
    chapter.route_id = Some(route.meta.id);
    queries::upsert_chapter(db.pool(), &chapter).await.unwrap();
    sqlx::query("UPDATE play_sessions SET chapter_id = ?, afk_ms = 200 WHERE id = ?")
        .bind(chapter.meta.id.to_string())
        .bind(old.meta.id.to_string())
        .execute(db.pool())
        .await
        .unwrap();
    let mut newer = old.clone();
    newer.meta.id = EntityId::new();
    newer.started_at = at(3000);
    newer.ended_at = at(4000);
    queries::insert_session(db.pool(), &newer).await.unwrap();
    let before = queries::list_sessions(db.pool(), game.meta.id)
        .await
        .unwrap();
    for _ in 0..12 {
        reading::save(db.pool(), &device, input(game.meta.id, Some(old.meta.id)))
            .await
            .unwrap();
    }
    reading::save(db.pool(), &device, input(game.meta.id, None))
        .await
        .unwrap();
    reading::save(db.pool(), &device, input(game.meta.id, Some(newer.meta.id)))
        .await
        .unwrap();
    let removed = reading::save(db.pool(), &device, input(game.meta.id, Some(old.meta.id)))
        .await
        .unwrap();
    reading::delete(
        db.pool(),
        &device,
        game.meta.id,
        removed.meta.id,
        removed.meta.updated_at.as_millis(),
    )
    .await
    .unwrap();

    let first = reading::session_context(db.pool(), game.meta.id, old.meta.id, 0)
        .await
        .unwrap();
    assert_eq!(first.session.id, old.meta.id.to_string());
    assert_eq!(first.session.started_at, 1000);
    assert_eq!(first.session.ended_at, 2000);
    assert_eq!(first.session.counted_ms, 1000);
    assert_eq!(first.session.afk_ms, 200);
    assert_eq!(first.session.route_name.as_deref(), Some("Old route"));
    assert_eq!(first.session.chapter_name.as_deref(), Some("Old chapter"));
    assert_eq!(first.entries.len(), 10);
    assert!(first.has_more);
    let second = reading::session_context(db.pool(), game.meta.id, old.meta.id, 10)
        .await
        .unwrap();
    assert_eq!(second.entries.len(), 2);
    assert!(!second.has_more);
    for entry in first.entries.iter().chain(&second.entries) {
        assert_eq!(entry.session_id, Some(old.meta.id));
        assert_eq!(entry.played_at, Some(old.ended_at));
        assert!(!entry.meta.deleted);
    }
    assert!(second.entries.iter().all(|entry| {
        !first
            .entries
            .iter()
            .any(|other| other.meta.id == entry.meta.id)
    }));
    assert_eq!(
        queries::list_sessions(db.pool(), game.meta.id)
            .await
            .unwrap(),
        before
    );
    let latest = reading::context(db.pool(), game.meta.id, 0)
        .await
        .unwrap()
        .latest_session
        .unwrap();
    assert_eq!(latest.id, newer.meta.id.to_string());
    assert_eq!(latest.chapter_name, None);
}

#[tokio::test]
async fn session_details_hide_missing_foreign_and_deleted_parents() {
    let (db, device, game, session) = fixture().await;
    let other = Game::new(&device, at(1), "Other", GameStatus::Playing).unwrap();
    queries::upsert_game(db.pool(), &other).await.unwrap();
    for (game_id, session_id) in [
        (other.meta.id, session.meta.id),
        (game.meta.id, EntityId::new()),
    ] {
        assert_eq!(
            reading::session_context(db.pool(), game_id, session_id, 0)
                .await
                .unwrap_err()
                .code,
            "not_found"
        );
    }
    // Even valid foreign keys must not leak another game's attribution.
    let foreign = Chapter::new(&device, at(1), other.meta.id, "Foreign chapter", 0).unwrap();
    queries::upsert_chapter(db.pool(), &foreign).await.unwrap();
    sqlx::query("UPDATE play_sessions SET chapter_id = ? WHERE id = ?")
        .bind(foreign.meta.id.to_string())
        .bind(session.meta.id.to_string())
        .execute(db.pool())
        .await
        .unwrap();
    assert_eq!(
        reading::session_context(db.pool(), game.meta.id, session.meta.id, 0)
            .await
            .unwrap()
            .session
            .chapter_name,
        None
    );
    sqlx::query("UPDATE play_sessions SET deleted = 1 WHERE id = ?")
        .bind(session.meta.id.to_string())
        .execute(db.pool())
        .await
        .unwrap();
    assert_eq!(
        reading::session_context(db.pool(), game.meta.id, session.meta.id, 0)
            .await
            .unwrap_err()
            .code,
        "not_found"
    );
    assert!(
        reading::save(
            db.pool(),
            &device,
            input(game.meta.id, Some(session.meta.id))
        )
        .await
        .is_err()
    );
    sqlx::query("UPDATE play_sessions SET deleted = 0 WHERE id = ?")
        .bind(session.meta.id.to_string())
        .execute(db.pool())
        .await
        .unwrap();
    sqlx::query("UPDATE games SET deleted = 1 WHERE id = ?")
        .bind(game.meta.id.to_string())
        .execute(db.pool())
        .await
        .unwrap();
    assert_eq!(
        reading::session_context(db.pool(), game.meta.id, session.meta.id, 0)
            .await
            .unwrap_err()
            .code,
        "not_found"
    );
    let page = queries::game_sessions_page(db.pool(), game.meta.id, 50)
        .await
        .unwrap();
    assert_eq!(page.sessions, Vec::<PlaySession>::new());
    assert_eq!(page.total_sessions, 0);
    assert_eq!(page.counted_ms, 0);
}

#[tokio::test]
async fn an_away_only_session_can_have_notes_without_becoming_the_latest_reading() {
    let (db, device, game, session) = fixture().await;
    sqlx::query("UPDATE play_sessions SET counted_ms = 0, afk_ms = 1000 WHERE id = ?")
        .bind(session.meta.id.to_string())
        .execute(db.pool())
        .await
        .unwrap();
    let note = reading::save(
        db.pool(),
        &device,
        input(game.meta.id, Some(session.meta.id)),
    )
    .await
    .unwrap();
    let selected = reading::session_context(db.pool(), game.meta.id, session.meta.id, 0)
        .await
        .unwrap();
    assert_eq!(selected.session.counted_ms, 0);
    assert_eq!(selected.session.afk_ms, 1000);
    assert_eq!(selected.entries, vec![note]);
    assert!(
        reading::context(db.pool(), game.meta.id, 0)
            .await
            .unwrap()
            .latest_session
            .is_none()
    );
}

#[tokio::test]
async fn session_pages_survive_new_rows_and_equal_timestamps() {
    use vn_tracker_lib::commands::library::SessionCursor;
    let (db, _, game, session) = fixture().await;
    for _ in 0..54 {
        let mut another = session.clone();
        another.meta.id = EntityId::new();
        queries::insert_session(db.pool(), &another).await.unwrap();
    }
    let first = queries::game_sessions_page(db.pool(), game.meta.id, 50)
        .await
        .unwrap();
    assert_eq!(first.sessions.len(), 50);
    assert_eq!(first.total_sessions, 55);
    let last = first.sessions.last().unwrap();
    let mut newest = session.clone();
    newest.meta.id = EntityId::new();
    newest.started_at = at(5000);
    newest.ended_at = at(6000);
    queries::insert_session(db.pool(), &newest).await.unwrap();
    let second = queries::game_sessions_before(
        db.pool(),
        game.meta.id,
        50,
        Some(SessionCursor {
            started_at: last.started_at.as_millis(),
            id: last.meta.id,
        }),
    )
    .await
    .unwrap();
    assert_eq!(second.sessions.len(), 5);
    assert_eq!(second.total_sessions, 56);
    assert_eq!(second.counted_ms, 56_000);
    assert!(second.sessions.iter().all(|row| {
        !first
            .sessions
            .iter()
            .any(|other| other.meta.id == row.meta.id)
    }));
}

#[tokio::test]
async fn newest_counted_session_never_borrows_an_older_chapter() {
    let (db, device, game, mut session) = fixture().await;
    let chapter = Chapter::new(&device, at(1), game.meta.id, "Old chapter", 0).unwrap();
    queries::upsert_chapter(db.pool(), &chapter).await.unwrap();
    sqlx::query("UPDATE play_sessions SET chapter_id = ? WHERE id = ?")
        .bind(chapter.meta.id.to_string())
        .bind(session.meta.id.to_string())
        .execute(db.pool())
        .await
        .unwrap();
    let first = reading::context(db.pool(), game.meta.id, 0).await.unwrap();
    assert_eq!(
        first.latest_session.unwrap().chapter_name.as_deref(),
        Some("Old chapter")
    );
    session.meta.id = EntityId::new();
    session.started_at = at(3000);
    session.ended_at = at(4000);
    queries::insert_session(db.pool(), &session).await.unwrap();
    let latest_id = session.meta.id.to_string();
    session.meta.id = EntityId::new();
    session.started_at = at(5000);
    session.ended_at = at(6000);
    session.counted_ms = 0;
    queries::insert_session(db.pool(), &session).await.unwrap();
    let context = reading::context(db.pool(), game.meta.id, 0).await.unwrap();
    let latest = context.latest_session.unwrap();
    assert_eq!(latest.id, latest_id);
    assert_eq!(latest.chapter_name, None);
    assert_eq!(latest.route_name, None);
}

#[tokio::test]
async fn editing_and_deleting_checkpoints_preserves_sessions_and_detects_stale_edits() {
    let (db, device, game, session) = fixture().await;
    let before = queries::list_sessions(db.pool(), game.meta.id)
        .await
        .unwrap();
    let entry = reading::save(
        db.pool(),
        &device,
        input(game.meta.id, Some(session.meta.id)),
    )
    .await
    .unwrap();
    assert_eq!(entry.note, "Before the choice\nAt the station");
    assert_eq!(entry.save_slot, "12");
    assert_eq!(entry.played_at, Some(session.ended_at));
    let edit = || reading::SaveReadingInput {
        id: Some(entry.meta.id),
        expected_updated_at: Some(entry.meta.updated_at.as_millis()),
        note: "Actually slot 13".to_owned(),
        ..input(game.meta.id, Some(session.meta.id))
    };
    let updated = reading::save(db.pool(), &device, edit()).await.unwrap();
    assert_eq!(updated.created_at, entry.created_at);
    assert_eq!(updated.played_at, entry.played_at);
    assert_eq!(
        reading::save(db.pool(), &device, edit())
            .await
            .unwrap_err()
            .code,
        "reading_conflict"
    );
    assert_eq!(
        reading::delete(
            db.pool(),
            &device,
            game.meta.id,
            entry.meta.id,
            entry.meta.updated_at.as_millis()
        )
        .await
        .unwrap_err()
        .code,
        "reading_conflict"
    );
    reading::delete(
        db.pool(),
        &device,
        game.meta.id,
        entry.meta.id,
        updated.meta.updated_at.as_millis(),
    )
    .await
    .unwrap();
    assert_eq!(
        reading::context(db.pool(), game.meta.id, 0)
            .await
            .unwrap()
            .entries,
        Vec::<ReadingEntry>::new()
    );
    assert!(
        reading::list_all(db.pool(), game.meta.id).await.unwrap()[0]
            .meta
            .deleted
    );
    assert_eq!(
        queries::list_sessions(db.pool(), game.meta.id)
            .await
            .unwrap(),
        before
    );
}

#[tokio::test]
async fn manual_checkpoints_work_without_sessions_and_cannot_reference_another_game() {
    let (db, device, game, session) = fixture().await;
    let other = Game::new(&device, at(1), "Another story", GameStatus::Playing).unwrap();
    queries::upsert_game(db.pool(), &other).await.unwrap();
    let manual = reading::save(db.pool(), &device, input(other.meta.id, None))
        .await
        .unwrap();
    assert_eq!(manual.played_at, None);
    assert!(
        reading::context(db.pool(), other.meta.id, 0)
            .await
            .unwrap()
            .latest_session
            .is_none()
    );
    assert!(
        reading::save(
            db.pool(),
            &device,
            input(other.meta.id, Some(session.meta.id))
        )
        .await
        .is_err()
    );
    queries::soft_delete_game(db.pool(), other.meta.id, at(3), &device)
        .await
        .unwrap();
    assert_eq!(
        reading::context(db.pool(), other.meta.id, 0)
            .await
            .unwrap_err()
            .code,
        "not_found"
    );
    assert!(
        reading::save(db.pool(), &device, input(other.meta.id, None))
            .await
            .is_err()
    );
    assert_eq!(
        reading::context(db.pool(), game.meta.id, 0)
            .await
            .unwrap()
            .entries,
        Vec::<ReadingEntry>::new()
    );
}

#[tokio::test]
async fn entries_and_tombstones_roundtrip_and_legacy_payloads_leave_them_alone() {
    let (db, device, game, session) = fixture().await;
    let entry = reading::save(
        db.pool(),
        &device,
        input(game.meta.id, Some(session.meta.id)),
    )
    .await
    .unwrap();
    let peer = Db::open_memory().await.unwrap();
    let payload = sync::collect_local(db.pool(), &device).await.unwrap();
    sync::merge_library_snapshot(peer.pool(), &payload)
        .await
        .unwrap();
    assert_eq!(
        reading::list_all(peer.pool(), game.meta.id).await.unwrap(),
        vec![entry.clone()]
    );
    assert_eq!(
        sync::merge_library_snapshot(peer.pool(), &payload)
            .await
            .unwrap()
            .pulled,
        0
    );

    let mut legacy = serde_json::to_value(&payload).unwrap();
    legacy.as_object_mut().unwrap().remove("reading_entries");
    legacy["schema_version"] = serde_json::json!(28);
    let old: sync::SyncPayload = serde_json::from_value(legacy).unwrap();
    sync::merge_library_snapshot(peer.pool(), &old)
        .await
        .unwrap();
    assert_eq!(
        reading::list_all(peer.pool(), game.meta.id).await.unwrap(),
        vec![entry.clone()]
    );

    reading::delete(
        db.pool(),
        &device,
        game.meta.id,
        entry.meta.id,
        entry.meta.updated_at.as_millis(),
    )
    .await
    .unwrap();
    let deleted = sync::collect_local(db.pool(), &device).await.unwrap();
    sync::merge_library_snapshot(peer.pool(), &deleted)
        .await
        .unwrap();
    sync::merge_library_snapshot(peer.pool(), &payload)
        .await
        .unwrap();
    assert_eq!(
        reading::context(peer.pool(), game.meta.id, 0)
            .await
            .unwrap()
            .entries,
        Vec::<ReadingEntry>::new()
    );
}

#[tokio::test]
async fn library_export_preview_merge_and_restore_keep_the_journal() {
    let (db, device, game, session) = fixture().await;
    let entry = reading::save(
        db.pool(),
        &device,
        input(game.meta.id, Some(session.meta.id)),
    )
    .await
    .unwrap();
    let file = data::collect_library_file(db.pool(), &device)
        .await
        .unwrap();
    let text = serde_json::to_string(&file).unwrap();
    for mode in [data::DataImportMode::Merge, data::DataImportMode::Restore] {
        let target = Db::open_memory().await.unwrap();
        let store = data::ImportDraftStore::default();
        let id = store.begin_open().unwrap();
        let preview = data::prepare_import(&store, &id, target.pool(), &device, &text)
            .await
            .unwrap();
        assert_eq!(preview.reading_entries, 1);
        data::apply_import_with_mode(&store, target.pool(), &device, &id, mode)
            .await
            .unwrap();
        assert_eq!(
            reading::list_all(target.pool(), game.meta.id)
                .await
                .unwrap(),
            vec![entry.clone()]
        );
    }
    // Old backups still decode without the new list.
    let backup = backup::BackupFile {
        header: vn_core::migration::BackupHeader {
            format: 1,
            schema_version: 28,
            created_at_ms: 1,
            app_version: "test".to_owned(),
        },
        games: file.payload.games,
        routes: file.payload.routes,
        chapters: file.payload.chapters,
        sessions: file.payload.sessions,
        endings: file.payload.endings,
        reading_entries: vec![],
        prefs: file.prefs,
    };
    let mut old = serde_json::to_value(&backup).unwrap();
    old.as_object_mut().unwrap().remove("reading_entries");
    assert_eq!(
        serde_json::from_value::<backup::BackupFile>(old)
            .unwrap()
            .reading_entries,
        Vec::<ReadingEntry>::new()
    );
}

#[tokio::test]
async fn invalid_journal_reference_rolls_back_the_entire_peer() {
    let (db, device, game, session) = fixture().await;
    let mut entry = reading::save(
        db.pool(),
        &device,
        input(game.meta.id, Some(session.meta.id)),
    )
    .await
    .unwrap();
    let mut payload = sync::collect_local(db.pool(), &device).await.unwrap();
    entry.session_id = Some(EntityId::new());
    payload.reading_entries = vec![entry];
    let target = Db::open_memory().await.unwrap();
    assert!(
        sync::merge_library_snapshot(target.pool(), &payload)
            .await
            .is_err()
    );
    assert_eq!(
        queries::list_games(target.pool()).await.unwrap(),
        Vec::<Game>::new()
    );
}

#[tokio::test]
async fn upgrading_from_schema_28_keeps_existing_data_and_foreign_keys() {
    let (db, _, game, session) = fixture().await;
    // Recreate a pre-feature database inside this isolated in-memory test.
    sqlx::query("DROP TABLE reading_entries")
        .execute(db.pool())
        .await
        .unwrap();
    sqlx::query("DELETE FROM _sqlx_migrations WHERE version = 29")
        .execute(db.pool())
        .await
        .unwrap();
    for after in [false, true] {
        if after {
            db.migrate().await.unwrap();
        }
        let violations = sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(db.pool())
            .await
            .unwrap();
        assert!(violations.is_empty());
        assert!(
            queries::get_game(db.pool(), game.meta.id)
                .await
                .unwrap()
                .is_some()
        );
        assert_eq!(
            queries::list_sessions(db.pool(), game.meta.id)
                .await
                .unwrap(),
            vec![session.clone()]
        );
    }
    assert_eq!(
        reading::context(db.pool(), game.meta.id, 0)
            .await
            .unwrap()
            .entries,
        Vec::<ReadingEntry>::new()
    );
}

#[tokio::test]
async fn pagination_has_no_gaps_and_hidden_parents_do_not_leak_into_reads() {
    let (db, device, game, _) = fixture().await;
    for _ in 0..12 {
        reading::save(db.pool(), &device, input(game.meta.id, None))
            .await
            .unwrap();
    }
    let first = reading::context(db.pool(), game.meta.id, 0).await.unwrap();
    let second = reading::context(db.pool(), game.meta.id, 10).await.unwrap();
    assert_eq!(first.entries.len(), 10);
    assert!(first.has_more);
    assert_eq!(second.entries.len(), 2);
    assert!(!second.has_more);
    assert!(
        second
            .entries
            .iter()
            .all(|entry| !first.entries.iter().any(|old| old.meta.id == entry.meta.id))
    );
    // A historical orphan must not become a visible journal row even with FKs off.
    let mut connection = db.pool().acquire().await.unwrap();
    sqlx::query("PRAGMA foreign_keys = OFF")
        .execute(&mut *connection)
        .await
        .unwrap();
    sqlx::query("DELETE FROM games WHERE id = ?")
        .bind(game.meta.id.to_string())
        .execute(&mut *connection)
        .await
        .unwrap();
    assert_eq!(
        reading::list_all(&mut *connection, game.meta.id)
            .await
            .unwrap(),
        Vec::<ReadingEntry>::new()
    );
}
