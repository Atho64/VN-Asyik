//! One library file and compatible legacy imports, with merge as the default.

use serde_json::{Value, json};
use vn_core::clock::Timestamp;
use vn_core::ids::DeviceId;
use vn_core::migration::{BACKUP_FORMAT, BackupHeader, SCHEMA_VERSION};
use vn_core::model::{Chapter, Ending, Game, GameStatus, PlaySession, SessionSource};
use vn_tracker_lib::commands::backup::BackupFile;
use vn_tracker_lib::commands::data::{
    DataImportKind, DataImportMode, DataImportResult, ImportDraftStore, LIBRARY_FORMAT,
    LibraryFile, apply_import, apply_import_with_mode, collect_library_file, prepare_import,
};
use vn_tracker_lib::db::{Db, models::PrefsSnapshot, queries};
use vn_tracker_lib::sync::{self, SyncPayload};

fn fixture() -> BackupFile {
    let device = DeviceId::parse("source-device").unwrap();
    let at = Timestamp::from_millis(1_000);
    let game = Game::new(&device, at, "Legacy collection", GameStatus::Playing).unwrap();
    let chapter = Chapter::new(&device, at, game.meta.id, "Chapter 1", 0).unwrap();
    let mut session = PlaySession::new(
        &device,
        at,
        game.meta.id,
        at,
        Timestamp::from_millis(61_000),
        60_000,
        SessionSource::Manual,
    )
    .unwrap();
    session.chapter_id = Some(chapter.meta.id);
    let ending = Ending::new(&device, at, game.meta.id, "True End").unwrap();
    BackupFile {
        header: BackupHeader {
            format: BACKUP_FORMAT,
            schema_version: SCHEMA_VERSION,
            created_at_ms: at.as_millis(),
            app_version: "0.1.0".to_owned(),
        },
        games: vec![game],
        routes: vec![],
        chapters: vec![chapter],
        sessions: vec![session],
        endings: vec![ending],
        prefs: PrefsSnapshot {
            theme: "light".to_owned(),
            language: "id".to_owned(),
            nsfw: "hide".to_owned(),
            reduce_effects: true,
        },
    }
}

fn transfer(file: &BackupFile, device: &str) -> SyncPayload {
    SyncPayload {
        schema_version: file.header.schema_version,
        device_id: device.to_owned(),
        written_at_ms: file.header.created_at_ms,
        generation: 0,
        games: file.games.clone(),
        routes: file.routes.clone(),
        chapters: file.chapters.clone(),
        sessions: file.sessions.clone(),
        endings: file.endings.clone(),
    }
}

async fn target() -> (Db, DeviceId) {
    let db = Db::open_memory().await.unwrap();
    let device = queries::ensure_device_id(db.pool()).await.unwrap();
    (db, device)
}

async fn preview(
    store: &ImportDraftStore,
    db: &Db,
    device: &DeviceId,
    file: &impl serde::Serialize,
) -> vn_tracker_lib::commands::data::DataImportPreview {
    let id = store.begin_open().unwrap();
    prepare_import(
        store,
        &id,
        db.pool(),
        device,
        &serde_json::to_string(file).unwrap(),
    )
    .await
    .unwrap()
}

async fn game_count(db: &Db) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM games")
        .fetch_one(db.pool())
        .await
        .unwrap()
}

fn library(file: &BackupFile, device: &str) -> LibraryFile {
    LibraryFile {
        format: LIBRARY_FORMAT.to_owned(),
        version: 1,
        payload: transfer(file, device),
        prefs: file.prefs.clone(),
    }
}

#[tokio::test]
async fn unified_and_legacy_backup_default_merge_preserves_newer_data_and_preferences() {
    for legacy in [false, true] {
        let (db, device) = target().await;
        let file = fixture();
        let value = if legacy {
            serde_json::to_value(&file).unwrap()
        } else {
            serde_json::to_value(library(&file, device.as_str())).unwrap()
        };
        let store = ImportDraftStore::default();
        let first = preview(&store, &db, &device, &value).await;
        assert!(first.can_restore);
        apply_import(&store, db.pool(), &device, &first.id)
            .await
            .unwrap();
        assert_eq!(queries::load_prefs(db.pool()).await.unwrap().theme, "dark");
        let mut newer = file.games[0].clone();
        newer.title = "Keep newer local edit".to_owned();
        newer.meta = newer.meta.touch(Timestamp::from_millis(9_000));
        queries::upsert_game(db.pool(), &newer).await.unwrap();
        let second = preview(&store, &db, &device, &value).await;
        apply_import(&store, db.pool(), &device, &second.id)
            .await
            .unwrap();
        assert_eq!(
            queries::get_game(db.pool(), newer.meta.id)
                .await
                .unwrap()
                .unwrap()
                .title,
            newer.title
        );
        assert_eq!(
            queries::list_sessions(db.pool(), newer.meta.id)
                .await
                .unwrap()
                .len(),
            1
        );
        newer.meta = newer.meta.tombstone(Timestamp::from_millis(10_000));
        queries::upsert_game(db.pool(), &newer).await.unwrap();
        let third = preview(&store, &db, &device, &value).await;
        apply_import(&store, db.pool(), &device, &third.id)
            .await
            .unwrap();
        assert!(
            queries::get_game(db.pool(), newer.meta.id)
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(queries::load_prefs(db.pool()).await.unwrap().theme, "dark");
    }
}

#[tokio::test]
async fn unified_export_roundtrips_tombstones_preferences_and_explicit_restore() {
    let (db, device) = target().await;
    let file = fixture();
    let store = ImportDraftStore::default();
    let first = preview(&store, &db, &device, &file).await;
    apply_import(&store, db.pool(), &device, &first.id)
        .await
        .unwrap();
    let mut deleted = file.games[0].clone();
    deleted.meta = deleted.meta.tombstone(Timestamp::from_millis(10_000));
    queries::upsert_game(db.pool(), &deleted).await.unwrap();
    queries::set_pref(db.pool(), "theme", "light")
        .await
        .unwrap();
    let exported = collect_library_file(db.pool(), &device).await.unwrap();
    let value = serde_json::to_value(&exported).unwrap();
    assert_eq!(value["format"], LIBRARY_FORMAT);
    assert_eq!(value["version"], 1);
    assert!(exported.payload.games[0].meta.deleted);
    assert_eq!(exported.prefs.theme, "light");
    let mut keys: Vec<_> = value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(keys, ["format", "payload", "prefs", "version"]);
    let (other, other_device) = target().await;
    let draft = preview(&store, &other, &other_device, &exported).await;
    assert_eq!(draft.kind, DataImportKind::Library);
    assert!(draft.can_restore);
    apply_import_with_mode(
        &store,
        other.pool(),
        &other_device,
        &draft.id,
        DataImportMode::Restore,
    )
    .await
    .unwrap();
    assert_eq!(
        queries::load_prefs(other.pool()).await.unwrap().theme,
        "light"
    );
    assert!(
        sync::collect_local(other.pool(), &other_device)
            .await
            .unwrap()
            .games[0]
            .meta
            .deleted
    );
    assert_eq!(
        queries::list_sessions(other.pool(), deleted.meta.id)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn legacy_transfer_cannot_restore_and_never_writes_on_rejection() {
    let (db, device) = target().await;
    let store = ImportDraftStore::default();
    let draft = preview(&store, &db, &device, &transfer(&fixture(), "source-device")).await;
    assert!(!draft.can_restore);
    assert_eq!(
        apply_import_with_mode(
            &store,
            db.pool(),
            &device,
            &draft.id,
            DataImportMode::Restore
        )
        .await
        .unwrap_err()
        .code,
        "restore_unavailable"
    );
    assert_eq!(game_count(&db).await, 0);
    assert_eq!(queries::load_prefs(db.pool()).await.unwrap().theme, "dark");
}

#[tokio::test]
async fn legacy_exports_preview_without_writes_and_cancel_invalidates_tokens() {
    let (db, device) = target().await;
    let file = fixture();
    let store = ImportDraftStore::default();
    let first = preview(&store, &db, &device, &file).await;
    assert_eq!(first.kind, DataImportKind::Backup);
    assert_eq!(
        (first.games, first.chapters, first.sessions, first.endings),
        (1, 1, 1, 1)
    );
    assert_eq!(first.created_at_ms, 1_000);
    assert!(!first.same_device);
    assert_eq!(serde_json::to_value(&first).unwrap()["createdAtMs"], 1_000);
    assert_eq!(game_count(&db).await, 0);
    assert_eq!(queries::load_prefs(db.pool()).await.unwrap().theme, "dark");
    store.cancel(&first.id).unwrap();
    assert_eq!(
        apply_import(&store, db.pool(), &device, &first.id)
            .await
            .unwrap_err()
            .code,
        "stale_import"
    );
    assert_eq!(game_count(&db).await, 0);

    let second = preview(&store, &db, &device, &transfer(&file, "source-device")).await;
    assert_eq!(second.kind, DataImportKind::Transfer);
    let third = preview(&store, &db, &device, &file).await;
    store.cancel(&second.id).unwrap(); // Old dialog cleanup cannot cancel the newer file.
    assert_eq!(
        apply_import(&store, db.pool(), &device, &second.id)
            .await
            .unwrap_err()
            .code,
        "stale_import"
    );
    assert_eq!(game_count(&db).await, 0);
    assert!(matches!(
        apply_import(&store, db.pool(), &device, &third.id)
            .await
            .unwrap(),
        DataImportResult::Transfer { .. }
    ));
    assert_eq!(game_count(&db).await, 1);
    assert_eq!(
        apply_import(&store, db.pool(), &device, &third.id)
            .await
            .unwrap_err()
            .code,
        "stale_import"
    );
}

#[tokio::test]
async fn cancelled_new_picker_or_late_old_completion_cannot_restore_old_drafts() {
    let (db, device) = target().await;
    let file = fixture();
    let store = ImportDraftStore::default();
    let old = preview(&store, &db, &device, &file).await;
    let newer = store.begin_open().unwrap();
    let error = prepare_import(
        &store,
        &old.id,
        db.pool(),
        &device,
        &serde_json::to_string(&file).unwrap(),
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, "stale_import");
    store.cancel(&newer).unwrap();
    assert_eq!(
        apply_import(&store, db.pool(), &device, &old.id)
            .await
            .unwrap_err()
            .code,
        "stale_import"
    );
    assert_eq!(
        apply_import(&store, db.pool(), &device, &newer)
            .await
            .unwrap_err()
            .code,
        "stale_import"
    );
    assert_eq!(game_count(&db).await, 0);
}

#[tokio::test]
async fn explicit_backup_restore_keeps_overwrite_semantics_and_sessions_append_only() {
    let (db, device) = target().await;
    let file = fixture();
    let store = ImportDraftStore::default();
    let first = preview(&store, &db, &device, &file).await;
    let result = apply_import_with_mode(
        &store,
        db.pool(),
        &device,
        &first.id,
        DataImportMode::Restore,
    )
    .await
    .unwrap();
    assert_eq!(serde_json::to_value(&result).unwrap()["kind"], "backup");
    assert_eq!(
        serde_json::to_value(&result).unwrap()["summary"]["sessions_skipped"],
        0
    );
    let mut newer = file.games[0].clone();
    newer.title = "Newer local edit".to_owned();
    newer.meta = newer.meta.touch(Timestamp::from_millis(9_000));
    queries::upsert_game(db.pool(), &newer).await.unwrap();
    let extra = Game::new(
        &device,
        Timestamp::from_millis(2_000),
        "Keep unrelated game",
        GameStatus::Wishlist,
    )
    .unwrap();
    queries::upsert_game(db.pool(), &extra).await.unwrap();
    queries::set_pref(db.pool(), "theme", "dark").await.unwrap();
    let second = preview(&store, &db, &device, &file).await;
    let DataImportResult::Backup { summary } = apply_import_with_mode(
        &store,
        db.pool(),
        &device,
        &second.id,
        DataImportMode::Restore,
    )
    .await
    .unwrap() else {
        panic!("backup result")
    };
    assert_eq!(summary.sessions, 0);
    assert_eq!(summary.sessions_skipped, 1);
    assert_eq!(
        queries::get_game(db.pool(), newer.meta.id)
            .await
            .unwrap()
            .unwrap()
            .title,
        "Legacy collection"
    );
    assert_eq!(queries::load_prefs(db.pool()).await.unwrap().theme, "light");
    assert_eq!(
        game_count(&db).await,
        2,
        "Restore does not clear unrelated rows"
    );
    assert_eq!(
        queries::list_sessions(db.pool(), newer.meta.id)
            .await
            .unwrap()[0]
            .counted_ms,
        60_000
    );
}

#[tokio::test]
async fn transfer_keeps_newer_edits_tombstones_prefs_and_conflict_logging() {
    let (db, device) = target().await;
    let file = fixture();
    let mut payload = transfer(&file, "source-device");
    let store = ImportDraftStore::default();
    let first = preview(&store, &db, &device, &payload).await;
    let result = apply_import(&store, db.pool(), &device, &first.id)
        .await
        .unwrap();
    assert_eq!(serde_json::to_value(&result).unwrap()["kind"], "transfer");
    assert_eq!(
        serde_json::to_value(&result).unwrap()["outcome"]["peers"],
        1
    );
    assert_eq!(queries::load_prefs(db.pool()).await.unwrap().theme, "dark");
    let mut newer = file.games[0].clone();
    newer.title = "Keep this newer edit".to_owned();
    newer.meta = newer.meta.touch(Timestamp::from_millis(9_000));
    queries::upsert_game(db.pool(), &newer).await.unwrap();
    payload.sessions[0].counted_ms = 10;
    let second = preview(&store, &db, &device, &payload).await;
    apply_import(&store, db.pool(), &device, &second.id)
        .await
        .unwrap();
    assert_eq!(
        queries::get_game(db.pool(), newer.meta.id)
            .await
            .unwrap()
            .unwrap()
            .title,
        newer.title
    );
    assert_eq!(
        queries::list_sessions(db.pool(), newer.meta.id)
            .await
            .unwrap()[0]
            .counted_ms,
        60_000
    );
    let logged: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM sync_conflicts WHERE table_name = 'games'")
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert!(logged > 0);
    payload.games[0].meta = payload.games[0]
        .meta
        .tombstone(Timestamp::from_millis(10_000));
    let deleted = preview(&store, &db, &device, &payload).await;
    apply_import(&store, db.pool(), &device, &deleted.id)
        .await
        .unwrap();
    assert!(
        queries::get_game(db.pool(), newer.meta.id)
            .await
            .unwrap()
            .is_none()
    );
    let stale = preview(&store, &db, &device, &transfer(&file, "source-device")).await;
    apply_import(&store, db.pool(), &device, &stale.id)
        .await
        .unwrap();
    assert!(
        queries::get_game(db.pool(), newer.meta.id)
            .await
            .unwrap()
            .is_none()
    );
    let at: String =
        sqlx::query_scalar("SELECT value FROM sync_state WHERE key = 'sync.last_sync_at'")
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert!(at.parse::<i64>().unwrap() > 0);
}

#[tokio::test]
async fn same_device_manual_import_merges_and_is_duplicate_safe() {
    let (db, device) = target().await;
    let store = ImportDraftStore::default();
    let file = transfer(&fixture(), device.as_str());
    let draft = preview(&store, &db, &device, &file).await;
    assert!(draft.same_device);
    let DataImportResult::Transfer { outcome } =
        apply_import(&store, db.pool(), &device, &draft.id)
            .await
            .unwrap()
    else {
        panic!("transfer result")
    };
    assert_eq!(outcome.peers, 1);
    assert_eq!(outcome.pulled, 4);
    assert_eq!(game_count(&db).await, 1);
    let again = preview(&store, &db, &device, &file).await;
    apply_import(&store, db.pool(), &device, &again.id)
        .await
        .unwrap();
    assert_eq!(game_count(&db).await, 1);
    assert_eq!(
        queries::list_sessions(db.pool(), file.games[0].meta.id)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        apply_import(&store, db.pool(), &device, &draft.id)
            .await
            .unwrap_err()
            .code,
        "stale_import"
    );
}

#[tokio::test]
async fn malformed_ambiguous_and_unsupported_files_never_publish_or_write() {
    let (db, device) = target().await;
    let store = ImportDraftStore::default();
    let good = serde_json::to_value(fixture()).unwrap();
    let sync_good = serde_json::to_value(transfer(&fixture(), "source-device")).unwrap();
    let mut ambiguous = good.clone();
    ambiguous["schema_version"] = json!(SCHEMA_VERSION);
    let mut format = good.clone();
    format["header"]["format"] = json!(BACKUP_FORMAT + 1);
    let mut new_schema = good.clone();
    new_schema["header"]["schema_version"] = json!(SCHEMA_VERSION + 1);
    let mut zero = sync_good.clone();
    zero["schema_version"] = json!(0);
    let mut empty_device = sync_good.clone();
    empty_device["device_id"] = json!(" ");
    let mut invalid_prefs = good.clone();
    invalid_prefs["prefs"]["language"] = json!("unknown");
    let mut invalid_time = good.clone();
    invalid_time["sessions"][0]["counted_ms"] = json!(-1);
    let mut invalid_rating = good.clone();
    invalid_rating["games"][0]["rating"] = json!(1);
    let mut invalid_parent = good.clone();
    invalid_parent["games"] = json!([]);
    let mut duplicates = good.clone();
    duplicates["sessions"]
        .as_array_mut()
        .unwrap()
        .push(good["sessions"][0].clone());
    let mut invalid_shape = good.clone();
    invalid_shape["games"] = json!("wrong type");
    let unified = serde_json::to_value(library(&fixture(), "source-device")).unwrap();
    let mut bad_version = unified.clone();
    bad_version["version"] = json!(2);
    let mut bad_format = unified.clone();
    bad_format["format"] = json!("unknown");
    let mut bad_schema = unified.clone();
    bad_schema["payload"]["schema_version"] = json!(SCHEMA_VERSION + 1);
    let mut hybrid = unified;
    hybrid["header"] = good["header"].clone();
    let mut files: Vec<String> = [
        ambiguous,
        format,
        new_schema,
        zero,
        empty_device,
        invalid_prefs,
        invalid_time,
        invalid_rating,
        invalid_parent,
        duplicates,
        invalid_shape,
        bad_version,
        bad_format,
        bad_schema,
        hybrid,
    ]
    .into_iter()
    .map(|value| value.to_string())
    .collect();
    files.extend([
        "null".to_owned(),
        "{".to_owned(),
        "{}".to_owned(),
        format!("{{\"header\":null,{}", &good.to_string()[1..]),
    ]);
    for contents in files {
        let id = store.begin_open().unwrap();
        assert!(
            prepare_import(&store, &id, db.pool(), &device, &contents)
                .await
                .is_err()
        );
        assert_eq!(
            apply_import(&store, db.pool(), &device, &id)
                .await
                .unwrap_err()
                .code,
            "stale_import"
        );
        assert_eq!(game_count(&db).await, 0);
        assert_eq!(queries::load_prefs(db.pool()).await.unwrap().theme, "dark");
    }
}

#[tokio::test]
async fn harmless_legacy_metadata_and_existing_local_references_remain_supported() {
    let (db, device) = target().await;
    let file = fixture();
    let store = ImportDraftStore::default();
    let mut value = serde_json::to_value(&file).unwrap();
    value["description"] = json!("Extra metadata tolerated by the legacy importer");
    let first = preview(&store, &db, &device, &value).await;
    apply_import(&store, db.pool(), &device, &first.id)
        .await
        .unwrap();
    value["games"] = json!([]);
    value["chapters"] = json!([]);
    let second = preview(&store, &db, &device, &value).await;
    assert_eq!((second.games, second.chapters, second.sessions), (0, 0, 1));
    apply_import(&store, db.pool(), &device, &second.id)
        .await
        .unwrap();
    let before: Value =
        serde_json::to_value(sync::collect_local(db.pool(), &device).await.unwrap()).unwrap();
    let exported = sync::collect_local(db.pool(), &device).await.unwrap();
    let after: Value =
        serde_json::to_value(sync::collect_local(db.pool(), &device).await.unwrap()).unwrap();
    assert_eq!(before["games"], after["games"]);
    assert_eq!(exported.games.len(), 1);
}

#[tokio::test]
async fn a_consumed_import_that_fails_storage_cannot_be_replayed() {
    let (db, device) = target().await;
    let file = fixture();
    let store = ImportDraftStore::default();
    let draft = preview(&store, &db, &device, &file).await;
    sqlx::query("CREATE TRIGGER reject_import BEFORE INSERT ON games BEGIN SELECT RAISE(ABORT, 'test write failure'); END")
        .execute(db.pool()).await.unwrap();
    assert!(
        apply_import(&store, db.pool(), &device, &draft.id)
            .await
            .is_err()
    );
    assert_eq!(game_count(&db).await, 0);
    assert_eq!(queries::load_prefs(db.pool()).await.unwrap().theme, "dark");
    sqlx::query("DROP TRIGGER reject_import")
        .execute(db.pool())
        .await
        .unwrap();
    assert_eq!(
        apply_import(&store, db.pool(), &device, &draft.id)
            .await
            .unwrap_err()
            .code,
        "stale_import"
    );
}
