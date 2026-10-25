//! Local, unsynced chat history. All mutations run under a per-chat runtime lease.
use super::{ChatMessage, ChatThread, ChatTrace, Source};
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};
use vn_core::chat::ChatOptions;

pub fn now() -> i64 {
    time::OffsetDateTime::now_utc()
        .unix_timestamp_nanos()
        .checked_div(1_000_000)
        .and_then(|v| i64::try_from(v).ok())
        .unwrap_or(0)
}

pub async fn list(pool: &SqlitePool) -> Result<Vec<ChatThread>, String> {
    let rows = sqlx::query("SELECT * FROM chat_threads ORDER BY updated_at DESC LIMIT 100")
        .fetch_all(pool)
        .await
        .map_err(|_| "db_error")?;
    rows.iter().map(thread_row).collect()
}

fn thread_row(row: &sqlx::sqlite::SqliteRow) -> Result<ChatThread, String> {
    Ok(ChatThread {
        id: row.get("id"),
        title: row.get("title"),
        options: serde_json::from_str(row.get("options_json")).map_err(|_| "invalid_history")?,
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

pub async fn get(pool: &SqlitePool, id: &str) -> Result<ChatThread, String> {
    let row = sqlx::query("SELECT * FROM chat_threads WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|_| "db_error")?
        .ok_or("chat_missing")?;
    thread_row(&row)
}

pub async fn create(pool: &SqlitePool, options: ChatOptions) -> Result<ChatThread, String> {
    options.validate()?;
    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM chat_threads")
        .fetch_one(pool)
        .await
        .map_err(|_| "db_error")?;
    if count.0 >= 100 {
        return Err("chat_limit".into());
    }
    let thread = ChatThread {
        id: uuid::Uuid::new_v4().to_string(),
        title: String::new(),
        options,
        created_at: now(),
        updated_at: now(),
    };
    sqlx::query(
        "INSERT INTO chat_threads(id,title,options_json,created_at,updated_at) VALUES(?,?,?,?,?)",
    )
    .bind(&thread.id)
    .bind(&thread.title)
    .bind(serde_json::to_string(&thread.options).map_err(|_| "invalid_options")?)
    .bind(thread.created_at)
    .bind(thread.updated_at)
    .execute(pool)
    .await
    .map_err(|_| "db_error")?;
    Ok(thread)
}

pub async fn messages(pool: &SqlitePool, id: &str) -> Result<Vec<ChatMessage>, String> {
    let rows =
        sqlx::query("SELECT * FROM chat_messages WHERE thread_id = ? ORDER BY position LIMIT 200")
            .bind(id)
            .fetch_all(pool)
            .await
            .map_err(|_| "db_error")?;
    rows.iter()
        .map(|row| {
            Ok(ChatMessage {
                id: row.get("id"),
                role: row.get("role"),
                content: row.get("content"),
                status: row.get("status"),
                error: row.get("error"),
                tools: decode(row.get("tools_json"))?,
                sources: decode(row.get("sources_json"))?,
                tokens: row.get("tokens"),
                created_at: row.get("created_at"),
            })
        })
        .collect()
}

fn decode<T: for<'de> Deserialize<'de>>(json: &str) -> Result<T, String> {
    serde_json::from_str(json).map_err(|_| "invalid_history".into())
}
fn encode<T: Serialize + ?Sized>(value: &T) -> Result<String, String> {
    serde_json::to_string(value).map_err(|_| "invalid_history".into())
}

pub async fn begin(
    pool: &SqlitePool,
    thread: &ChatThread,
    text: &str,
    retry: bool,
) -> Result<String, String> {
    let history = messages(pool, &thread.id).await?;
    let mut tx = pool.begin().await.map_err(|_| "db_error")?;
    let assistant_id = uuid::Uuid::new_v4().to_string();
    let position = if retry {
        let last = history.last().ok_or("nothing_to_retry")?;
        if last.role != "assistant" || !matches!(last.status.as_str(), "error" | "cancelled") {
            return Err("nothing_to_retry".into());
        }
        sqlx::query("DELETE FROM chat_messages WHERE id = ?")
            .bind(&last.id)
            .execute(&mut *tx)
            .await
            .map_err(|_| "db_error")?;
        history.len() - 1
    } else {
        vn_core::chat::validate_message(text)?;
        if history.len() + 2 > vn_core::chat::MAX_MESSAGES {
            return Err("message_limit".into());
        }
        sqlx::query("INSERT INTO chat_messages(id,thread_id,position,role,content,status,created_at) VALUES(?,?,?,'user',?,'complete',?)")
            .bind(uuid::Uuid::new_v4().to_string()).bind(&thread.id).bind(i64::try_from(history.len()).unwrap_or(0)).bind(text.trim()).bind(now())
            .execute(&mut *tx).await.map_err(|_| "db_error")?;
        history.len() + 1
    };
    sqlx::query("INSERT INTO chat_messages(id,thread_id,position,role,content,status,created_at) VALUES(?,?,?,'assistant','','running',?)")
        .bind(&assistant_id).bind(&thread.id).bind(i64::try_from(position).unwrap_or(0)).bind(now())
        .execute(&mut *tx).await.map_err(|_| "db_error")?;
    let title = if thread.title.is_empty() {
        text.trim().chars().take(64).collect::<String>()
    } else {
        thread.title.clone()
    };
    sqlx::query("UPDATE chat_threads SET title = ?, updated_at = ? WHERE id = ?")
        .bind(title)
        .bind(now())
        .bind(&thread.id)
        .execute(&mut *tx)
        .await
        .map_err(|_| "db_error")?;
    tx.commit().await.map_err(|_| "db_error")?;
    Ok(assistant_id)
}

pub async fn save(pool: &SqlitePool, message: &ChatMessage) -> Result<(), String> {
    sqlx::query("UPDATE chat_messages SET content=?,status=?,error=?,tools_json=?,sources_json=?,tokens=? WHERE id=?")
        .bind(&message.content).bind(&message.status).bind(&message.error).bind(encode::<[ChatTrace]>(&message.tools)?)
        .bind(encode::<[Source]>(&message.sources)?).bind(message.tokens).bind(&message.id)
        .execute(pool).await.map_err(|_| "db_error")?;
    Ok(())
}

pub async fn recover(pool: &SqlitePool) -> Result<(), String> {
    sqlx::query(
        "UPDATE chat_messages SET status='error', error='interrupted' WHERE status='running'",
    )
    .execute(pool)
    .await
    .map_err(|_| "db_error")?;
    Ok(())
}
