//! Integration tests for the query layer, run against a real in-memory `SQLite`
//! database with the real migrations applied.

use vn_core::clock::Timestamp;
use vn_core::ids::{DeviceId, EntityId};
use vn_core::model::{
    Chapter, Ending, Game, GameStatus, NsfwMode, PlaySession, Route, SessionSource, ThemeMode,
};

use super::*;
use crate::commands::journey::MonthMoment;
use crate::db::Db;
use crate::db::reading;

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
    assert_eq!(
        list_games(db.pool()).await.expect("list").len(),
        0,
        "list_games(db.pool()).await.expect(\"list\") should be empty"
    );
    assert_eq!(
        list_chapters(db.pool(), game.meta.id)
            .await
            .expect("chapters")
            .len(),
        0
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

    assert_eq!(
        list_endings(db.pool(), game.meta.id)
            .await
            .expect("list")
            .len(),
        0,
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

    assert_eq!(
        list_endings(db.pool(), game.meta.id)
            .await
            .expect("list")
            .len(),
        0,
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
    assert!(!defaults.beta_features);

    set_pref(db.pool(), "theme", ThemeMode::Light.as_str())
        .await
        .expect("set theme");
    set_pref(db.pool(), "reduce_effects", "true")
        .await
        .expect("set effects");

    let updated = load_prefs(db.pool()).await.expect("prefs");
    assert_eq!(updated.theme, "light");
    assert!(updated.reduce_effects);
    set_pref(db.pool(), "beta_features", "true").await.unwrap();
    assert!(load_prefs(db.pool()).await.unwrap().beta_features);
    set_pref(db.pool(), "beta_features", "false").await.unwrap();
    assert!(!load_prefs(db.pool()).await.unwrap().beta_features);
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

    // 06:00Z and 06:30Z fall on one local calendar day in all real timezones (-12..+14).
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
    assert_eq!(
        listed.len(),
        0,
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

fn journal_entry(
    device: &DeviceId,
    game: EntityId,
    created_ms: i64,
    note: &str,
    route: &str,
    chapter: &str,
) -> vn_core::reading::ReadingEntry {
    vn_core::reading::ReadingEntry {
        meta: vn_core::model::SyncMeta::new(device, Timestamp::from_millis(created_ms)),
        game_id: game,
        session_id: None,
        created_at: Timestamp::from_millis(created_ms),
        played_at: None,
        note: note.to_owned(),
        save_slot: String::new(),
        route_name: route.to_owned(),
        chapter_name: chapter.to_owned(),
        photo_image_id: String::new(),
    }
}

#[tokio::test]
async fn timeline_orders_mixed_events_newest_first() {
    let (db, device) = fixture().await;
    let game = Game::new(&device, now(), "Ordered", GameStatus::Playing).unwrap();
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
    insert_session(db.pool(), &session).await.expect("session");

    reading::upsert(
        db.pool(),
        &journal_entry(&device, game.meta.id, 3_000, "note", "Mio", "Chapter 1"),
    )
    .await
    .expect("entry");

    let mut ending = Ending::new(&device, now(), game.meta.id, "True End").unwrap();
    ending.achieved = true;
    ending.achieved_at = Some(Timestamp::from_millis(5_000));
    upsert_ending(db.pool(), &ending).await.expect("ending");

    let mut finished = game.clone();
    finished.finished_at = Some(Timestamp::from_millis(9_000));
    finished.meta = finished.meta.touch(now());
    upsert_game(db.pool(), &finished).await.expect("update");

    let page = journey::timeline_page(db.pool(), None).await.expect("page");
    let kinds: Vec<&str> = page.events.iter().map(|e| e.kind.as_str()).collect();
    assert_eq!(kinds, vec!["finished", "ending", "journal", "session"]);
    assert_eq!(page.events[0].ts, 9_000);
    assert_eq!(page.events[0].game.title, "Ordered");
    let ending_event = &page.events[1];
    assert_eq!(ending_event.ending_name.as_deref(), Some("True End"));
    assert_eq!(ending_event.route_name.as_deref(), None);
    let journal_event = &page.events[2];
    assert_eq!(journal_event.note.as_deref(), Some("note"));
    assert_eq!(journal_event.route_name.as_deref(), Some("Mio"));
    let session_event = &page.events[3];
    assert_eq!(session_event.counted_ms, Some(1_000));
    assert_eq!(session_event.source, Some(SessionSource::Desktop));
    assert!(!page.has_more);

    // The tie-break and ordering are a property of the query, not of luck:
    // two reads must agree.
    let again = journey::timeline_page(db.pool(), None)
        .await
        .expect("again");
    assert_eq!(again.events, page.events);
}

#[tokio::test]
async fn timeline_keyset_pages_walk_backwards_without_skipping() {
    let (db, device) = fixture().await;
    let game = Game::new(&device, now(), "Paged", GameStatus::Playing).unwrap();
    upsert_game(db.pool(), &game).await.expect("insert");

    // 51 sessions one second apart: exactly one more event than a page holds.
    let base = 1_700_000_000_000_i64;
    for i in 0..51_i64 {
        let session = PlaySession::new(
            &device,
            now(),
            game.meta.id,
            Timestamp::from_millis(base + i * 1_000),
            Timestamp::from_millis(base + i * 1_000 + 500),
            500,
            SessionSource::Desktop,
        )
        .unwrap();
        insert_session(db.pool(), &session).await.expect("session");
    }

    let first = journey::timeline_page(db.pool(), None)
        .await
        .expect("page 1");
    assert_eq!(first.events.len(), 50);
    assert!(first.has_more);

    // The cursor is the last row's (ts, id); the next page continues from it.
    let last = first.events.last().expect("nonempty");
    let cursor = crate::commands::journey::TimelineCursor {
        ts: last.ts,
        id: EntityId::parse(&last.id).expect("event id"),
    };
    let second = journey::timeline_page(db.pool(), Some(cursor))
        .await
        .expect("page 2");
    assert_eq!(second.events.len(), 1);
    assert!(!second.has_more);
    assert_eq!(
        second.events[0].ts, base,
        "the oldest event is exactly one step past page 1's last"
    );
}

#[tokio::test]
async fn timeline_hides_a_deleted_games_history() {
    let (db, device) = fixture().await;
    let mut game = Game::new(&device, now(), "Ghosted", GameStatus::Playing).unwrap();
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
    insert_session(db.pool(), &session).await.expect("session");
    reading::upsert(
        db.pool(),
        &journal_entry(&device, game.meta.id, 3_000, "gone soon", "", ""),
    )
    .await
    .expect("entry");
    let mut ending = Ending::new(&device, now(), game.meta.id, "End").unwrap();
    ending.achieved = true;
    ending.achieved_at = Some(Timestamp::from_millis(5_000));
    upsert_ending(db.pool(), &ending).await.expect("ending");

    // Soft delete: the tombstone hides the game and every event joins it.
    game.meta.deleted = true;
    game.meta = game.meta.touch(now());
    upsert_game(db.pool(), &game).await.expect("delete");

    let page = journey::timeline_page(db.pool(), None).await.expect("page");
    assert!(
        page.events.is_empty(),
        "a deleted game's history must not surface: {page:?}"
    );

    // Insert an orphan via a dedicated connection (`PRAGMA foreign_keys` is per connection)
    // and assert both timeline queries exclude it.
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
    .bind(uuid::Uuid::now_v7().to_string())
    .bind(9_000_i64)
    .bind(10_000_i64)
    .bind(1_000_i64)
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

    let page = journey::timeline_page(db.pool(), None).await.expect("page");
    assert!(
        page.events.is_empty(),
        "an orphaned session is not a journey event: {page:?}"
    );
}

#[tokio::test]
async fn a_finished_falls_back_to_the_first_achieved_ending() {
    let (db, device) = fixture().await;
    let game = Game::new(&device, now(), "Manually finished", GameStatus::Playing).unwrap();
    upsert_game(db.pool(), &game).await.expect("insert");

    // A zero-time sitting is a game-page detail, not a journey event.
    let empty = PlaySession::new(
        &device,
        now(),
        game.meta.id,
        Timestamp::from_millis(1_000),
        Timestamp::from_millis(1_000),
        0,
        SessionSource::Desktop,
    )
    .unwrap();
    insert_session(db.pool(), &empty).await.expect("session");

    let mut ignored = Ending::new(&device, now(), game.meta.id, "Not reached").unwrap();
    ignored.achieved = false;
    upsert_ending(db.pool(), &ignored).await.expect("ending");
    let mut reached = Ending::new(&device, now(), game.meta.id, "First End").unwrap();
    reached.achieved = true;
    reached.achieved_at = Some(Timestamp::from_millis(7_000));
    upsert_ending(db.pool(), &reached).await.expect("ending");

    // `finished_at` stays unset on this fixture: rows built directly here
    // bypass `update_game`, which is what records the date on a local
    // completion. The fallback dates the finish by the first achieved
    // ending, so the day legitimately carries both events: the ending
    // reached, and the game it completed.
    let page = journey::timeline_page(db.pool(), None).await.expect("page");
    let kinds: Vec<&str> = page.events.iter().map(|e| e.kind.as_str()).collect();
    assert_eq!(kinds, vec!["ending", "finished"], "{page:?}");
    assert_eq!(page.events[0].ts, 7_000);
    assert_eq!(page.events[1].ts, 7_000);
    assert_eq!(page.events[1].game.meta.id, game.meta.id);
}

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "one fixture proves every figure of a single year agrees"
)]
async fn year_review_aggregates_one_local_year() {
    let (db, device) = fixture().await;
    let mut autumn = Game::new(&device, now(), "Autumn VN", GameStatus::Playing).unwrap();
    upsert_game(db.pool(), &autumn).await.expect("insert");

    // 2023-11-14 06:00Z and 06:30Z: one local calendar day and one local year
    // from every real timezone, which keeps the assertion deterministic
    // without pinning the test machine's timezone (same trick as
    // `playtime_buckets_one_local_day_together_and_reports_day_indexes`).
    let mut first = PlaySession::new(
        &device,
        now(),
        autumn.meta.id,
        Timestamp::from_millis(1_699_941_600_000),
        Timestamp::from_millis(1_699_941_600_000 + 40_000),
        40_000,
        SessionSource::Desktop,
    )
    .unwrap();
    first.afk_ms = 1_000;
    insert_session(db.pool(), &first).await.expect("session");
    let second = PlaySession::new(
        &device,
        now(),
        autumn.meta.id,
        Timestamp::from_millis(1_699_943_400_000),
        Timestamp::from_millis(1_699_943_400_000 + 20_000),
        20_000,
        SessionSource::Desktop,
    )
    .unwrap();
    insert_session(db.pool(), &second).await.expect("session");

    reading::upsert(
        db.pool(),
        &journal_entry(
            &device,
            autumn.meta.id,
            1_699_950_000_000,
            "At the gate",
            "Shiroha",
            "",
        ),
    )
    .await
    .expect("entry");

    let mut ending = Ending::new(&device, now(), autumn.meta.id, "True End").unwrap();
    ending.achieved = true;
    ending.achieved_at = Some(Timestamp::from_millis(1_700_049_600_000));
    upsert_ending(db.pool(), &ending).await.expect("ending");

    autumn.set_rating(Some(85)).unwrap();
    autumn.finished_at = Some(Timestamp::from_millis(1_700_049_600_000));
    autumn.meta = autumn.meta.touch(now());
    upsert_game(db.pool(), &autumn).await.expect("update");

    let summer = Game::new(&device, now(), "Summer VN", GameStatus::Playing).unwrap();
    upsert_game(db.pool(), &summer).await.expect("insert");
    let midyear = PlaySession::new(
        &device,
        now(),
        summer.meta.id,
        Timestamp::from_millis(1_717_456_000_000), // 2024-06-04T00:00Z: June in every timezone
        Timestamp::from_millis(1_717_456_000_000 + 5_000),
        5_000,
        SessionSource::Desktop,
    )
    .unwrap();
    insert_session(db.pool(), &midyear).await.expect("session");

    let review = journey::year_review(db.pool(), 2023).await.expect("review");
    assert_eq!(review.year, 2023);
    assert_eq!(review.total_ms, 60_000);
    assert_eq!(review.afk_ms, 1_000);
    assert_eq!(review.session_count, 2);
    assert_eq!(review.active_days, 1);
    assert_eq!(review.endings_reached, 1);
    assert_eq!(review.journal_entries, 1);
    assert_eq!(
        review.games_started, 1,
        "only Autumn VN's first session is 2023"
    );
    assert_eq!(review.games_finished, 1);
    assert_eq!(review.avg_rating_decivotes, Some(85));
    let longest = review.longest_session.expect("longest");
    assert_eq!(longest.counted_ms, 40_000);
    assert_eq!(longest.game_id, autumn.meta.id);
    let begun = review.first_game.expect("first game");
    assert_eq!(begun.game_id, autumn.meta.id);
    assert_eq!(begun.started_at, 1_699_941_600_000);
    assert_eq!(review.top_games.len(), 1);
    assert_eq!(review.top_games[0].game_id, autumn.meta.id);
    assert_eq!(review.top_games[0].counted_ms, 60_000);
    // November 14 06:00Z is November in every real timezone, so the month is
    // stable even though the local day index is not.
    assert_eq!(
        review.busiest_month,
        Some(MonthMoment {
            month: 11,
            counted_ms: 60_000
        })
    );
    let busiest_day = review.busiest_day.expect("busiest day");
    assert_eq!(busiest_day.counted_ms, 60_000);
    assert_eq!(review.average_session_ms, Some(30_000));
    assert_eq!(review.longest_streak_days, 1);
    assert!(review.top_route.is_none(), "no session names a route yet");
    assert_eq!(review.games_continued, 0, "Autumn VN starts in 2023");
    assert_eq!(review.weekday_ms.len(), 7);
    assert_eq!(review.weekday_ms.iter().sum::<i64>(), 60_000);
    assert_eq!(review.hour_ms.len(), 24);
    assert_eq!(review.hour_ms.iter().sum::<i64>(), 60_000);

    let review_2024 = journey::year_review(db.pool(), 2024).await.expect("review");
    assert_eq!(review_2024.total_ms, 5_000);
    assert_eq!(review_2024.session_count, 1);
    assert_eq!(review_2024.games_started, 1, "Summer VN starts in 2024");
    assert_eq!(review_2024.games_finished, 0);
    assert_eq!(review_2024.avg_rating_decivotes, None);
    assert_eq!(review_2024.top_games[0].game_id, summer.meta.id);

    let quiet = journey::year_review(db.pool(), 2025).await.expect("review");
    assert_eq!(quiet.total_ms, 0);
    assert_eq!(quiet.session_count, 0);
    assert_eq!(quiet.active_days, 0);
    assert_eq!(quiet.games_started, 0);
    assert_eq!(quiet.games_finished, 0);
    assert!(quiet.longest_session.is_none());
    assert!(quiet.first_game.is_none());
    assert!(quiet.top_games.is_empty());
}

#[tokio::test]
async fn finished_by_day_buckets_the_timeline_finish_rule_locally() {
    let (db, device) = fixture().await;

    // Two games finished at the same instant land in one bucket; the counts,
    // not the dates, are the assertion, so no timezone is pinned. Both dates
    // are the "one local calendar day from every real timezone" fixtures the
    // other journey tests use.
    let shared_day = 1_700_049_600_000;
    for (offset, title) in [(0_i64, "First"), (1, "Second")] {
        let mut game = Game::new(&device, now(), title, GameStatus::Completed).unwrap();
        game.finished_at = Some(Timestamp::from_millis(shared_day + offset));
        upsert_game(db.pool(), &game).await.expect("game");
    }

    // A game whose only finish evidence is its first achieved ending: the
    // fallback half of the same rule must bucket it too.
    let fallback = Game::new(&device, now(), "Fallback", GameStatus::Playing).unwrap();
    upsert_game(db.pool(), &fallback).await.expect("game");
    let mut ending = Ending::new(&device, now(), fallback.meta.id, "True End").unwrap();
    ending.achieved = true;
    ending.achieved_at = Some(Timestamp::from_millis(1_699_941_600_000));
    upsert_ending(db.pool(), &ending).await.expect("ending");

    // A deleted game's finish must not surface, per the join-everything rule.
    let mut ghost = Game::new(&device, now(), "Ghost", GameStatus::Completed).unwrap();
    ghost.finished_at = Some(Timestamp::from_millis(shared_day));
    upsert_game(db.pool(), &ghost).await.expect("game");
    ghost.meta.deleted = true;
    ghost.meta = ghost.meta.touch(now());
    upsert_game(db.pool(), &ghost).await.expect("delete");

    let buckets = journey::finished_by_day(db.pool()).await.expect("buckets");
    assert_eq!(buckets.len(), 2, "two distinct days, ghost excluded");
    assert!(
        buckets.iter().any(|&(_, count)| count == 2),
        "two live games share one day: {buckets:?}"
    );
    assert!(
        buckets.iter().any(|&(_, count)| count == 1),
        "the fallback-ending day is its own bucket: {buckets:?}"
    );
}

#[tokio::test]
async fn year_review_reads_routes_streaks_and_continued_games() {
    let (db, device) = fixture().await;
    let game = Game::new(&device, now(), "Long VN", GameStatus::Playing).unwrap();
    upsert_game(db.pool(), &game).await.expect("game");
    let route = Route::new(&device, now(), game.meta.id, "Shiroha").unwrap();
    upsert_route(db.pool(), &route).await.expect("route");
    let mut chapter = Chapter::new(&device, now(), game.meta.id, "Ch 1", 0).unwrap();
    chapter.route_id = Some(route.meta.id);
    upsert_chapter(db.pool(), &chapter).await.expect("chapter");

    // One 2023 session makes the game a 2023 start; the three 2024 sessions are
    // consecutive local days at noon UTC, so the streak is three in every real
    // timezone. Two sessions name the route directly, one only its chapter, and
    // both paths must roll up to the same route name.
    let sessions = [
        (
            1_702_641_600_000_i64,
            10 * 60_000_i64,
            None,
            Some(chapter.meta.id),
        ),
        (1_717_243_200_000, 20 * 60_000, Some(route.meta.id), None),
        (1_717_329_600_000, 30 * 60_000, None, Some(chapter.meta.id)),
        (1_717_416_000_000, 40 * 60_000, Some(route.meta.id), None),
    ];
    for (started_at, counted_ms, route_id, chapter_id) in sessions {
        let mut session = PlaySession::new(
            &device,
            now(),
            game.meta.id,
            Timestamp::from_millis(started_at),
            Timestamp::from_millis(started_at + counted_ms),
            counted_ms,
            SessionSource::Desktop,
        )
        .unwrap();
        session.route_id = route_id;
        session.chapter_id = chapter_id;
        insert_session(db.pool(), &session).await.expect("session");
    }

    let review = journey::year_review(db.pool(), 2024).await.expect("review");
    assert_eq!(review.total_ms, 90 * 60_000);
    assert_eq!(review.session_count, 3);
    assert_eq!(review.active_days, 3);
    assert_eq!(review.longest_streak_days, 3);
    assert_eq!(review.average_session_ms, Some(30 * 60_000));
    assert_eq!(review.busiest_month.map(|month| month.month), Some(6));
    assert_eq!(
        review.top_route,
        Some(crate::commands::journey::RouteMoment {
            name: "Shiroha".to_owned(),
            counted_ms: 90 * 60_000,
        })
    );
    assert_eq!(review.games_continued, 1, "the game started in 2023");
    assert_eq!(review.games_started, 0, "no game starts in 2024");
    assert_eq!(review.weekday_ms.iter().sum::<i64>(), 90 * 60_000);
    assert_eq!(review.hour_ms.iter().sum::<i64>(), 90 * 60_000);

    let first_year = journey::year_review(db.pool(), 2023).await.expect("review");
    assert_eq!(first_year.total_ms, 10 * 60_000);
    assert_eq!(first_year.longest_streak_days, 1);
    assert_eq!(first_year.games_continued, 0);
    assert_eq!(
        first_year.top_route.map(|route| route.name),
        Some("Shiroha".to_owned()),
        "a chapter's route is the same fact as a direct route"
    );
}

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "one fixture proves every all-time aggregate agrees"
)]
async fn library_wrapped_aggregates_every_year() {
    let (db, device) = fixture().await;
    let mut long = Game::new(&device, now(), "Long VN", GameStatus::Completed).unwrap();
    long.set_rating(Some(85)).unwrap();
    long.finished_at = Some(Timestamp::from_millis(1_696_000_000_000));
    upsert_game(db.pool(), &long).await.expect("long game");
    let short = Game::new(&device, now(), "Short VN", GameStatus::Playing).unwrap();
    upsert_game(db.pool(), &short).await.expect("short game");

    let route = Route::new(&device, now(), long.meta.id, "Shiroha").unwrap();
    upsert_route(db.pool(), &route).await.expect("route");
    let mut chapter = Chapter::new(&device, now(), long.meta.id, "Ch 1", 0).unwrap();
    chapter.route_id = Some(route.meta.id);
    upsert_chapter(db.pool(), &chapter).await.expect("chapter");

    // Three consecutive 2023 days for the long game, then one 2024 day for the
    // short one. Noon UTC keeps each session on the same local day in every real
    // timezone, so the streak and the day buckets are deterministic.
    let sessions = [
        (
            1_685_620_800_000_i64,
            10 * 60_000_i64,
            long.meta.id,
            None,
            Some(chapter.meta.id),
        ),
        (
            1_685_707_200_000,
            20 * 60_000,
            long.meta.id,
            Some(route.meta.id),
            None,
        ),
        (
            1_685_793_600_000,
            30 * 60_000,
            long.meta.id,
            None,
            Some(chapter.meta.id),
        ),
        (1_717_243_200_000, 40 * 60_000, short.meta.id, None, None),
    ];
    for (started_at, counted_ms, game_id, route_id, chapter_id) in sessions {
        let mut session = PlaySession::new(
            &device,
            now(),
            game_id,
            Timestamp::from_millis(started_at),
            Timestamp::from_millis(started_at + counted_ms),
            counted_ms,
            SessionSource::Desktop,
        )
        .unwrap();
        session.route_id = route_id;
        session.chapter_id = chapter_id;
        insert_session(db.pool(), &session).await.expect("session");
    }

    let mut ending = Ending::new(&device, now(), long.meta.id, "True End").unwrap();
    ending.achieved = true;
    ending.achieved_at = Some(Timestamp::from_millis(1_696_000_000_000));
    upsert_ending(db.pool(), &ending).await.expect("ending");
    reading::upsert(
        db.pool(),
        &journal_entry(
            &device,
            long.meta.id,
            1_696_000_000_000,
            "At the gate",
            "Shiroha",
            "",
        ),
    )
    .await
    .expect("entry");

    let wrapped = wrapped::library_wrapped(db.pool()).await.expect("wrapped");
    assert_eq!(wrapped.total_ms, 100 * 60_000);
    assert_eq!(wrapped.session_count, 4);
    assert_eq!(wrapped.active_days, 4);
    assert_eq!(wrapped.games_tracked, 2);
    assert_eq!(wrapped.games_in_library, 2);
    assert_eq!(wrapped.games_finished, 1);
    assert_eq!(wrapped.endings_reached, 1);
    assert_eq!(wrapped.journal_entries, 1);
    assert_eq!(wrapped.average_rating_decivotes, Some(85));
    assert_eq!(wrapped.average_session_ms, Some(25 * 60_000));
    assert_eq!(wrapped.longest_streak_days, 3);
    assert_eq!(
        wrapped.longest_session.map(|moment| moment.counted_ms),
        Some(40 * 60_000)
    );
    assert_eq!(
        wrapped.first_game.map(|moment| moment.game_id),
        Some(long.meta.id)
    );
    assert_eq!(wrapped.first_year, Some(2023));
    assert_eq!(wrapped.top_games.len(), 2);
    assert_eq!(wrapped.top_games[0].game_id, long.meta.id);
    assert_eq!(wrapped.top_games[0].counted_ms, 60 * 60_000);
    assert_eq!(wrapped.top_games[1].game_id, short.meta.id);
    assert_eq!(
        wrapped.top_route.map(|route| route.name),
        Some("Shiroha".to_owned())
    );
    assert_eq!(wrapped.busiest_month.map(|month| month.month), Some(6));
    assert_eq!(
        wrapped.busiest_day.map(|day| day.counted_ms),
        Some(40 * 60_000)
    );
    assert_eq!(wrapped.most_read_year.map(|year| year.year), Some(2023));
    assert_eq!(wrapped.weekday_ms.len(), 7);
    assert_eq!(wrapped.weekday_ms.iter().sum::<i64>(), 100 * 60_000);
    assert_eq!(wrapped.hour_ms.len(), 24);
    assert_eq!(wrapped.hour_ms.iter().sum::<i64>(), 100 * 60_000);
}

/// A timestamp whose *local* date is one year before today, at noon.
///
/// Built by `SQLite` rather than by arithmetic here: the query matches on the
/// local month and day, and only the same calendar authority can produce a value
/// that lands on it.
async fn one_year_ago_noon(pool: &sqlx::SqlitePool) -> Option<i64> {
    let (today, target): (String, String) = sqlx::query_as(
        "SELECT strftime('%m-%d', 'now', 'localtime'), \
                strftime('%m-%d', 'now', 'localtime', '-1 year')",
    )
    .fetch_one(pool)
    .await
    .expect("calendar");
    // 29 February has no anniversary in a common year; SQLite normalizes it to
    // 1 March, so there is no "same day last year" to test on that date.
    if today != target {
        return None;
    }
    let (ms,): (i64,) = sqlx::query_as(
        "SELECT CAST(strftime('%s', 'now', 'localtime', '-1 year', 'start of day', \
                               '+12 hours', 'utc') AS INTEGER) * 1000",
    )
    .fetch_one(pool)
    .await
    .expect("timestamp");
    Some(ms)
}

#[tokio::test]
async fn on_this_day_resurfaces_last_years_checkpoint_and_ending() {
    let (db, device) = fixture().await;
    let Some(anniversary) = one_year_ago_noon(db.pool()).await else {
        return;
    };
    let game = Game::new(&device, now(), "Anniversary", GameStatus::Playing).unwrap();
    upsert_game(db.pool(), &game).await.expect("insert");

    reading::upsert(
        db.pool(),
        &journal_entry(
            &device,
            game.meta.id,
            anniversary,
            "A year ago today",
            "Mio",
            "",
        ),
    )
    .await
    .expect("entry");
    let mut ending = Ending::new(&device, now(), game.meta.id, "True End").unwrap();
    ending.achieved = true;
    ending.achieved_at = Some(Timestamp::from_millis(anniversary + 1_000));
    upsert_ending(db.pool(), &ending).await.expect("ending");

    // Today's own checkpoint is not a memory: the card must not resurface
    // something written this morning as a year-old anniversary. The fixture's
    // `now()` is a fixed 2023 instant, so this row needs the real clock to land
    // on today's date at all.
    let real_now = Timestamp::now().expect("clock").as_millis();
    reading::upsert(
        db.pool(),
        &journal_entry(&device, game.meta.id, real_now, "Written today", "", ""),
    )
    .await
    .expect("today entry");

    let memories = journey::on_this_day(db.pool()).await.expect("memories");
    assert_eq!(
        memories.len(),
        2,
        "one journal entry and one ending: {memories:?}"
    );
    assert!(memories.iter().all(|memory| memory.years_ago == 1));
    assert_eq!(memories[0].kind, "ending", "newest first");
    assert_eq!(memories[0].ending_name.as_deref(), Some("True End"));
    assert_eq!(memories[1].kind, "journal");
    assert_eq!(memories[1].note.as_deref(), Some("A year ago today"));
    assert_eq!(memories[1].game.title, "Anniversary");
    assert!(
        memories
            .iter()
            .all(|memory| memory.note.as_deref() != Some("Written today")),
        "today's rows are excluded"
    );
}

#[tokio::test]
async fn on_this_day_hides_a_deleted_games_memories() {
    let (db, device) = fixture().await;
    let Some(anniversary) = one_year_ago_noon(db.pool()).await else {
        return;
    };
    let mut game = Game::new(&device, now(), "Ghosted", GameStatus::Playing).unwrap();
    upsert_game(db.pool(), &game).await.expect("insert");
    reading::upsert(
        db.pool(),
        &journal_entry(&device, game.meta.id, anniversary, "gone", "", ""),
    )
    .await
    .expect("entry");

    game.meta.deleted = true;
    game.meta = game.meta.touch(now());
    upsert_game(db.pool(), &game).await.expect("delete");

    let memories = journey::on_this_day(db.pool()).await.expect("memories");
    assert!(
        memories.is_empty(),
        "a deleted game's memories must not surface: {memories:?}"
    );
}

#[tokio::test]
async fn the_journal_export_reads_live_entries_in_reading_order() {
    let (db, device) = fixture().await;
    let game = Game::new(&device, now(), "Exportable", GameStatus::Playing).unwrap();
    upsert_game(db.pool(), &game).await.expect("insert");
    for (created, note) in [(3_000_i64, "second"), (1_000, "first"), (2_000, "middle")] {
        reading::upsert(
            db.pool(),
            &journal_entry(&device, game.meta.id, created, note, "", ""),
        )
        .await
        .expect("entry");
    }
    let deleted = journal_entry(&device, game.meta.id, 4_000, "deleted", "", "");
    reading::upsert(db.pool(), &deleted).await.expect("entry");
    reading::delete(
        db.pool(),
        &device,
        game.meta.id,
        deleted.meta.id,
        deleted.meta.updated_at.as_millis(),
    )
    .await
    .expect("delete");

    let rows = reading::list_for_export(db.pool(), game.meta.id)
        .await
        .expect("rows");
    let notes: Vec<&str> = rows.iter().map(|row| row.entry.note.as_str()).collect();
    assert_eq!(
        notes,
        ["first", "middle", "second"],
        "oldest first, tombstones gone"
    );
    // Every row carries a local calendar date; the export renders these strings
    // rather than formatting timestamps itself.
    assert!(
        rows.iter()
            .all(|row| row.date.len() == 10 && row.date.contains('-'))
    );
    assert!(
        rows.iter()
            .all(|row| row.time.len() == 5 && row.time.contains(':'))
    );
}

#[tokio::test]
async fn a_pinned_photo_survives_the_round_trip_and_a_photo_only_entry_saves() {
    let (db, device) = fixture().await;
    let game = Game::new(&device, now(), "Photo diary", GameStatus::Playing).unwrap();
    upsert_game(db.pool(), &game).await.expect("insert");

    let mut entry = journal_entry(&device, game.meta.id, 5_000, "", "", "");
    entry.photo_image_id = "sf83040".to_owned();
    reading::upsert(db.pool(), &entry)
        .await
        .expect("photo only");

    let loaded = reading::list_all(db.pool(), game.meta.id)
        .await
        .expect("read");
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].photo_image_id, "sf83040");
    assert!(loaded[0].note.is_empty(), "a photo needs no caption");

    // A later edit can drop the photo again; the update path must write the
    // column, not only the insert path.
    let mut edited = loaded[0].clone();
    edited.photo_image_id.clear();
    edited.note = "caption instead".to_owned();
    edited.meta = edited.meta.touch(now());
    reading::upsert(db.pool(), &edited).await.expect("edit");
    let reloaded = reading::list_all(db.pool(), game.meta.id)
        .await
        .expect("read");
    assert_eq!(reloaded[0].photo_image_id, "");
    assert_eq!(reloaded[0].note, "caption instead");
}

#[tokio::test]
async fn journal_search_matches_note_route_and_chapter_newest_first() {
    let (db, device) = fixture().await;
    let game = Game::new(&device, now(), "Searchable", GameStatus::Playing).unwrap();
    upsert_game(db.pool(), &game).await.expect("insert");
    for (created, note, route, chapter) in [
        (1_000_i64, "The fireworks scene", "Mio", ""),
        (2_000, "A quiet morning", "Shizuku", "August 7"),
        (3_000, "Fireworks again, one year later", "", ""),
    ] {
        reading::upsert(
            db.pool(),
            &journal_entry(&device, game.meta.id, created, note, route, chapter),
        )
        .await
        .expect("entry");
    }

    // ASCII case folds; the note is the primary target.
    let (hits, more) = reading::search(db.pool(), "FIREWORKS", None, None)
        .await
        .expect("search");
    assert!(!more);
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].created_at.as_millis(), 3_000, "newest first");
    assert_eq!(hits[1].created_at.as_millis(), 1_000);

    // Route and chapter snapshots are searchable too.
    let (hits, _) = reading::search(db.pool(), "shizuku", None, None)
        .await
        .expect("route");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].note, "A quiet morning");
    let (hits, _) = reading::search(db.pool(), "august", None, None)
        .await
        .expect("chapter");
    assert_eq!(hits.len(), 1);

    // CJK matches exactly, with no case folding to get wrong.
    reading::upsert(
        db.pool(),
        &journal_entry(&device, game.meta.id, 4_000, "花火の夜", "", ""),
    )
    .await
    .expect("entry");
    let (hits, _) = reading::search(db.pool(), "花火", None, None)
        .await
        .expect("cjk");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].note, "花火の夜");
}

#[tokio::test]
async fn journal_search_treats_wildcards_as_literal_text() {
    let (db, device) = fixture().await;
    let game = Game::new(&device, now(), "Literal", GameStatus::Playing).unwrap();
    upsert_game(db.pool(), &game).await.expect("insert");
    reading::upsert(
        db.pool(),
        &journal_entry(&device, game.meta.id, 1_000, "100% completed", "", ""),
    )
    .await
    .expect("entry");
    reading::upsert(
        db.pool(),
        &journal_entry(&device, game.meta.id, 2_000, "1000 lines read", "", ""),
    )
    .await
    .expect("entry");

    // An unescaped `%` would match both rows; the reader typed a percent sign.
    let (hits, _) = reading::search(db.pool(), "100%", None, None)
        .await
        .expect("search");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].note, "100% completed");
    // `_` is a single-character wildcard unescaped; here it is an underscore.
    let (hits, _) = reading::search(db.pool(), "0_", None, None)
        .await
        .expect("search");
    assert!(hits.is_empty());
}

#[tokio::test]
async fn journal_search_hides_deleted_rows_and_filters_by_game() {
    let (db, device) = fixture().await;
    let mut game = Game::new(&device, now(), "Live", GameStatus::Playing).unwrap();
    upsert_game(db.pool(), &game).await.expect("insert");
    let other = Game::new(&device, now(), "Other", GameStatus::Playing).unwrap();
    upsert_game(db.pool(), &other).await.expect("insert");
    reading::upsert(
        db.pool(),
        &journal_entry(&device, game.meta.id, 1_000, "a shared word", "", ""),
    )
    .await
    .expect("entry");
    reading::upsert(
        db.pool(),
        &journal_entry(&device, other.meta.id, 2_000, "a shared word", "", ""),
    )
    .await
    .expect("entry");
    let deleted = journal_entry(&device, game.meta.id, 3_000, "a shared word", "", "");
    reading::upsert(db.pool(), &deleted).await.expect("entry");
    reading::delete(
        db.pool(),
        &device,
        game.meta.id,
        deleted.meta.id,
        deleted.meta.updated_at.as_millis(),
    )
    .await
    .expect("delete");

    let (hits, _) = reading::search(db.pool(), "shared", None, None)
        .await
        .expect("search");
    assert_eq!(hits.len(), 2, "the tombstone must not surface");

    let (hits, _) = reading::search(db.pool(), "shared", Some(other.meta.id), None)
        .await
        .expect("filtered");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].game_id, other.meta.id);

    // A deleted game's checkpoints stay hidden even though the rows exist.
    game.meta.deleted = true;
    game.meta = game.meta.touch(now());
    upsert_game(db.pool(), &game).await.expect("delete");
    let (hits, _) = reading::search(db.pool(), "shared", None, None)
        .await
        .expect("search");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].game_id, other.meta.id);
}

#[tokio::test]
async fn journal_search_pages_with_a_cursor() {
    let (db, device) = fixture().await;
    let game = Game::new(&device, now(), "Paged", GameStatus::Playing).unwrap();
    upsert_game(db.pool(), &game).await.expect("insert");
    for index in 0..51_i64 {
        reading::upsert(
            db.pool(),
            &journal_entry(
                &device,
                game.meta.id,
                1_000 + index,
                &format!("entry number {index}"),
                "",
                "",
            ),
        )
        .await
        .expect("entry");
    }

    let (first, more) = reading::search(db.pool(), "entry", None, None)
        .await
        .expect("first page");
    assert!(more, "51 matches over a 50-row page");
    assert_eq!(first.len(), 50);
    assert_eq!(first[0].created_at.as_millis(), 1_050, "newest first");

    let last = first.last().expect("last row");
    let (second, more) = reading::search(
        db.pool(),
        "entry",
        None,
        Some((last.created_at.as_millis(), &last.meta.id.to_string())),
    )
    .await
    .expect("second page");
    assert!(!more);
    assert_eq!(second.len(), 1);
    assert_eq!(second[0].created_at.as_millis(), 1_000);
}

#[tokio::test]
async fn window_launch_and_ui_prefs_round_trip() {
    let (db, _device) = fixture().await;

    let initial_win = load_window_launch_prefs(db.pool())
        .await
        .expect("default window prefs");
    assert_eq!(
        initial_win,
        vn_core::window::WindowLaunchPrefs {
            mode: vn_core::window::WindowLaunchMode::Centered,
            x: None,
            y: None,
        }
    );

    let custom_win = vn_core::window::WindowLaunchPrefs {
        mode: vn_core::window::WindowLaunchMode::Customized,
        x: Some(240),
        y: Some(120),
    };
    save_window_launch_prefs(db.pool(), &custom_win)
        .await
        .expect("save window prefs");
    assert_eq!(
        load_window_launch_prefs(db.pool())
            .await
            .expect("reload window prefs"),
        custom_win
    );

    let initial_ui = load_ui_prefs(db.pool()).await.expect("default ui prefs");
    assert!(!initial_ui.sidebar_compact);
    assert!(!initial_ui.library_remember_browse);
    assert_eq!(
        initial_ui.library_default_sort,
        vn_core::library::SortKey::Recent
    );

    let custom_ui = vn_core::ui_prefs::UiPrefs {
        sidebar_compact: true,
        library_remember_browse: true,
        library_default_sort: vn_core::library::SortKey::Playtime,
        library_saved_browse: Some(vn_core::ui_prefs::SavedLibraryBrowse {
            statuses: vec![GameStatus::Playing, GameStatus::Wishlist],
            rated_only: true,
            view: vn_core::ui_prefs::LibraryViewMode::Shelf,
            sort: vn_core::library::SortKey::Rating,
        }),
    };
    save_ui_prefs(db.pool(), &custom_ui)
        .await
        .expect("save ui prefs");
    assert_eq!(
        load_ui_prefs(db.pool()).await.expect("reload ui prefs"),
        custom_ui
    );
}
