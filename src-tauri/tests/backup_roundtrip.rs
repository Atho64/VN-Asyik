//! Backup round-trip: export from one database, import into a fresh one, and
//! verify nothing is lost, duplicated, or silently overwritten.

use vn_core::clock::Timestamp;
use vn_core::ids::EntityId;
use vn_core::migration::{BACKUP_FORMAT, BackupHeader};
use vn_core::model::{Chapter, Ending, Game, GameStatus, PlaySession, SessionSource};

use vn_tracker_lib::commands::backup::{BackupFile, import_payload};
use vn_tracker_lib::db::{Db, models::PrefsSnapshot, queries};

fn now() -> Timestamp {
    Timestamp::from_millis(1_700_000_000_000)
}

/// Build a populated database and return it with the payload it represents.
async fn populated() -> (Db, BackupFile) {
    let db = Db::open_memory().await.expect("open");
    let device = queries::ensure_device_id(db.pool()).await.expect("device");
    let pool = db.pool();

    let mut game = Game::new(&device, now(), "うみねこのなく頃に", GameStatus::Playing).unwrap();
    game.set_rating(Some(10)).unwrap();
    game.set_notes(Some("chapter 3 notes".to_owned())).unwrap();
    game.accent_color = Some("#8b9cff".to_owned());
    game.cover_sexual = Some(1);
    queries::upsert_game(pool, &game).await.expect("game");

    let mut chapter = Chapter::new(&device, now(), game.meta.id, "Chapter 1", 1).unwrap();
    chapter.reached = true;
    queries::upsert_chapter(pool, &chapter)
        .await
        .expect("chapter");

    let mut session = PlaySession::new(
        &device,
        now(),
        game.meta.id,
        Timestamp::from_millis(1_000),
        Timestamp::from_millis(61_000),
        60_000,
        SessionSource::Manual,
    )
    .unwrap();
    session.chapter_id = Some(chapter.meta.id);
    queries::insert_session(pool, &session)
        .await
        .expect("session");

    let mut ending = Ending::new(&device, now(), game.meta.id, "True End").unwrap();
    ending.achieved = true;
    queries::upsert_ending(pool, &ending).await.expect("ending");

    queries::set_pref(pool, "theme", "light")
        .await
        .expect("pref");
    queries::set_pref(pool, "nsfw", "hide").await.expect("pref");

    let payload = BackupFile {
        header: BackupHeader {
            format: BACKUP_FORMAT,
            schema_version: vn_core::migration::SCHEMA_VERSION,
            created_at_ms: now().as_millis(),
            app_version: "0.1.0".to_owned(),
        },
        games: queries::list_games(pool).await.expect("games"),
        routes: queries::list_routes(pool, game.meta.id)
            .await
            .expect("routes"),
        chapters: queries::list_chapters(pool, game.meta.id)
            .await
            .expect("chapters"),
        sessions: queries::list_sessions(pool, game.meta.id)
            .await
            .expect("sessions"),
        endings: queries::list_endings(pool, game.meta.id)
            .await
            .expect("endings"),
        prefs: PrefsSnapshot {
            theme: "light".to_owned(),
            language: "en".to_owned(),
            nsfw: "hide".to_owned(),
            reduce_effects: true,
        },
    };

    (db, payload)
}

/// Exercise the same importer as the Tauri command without its `State` wrapper.
async fn import(db: &Db, payload: &BackupFile) -> (usize, usize) {
    let summary = import_payload(db.pool(), payload).await.expect("import");
    (summary.sessions, summary.sessions_skipped)
}

#[tokio::test]
async fn round_trip_preserves_every_row() {
    let (_source, payload) = populated().await;
    let target = Db::open_memory().await.expect("target");

    let (inserted, skipped) = import(&target, &payload).await;
    assert_eq!(inserted, 1);
    assert_eq!(skipped, 0);

    let games = queries::list_games(target.pool()).await.expect("games");
    assert_eq!(games.len(), 1);
    assert_eq!(games[0].title, "うみねこのなく頃に");
    assert_eq!(games[0].rating, Some(10));
    assert_eq!(games[0].notes.as_deref(), Some("chapter 3 notes"));

    let chapters = queries::list_chapters(target.pool(), games[0].meta.id)
        .await
        .expect("chapters");
    assert_eq!(chapters.len(), 1);
    assert!(chapters[0].reached);

    let sessions = queries::list_sessions(target.pool(), games[0].meta.id)
        .await
        .expect("sessions");
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].counted_ms, 60_000);
    assert_eq!(sessions[0].chapter_id, Some(chapters[0].meta.id));

    let endings = queries::list_endings(target.pool(), games[0].meta.id)
        .await
        .expect("endings");
    assert_eq!(endings.len(), 1);
    assert!(endings[0].achieved);

    let prefs = queries::load_prefs(target.pool()).await.expect("prefs");
    assert_eq!(prefs.theme, "light");
    assert_eq!(prefs.nsfw, "hide");
    assert!(
        prefs.reduce_effects,
        "reduced effects must also be restored"
    );
}

#[tokio::test]
async fn ids_are_stable_across_export_and_import() {
    let (source, payload) = populated().await;
    let original = queries::list_games(source.pool()).await.expect("games")[0]
        .meta
        .id;

    let target = Db::open_memory().await.expect("target");
    import(&target, &payload).await;

    let imported = queries::list_games(target.pool()).await.expect("games")[0]
        .meta
        .id;
    assert_eq!(original, imported);
}

#[tokio::test]
async fn importing_twice_does_not_duplicate_games_but_skips_sessions() {
    let (_source, payload) = populated().await;
    let target = Db::open_memory().await.expect("target");

    import(&target, &payload).await;
    let (inserted, skipped) = import(&target, &payload).await;

    assert_eq!(inserted, 0, "sessions are append-only");
    assert_eq!(
        skipped, 1,
        "the duplicate session is skipped, not rewritten"
    );

    let games = queries::list_games(target.pool()).await.expect("games");
    assert_eq!(games.len(), 1, "games upsert in place");

    let sessions = queries::list_sessions(target.pool(), games[0].meta.id)
        .await
        .expect("sessions");
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].counted_ms, 60_000);
}

#[tokio::test]
async fn backup_payload_survives_json() {
    let (_source, payload) = populated().await;
    let json = serde_json::to_string(&payload).expect("serialize");
    let decoded: BackupFile = serde_json::from_str(&json).expect("deserialize");

    assert_eq!(decoded.games.len(), payload.games.len());
    assert_eq!(decoded.sessions.len(), payload.sessions.len());
    assert_eq!(decoded.games[0].title, payload.games[0].title);
    assert_eq!(decoded.header.schema_version, payload.header.schema_version);
}

#[tokio::test]
async fn a_backup_from_a_newer_schema_is_refused() {
    let (_source, mut payload) = populated().await;
    let target = Db::open_memory().await.expect("target");
    payload.header.schema_version = vn_core::migration::MAX_READABLE_SCHEMA + 1;
    assert_eq!(
        import_payload(target.pool(), &payload)
            .await
            .expect_err("new schema")
            .code,
        "backup_too_new"
    );

    payload.header.schema_version = 0;
    assert!(import_payload(target.pool(), &payload).await.is_err());
    assert!(
        queries::list_games(target.pool())
            .await
            .expect("games")
            .is_empty()
    );
}

#[tokio::test]
async fn missing_parent_game_still_rejects_orphan_sessions() {
    let (_source, payload) = populated().await;
    let target = Db::open_memory().await.expect("target");

    // A hand-edited backup that drops games but keeps sessions must fail loudly
    // rather than write orphan rows.
    let orphaned = BackupFile {
        games: Vec::new(),
        chapters: Vec::new(),
        endings: Vec::new(),
        ..payload
    };

    let pool = target.pool();
    let result = import_payload(pool, &orphaned).await;
    assert!(
        result.is_err(),
        "foreign key must reject the orphan session"
    );

    let any_game: Option<(String,)> = sqlx::query_as("SELECT id FROM games LIMIT 1")
        .fetch_optional(pool)
        .await
        .expect("query");
    assert!(any_game.is_none());
    assert_ne!(orphaned.sessions[0].game_id, EntityId::new());
}

#[tokio::test]
async fn invalid_session_rolls_back_earlier_backup_overwrites_and_inserts() {
    let (target, mut payload) = populated().await;
    let original_game = payload.games[0].clone();
    let original_chapter = payload.chapters[0].clone();
    let original_ending = payload.endings[0].clone();
    let original_sessions = payload.sessions.clone();
    let original_prefs = queries::load_prefs(target.pool()).await.expect("prefs");
    let device_id = queries::ensure_device_id(target.pool())
        .await
        .expect("device");

    payload.games[0].title = "Changed before failure".to_owned();
    payload.chapters[0].name = "Changed chapter".to_owned();
    payload.endings[0].name = "Changed ending".to_owned();
    payload.games.push(
        Game::new(
            &device_id,
            now(),
            "New before failure",
            GameStatus::Wishlist,
        )
        .expect("new game"),
    );
    payload.prefs.theme = "dark".to_owned();
    let mut invalid = payload.sessions[0].clone();
    invalid.meta.id = EntityId::new();
    invalid.counted_ms = -1;
    payload.sessions.push(invalid);

    assert!(import_payload(target.pool(), &payload).await.is_err());
    assert_eq!(
        queries::list_games(target.pool()).await.expect("games"),
        vec![original_game.clone()]
    );
    assert_eq!(
        queries::list_chapters(target.pool(), original_game.meta.id)
            .await
            .expect("chapters"),
        vec![original_chapter]
    );
    assert_eq!(
        queries::list_endings(target.pool(), original_game.meta.id)
            .await
            .expect("endings"),
        vec![original_ending]
    );
    assert_eq!(
        queries::list_sessions(target.pool(), original_game.meta.id)
            .await
            .expect("sessions"),
        original_sessions
    );
    let prefs = queries::load_prefs(target.pool()).await.expect("prefs");
    assert_eq!(prefs.theme, original_prefs.theme);
    assert_eq!(prefs.reduce_effects, original_prefs.reduce_effects);
}

#[tokio::test]
async fn invalid_preferences_or_negative_idle_time_leave_backup_target_unchanged() {
    let (target, mut payload) = populated().await;
    let original = payload.games[0].clone();
    payload.games[0].title = "Must not be applied".to_owned();
    payload.prefs.language = "unsupported".to_owned();
    assert_eq!(
        import_payload(target.pool(), &payload)
            .await
            .expect_err("language")
            .code,
        "invalid_language"
    );
    payload.prefs.language = "id".to_owned();
    payload.sessions[0].afk_ms = -1;
    assert_eq!(
        import_payload(target.pool(), &payload)
            .await
            .expect_err("idle time")
            .code,
        "invalid_backup"
    );
    assert_eq!(
        queries::list_games(target.pool()).await.expect("games"),
        vec![original]
    );
}
