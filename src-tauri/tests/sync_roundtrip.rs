//! Two devices syncing through a shared folder.
//!
//! These tests use two real in-memory databases and a real folder on disk, because
//! the failure modes worth guarding against only appear at that level: a payload
//! that drops a column, a delete that fails to travel, a merge that never
//! converges. Unit tests on the merge rules cannot catch any of those.

use std::path::PathBuf;

use sqlx::SqlitePool;
use vn_core::clock::Timestamp;
use vn_core::ids::{DeviceId, EntityId};
use vn_core::model::{Chapter, Ending, Game, GameStatus, PlaySession, SessionSource};

use vn_tracker_lib::db::Db;
use vn_tracker_lib::db::queries;
use vn_tracker_lib::sync;

fn temp_folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("vntracker-sync-it-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create folder");
    dir
}

struct Device {
    db: Db,
    device_id: DeviceId,
}

async fn device() -> Device {
    let db = Db::open_memory().await.expect("db");
    let device_id = queries::ensure_device_id(db.pool()).await.expect("device");
    Device { db, device_id }
}

async fn add_game(device: &Device, title: &str, at_ms: i64) -> EntityId {
    let game = Game::new(
        &device.device_id,
        Timestamp::from_millis(at_ms),
        title,
        GameStatus::Playing,
    )
    .expect("game");
    queries::upsert_game(device.db.pool(), &game)
        .await
        .expect("insert");
    game.meta.id
}

/// A game's title as the UI would see it: tombstoned rows count as absent.
///
/// Without the `deleted = 0` filter this returns a row for a soft-deleted game,
/// which made the deletion test pass for the wrong reason until it was fixed.
async fn title_of(pool: &SqlitePool, id: EntityId) -> Option<String> {
    use sqlx::Row as _;
    sqlx::query("SELECT title FROM games WHERE id = ? AND deleted = 0")
        .bind(id.to_string())
        .fetch_optional(pool)
        .await
        .expect("query")
        .map(|row| row.get::<String, _>("title"))
}

#[tokio::test]
async fn a_game_created_on_one_device_appears_on_the_other() {
    let folder = temp_folder("basic");
    let a = device().await;
    let b = device().await;

    let game = add_game(&a, "Shared Title", 1_000).await;
    assert_eq!(
        title_of(a.db.pool(), game).await.as_deref(),
        Some("Shared Title")
    );
    assert!(title_of(b.db.pool(), game).await.is_none());

    // A writes its payload, B pulls it.
    sync::sync_folder(a.db.pool(), &a.device_id, &folder)
        .await
        .expect("a syncs");

    let outcome = sync::sync_folder(b.db.pool(), &b.device_id, &folder)
        .await
        .expect("b syncs");
    assert_eq!(outcome.peers, 1);
    assert_eq!(
        title_of(b.db.pool(), game).await.as_deref(),
        Some("Shared Title")
    );

    let _ = std::fs::remove_dir_all(&folder);
}

#[tokio::test]
async fn portable_transfer_merges_without_a_shared_folder_and_preserves_deletes() {
    let a = device().await;
    let b = device().await;
    let game = add_game(&a, "Portable VN", 1_000).await;
    let payload = sync::collect_local(a.db.pool(), &a.device_id)
        .await
        .expect("export");
    let encoded = serde_json::to_string(&payload).expect("encode");
    let imported = serde_json::from_str(&encoded).expect("decode");
    let result = sync::merge_transfer(b.db.pool(), &b.device_id, &imported)
        .await
        .expect("import");
    assert_eq!(result.peers, 1);
    assert_eq!(
        title_of(b.db.pool(), game).await.as_deref(),
        Some("Portable VN")
    );
    let repeated = sync::merge_transfer(b.db.pool(), &b.device_id, &imported)
        .await
        .expect("repeat");
    assert_eq!(repeated.pulled, 0, "reimport must be idempotent");

    let mut deleted = queries::get_game(a.db.pool(), game)
        .await
        .expect("read")
        .expect("game");
    deleted.meta.deleted = true;
    deleted.meta = deleted.meta.touch(Timestamp::from_millis(2_000));
    queries::upsert_game(a.db.pool(), &deleted)
        .await
        .expect("delete");
    let payload = sync::collect_local(a.db.pool(), &a.device_id)
        .await
        .expect("export deletion");
    sync::merge_transfer(b.db.pool(), &b.device_id, &payload)
        .await
        .expect("import deletion");
    assert!(title_of(b.db.pool(), game).await.is_none());
    sync::merge_transfer(b.db.pool(), &b.device_id, &imported)
        .await
        .expect("stale import");
    assert!(
        title_of(b.db.pool(), game).await.is_none(),
        "stale transfer must not resurrect deleted game"
    );
}

#[tokio::test]
async fn portable_transfer_refuses_newer_schema_before_writing_any_rows() {
    let a = device().await;
    let b = device().await;
    let game = add_game(&a, "Too new", 1_000).await;
    let mut payload = sync::collect_local(a.db.pool(), &a.device_id)
        .await
        .expect("export");
    payload.schema_version = vn_core::migration::SCHEMA_VERSION + 1;
    assert!(matches!(
        sync::merge_transfer(b.db.pool(), &b.device_id, &payload).await,
        Err(sync::SyncError::PayloadTooNew { .. })
    ));
    assert!(title_of(b.db.pool(), game).await.is_none());
}

#[tokio::test]
async fn invalid_session_rolls_back_earlier_transfer_tombstones_and_chapters() {
    let a = device().await;
    let b = device().await;
    let game = add_game(&a, "Original title", 1_000).await;
    let chapter = Chapter::new(
        &a.device_id,
        Timestamp::from_millis(1_000),
        game,
        "Original chapter",
        1,
    )
    .expect("chapter");
    queries::upsert_chapter(a.db.pool(), &chapter)
        .await
        .expect("insert chapter");
    let session = PlaySession::new(
        &a.device_id,
        Timestamp::from_millis(1_000),
        game,
        Timestamp::from_millis(100),
        Timestamp::from_millis(200),
        100,
        SessionSource::AndroidUsage,
    )
    .expect("session");
    queries::insert_session(a.db.pool(), &session)
        .await
        .expect("insert session");
    let mut payload = sync::collect_local(a.db.pool(), &a.device_id)
        .await
        .expect("export");
    sync::merge_transfer(b.db.pool(), &b.device_id, &payload)
        .await
        .expect("initial transfer");
    let original = payload.games[0].clone();

    payload.games[0].meta = payload.games[0]
        .meta
        .tombstone(Timestamp::from_millis(5_000));
    payload.chapters[0].name = "Changed before failure".to_owned();
    payload.chapters[0].meta = payload.chapters[0]
        .meta
        .touch(Timestamp::from_millis(5_000));
    let added = Game::new(
        &a.device_id,
        Timestamp::from_millis(5_000),
        "New before failure",
        GameStatus::Wishlist,
    )
    .expect("game");
    payload.games.push(added.clone());
    let mut invalid = session.clone();
    invalid.meta.id = EntityId::new();
    invalid.counted_ms = -1;
    payload.sessions.push(invalid);

    assert!(
        sync::merge_transfer(b.db.pool(), &b.device_id, &payload)
            .await
            .is_err()
    );
    assert_eq!(
        queries::list_games_including_deleted(b.db.pool())
            .await
            .expect("games"),
        vec![original]
    );
    assert_eq!(
        title_of(b.db.pool(), game).await.as_deref(),
        Some("Original title")
    );
    assert_eq!(
        queries::list_chapters(b.db.pool(), game)
            .await
            .expect("chapters"),
        vec![chapter]
    );
    assert_eq!(
        queries::list_sessions(b.db.pool(), game)
            .await
            .expect("sessions"),
        vec![session]
    );
    assert!(title_of(b.db.pool(), added.meta.id).await.is_none());
}

#[tokio::test]
async fn invalid_ending_rolls_back_transfer_sessions_and_negative_idle_time_is_rejected() {
    let a = device().await;
    let b = device().await;
    let game = add_game(&a, "Must remain absent", 1_000).await;
    let mut payload = sync::collect_local(a.db.pool(), &a.device_id)
        .await
        .expect("export");
    let session = PlaySession::new(
        &a.device_id,
        Timestamp::from_millis(1_000),
        game,
        Timestamp::from_millis(100),
        Timestamp::from_millis(200),
        100,
        SessionSource::AndroidUsage,
    )
    .expect("session");
    payload.sessions.push(session);
    payload.endings.push(
        Ending::new(
            &a.device_id,
            Timestamp::from_millis(1_000),
            EntityId::new(),
            "Missing parent",
        )
        .expect("ending"),
    );

    assert!(
        sync::merge_transfer(b.db.pool(), &b.device_id, &payload)
            .await
            .is_err()
    );
    assert!(title_of(b.db.pool(), game).await.is_none());
    assert_eq!(
        queries::list_sessions(b.db.pool(), game)
            .await
            .expect("sessions")
            .len(),
        0
    );

    payload.endings.clear();
    payload.sessions[0].afk_ms = -1;
    assert!(
        sync::merge_transfer(b.db.pool(), &b.device_id, &payload)
            .await
            .is_err()
    );
    assert!(title_of(b.db.pool(), game).await.is_none());
}

#[tokio::test]
async fn a_newer_edit_wins_and_the_older_value_does_not_come_back() {
    let folder = temp_folder("newer");
    let a = device().await;
    let b = device().await;

    let game = add_game(&a, "Original", 1_000).await;
    sync::sync_folder(a.db.pool(), &a.device_id, &folder)
        .await
        .expect("a");
    sync::sync_folder(b.db.pool(), &b.device_id, &folder)
        .await
        .expect("b");

    // B edits later than A's original write.
    let mut edited = queries::get_game(b.db.pool(), game)
        .await
        .expect("get")
        .expect("exists");
    edited.title = "B's newer title".to_owned();
    edited.meta = edited.meta.touch(Timestamp::from_millis(5_000));
    queries::upsert_game(b.db.pool(), &edited)
        .await
        .expect("edit b");

    sync::sync_folder(b.db.pool(), &b.device_id, &folder)
        .await
        .expect("b push");
    let outcome = sync::sync_folder(a.db.pool(), &a.device_id, &folder)
        .await
        .expect("a pull");

    assert_eq!(
        title_of(a.db.pool(), game).await.as_deref(),
        Some("B's newer title"),
    );
    assert!(
        outcome
            .conflicts
            .iter()
            .any(|c| c.record_id == game.to_string()),
        "the overwrite should be reported, got {:?}",
        outcome.conflicts,
    );

    let _ = std::fs::remove_dir_all(&folder);
}

#[tokio::test]
async fn a_deletion_travels_instead_of_the_game_coming_back() {
    // The resurrection bug: deleting on one device and then syncing must not
    // restore the row from the other device's older copy.
    let folder = temp_folder("delete");
    let a = device().await;
    let b = device().await;

    let game = add_game(&a, "Doomed", 1_000).await;
    sync::sync_folder(a.db.pool(), &a.device_id, &folder)
        .await
        .expect("a");
    sync::sync_folder(b.db.pool(), &b.device_id, &folder)
        .await
        .expect("b");
    assert!(title_of(b.db.pool(), game).await.is_some());

    // A deletes it, later.
    let mut gone = queries::get_game(a.db.pool(), game)
        .await
        .expect("get")
        .expect("exists");
    gone.meta = gone.meta.tombstone(Timestamp::from_millis(9_000));
    queries::upsert_game(a.db.pool(), &gone)
        .await
        .expect("delete a");

    sync::sync_folder(a.db.pool(), &a.device_id, &folder)
        .await
        .expect("a push");
    sync::sync_folder(b.db.pool(), &b.device_id, &folder)
        .await
        .expect("b pull");
    assert!(
        title_of(b.db.pool(), game).await.is_none(),
        "the tombstone must reach B",
    );

    // And a second round must not resurrect it either.
    sync::sync_folder(b.db.pool(), &b.device_id, &folder)
        .await
        .expect("b push");
    sync::sync_folder(a.db.pool(), &a.device_id, &folder)
        .await
        .expect("a pull");
    assert!(title_of(a.db.pool(), game).await.is_none());

    let _ = std::fs::remove_dir_all(&folder);
}

#[tokio::test]
async fn a_chapter_rename_survives_a_round_trip() {
    // Chapter renames and routes live in columns the model did not originally
    // carry. Without them in the payload, renaming on one device would be undone
    // by the next sync.
    let folder = temp_folder("rename");
    let a = device().await;
    let b = device().await;

    let game = add_game(&a, "With Chapters", 1_000).await;
    let mut chapter = Chapter::new(
        &a.device_id,
        Timestamp::from_millis(1_000),
        game,
        "70_終章_0726",
        1,
    )
    .expect("chapter");
    chapter.display_name = Some("Final Chapter".to_owned());
    chapter.route_name = Some("Final".to_owned());
    chapter.route_key = Some("route:70".to_owned());
    chapter.position = Some("7月26日".to_owned());
    chapter.renamed = true;
    queries::upsert_chapter(a.db.pool(), &chapter)
        .await
        .expect("insert");

    sync::sync_folder(a.db.pool(), &a.device_id, &folder)
        .await
        .expect("a");
    sync::sync_folder(b.db.pool(), &b.device_id, &folder)
        .await
        .expect("b");

    let on_b = queries::list_chapters(b.db.pool(), game)
        .await
        .expect("chapters");
    assert_eq!(on_b.len(), 1);
    assert_eq!(
        on_b[0].display_name.as_deref(),
        Some("Final Chapter"),
        "the rename must survive, got {on_b:?}",
    );
    assert_eq!(on_b[0].route_name.as_deref(), Some("Final"));
    assert_eq!(on_b[0].position.as_deref(), Some("7月26日"));
    assert!(on_b[0].renamed);

    let _ = std::fs::remove_dir_all(&folder);
}

#[tokio::test]
async fn syncing_twice_changes_nothing_the_second_time() {
    // A merge must be idempotent, or every sync would produce conflicts forever.
    let folder = temp_folder("idempotent");
    let a = device().await;
    let b = device().await;

    let game = add_game(&a, "Stable", 1_000).await;
    sync::sync_folder(a.db.pool(), &a.device_id, &folder)
        .await
        .expect("a");
    sync::sync_folder(b.db.pool(), &b.device_id, &folder)
        .await
        .expect("b");

    let second = sync::sync_folder(b.db.pool(), &b.device_id, &folder)
        .await
        .expect("b again");
    assert_eq!(second.pulled, 0, "nothing new to pull");
    assert_eq!(
        second.conflicts.len(),
        0,
        "conflicts: {:?}",
        second.conflicts
    );

    let twice = sync::sync_folder(a.db.pool(), &a.device_id, &folder)
        .await
        .expect("a again");
    assert_eq!(twice.pulled, 0);
    assert_eq!(twice.conflicts.len(), 0, "conflicts: {:?}", twice.conflicts);

    assert!(title_of(a.db.pool(), game).await.is_some());
    let _ = std::fs::remove_dir_all(&folder);
}

#[tokio::test]
async fn a_corrupt_peer_file_is_reported_and_does_not_block_the_rest() {
    let folder = temp_folder("corrupt");
    let a = device().await;
    let b = device().await;

    add_game(&a, "Good Title", 1_000).await;
    sync::sync_folder(a.db.pool(), &a.device_id, &folder)
        .await
        .expect("a");

    // Something unreadable lands in the folder, as a half-written file or a
    // deleted-from-sync placeholder could.
    std::fs::write(folder.join("vntracker-broken.json"), "{not json").expect("write corrupt");

    let outcome = sync::sync_folder(b.db.pool(), &b.device_id, &folder)
        .await
        .expect("b syncs anyway");

    assert_eq!(outcome.peers, 1, "the good peer was still merged");
    assert_eq!(outcome.skipped.len(), 1);
    assert!(outcome.skipped[0].file.contains("broken"));

    let _ = std::fs::remove_dir_all(&folder);
}

#[tokio::test]
async fn a_payload_from_a_newer_schema_is_skipped_rather_than_misread() {
    // Merging rows written by a future version could drop fields that version
    // added, silently corrupting the library.
    let folder = temp_folder("future");
    let a = device().await;
    add_game(&a, "From The Future", 1_000).await;
    sync::sync_folder(a.db.pool(), &a.device_id, &folder)
        .await
        .expect("a");
    // Rewrite the file claiming a newer schema.
    let file = folder.join(sync::payload_file_name(a.device_id.as_str()));
    let text = std::fs::read_to_string(&file).expect("read");
    let mut value: serde_json::Value = serde_json::from_str(&text).expect("parse");
    value["schema_version"] = serde_json::json!(99);
    // Give it another device id so it is not treated as our own file.
    value["device_id"] = serde_json::json!("01a00000-0000-7000-8000-0000000000ff");
    std::fs::write(folder.join("vntracker-future.json"), value.to_string()).expect("write");

    let b = device().await;
    let outcome = sync::sync_folder(b.db.pool(), &b.device_id, &folder)
        .await
        .expect("b syncs");
    assert!(
        outcome
            .skipped
            .iter()
            .any(|s| s.reason.contains("newer version")),
        "expected a schema refusal, got {:?}",
        outcome.skipped,
    );

    let _ = std::fs::remove_dir_all(&folder);
}
