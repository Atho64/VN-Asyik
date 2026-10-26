//! `search_library`: filtering, ordering, paging and tie detection in SQL.
//!
//! These run against a real migrated database, because the part that was
//! rewritten is the SQL: the old version filtered and sorted decoded JSON cards
//! in Rust, so a shape mistake here (a lost spoiler filter, a status that no
//! longer narrows, an offset that starts at the wrong row) would only show up in
//! a live chat answer.

use serde_json::Value;
use vn_core::chat::{ChatOptions, Spoilers};
use vn_tracker_lib::ai::conversation::tools::{self, ToolContext};
use vn_tracker_lib::db::Db;

fn game_id(n: u8) -> String {
    format!("01a151db-5555-7000-8000-0000000000{n:02}")
}

/// Seed a library: one tag blob per game, a session so playtime is real, and a
/// rating on the games that should tie.
async fn seed(db: &Db) -> Vec<String> {
    let mut ids = Vec::new();
    // (title, status, rating decivotes, tag names, session minutes)
    let rows: [(&str, &str, Option<i64>, &str, i64); 4] = [
        ("CLANNAD", "completed", Some(90), "Nakige|School", 120),
        ("Kanade", "playing", Some(90), "Nakige", 30),
        ("Saku Saku", "wishlist", None, "Comedy", 0),
        ("Planetarian", "completed", Some(70), "Kinetic", 45),
    ];
    for (index, row) in rows.iter().enumerate() {
        let id = game_id(u8::try_from(index).expect("index fits"));
        let (title, status, rating, tags, minutes) = *row;
        sqlx::query(
            "INSERT INTO games (id, title, status, rating, updated_at, device_id) \
             VALUES (?,?,?,?,1,'dev')",
        )
        .bind(&id)
        .bind(title)
        .bind(status)
        .bind(rating)
        .execute(db.pool())
        .await
        .expect("insert game");
        if minutes > 0 {
            sqlx::query(
                "INSERT INTO play_sessions (id, game_id, started_at, ended_at, counted_ms, afk_ms, source, updated_at, device_id) \
                 VALUES (?,?,1000,2000,?,0,'desktop',1,'dev')",
            )
            // Ids are uuids in this codebase; the seed has to look like one too.
            .bind(uuid::Uuid::new_v4().to_string())
            .bind(&id)
            .bind(minutes * 60_000)
            .execute(db.pool())
            .await
            .expect("insert session");
        }
        // The tag blob is what the search reads, so it is stored the way the VNDB
        // cache stores it.
        // `title` predates the tag column and has no default, so it is written
        // explicitly the way the VNDB importer writes it.
        sqlx::query(
            "INSERT INTO vndb_details (vn_id, title, tags_json, fetched_at) VALUES (?,?,?,1)",
        )
        .bind(format!("v{index}"))
        .bind(title)
        .bind(tag_blob(tags, index))
        .execute(db.pool())
        .await
        .expect("insert details");
        sqlx::query("UPDATE games SET vndb_id = ? WHERE id = ?")
            .bind(format!("v{index}"))
            .bind(&id)
            .execute(db.pool())
            .await
            .expect("link vndb id");
        ids.push(id);
    }
    ids
}

/// Build a VNDB-shaped tag blob. The first tag of each game except CLANNAD is
/// spoiler-free; CLANNAD's second tag is spoiler-rated, which is what the
/// spoiler test reads.
fn tag_blob(tags: &str, index: usize) -> String {
    let mut blob = String::from("[");
    for (position, name) in tags.split('|').enumerate() {
        if position > 0 {
            blob.push(',');
        }
        let spoiler = if index == 0 && position == 1 {
            2.0
        } else {
            0.0
        };
        blob.push_str(&format!(
            r#"{{"name":"{name}","rating":2.0,"spoiler":{spoiler}}}"#
        ));
    }
    blob.push(']');
    blob
}

fn options(spoilers: Spoilers) -> ChatOptions {
    ChatOptions {
        language: "en".into(),
        spoilers,
        ..ChatOptions::default()
    }
}

async fn search(db: &Db, opts: ChatOptions, raw: &str) -> Value {
    let client = reqwest::Client::new();
    // `search_library` reads the pool only; the network client, the VNDB client
    // and the hook ring are for other tools, so they are real but unused.
    let vndb = vn_tracker_lib::vndb::VndbClient::with_base("https://api.vndb.org/kana")
        .expect("vndb client");
    let hook = std::sync::Arc::new(vn_tracker_lib::tracking::hook::HookHost::default());
    let ctx = ToolContext {
        pool: db.pool(),
        vndb: &vndb,
        client: &client,
        options: &opts,
        hook: &hook,
    };
    let mut sources = Vec::new();
    tools::run(&ctx, "search_library", raw, &mut sources)
        .await
        .expect("tool call")
}

fn titles(result: &Value) -> Vec<&str> {
    result["data"]["games"]
        .as_array()
        .expect("games")
        .iter()
        .map(|game| game["title"].as_str().expect("title"))
        .collect()
}

#[tokio::test]
async fn an_empty_query_lists_the_whole_live_library_by_title() {
    let db = Db::open_memory().await.expect("db");
    seed(&db).await;
    let result = search(&db, options(Spoilers::None), r#"{"query":""}"#).await;
    assert_eq!(result["data"]["total"], 4);
    assert_eq!(
        titles(&result),
        vec!["CLANNAD", "Kanade", "Planetarian", "Saku Saku"]
    );
}

#[tokio::test]
async fn a_query_matches_titles_original_titles_ids_and_tags() {
    let db = Db::open_memory().await.expect("db");
    seed(&db).await;

    // Title, case-insensitively.
    let result = search(&db, options(Spoilers::None), r#"{"query":"clannad"}"#).await;
    assert_eq!(titles(&result), vec!["CLANNAD"]);

    // A tag name only exists inside the stored blob.
    let result = search(&db, options(Spoilers::None), r#"{"query":"nakige"}"#).await;
    assert_eq!(titles(&result), vec!["CLANNAD", "Kanade"]);

    // VNDB id.
    let result = search(&db, options(Spoilers::None), r#"{"query":"v3"}"#).await;
    assert_eq!(titles(&result), vec!["Planetarian"]);
}

#[tokio::test]
async fn status_narrows_and_a_hidden_spoiler_tag_is_not_returned() {
    let db = Db::open_memory().await.expect("db");
    seed(&db).await;

    let result = search(
        &db,
        options(Spoilers::None),
        r#"{"query":"","status":"completed"}"#,
    )
    .await;
    assert_eq!(titles(&result), vec!["CLANNAD", "Planetarian"]);

    // CLANNAD carries a spoiler-rated tag; without full spoilers it is withheld.
    let result = search(&db, options(Spoilers::None), r#"{"query":"clannad"}"#).await;
    let tags = result["data"]["games"][0]["tags"].as_array().expect("tags");
    assert_eq!(tags.len(), 1);
    assert_eq!(tags[0]["name"], "Nakige");

    let result = search(&db, options(Spoilers::All), r#"{"query":"clannad"}"#).await;
    let tags = result["data"]["games"][0]["tags"].as_array().expect("tags");
    assert_eq!(tags.len(), 2);
}

#[tokio::test]
async fn value_sorts_report_how_many_share_the_top_value() {
    let db = Db::open_memory().await.expect("db");
    seed(&db).await;

    let result = search(
        &db,
        options(Spoilers::None),
        r#"{"query":"","sort":"rating"}"#,
    )
    .await;
    // Two games are rated 9.0, so the pair is tied rather than ranked.
    assert_eq!(result["data"]["tied_at_leading_value"], 2);
    assert_eq!(titles(&result)[0], "CLANNAD");

    let result = search(
        &db,
        options(Spoilers::None),
        r#"{"query":"","sort":"playtime"}"#,
    )
    .await;
    assert_eq!(titles(&result)[0], "CLANNAD");
    // A unique top playtime is not a tie.
    assert_eq!(result["data"]["tied_at_leading_value"], Value::Null);
}
