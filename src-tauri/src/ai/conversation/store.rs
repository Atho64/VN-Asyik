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
    let rows = sqlx::query("SELECT * FROM chat_threads t WHERE NOT EXISTS(SELECT 1 FROM companion_sessions c WHERE c.thread_id=t.id) ORDER BY updated_at DESC LIMIT 100")
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
    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM chat_threads t WHERE NOT EXISTS(SELECT 1 FROM companion_sessions c WHERE c.thread_id=t.id)")
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
    // Newest window, returned oldest first. Ordering by position descending and
    // reversing means the cap keeps the *latest* rows; ascending with LIMIT kept
    // the first 200 and silently hid everything written after that.
    let rows = sqlx::query(
        "SELECT * FROM (SELECT * FROM chat_messages WHERE thread_id = ? \
         ORDER BY position DESC LIMIT ?) ORDER BY position ASC",
    )
    .bind(id)
    .bind(i64::try_from(vn_core::chat::MAX_MESSAGES).unwrap_or(200))
    .fetch_all(pool)
    .await
    .map_err(|_| "db_error")?;
    rows.iter().map(message_row).collect()
}

pub(crate) fn message_row(row: &sqlx::sqlite::SqliteRow) -> Result<ChatMessage, String> {
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
}

/// Display reads deliberately ignore this barrier; only provider replay uses it.
pub(crate) async fn model_messages(
    pool: &SqlitePool,
    id: &str,
) -> Result<Vec<ChatMessage>, String> {
    let rows = sqlx::query("SELECT m.* FROM chat_messages m JOIN chat_threads t ON t.id=m.thread_id LEFT JOIN companion_replay_barriers b ON b.thread_id=t.id WHERE t.id=? AND m.position>COALESCE(b.excluded_through,-1) ORDER BY m.position DESC LIMIT 200")
        .bind(id).fetch_all(pool).await.map_err(|_| "db_error")?;
    rows.iter().rev().map(message_row).collect()
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
    begin_scoped(pool, thread, text, retry, None).await
}

pub(crate) async fn begin_scoped(
    pool: &SqlitePool,
    thread: &ChatThread,
    text: &str,
    retry: bool,
    memory: Option<&crate::companion::memory::TurnMemory>,
) -> Result<String, String> {
    // A thread may hold more rows than the window; the newest window is all the
    // retry check needs, so this does not decode a long history to learn its size.
    let history = messages(pool, &thread.id).await?;
    let mut tx = pool.begin().await.map_err(|_| "db_error")?;
    if let Some(memory) = memory {
        crate::companion::memory::check_revision(&mut tx, memory.revision).await?;
    }
    let assistant_id = uuid::Uuid::new_v4().to_string();
    let (position, room) = if retry {
        let last = history.last().ok_or("nothing_to_retry")?;
        if last.role != "assistant" || !matches!(last.status.as_str(), "error" | "cancelled") {
            return Err("nothing_to_retry".into());
        }
        sqlx::query("DELETE FROM chat_messages WHERE id = ?")
            .bind(&last.id)
            .execute(&mut *tx)
            .await
            .map_err(|_| "db_error")?;
        (last_position(&mut tx, &thread.id).await?, 1)
    } else {
        vn_core::chat::validate_message(text)?;
        let position = last_position(&mut tx, &thread.id).await? + 1;
        sqlx::query("INSERT INTO chat_messages(id,thread_id,position,role,content,status,created_at) VALUES(?,?,?,'user',?,'complete',?)")
            .bind(uuid::Uuid::new_v4().to_string()).bind(&thread.id).bind(position).bind(text.trim()).bind(now())
            .execute(&mut *tx).await.map_err(|_| "db_error")?;
        (position + 1, 2)
    };
    // Roll the window before inserting: the oldest exchange goes, so the newest
    // exchange always fits and the thread never becomes permanently unusable.
    trim_window(
        &mut tx,
        &thread.id,
        room,
        memory.and_then(|m| m.capture.as_ref()),
    )
    .await?;
    sqlx::query("INSERT INTO chat_messages(id,thread_id,position,role,content,status,created_at) VALUES(?,?,?,'assistant','','running',?)")
        .bind(&assistant_id).bind(&thread.id).bind(position).bind(now())
        .execute(&mut *tx).await.map_err(|_| "db_error")?;
    let title = if thread.title.is_empty() {
        let source_text = if text.trim().is_empty() {
            history
                .iter()
                .rev()
                .find(|m| m.role == "user" && !m.content.trim().is_empty())
                .map_or("", |m| m.content.as_str())
        } else {
            text.trim()
        };
        source_text.chars().take(64).collect::<String>()
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

/// Next free `position` in a thread, or 0 for an empty one.
pub(crate) async fn last_position(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    thread_id: &str,
) -> Result<i64, String> {
    // Read through the open transaction. Asking the pool instead would need a
    // second connection while this one holds SQLite's write lock, and the busy
    // timeout would eventually fail the whole call.
    let row: Option<(Option<i64>,)> =
        sqlx::query_as("SELECT MAX(position) FROM chat_messages WHERE thread_id = ?")
            .bind(thread_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(|_| "db_error")?;
    // An empty thread answers NULL, which is a free slot at 0.
    Ok(row
        .and_then(|(max,)| max)
        .map_or(0, |max| max.saturating_add(1)))
}

/// Drops oldest exchange pairs until `room` additional rows fit inside `MAX_MESSAGES`.
/// Deletes complete two-row exchanges to preserve user/assistant alternation.
pub(crate) async fn trim_window(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    thread_id: &str,
    room: i64,
    capture: Option<&crate::companion::memory::jobs::Capture>,
) -> Result<(), String> {
    let window = i64::try_from(vn_core::chat::MAX_MESSAGES).unwrap_or(200);
    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM chat_messages WHERE thread_id = ?")
        .bind(thread_id)
        .fetch_one(&mut **tx)
        .await
        .map_err(|_| "db_error")?;
    let excess = count.0 + room - window;
    if excess <= 0 {
        return Ok(());
    }
    // Advance until at least `excess` rows are dropped and the surviving window starts on a user turn.
    let rows = sqlx::query(
        "SELECT position, role FROM chat_messages WHERE thread_id = ? ORDER BY position ASC LIMIT ?",
    )
    .bind(thread_id)
    .bind(excess + 6)
    .fetch_all(&mut **tx)
    .await
    .map_err(|_| "db_error")?;
    let min_drop = usize::try_from(excess).unwrap_or(0).min(rows.len());
    let drop_count = (min_drop..rows.len())
        .find(|&i| rows[i].get::<String, _>("role") == "user")
        .unwrap_or(rows.len());
    let drop_limit = i64::try_from(drop_count).unwrap_or(excess);
    if let Some(capture) = capture {
        let (through,):(Option<i64>,) = sqlx::query_as("SELECT MAX(position) FROM (SELECT position FROM chat_messages WHERE thread_id=? ORDER BY position LIMIT ?)")
            .bind(thread_id).bind(drop_limit).fetch_one(&mut **tx).await.map_err(|_| "db_error")?;
        if let Some(through) = through {
            crate::companion::memory::jobs::capture(tx, capture, Some(through), true).await?;
        }
    } else {
        sqlx::query("UPDATE companion_memory_state SET gap='disabled' WHERE id=1 AND EXISTS(SELECT 1 FROM companion_sessions WHERE thread_id=?)")
            .bind(thread_id).execute(&mut **tx).await.map_err(|_| "db_error")?;
    }
    sqlx::query(
        "DELETE FROM chat_messages WHERE id IN (SELECT id FROM chat_messages \
         WHERE thread_id = ? ORDER BY position ASC LIMIT ?)",
    )
    .bind(thread_id)
    .bind(drop_limit)
    .execute(&mut **tx)
    .await
    .map_err(|_| "db_error")?;
    Ok(())
}
