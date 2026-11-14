//! Queries for "For you" recommendations and `ai_taste_profile`.
//! Both consumers share this view of live games and cached VNDB credits.

use sqlx::SqlitePool;
use vn_core::taste::{NamedEntity, ProfiledGame, StaffCredit, TagWeight};

use crate::db::pool::DbError;
use crate::vndb::dto;

/// Live games with cached VNDB tags and credits; malformed or missing JSON blobs degrade to empty.
pub async fn seeded_games(pool: &SqlitePool) -> Result<Vec<ProfiledGame>, DbError> {
    use sqlx::Row as _;
    let rows = sqlx::query(
        "SELECT g.id AS id, g.title AS title, g.status AS status, g.rating AS rating, \
                g.vndb_id AS vndb_id, d.tags_json AS tags_json, \
                d.developers_json AS developers_json, d.staff_json AS staff_json, \
                d.va_json AS va_json \
         FROM games g LEFT JOIN vndb_details d ON d.vn_id = g.vndb_id \
         WHERE g.deleted = 0 ORDER BY g.title",
    )
    .fetch_all(pool)
    .await?;

    let games: Vec<ProfiledGame> = rows
        .into_iter()
        .map(|row| {
            let tags: Vec<TagWeight> = decode(&row, "tags_json");
            let developers: Vec<NamedEntity> = decode::<dto::Producer>(&row, "developers_json")
                .into_iter()
                .filter_map(|producer| {
                    Some(NamedEntity {
                        id: producer.id?,
                        name: producer.name.unwrap_or_default(),
                    })
                })
                .collect();
            let staff: Vec<StaffCredit> = decode::<dto::StaffMember>(&row, "staff_json")
                .into_iter()
                .filter_map(|member| {
                    Some(StaffCredit {
                        id: member.id?,
                        name: member.name.unwrap_or_default(),
                        role: member.role.unwrap_or_default(),
                    })
                })
                .collect();
            let voices: Vec<NamedEntity> = decode::<dto::StaffCredit>(&row, "va_json")
                .into_iter()
                .filter_map(|actor| {
                    Some(NamedEntity {
                        id: actor.id?,
                        name: actor.name.unwrap_or_default(),
                    })
                })
                .collect();
            ProfiledGame {
                id: row.get("id"),
                title: row.get("title"),
                status: row.get("status"),
                rating: row
                    .get::<Option<i64>, _>("rating")
                    .and_then(|value| u8::try_from(value).ok()),
                tags,
                work_id: row.get::<Option<String>, _>("vndb_id"),
                developers,
                staff,
                voices,
            }
        })
        .collect();
    Ok(games)
}

/// Decodes a JSON column; missing/legacy `NULL` or malformed values degrade to empty (AGENTS.md §6).
fn decode<T: serde::de::DeserializeOwned>(row: &sqlx::sqlite::SqliteRow, column: &str) -> Vec<T> {
    use sqlx::Row as _;
    row.get::<Option<String>, _>(column)
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;

    async fn game_with_details(
        pool: &sqlx::SqlitePool,
        vndb_id: &str,
        tags_json: &str,
        developers_json: &str,
        staff_json: Option<&str>,
        va_json: Option<&str>,
    ) {
        sqlx::query(
            "INSERT INTO games (id, title, status, updated_at, device_id, deleted, vndb_id) \
             VALUES ('g1', 'Ever17', 'completed', 1, 'd1', 0, ?)",
        )
        .bind(vndb_id)
        .execute(pool)
        .await
        .expect("game");
        sqlx::query("DELETE FROM vndb_details WHERE vn_id = ?")
            .bind(vndb_id)
            .execute(pool)
            .await
            .expect("clean");
        sqlx::query(
            "INSERT INTO vndb_details (vn_id, title, tags_json, developers_json, staff_json, va_json, fetched_at) \
             VALUES (?, 'Ever17', ?, ?, ?, ?, 1)",
        )
        .bind(vndb_id)
        .bind(tags_json)
        .bind(developers_json)
        .bind(staff_json)
        .bind(va_json)
        .execute(pool)
        .await
        .expect("details");
    }

    #[tokio::test]
    async fn a_row_from_before_the_staff_columns_decodes_to_empty() {
        // The rule the detail read path lives by: a cached row can predate its
        // own field list. NULL staff_json/va_json must not fail the profile.
        let db = Db::open_memory().await.expect("open");
        game_with_details(
            db.pool(),
            "v17",
            r#"[{"id":"g8","name":"Fear of Death","category":"cont","rating":2.2,"spoiler":2,"lie":false}]"#,
            r#"[{"id":"p8","name":"KID","original":"キッド","type":"co"}]"#,
            None,
            None,
        )
        .await;
        let games = seeded_games(db.pool()).await.expect("games");
        assert_eq!(games.len(), 1);
        assert_eq!(games[0].developers[0].id, "p8");
        assert!(games[0].staff.is_empty());
        assert!(games[0].voices.is_empty());
        // The tag carries its id through: the VNDB filter is written against ids.
        assert_eq!(games[0].tags[0].id.as_deref(), Some("g8"));
    }

    #[tokio::test]
    async fn staff_and_voice_blobs_decode_into_their_seed_shapes() {
        let db = Db::open_memory().await.expect("open");
        game_with_details(
            db.pool(),
            "v17",
            "[]",
            r#"[{"id":"p8","name":"KID","type":"co"}]"#,
            Some(
                r#"[{"id":"s545","name":"Nakazawa Takumi","original":"中澤 工","eid":null,"role":"scenario","note":null}]"#,
            ),
            Some(r#"[{"id":"s2001","name":"Suzumura Kenichi","original":"鈴村 健一"}]"#),
        )
        .await;
        let games = seeded_games(db.pool()).await.expect("games");
        assert_eq!(games[0].staff[0].id, "s545");
        assert_eq!(games[0].staff[0].role, "scenario");
        assert_eq!(games[0].voices[0].id, "s2001");
    }

    #[tokio::test]
    async fn a_corrupt_blob_degrades_to_empty() {
        let db = Db::open_memory().await.expect("open");
        game_with_details(
            db.pool(),
            "v17",
            "not json",
            "not json",
            Some("not json"),
            Some("not json"),
        )
        .await;
        let games = seeded_games(db.pool()).await.expect("games");
        assert!(games[0].tags.is_empty());
        assert!(games[0].developers.is_empty());
        assert!(games[0].staff.is_empty());
        assert!(games[0].voices.is_empty());
    }
}
