//! Local reading contexts. All writes are serialized by the companion runtime.
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};
use vn_core::companion::SessionOptions;

use crate::ai::conversation::{ChatMessage, store as chat};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    #[serde(default)]
    pub replay_excluded: bool,
    pub id: String,
    pub thread_id: String,
    pub playthrough_id: String,
    pub game_title: Option<String>,
    pub options: SessionOptions,
    pub source_url: Option<String>,
    pub state: String,
    pub reason: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub ended_at: Option<i64>,
}

fn decode(row: &sqlx::sqlite::SqliteRow) -> Result<Session, String> {
    let options: SessionOptions =
        serde_json::from_str(row.get("options_json")).map_err(|_| "invalid_history")?;
    options.validate()?;
    if options.game_id != row.get::<String, _>("game_id") {
        return Err("invalid_history".into());
    }
    Ok(Session {
        replay_excluded: row.get("replay_excluded"),
        id: row.get("id"),
        thread_id: row.get("thread_id"),
        playthrough_id: row.get("playthrough_id"),
        game_title: row.get("game_title"),
        options,
        source_url: row.get("source_url"),
        state: row.get("state"),
        reason: row.get("reason"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
        ended_at: row.get("ended_at"),
    })
}

pub async fn list(pool: &SqlitePool) -> Result<Vec<Session>, String> {
    let rows = sqlx::query(
        "SELECT c.*,g.title AS game_title,EXISTS(SELECT 1 FROM companion_replay_barriers b WHERE b.thread_id=c.thread_id) AS replay_excluded FROM companion_sessions c LEFT JOIN games g ON g.id=c.game_id AND g.deleted=0 ORDER BY c.updated_at DESC,c.id DESC LIMIT 100"
    ).fetch_all(pool).await.map_err(|_| "db_error")?;
    rows.iter().map(decode).collect()
}

pub async fn get(pool: &SqlitePool, id: &str) -> Result<Session, String> {
    let row = sqlx::query("SELECT c.*,g.title AS game_title,EXISTS(SELECT 1 FROM companion_replay_barriers b WHERE b.thread_id=c.thread_id) AS replay_excluded FROM companion_sessions c JOIN games g ON g.id=c.game_id AND g.deleted=0 WHERE c.id=?")
        .bind(id).fetch_optional(pool).await.map_err(|_| "db_error")?
        .ok_or("context_game_missing")?;
    decode(&row)
}

pub async fn create(
    pool: &SqlitePool,
    id: &str,
    options: SessionOptions,
    source_url: Option<String>,
) -> Result<Session, String> {
    uuid::Uuid::parse_str(id).map_err(|_| "invalid_request")?;
    options.validate()?;
    let mut tx = pool.begin().await.map_err(|_| "db_error")?;
    let game: Option<(String,)> =
        sqlx::query_as("SELECT title FROM games WHERE id=? AND deleted=0")
            .bind(&options.game_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|_| "db_error")?;
    let (game_title,) = game.ok_or("context_game_missing")?;
    let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM companion_sessions")
        .fetch_one(&mut *tx)
        .await
        .map_err(|_| "db_error")?;
    if count >= 100 {
        return Err("companion_limit".into());
    }
    let session = Session {
        replay_excluded: false,
        id: id.into(),
        thread_id: uuid::Uuid::new_v4().to_string(),
        playthrough_id: uuid::Uuid::new_v4().to_string(),
        game_title: Some(game_title),
        options,
        source_url,
        state: "active".into(),
        reason: None,
        created_at: chat::now(),
        updated_at: chat::now(),
        ended_at: None,
    };
    sqlx::query(
        "INSERT INTO chat_threads(id,title,options_json,created_at,updated_at) VALUES(?,?,?,?,?)",
    )
    .bind(&session.thread_id)
    .bind(&session.options.playthrough_label)
    .bind(serde_json::to_string(&session.options.chat_options()).map_err(|_| "invalid_options")?)
    .bind(session.created_at)
    .bind(session.updated_at)
    .execute(&mut *tx)
    .await
    .map_err(|_| "db_error")?;
    sqlx::query("INSERT INTO companion_sessions(id,thread_id,game_id,playthrough_id,options_json,source_url,state,created_at,updated_at) VALUES(?,?,?,?,?,?,'active',?,?)")
        .bind(&session.id).bind(&session.thread_id).bind(&session.options.game_id)
        .bind(&session.playthrough_id)
        .bind(serde_json::to_string(&session.options).map_err(|_| "invalid_options")?)
        .bind(&session.source_url).bind(session.created_at).bind(session.updated_at)
        .execute(&mut *tx).await.map_err(|_| "db_error")?;
    tx.commit().await.map_err(|_| "db_error")?;
    Ok(session)
}

pub async fn set_state(
    pool: &SqlitePool,
    session: &mut Session,
    state: &str,
    reason: Option<&str>,
) -> Result<(), String> {
    session.state = state.into();
    session.reason = reason.map(str::to_owned);
    session.updated_at = chat::now();
    session.ended_at = (state == "ended").then_some(session.updated_at);
    sqlx::query(
        "UPDATE companion_sessions SET state=?,reason=?,updated_at=?,ended_at=? WHERE id=?",
    )
    .bind(state)
    .bind(reason)
    .bind(session.updated_at)
    .bind(session.ended_at)
    .bind(&session.id)
    .execute(pool)
    .await
    .map_err(|_| "db_error")?;
    Ok(())
}

/// Changes only delivery preferences. Reading scope and transcript ownership stay fixed.
pub async fn set_options(pool: &SqlitePool, session: &Session) -> Result<(), String> {
    session.options.validate()?;
    let mut tx = pool.begin().await.map_err(|_| "db_error")?;
    sqlx::query("UPDATE companion_sessions SET options_json=? WHERE id=?")
        .bind(serde_json::to_string(&session.options).map_err(|_| "invalid_options")?)
        .bind(&session.id)
        .execute(&mut *tx)
        .await
        .map_err(|_| "db_error")?;
    sqlx::query("UPDATE chat_threads SET options_json=? WHERE id=?")
        .bind(
            serde_json::to_string(&session.options.chat_options())
                .map_err(|_| "invalid_options")?,
        )
        .bind(&session.thread_id)
        .execute(&mut *tx)
        .await
        .map_err(|_| "db_error")?;
    tx.commit().await.map_err(|_| "db_error")?;
    Ok(())
}

pub async fn recover(pool: &SqlitePool) -> Result<(), String> {
    sqlx::query("UPDATE companion_sessions SET state='ended',reason='interrupted',updated_at=?,ended_at=? WHERE state<>'ended'")
        .bind(chat::now()).bind(chat::now()).execute(pool).await.map_err(|_| "db_error")?;
    Ok(())
}

/// Ownership remains even if the game was removed. Normal chat commands cannot
/// mutate options, send through an unscoped source path, or delete an active run.
pub async fn require_regular_chat(pool: &SqlitePool, thread: &str) -> Result<(), String> {
    let (owned,): (bool,) =
        sqlx::query_as("SELECT EXISTS(SELECT 1 FROM companion_sessions WHERE thread_id=?)")
            .bind(thread)
            .fetch_one(pool)
            .await
            .map_err(|_| "db_error")?;
    if owned {
        Err("companion_owned".into())
    } else {
        Ok(())
    }
}

pub async fn messages(pool: &SqlitePool, id: &str) -> Result<Vec<ChatMessage>, String> {
    // History stays readable/removable after game deletion. Runtime dispatch
    // and resume still use get(), which requires a live parent game.
    let (thread,): (String,) =
        sqlx::query_as("SELECT thread_id FROM companion_sessions WHERE id=?")
            .bind(id)
            .fetch_optional(pool)
            .await
            .map_err(|_| "db_error")?
            .ok_or("companion_missing")?;
    Ok(chat::messages(pool, &thread)
        .await?
        .into_iter()
        .filter(|m| {
            m.status == "complete" || !m.sources.iter().any(|s| s.id == "automatic_reaction")
        })
        .collect())
}

pub async fn delete(pool: &SqlitePool, id: &str) -> Result<(), String> {
    let mut tx = pool.begin().await.map_err(|_| "db_error")?;
    let row: Option<(String, String)> =
        sqlx::query_as("SELECT thread_id,state FROM companion_sessions WHERE id=?")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|_| "db_error")?;
    let (thread, state) = row.ok_or("companion_missing")?;
    if state != "ended" {
        return Err("companion_active".into());
    }
    super::memory::forget_tx(&mut tx, &super::memory::Forget::Session { id: id.into() }).await?;
    sqlx::query("DELETE FROM chat_messages WHERE thread_id=?")
        .bind(&thread)
        .execute(&mut *tx)
        .await
        .map_err(|_| "db_error")?;
    sqlx::query("DELETE FROM companion_sessions WHERE id=?")
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(|_| "db_error")?;
    sqlx::query("DELETE FROM chat_threads WHERE id=?")
        .bind(&thread)
        .execute(&mut *tx)
        .await
        .map_err(|_| "db_error")?;
    tx.commit().await.map_err(|_| "db_error")?;
    Ok(())
}
