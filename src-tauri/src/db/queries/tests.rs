//! Integration tests for the query layer, run against a real in-memory `SQLite`
//! database with the real migrations applied.

use vn_core::clock::Timestamp;
use vn_core::ids::{DeviceId, EntityId};
use vn_core::model::{
    Chapter, Ending, Game, GameStatus, NsfwMode, PlaySession, Route, SessionSource, ThemeMode,
};

use super::*;
use crate::db::Db;

async fn fixture() -> (Db, DeviceId) {
    let db = Db::open_memory().await.expect("open in-memory db");
    let device = ensure_device_id(db.pool()).await.expect("device id");
    (db, device)
}

fn now() -> Timestamp {
    Timestamp::from_millis(1_700_000_000_000)
}

#[tokio::test]
async fn device_id_is_created_once_and_stable() {
    let (db, first) = fixture().await;
    let second = ensure_device_id(db.pool()).await.expect("second read");
    assert_eq!(first, second);
}

#[tokio::test]
async fn games_round_trip_through_storage() {
    let (db, device) = fixture().await;
    let mut game = Game::new(&device, now(), "Symphonic Rain", GameStatus::Playing).unwrap();
    game.set_rating(Some(90)).unwrap();
    game.title_original = Some("シンフォニックレイン".to_owned());
    game.accent_color = Some("#7aa2f7".to_owned());
    upsert_game(db.pool(), &game).await.expect("insert");

    let loaded = get_game(db.pool(), game.meta.id)
        .await
        .expect("read")
        .expect("present");
    assert_eq!(loaded, game);
}

#[tokio::test]
async fn unicode_titles_survive_the_round_trip() {
    let (db, device) = fixture().await;
    let game = Game::new(&device, now(), "うみねこのなく頃に", GameStatus::Playing).unwrap();
    upsert_game(db.pool(), &game).await.expect("insert");
    let loaded = get_game(db.pool(), game.meta.id)
        .await
        .expect("read")
        .expect("present");
    assert_eq!(loaded.title, "うみねこのなく頃に");
}

#[tokio::test]
async fn upsert_updates_in_place_without_duplicating() {
    let (db, device) = fixture().await;
    let mut game = Game::new(&device, now(), "A", GameStatus::Playing).unwrap();
    upsert_game(db.pool(), &game).await.expect("insert");

    game.status = GameStatus::Completed;
    game.meta = game
        .meta
        .touch(Timestamp::from_millis(now().as_millis() + 1_000));
    upsert_game(db.pool(), &game).await.expect("update");

    let all = list_games(db.pool()).await.expect("list");
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].status, GameStatus::Completed);
}

#[tokio::test]
async fn soft_delete_hides_game_and_children() {
    let (db, device) = fixture().await;
    let game = Game::new(&device, now(), "A", GameStatus::Playing).unwrap();
    upsert_game(db.pool(), &game).await.expect("insert");
    let chapter = Chapter::new(&device, now(), game.meta.id, "Ch 1", 0).unwrap();
    upsert_chapter(db.pool(), &chapter).await.expect("chapter");
    let session = PlaySession::new(
        &device,
        now(),
        game.meta.id,
        Timestamp::from_millis(1_000),
        Timestamp::from_millis(2_000),
        1_000,
        SessionSource::Desktop,
    )
    .unwrap();
    insert_session(db.pool(), &session).await.expect("session");

    soft_delete_game(db.pool(), game.meta.id, now(), &device)
        .await
        .expect("delete");

    assert!(
        get_game(db.pool(), game.meta.id)
            .await
            .expect("read")
            .is_none()
    );
    assert!(list_games(db.pool()).await.expect("list").is_empty());
    assert!(
        list_chapters(db.pool(), game.meta.id)
            .await
            .expect("chapters")
            .is_empty()
    );
    assert_eq!(
        tracked_playtime_ms(db.pool(), game.meta.id)
            .await
            .expect("pt"),
        0
    );
}

#[tokio::test]
async fn sessions_are_append_only() {
    let (db, device) = fixture().await;
    let game = Game::new(&device, now(), "A", GameStatus::Playing).unwrap();
    upsert_game(db.pool(), &game).await.expect("insert");
    let session = PlaySession::new(
        &device,
        now(),
        game.meta.id,
        Timestamp::from_millis(1_000),
        Timestamp::from_millis(2_000),
        1_000,
        SessionSource::Desktop,
    )
    .unwrap();
    assert!(
        insert_session(db.pool(), &session)
            .await
            .expect("first insert"),
        "the first insert writes a row"
    );

    // A duplicate is the ordinary case on a second device or a re-import, not a
    // failure: sessions merge by uuid and are never overwritten.
    let duplicate = insert_session(db.pool(), &session).await;
    assert!(!duplicate.expect("duplicate is not an error"));

    let total = tracked_playtime_ms(db.pool(), game.meta.id)
        .await
        .expect("pt");
    assert_eq!(total, 1_000);
}

#[tokio::test]
async fn playtime_sums_across_sessions() {
    let (db, device) = fixture().await;
    let game = Game::new(&device, now(), "A", GameStatus::Playing).unwrap();
    upsert_game(db.pool(), &game).await.expect("insert");
    for (start, count) in [(0, 1_000), (10_000, 2_500), (20_000, 500)] {
        let session = PlaySession::new(
            &device,
            now(),
            game.meta.id,
            Timestamp::from_millis(start),
            Timestamp::from_millis(start + count),
            count,
            SessionSource::Desktop,
        )
        .unwrap();
        insert_session(db.pool(), &session).await.expect("insert");
    }
    assert_eq!(
        tracked_playtime_ms(db.pool(), game.meta.id)
            .await
            .expect("pt"),
        4_000
    );

    let by_game = playtime_by_game(db.pool()).await.expect("by game");
    assert_eq!(by_game, vec![(game.meta.id, 4_000)]);
}

#[tokio::test]
async fn chapters_are_ordered_by_sort_index() {
    let (db, device) = fixture().await;
    let game = Game::new(&device, now(), "A", GameStatus::Playing).unwrap();
    upsert_game(db.pool(), &game).await.expect("insert");
    for (name, idx) in [("Prologue", 0), ("Chapter 2", 2), ("Chapter 1", 1)] {
        let chapter = Chapter::new(&device, now(), game.meta.id, name, idx).unwrap();
        upsert_chapter(db.pool(), &chapter).await.expect("chapter");
    }
    let names: Vec<String> = list_chapters(db.pool(), game.meta.id)
        .await
        .expect("list")
        .into_iter()
        .map(|c| c.name)
        .collect();
    assert_eq!(names, vec!["Prologue", "Chapter 1", "Chapter 2"]);
}

#[tokio::test]
async fn status_breakdown_counts_stored_games() {
    let (db, device) = fixture().await;
    for (title, status) in [
        ("a", GameStatus::Playing),
        ("b", GameStatus::Playing),
        ("c", GameStatus::Completed),
    ] {
        let game = Game::new(&device, now(), title, status).unwrap();
        upsert_game(db.pool(), &game).await.expect("insert");
    }
    let breakdown = status_breakdown(db.pool()).await.expect("breakdown");
    assert_eq!(breakdown.playing, 2);
    assert_eq!(breakdown.completed, 1);
    assert_eq!(breakdown.total(), 3);
}

#[tokio::test]
async fn endings_round_trip() {
    let (db, device) = fixture().await;
    let game = Game::new(&device, now(), "A", GameStatus::Playing).unwrap();
    upsert_game(db.pool(), &game).await.expect("insert");
    let mut ending = Ending::new(&device, now(), game.meta.id, "True End").unwrap();
    ending.route = Some("Mio".to_owned());
    upsert_ending(db.pool(), &ending).await.expect("insert");

    let loaded = list_endings(db.pool(), game.meta.id).await.expect("list");
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].route.as_deref(), Some("Mio"));
}

#[tokio::test]
async fn an_ending_is_found_by_its_own_id_not_just_its_game() {
    // The edit and delete commands are handed an ending id and nothing else, so a
    // game-keyed lookup would mean loading every ending to find one.
    let (db, device) = fixture().await;
    let game = Game::new(&device, now(), "A", GameStatus::Playing).unwrap();
    upsert_game(db.pool(), &game).await.expect("insert");
    let ending = Ending::new(&device, now(), game.meta.id, "True End").unwrap();
    upsert_ending(db.pool(), &ending).await.expect("insert");

    let found = ending_by_id(db.pool(), ending.meta.id)
        .await
        .expect("lookup")
        .expect("present");
    assert_eq!(found.name, "True End");
    assert_eq!(found.game_id, game.meta.id);
}

#[tokio::test]
async fn an_ending_can_be_soft_deleted_and_the_tombstone_hides_it() {
    // Sync merges by id, so a row removal would let the other device put the ending
    // back on the next pull. The tombstone is what makes the delete travel.
    let (db, device) = fixture().await;
    let game = Game::new(&device, now(), "A", GameStatus::Playing).unwrap();
    upsert_game(db.pool(), &game).await.expect("insert");
    let ending = Ending::new(&device, now(), game.meta.id, "Bad End").unwrap();
    upsert_ending(db.pool(), &ending).await.expect("insert");

    soft_delete_ending(db.pool(), ending.meta.id, now(), &device)
        .await
        .expect("delete");

    assert!(
        list_endings(db.pool(), game.meta.id)
            .await
            .expect("list")
            .is_empty(),
        "a deleted ending must not appear in the game's list"
    );
    let row = ending_by_id(db.pool(), ending.meta.id)
        .await
        .expect("lookup")
        .expect("the row is kept, not removed");
    assert!(
        row.meta.deleted,
        "the tombstone is what travels between devices"
    );
}

#[tokio::test]
async fn clearing_achieved_removes_the_date_instead_of_leaving_it_behind() {
    // The command treats `achieved` as a date rather than a flag, so un-ticking it
    // must clear the timestamp too. A row with achieved=false and a date set would
    // display as reached on one device and not the other.
    let (db, device) = fixture().await;
    let game = Game::new(&device, now(), "A", GameStatus::Playing).unwrap();
    upsert_game(db.pool(), &game).await.expect("insert");

    let mut ending = Ending::new(&device, now(), game.meta.id, "True End").unwrap();
    ending.achieved = true;
    ending.achieved_at = Some(now());
    upsert_ending(db.pool(), &ending).await.expect("insert");

    let mut cleared = ending.clone();
    cleared.achieved = false;
    cleared.achieved_at = None;
    upsert_ending(db.pool(), &cleared).await.expect("update");

    let loaded = ending_by_id(db.pool(), ending.meta.id)
        .await
        .expect("lookup")
        .expect("present");
    assert!(!loaded.achieved, "the flag is cleared");
    assert!(
        loaded.achieved_at.is_none(),
        "a cleared ending keeps no date, so the two can never disagree"
    );
}

#[tokio::test]
async fn resaving_an_achieved_ending_keeps_its_original_date() {
    // Renaming an ending must not rewrite when it was reached. The date is the part
    // a user actually cares about and cannot reconstruct.
    let (db, device) = fixture().await;
    let game = Game::new(&device, now(), "A", GameStatus::Playing).unwrap();
    upsert_game(db.pool(), &game).await.expect("insert");

    let reached_at = Timestamp::from_millis(1_600_000_000_000);
    let mut ending = Ending::new(&device, reached_at, game.meta.id, "Bad End").unwrap();
    ending.achieved = true;
    ending.achieved_at = Some(reached_at);
    upsert_ending(db.pool(), &ending).await.expect("insert");

    // Mirrors `update_ending`: keep the stored date when it is already achieved.
    let mut renamed = ending.clone();
    renamed.name = "Bad End (Mio)".to_owned();
    renamed.achieved_at = ending.achieved_at.or(Some(now()));
    upsert_ending(db.pool(), &renamed).await.expect("update");

    let loaded = ending_by_id(db.pool(), ending.meta.id)
        .await
        .expect("lookup")
        .expect("present");
    assert_eq!(loaded.name, "Bad End (Mio)");
    assert_eq!(
        loaded.achieved_at.map(Timestamp::as_millis),
        Some(reached_at.as_millis()),
        "the original date must survive a rename"
    );
}

#[tokio::test]
async fn an_ending_of_a_removed_game_is_not_listed() {
    // AGENTS.md section 5: a parent can be soft-deleted without cascading, because
    // `PRAGMA foreign_keys` is per connection and a no-op inside a transaction. The
    // read path has to join the parent or the UI shows endings for a game that is
    // no longer in the library.
    let (db, device) = fixture().await;
    let game = Game::new(&device, now(), "Gone", GameStatus::Playing).unwrap();
    upsert_game(db.pool(), &game).await.expect("insert");
    let ending = Ending::new(&device, now(), game.meta.id, "True End").unwrap();
    upsert_ending(db.pool(), &ending).await.expect("insert");

    // Soft-delete the game only. The ending row stays live and reachable by id,
    // which is exactly the orphan the join exists to hide.
    let mut removed = game.clone();
    removed.meta.deleted = true;
    removed.meta.updated_at = now();
    upsert_game(db.pool(), &removed).await.expect("delete game");

    assert!(
        list_endings(db.pool(), game.meta.id)
            .await
            .expect("list")
            .is_empty(),
        "an ending must not outlive its game in the read path"
    );
    assert!(
        list_endings_including_deleted(db.pool(), game.meta.id)
            .await
            .expect("list all")
            .len()
            == 1,
        "backups still need the row, so this is a read-path rule only"
    );
}

#[tokio::test]
async fn prefs_default_then_persist() {
    let (db, _device) = fixture().await;
    let defaults = load_prefs(db.pool()).await.expect("prefs");
    assert_eq!(defaults.theme, "dark");
    assert_eq!(defaults.nsfw_mode(), NsfwMode::Blur);

    set_pref(db.pool(), "theme", ThemeMode::Light.as_str())
        .await
        .expect("set theme");
    set_pref(db.pool(), "reduce_effects", "true")
        .await
        .expect("set effects");

    let updated = load_prefs(db.pool()).await.expect("prefs");
    assert_eq!(updated.theme, "light");
    assert!(updated.reduce_effects);
}

#[tokio::test]
async fn deleting_a_chapter_nulls_session_attribution() {
    let (db, device) = fixture().await;
    let game = Game::new(&device, now(), "A", GameStatus::Playing).unwrap();
    upsert_game(db.pool(), &game).await.expect("insert");
    let chapter = Chapter::new(&device, now(), game.meta.id, "Ch 1", 0).unwrap();
    upsert_chapter(db.pool(), &chapter).await.expect("chapter");

    let mut session = PlaySession::new(
        &device,
        now(),
        game.meta.id,
        Timestamp::from_millis(1_000),
        Timestamp::from_millis(2_000),
        1_000,
        SessionSource::Desktop,
    )
    .unwrap();
    session.chapter_id = Some(chapter.meta.id);
    insert_session(db.pool(), &session).await.expect("session");

    // Hard delete the chapter to exercise ON DELETE SET NULL; the app soft-deletes,
    // but the constraint must still hold if a row is ever removed.
    sqlx::query("DELETE FROM chapters WHERE id = ?")
        .bind(chapter.meta.id.to_string())
        .execute(db.pool())
        .await
        .expect("delete chapter");

    let sessions = list_sessions(db.pool(), game.meta.id).await.expect("list");
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].chapter_id, None);
}

#[tokio::test]
async fn ids_are_unique_per_insert() {
    let (db, device) = fixture().await;
    let a = Game::new(&device, now(), "A", GameStatus::Playing).unwrap();
    let b = Game::new(&device, now(), "B", GameStatus::Playing).unwrap();
    assert_ne!(a.meta.id, b.meta.id);
    upsert_game(db.pool(), &a).await.expect("a");
    upsert_game(db.pool(), &b).await.expect("b");
    assert_eq!(list_games(db.pool()).await.expect("list").len(), 2);
}

#[tokio::test]
async fn unknown_game_id_returns_none() {
    let (db, _device) = fixture().await;
    let missing = EntityId::parse("0192f0aa-0000-7000-8000-0000000000ff").unwrap();
    assert!(get_game(db.pool(), missing).await.expect("read").is_none());
}

#[tokio::test]
async fn afk_time_persists_and_stays_separate_from_active_time() {
    let (db, device) = fixture().await;
    let game = Game::new(&device, now(), "A", GameStatus::Playing).unwrap();
    upsert_game(db.pool(), &game).await.expect("insert");

    let session = PlaySession::new(
        &device,
        now(),
        game.meta.id,
        Timestamp::from_millis(0),
        Timestamp::from_millis(100_000),
        40_000,
        SessionSource::Desktop,
    )
    .unwrap()
    .with_afk(60_000)
    .unwrap();
    insert_session(db.pool(), &session).await.expect("insert");

    let loaded = list_sessions(db.pool(), game.meta.id).await.expect("list");
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].counted_ms, 40_000);
    assert_eq!(loaded[0].afk_ms, 60_000);

    // The headline total is active time only.
    assert_eq!(
        tracked_playtime_ms(db.pool(), game.meta.id)
            .await
            .expect("active"),
        40_000
    );
    assert_eq!(
        afk_playtime_ms(db.pool(), game.meta.id).await.expect("afk"),
        60_000
    );

    let totals = playtime_totals(db.pool()).await.expect("totals");
    assert_eq!(totals, vec![(game.meta.id, 40_000, 60_000)]);
}

#[tokio::test]
async fn sessions_without_afk_read_as_fully_active() {
    let (db, device) = fixture().await;
    let game = Game::new(&device, now(), "A", GameStatus::Playing).unwrap();
    upsert_game(db.pool(), &game).await.expect("insert");
    let session = PlaySession::new(
        &device,
        now(),
        game.meta.id,
        Timestamp::from_millis(0),
        Timestamp::from_millis(10_000),
        10_000,
        SessionSource::Manual,
    )
    .unwrap();
    insert_session(db.pool(), &session).await.expect("insert");

    let loaded = list_sessions(db.pool(), game.meta.id).await.expect("list");
    assert_eq!(loaded[0].afk_ms, 0);
}

#[tokio::test]
async fn afk_settings_default_and_clamp() {
    let (db, _device) = fixture().await;
    let defaults = load_afk_config(db.pool()).await.expect("defaults");
    assert_eq!(defaults.threshold_ms, 60_000);
    assert!(defaults.exclude_afk);

    let saved = save_afk_config(db.pool(), vn_core::afk::AfkConfig::new(1_000, false), now())
        .await
        .expect("save");
    assert_eq!(
        saved.threshold_ms,
        vn_core::afk::AfkConfig::MIN_THRESHOLD_MS
    );
    assert!(!saved.exclude_afk);

    let reloaded = load_afk_config(db.pool()).await.expect("reload");
    assert_eq!(reloaded, saved);
}

#[tokio::test]
async fn a_session_whose_game_is_gone_is_not_counted() {
    // Sessions can outlive their game. Nothing in the app hard-deletes a game,
    // but a table rebuild does: `0011` drops `games` with `PRAGMA foreign_keys`
    // off, and external edits during development do the same. This database has
    // 14 such rows, holding 2h07m. Without the join to `games` they made the
    // stats page report 8.8 hours of playtime for games that no longer existed.
    let (db, device) = fixture().await;
    let game = Game::new(&device, now(), "Kept", GameStatus::Playing).unwrap();
    upsert_game(db.pool(), &game).await.expect("insert");

    for ms in [1_000_i64, 2_000] {
        let session = PlaySession::new(
            &device,
            now(),
            game.meta.id,
            Timestamp::from_millis(1_700_000_000_000),
            Timestamp::from_millis(1_700_000_000_000 + ms),
            ms,
            SessionSource::Desktop,
        )
        .unwrap();
        insert_session(db.pool(), &session).await.expect("insert");
    }

    // An orphan inserted the way one really appears. `DELETE FROM games` is the
    // wrong shape: the foreign key cascades, so there would be no orphan to test.
    // The pragma is per connection and a no-op inside a transaction, so the
    // insert needs one explicitly acquired connection.
    let ghost_id = uuid::Uuid::now_v7().to_string();
    let mut conn = db.pool().acquire().await.expect("connection");
    sqlx::query("PRAGMA foreign_keys = OFF")
        .execute(&mut *conn)
        .await
        .expect("pragma");
    sqlx::query(
        "INSERT INTO play_sessions (id, game_id, chapter_id, started_at, ended_at, \
         counted_ms, afk_ms, source, updated_at, device_id, deleted) \
         VALUES (?, ?, NULL, ?, ?, ?, 0, 'desktop', ?, ?, 0)",
    )
    .bind(uuid::Uuid::now_v7().to_string())
    .bind(&ghost_id)
    .bind(1_700_000_000_000_i64)
    .bind(1_700_003_600_000_i64)
    .bind(3_600_000_i64)
    .bind(now().as_millis())
    .bind(device.to_string())
    .execute(&mut *conn)
    .await
    .expect("orphan insert");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&mut *conn)
        .await
        .expect("pragma");
    drop(conn);

    let by_game = playtime_by_game(db.pool()).await.expect("by game");
    assert_eq!(
        by_game,
        vec![(game.meta.id, 3_000)],
        "the orphaned hour must not be attributed to any game"
    );

    let totals = playtime_totals(db.pool()).await.expect("totals");
    assert_eq!(totals.len(), 1, "one surviving game: {totals:?}");
    assert_eq!(totals[0].1, 3_000);

    let by_day = playtime_by_day(db.pool()).await.expect("by day");
    let summed: i64 = by_day.iter().map(|(_, ms)| ms).sum();
    assert_eq!(
        summed, 3_000,
        "the heatmap and time series must not count the orphan"
    );
}

#[tokio::test]
async fn playtime_buckets_one_local_day_together_and_reports_day_indexes() {
    let (db, device) = fixture().await;
    let game = Game::new(&device, now(), "Hoshi Memo", GameStatus::Playing).unwrap();
    upsert_game(db.pool(), &game).await.expect("insert");

    // 2023-11-14 06:00Z and 06:30Z: inside one local calendar day for every
    // real timezone (offsets run −12..+14, so no local midnight falls between
    // the two), which keeps this assertion deterministic without pinning the
    // test machine's timezone.
    for (start, counted) in [
        (1_699_941_600_000_i64, 40_000_i64),
        (1_699_943_400_000_i64, 20_000),
    ] {
        let session = PlaySession::new(
            &device,
            now(),
            game.meta.id,
            Timestamp::from_millis(start),
            Timestamp::from_millis(start + counted),
            counted,
            SessionSource::Desktop,
        )
        .unwrap();
        insert_session(db.pool(), &session).await.expect("session");
    }

    let by_day = playtime_by_day(db.pool()).await.expect("by day");
    assert_eq!(by_day.len(), 1, "one local day, one bucket: {by_day:?}");
    let (day, ms) = by_day[0];
    assert_eq!(ms, 60_000);
    // The value is a day index (days since epoch), not milliseconds: the
    // heatmap walks its grid in indexes, and a bucket reported in ms matched
    // every lookup against nothing.
    assert!(
        (19_674..=19_676).contains(&day),
        "a 2023-11-14 session is day index ~19675, got {day}"
    );
}

// Routes

#[tokio::test]
async fn routes_round_trip_and_sort_with_nulls_last() {
    let (db, device) = fixture().await;
    let game = Game::new(&device, now(), "Sanoba Witch", GameStatus::Playing).unwrap();
    upsert_game(db.pool(), &game).await.expect("insert game");

    let mut unsorted = Route::new(&device, now(), game.meta.id, "No order").unwrap();
    unsorted.sort_order = None;
    let mut second = Route::new(&device, now(), game.meta.id, "Second").unwrap();
    second.sort_order = Some(2);
    let mut first = Route::new(&device, now(), game.meta.id, "First").unwrap();
    first.sort_order = Some(1);

    for route in [&unsorted, &second, &first] {
        upsert_route(db.pool(), route).await.expect("insert route");
    }

    let listed = list_routes(db.pool(), game.meta.id).await.expect("list");
    let names: Vec<&str> = listed.iter().map(|r| r.name.as_str()).collect();
    // Ordered rows first, in their order; the unsorted one last rather than
    // jumping to the front on a NULL comparison.
    assert_eq!(names, vec!["First", "Second", "No order"]);
    assert_eq!(listed[0], first);
}

#[tokio::test]
async fn a_route_of_a_removed_game_is_not_listed() {
    let (db, device) = fixture().await;
    let game = Game::new(&device, now(), "Removed", GameStatus::Playing).unwrap();
    upsert_game(db.pool(), &game).await.expect("insert game");
    let route = Route::new(&device, now(), game.meta.id, "Shiroha").unwrap();
    upsert_route(db.pool(), &route).await.expect("insert route");

    // Tombstone the game without cascading the child row, so the route survives as
    // an orphan exactly as a real interrupted cascade would leave it.
    sqlx::query("UPDATE games SET deleted = 1 WHERE id = ?")
        .bind(game.meta.id.to_string())
        .execute(db.pool())
        .await
        .expect("tombstone game");

    let listed = list_routes(db.pool(), game.meta.id).await.expect("list");
    assert!(
        listed.is_empty(),
        "a route whose game is gone must not be listed: {listed:?}"
    );
}

#[tokio::test]
async fn merging_a_route_moves_both_children_and_keeps_the_source_row() {
    let (db, device) = fixture().await;
    let game = Game::new(&device, now(), "Merged", GameStatus::Playing).unwrap();
    upsert_game(db.pool(), &game).await.expect("insert game");

    let engine_side = Route::new(&device, now(), game.meta.id, "Shiroha").unwrap();
    let ending_side = Route::new(&device, now(), game.meta.id, "Shiroha").unwrap();
    upsert_route(db.pool(), &engine_side).await.expect("engine");
    upsert_route(db.pool(), &ending_side).await.expect("ending");

    let mut chapter = Chapter::new(&device, now(), game.meta.id, "Prologue", 0).unwrap();
    chapter.route_id = Some(engine_side.meta.id);
    upsert_chapter(db.pool(), &chapter).await.expect("chapter");

    let mut ending = Ending::new(&device, now(), game.meta.id, "Good End").unwrap();
    ending.route_id = Some(ending_side.meta.id);
    upsert_ending(db.pool(), &ending).await.expect("ending");

    let mut tx = db.pool().begin().await.expect("tx");
    reassign_route_children(&mut tx, ending_side.meta.id, engine_side.meta.id)
        .await
        .expect("merge");
    tx.commit().await.expect("commit");

    let chapters = list_chapters(db.pool(), game.meta.id)
        .await
        .expect("chapters");
    assert_eq!(chapters[0].route_id, Some(engine_side.meta.id));
    let endings = list_endings(db.pool(), game.meta.id)
        .await
        .expect("endings");
    assert_eq!(endings[0].route_id, Some(engine_side.meta.id));

    // The merged-away row is still present so a device that had not seen the merge
    // converges instead of resurrecting it as a second live route.
    let all = list_routes_including_deleted(db.pool(), game.meta.id)
        .await
        .expect("all routes");
    assert_eq!(all.len(), 2, "merge must not delete the source row");
}

#[tokio::test]
async fn route_links_survive_the_chapter_and_ending_upsert_paths() {
    // The upsert is the only write path sync uses, so a column missing from it
    // would silently drop the link on the first sync and nowhere else.
    let (db, device) = fixture().await;
    let game = Game::new(&device, now(), "Linked", GameStatus::Playing).unwrap();
    upsert_game(db.pool(), &game).await.expect("insert game");
    let route = Route::new(&device, now(), game.meta.id, "Mio").unwrap();
    upsert_route(db.pool(), &route).await.expect("route");

    let mut chapter = Chapter::new(&device, now(), game.meta.id, "Ch1", 0).unwrap();
    chapter.route_id = Some(route.meta.id);
    upsert_chapter(db.pool(), &chapter).await.expect("chapter");
    let mut ending = Ending::new(&device, now(), game.meta.id, "True End").unwrap();
    ending.route_id = Some(route.meta.id);
    upsert_ending(db.pool(), &ending).await.expect("ending");

    let reread_chapter = list_chapters(db.pool(), game.meta.id)
        .await
        .expect("chapters");
    assert_eq!(reread_chapter[0].route_id, Some(route.meta.id));
    let reread_ending = list_endings(db.pool(), game.meta.id)
        .await
        .expect("endings");
    assert_eq!(reread_ending[0].route_id, Some(route.meta.id));
}

#[tokio::test]
async fn a_session_can_target_a_route_instead_of_a_chapter() {
    // "Time the Shiroha route" persists a session with a route and no chapter. If the
    // column were dropped on the way through, the time would land unattributed and
    // the route total would stay at zero while the hours vanished into the game.
    let (db, device) = fixture().await;
    let game = Game::new(&device, now(), "Sanoba Witch", GameStatus::Playing).unwrap();
    upsert_game(db.pool(), &game).await.expect("game");
    let route = Route::new(&device, now(), game.meta.id, "Shiroha").unwrap();
    upsert_route(db.pool(), &route).await.expect("route");

    let mut session = PlaySession::new(
        &device,
        now(),
        game.meta.id,
        now(),
        Timestamp::from_millis(now().as_millis() + 60_000),
        60_000,
        SessionSource::Manual,
    )
    .unwrap();
    session.route_id = Some(route.meta.id);
    insert_session(db.pool(), &session).await.expect("insert");

    let loaded = list_sessions(db.pool(), game.meta.id).await.expect("list");
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].route_id, Some(route.meta.id));
    assert_eq!(loaded[0].chapter_id, None, "a route run names no chapter");
}

#[tokio::test]
async fn a_custom_cover_path_round_trips_and_defaults_to_absent() {
    let (db, device) = fixture().await;
    let game = Game::new(&device, now(), "Non-VNDB", GameStatus::Playing).unwrap();
    // A manually added game has no VNDB id, which is the case this column exists
    // for. It must persist without one.
    assert_eq!(game.vndb_id, None);
    upsert_game(db.pool(), &game).await.expect("insert");

    let loaded = get_game(db.pool(), game.meta.id)
        .await
        .expect("get")
        .expect("present");
    assert_eq!(loaded.custom_cover_path, None);
    assert!(!loaded.custom_cover_source_vndb);

    let mut updated = loaded;
    updated.custom_cover_path = Some("covers/custom/abc.jpg".to_owned());
    updated.custom_cover_source_vndb = false;
    updated.meta = updated.meta.touch(now());
    upsert_game(db.pool(), &updated).await.expect("update");

    let reloaded = get_game(db.pool(), game.meta.id)
        .await
        .expect("get")
        .expect("present");
    assert_eq!(
        reloaded.custom_cover_path.as_deref(),
        Some("covers/custom/abc.jpg")
    );
    assert!(!reloaded.custom_cover_source_vndb);
}

#[tokio::test]
async fn clearing_a_custom_cover_path_persists_as_null() {
    let (db, device) = fixture().await;
    let mut game = Game::new(&device, now(), "Cleared", GameStatus::Playing).unwrap();
    game.custom_cover_path = Some("covers/custom/abc.jpg".to_owned());
    upsert_game(db.pool(), &game).await.expect("insert");

    game.custom_cover_path = None;
    game.meta = game.meta.touch(now());
    upsert_game(db.pool(), &game).await.expect("clear");

    let reloaded = get_game(db.pool(), game.meta.id)
        .await
        .expect("get")
        .expect("present");
    // NULL, not the old value: an upsert that omitted the column would silently
    // keep the previous cover and "remove" would do nothing.
    assert_eq!(reloaded.custom_cover_path, None);
}

#[tokio::test]
async fn a_vndb_refresh_does_not_replace_a_user_chosen_cover() {
    let (db, device) = fixture().await;
    let mut game = Game::new(&device, now(), "Kept", GameStatus::Playing).unwrap();
    game.custom_cover_path = Some("covers/custom/mine.jpg".to_owned());
    game.custom_cover_source_vndb = false;
    upsert_game(db.pool(), &game).await.expect("insert");

    // Applying VNDB metadata updates the cover rating and URL only; the custom
    // path is the user's own file and must survive.
    let vn = crate::vndb::dto::Vn::default();
    crate::vndb::import::apply_vn_to_game(&mut game, &vn);
    game.meta = game.meta.touch(now());
    upsert_game(db.pool(), &game).await.expect("update");

    let reloaded = get_game(db.pool(), game.meta.id)
        .await
        .expect("get")
        .expect("present");
    assert_eq!(
        reloaded.custom_cover_path.as_deref(),
        Some("covers/custom/mine.jpg")
    );
}
