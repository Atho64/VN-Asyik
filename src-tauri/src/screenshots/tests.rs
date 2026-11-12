use super::*;

fn frame(width: u32, height: u32) -> Captured {
    Captured {
        width,
        height,
        rgba: vec![120; (width as usize) * (height as usize) * 4],
        source: "windows_window",
    }
}

#[test]
fn a_large_capture_is_bounded_to_the_stored_edge() {
    // A 4K window is 3840 px wide; storing it whole would cost several MB per
    // shot for an image the library shows at a few hundred.
    let jpeg = encode_jpeg(&frame(3840, 2160)).expect("encode");
    let decoded = image::load_from_memory(&jpeg).expect("valid jpeg");
    assert!(decoded.width() <= MAX_EDGE);
    assert!(decoded.height() <= MAX_EDGE);
}

#[test]
fn a_small_capture_is_not_upscaled() {
    let jpeg = encode_jpeg(&frame(320, 180)).expect("encode");
    let decoded = image::load_from_memory(&jpeg).expect("valid jpeg");
    assert_eq!((decoded.width(), decoded.height()), (320, 180));
}

#[test]
fn a_buffer_that_disagrees_with_its_dimensions_is_refused() {
    let mut broken = frame(10, 10);
    broken.rgba.truncate(40);
    assert!(matches!(
        encode_jpeg(&broken),
        Err(ScreenshotError::Encode(_))
    ));
}

#[tokio::test]
async fn a_capture_round_trips_through_the_store() {
    let root = std::env::temp_dir().join(format!("vnt-shots-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let store = ScreenshotStore::new(&root);

    let relative = store
        .write("game-1", "shot-1", b"jpeg-bytes")
        .await
        .expect("write");
    assert_eq!(relative, "screenshots/game-1/shot-1.jpg");
    assert_eq!(store.read(&relative).await.expect("read"), b"jpeg-bytes");

    store.remove(&relative).await;
    assert!(store.read(&relative).await.is_err());
    // Removing a file that is already gone is not an error.
    store.remove(&relative).await;

    let _ = std::fs::remove_dir_all(&root);
}

async fn seeded(tag: &str) -> (crate::db::Db, String) {
    let db = crate::db::Db::open_memory().await.expect("db");
    let game_id = format!("0192f0aa-0000-7000-8000-{:012}", tag.len());
    sqlx::query(
        "INSERT INTO games (id, title, status, updated_at, device_id) VALUES (?, 'Saku Saku', 'playing', 1, 'dev')",
    )
    .bind(&game_id)
    .execute(db.pool())
    .await
    .expect("insert game");
    (db, game_id)
}

fn row(id: &str, game_id: &str, at: i64) -> ScreenshotRow {
    ScreenshotRow {
        id: id.to_owned(),
        game_id: game_id.to_owned(),
        relative_path: ScreenshotStore::relative(game_id, id),
        width: 1280,
        height: 720,
        captured_at: at,
        source: "windows_window".to_owned(),
        caption: String::new(),
    }
}

#[tokio::test]
async fn captures_list_newest_first_and_skip_deleted_ones() {
    let (db, game_id) = seeded("list").await;
    insert(db.pool(), &row("a", &game_id, 100))
        .await
        .expect("a");
    insert(db.pool(), &row("b", &game_id, 300))
        .await
        .expect("b");
    insert(db.pool(), &row("c", &game_id, 200))
        .await
        .expect("c");

    let ids: Vec<String> = list(db.pool(), &game_id)
        .await
        .expect("list")
        .into_iter()
        .map(|shot| shot.id)
        .collect();
    assert_eq!(ids, ["b", "c", "a"]);

    soft_delete(db.pool(), "b").await.expect("delete");
    let ids: Vec<String> = list(db.pool(), &game_id)
        .await
        .expect("list")
        .into_iter()
        .map(|shot| shot.id)
        .collect();
    assert_eq!(ids, ["c", "a"]);
}

#[tokio::test]
async fn a_capture_of_a_deleted_game_is_not_listed() {
    // AGENTS.md §5: every read of a child table joins its live parent. A shot
    // whose game is gone must not appear in a list or be fetched by id.
    let (db, game_id) = seeded("gone").await;
    insert(db.pool(), &row("a", &game_id, 100))
        .await
        .expect("a");
    sqlx::query("UPDATE games SET deleted = 1 WHERE id = ?1")
        .bind(&game_id)
        .execute(db.pool())
        .await
        .expect("soft delete game");

    assert!(list(db.pool(), &game_id).await.expect("list").is_empty());
    assert!(get(db.pool(), "a").await.expect("get").is_none());
}

#[tokio::test]
async fn a_capture_is_readable_by_id_until_it_is_deleted() {
    let (db, game_id) = seeded("get").await;
    insert(db.pool(), &row("a", &game_id, 100))
        .await
        .expect("a");
    assert!(get(db.pool(), "a").await.expect("get").is_some());
    soft_delete(db.pool(), "a").await.expect("delete");
    assert!(get(db.pool(), "a").await.expect("get").is_none());
}

#[tokio::test]
async fn the_cross_game_library_is_newest_first_and_skips_deleted() {
    let (db, game_id) = seeded("feed").await;
    let other = "0192f0aa-0000-7000-8000-000000000099";
    sqlx::query(
        "INSERT INTO games (id, title, status, updated_at, device_id) VALUES (?, 'CLANNAD', 'playing', 1, 'dev')",
    )
    .bind(other)
    .execute(db.pool())
    .await
    .expect("insert second game");

    insert(db.pool(), &row("a", &game_id, 100))
        .await
        .expect("a");
    insert(db.pool(), &row("b", other, 300)).await.expect("b");
    insert(db.pool(), &row("c", &game_id, 200))
        .await
        .expect("c");

    let feed = list_all(db.pool(), 50).await.expect("feed");
    let ids: Vec<&str> = feed.iter().map(|shot| shot.id.as_str()).collect();
    assert_eq!(ids, ["b", "c", "a"]);
    // The game title travels with the shot, so the library can group without a
    // second query per game.
    assert_eq!(feed[0].game_title, "CLANNAD");

    soft_delete(db.pool(), "b").await.expect("delete");
    sqlx::query("UPDATE games SET deleted = 1 WHERE id = ?1")
        .bind(&game_id)
        .execute(db.pool())
        .await
        .expect("delete game");
    assert!(list_all(db.pool(), 50).await.expect("feed").is_empty());
}
