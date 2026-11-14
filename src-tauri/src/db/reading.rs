//! Local reading history and user-written checkpoints. No game-file access.

use serde::{Deserialize, Serialize};
use sqlx::{Executor, FromRow, Sqlite, SqlitePool};
use vn_core::model::{DeviceTag, SyncMeta};
use vn_core::reading::ReadingEntry;
use vn_core::{DeviceId, EntityId, Timestamp};

use super::DbError;
use crate::commands::{CommandError, CommandResult};

const PAGE_SIZE: i64 = 10;

#[derive(Debug, FromRow)]
struct EntryRow {
    id: String,
    game_id: String,
    session_id: Option<String>,
    created_at: i64,
    played_at: Option<i64>,
    note: String,
    save_slot: String,
    route_name: String,
    chapter_name: String,
    photo_image_id: String,
    updated_at: i64,
    device_id: String,
    deleted: bool,
}

impl TryFrom<EntryRow> for ReadingEntry {
    type Error = DbError;

    fn try_from(row: EntryRow) -> Result<Self, Self::Error> {
        let parse = |id: &str| EntityId::parse(id).map_err(|e| DbError::Corrupt(e.to_string()));
        Ok(Self {
            meta: SyncMeta {
                id: parse(&row.id)?,
                updated_at: Timestamp::from_millis(row.updated_at),
                device_id_device: DeviceTag::new(
                    &DeviceId::parse(&row.device_id)
                        .map_err(|e| DbError::Corrupt(e.to_string()))?,
                ),
                deleted: row.deleted,
            },
            game_id: parse(&row.game_id)?,
            session_id: row.session_id.as_deref().map(parse).transpose()?,
            created_at: Timestamp::from_millis(row.created_at),
            played_at: row.played_at.map(Timestamp::from_millis),
            note: row.note,
            save_slot: row.save_slot,
            route_name: row.route_name,
            chapter_name: row.chapter_name,
            photo_image_id: row.photo_image_id,
        })
    }
}

#[derive(Debug, Clone, FromRow, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadingSession {
    pub id: String,
    pub started_at: i64,
    pub ended_at: i64,
    pub counted_ms: i64,
    pub afk_ms: i64,
    pub route_name: Option<String>,
    pub chapter_name: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadingContext {
    pub latest_session: Option<ReadingSession>,
    pub entries: Vec<ReadingEntry>,
    pub has_more: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionReadingContext {
    pub session: ReadingSession,
    pub entries: Vec<ReadingEntry>,
    pub has_more: bool,
}

/// The latest *counted* session supplies the position, never a chapter reached
/// in some older session. Every join checks ownership and hides tombstones.
pub async fn context(
    pool: &SqlitePool,
    game: EntityId,
    offset: i64,
) -> CommandResult<ReadingContext> {
    if super::queries::get_game(pool, game).await?.is_none() {
        return Err(CommandError::new("not_found", "game not found"));
    }
    let latest_session = sqlx::query_as::<_, ReadingSession>(
        "SELECT s.id, s.started_at, s.ended_at, s.counted_ms, s.afk_ms,
                r.name AS route_name, c.name AS chapter_name
         FROM play_sessions s JOIN games g ON g.id = s.game_id AND g.deleted = 0
         LEFT JOIN chapters c ON c.id = s.chapter_id AND c.game_id = g.id AND c.deleted = 0
         LEFT JOIN routes r ON r.id = COALESCE(s.route_id, c.route_id)
              AND r.game_id = g.id AND r.deleted = 0 AND r.merged_into IS NULL
         WHERE s.game_id = ? AND s.deleted = 0 AND s.counted_ms > 0
         ORDER BY s.ended_at DESC, s.started_at DESC, s.id DESC LIMIT 1",
    )
    .bind(game.to_string())
    .fetch_optional(pool)
    .await
    .map_err(DbError::from)?;
    let (entries, has_more) = entries_page(pool, game, None, offset).await?;
    Ok(ReadingContext {
        latest_session,
        entries,
        has_more,
    })
}

/// A selected live session, including sessions with no counted reading time.
/// Its position comes only from its own attribution, never from a newer session
/// or from the editable text snapshots in journal entries.
pub async fn session_context(
    pool: &SqlitePool,
    game: EntityId,
    session: EntityId,
    offset: i64,
) -> CommandResult<SessionReadingContext> {
    let selected = sqlx::query_as::<_, ReadingSession>(
        "SELECT s.id, s.started_at, s.ended_at, s.counted_ms, s.afk_ms,
                r.name AS route_name, c.name AS chapter_name
         FROM play_sessions s JOIN games g ON g.id = s.game_id AND g.deleted = 0
         LEFT JOIN chapters c ON c.id = s.chapter_id AND c.game_id = g.id AND c.deleted = 0
         LEFT JOIN routes r ON r.id = COALESCE(s.route_id, c.route_id)
              AND r.game_id = g.id AND r.deleted = 0 AND r.merged_into IS NULL
         WHERE s.id = ? AND s.game_id = ? AND s.deleted = 0",
    )
    .bind(session.to_string())
    .bind(game.to_string())
    .fetch_optional(pool)
    .await
    .map_err(DbError::from)?
    .ok_or_else(|| CommandError::new("not_found", "session not found"))?;
    let (entries, has_more) = entries_page(pool, game, Some(session), offset).await?;
    Ok(SessionReadingContext {
        session: selected,
        entries,
        has_more,
    })
}

async fn entries_page(
    pool: &SqlitePool,
    game: EntityId,
    session: Option<EntityId>,
    offset: i64,
) -> Result<(Vec<ReadingEntry>, bool), DbError> {
    let session = session.map(|id| id.to_string());
    let rows = sqlx::query_as::<_, EntryRow>(
        "SELECT e.* FROM reading_entries e JOIN games g ON g.id = e.game_id
         WHERE e.game_id = ? AND e.deleted = 0 AND g.deleted = 0
           AND (? IS NULL OR e.session_id = ?)
         ORDER BY e.created_at DESC, e.id DESC LIMIT ? OFFSET ?",
    )
    .bind(game.to_string())
    .bind(&session)
    .bind(&session)
    .bind(PAGE_SIZE + 1)
    .bind(offset.max(0))
    .fetch_all(pool)
    .await
    .map_err(DbError::from)?;
    let has_more = rows.len() > usize::try_from(PAGE_SIZE).unwrap_or(10);
    let entries = rows
        .into_iter()
        .take(10)
        .map(ReadingEntry::try_from)
        .collect::<Result<Vec<_>, _>>()?;
    Ok((entries, has_more))
}

pub async fn list_all<'e, E>(executor: E, game: EntityId) -> Result<Vec<ReadingEntry>, DbError>
where
    E: Executor<'e, Database = Sqlite>,
{
    let rows = sqlx::query_as::<_, EntryRow>(
        "SELECT e.* FROM reading_entries e JOIN games g ON g.id = e.game_id
         WHERE e.game_id = ? ORDER BY e.created_at DESC, e.id DESC",
    )
    .bind(game.to_string())
    .fetch_all(executor)
    .await?;
    rows.into_iter().map(ReadingEntry::try_from).collect()
}

/// One checkpoint with its dates already in the reader's local calendar.
#[derive(Debug)]
pub struct JournalExportRow {
    pub entry: ReadingEntry,
    /// Local calendar date, `YYYY-MM-DD`.
    pub date: String,
    /// Local clock time, `HH:MM`.
    pub time: String,
    /// Local date of the linked reading session, when the checkpoint has one.
    pub played_date: Option<String>,
}

/// Row shape for [`list_for_export`]: the entry columns plus `SQLite`'s local
/// calendar rendering of the two timestamps.
#[derive(Debug, FromRow)]
struct ExportRow {
    #[sqlx(flatten)]
    entry: EntryRow,
    local_date: String,
    local_time: String,
    played_date: Option<String>,
}

/// Live checkpoints in chronological reading order with `SQLite` `'localtime'` dates for export.
/// Excludes tombstones (unlike [`list_all`]) so exported diaries never resurrect deleted notes.
pub async fn list_for_export(
    pool: &SqlitePool,
    game: EntityId,
) -> Result<Vec<JournalExportRow>, DbError> {
    let rows = sqlx::query_as::<_, ExportRow>(
        "SELECT e.*, \
                strftime('%Y-%m-%d', e.created_at / 1000, 'unixepoch', 'localtime') AS local_date, \
                strftime('%H:%M', e.created_at / 1000, 'unixepoch', 'localtime') AS local_time, \
                CASE WHEN e.played_at IS NULL THEN NULL \
                     ELSE strftime('%Y-%m-%d', e.played_at / 1000, 'unixepoch', 'localtime') \
                END AS played_date \
         FROM reading_entries e JOIN games g ON g.id = e.game_id AND g.deleted = 0 \
         WHERE e.game_id = ? AND e.deleted = 0 \
         ORDER BY e.created_at ASC, e.id ASC",
    )
    .bind(game.to_string())
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|row| {
            Ok(JournalExportRow {
                entry: ReadingEntry::try_from(row.entry)?,
                date: row.local_date,
                time: row.local_time,
                played_date: row.played_date,
            })
        })
        .collect()
}

/// Searches live checkpoints (note, route, and chapter snapshots) newest first in 50-row pages.
/// Joins live `games`, escapes `%`/`_`/`\`, and uses `LIKE` (ASCII case-folding, exact CJK; ADR 0005).
pub async fn search(
    pool: &SqlitePool,
    query: &str,
    game: Option<EntityId>,
    before: Option<(i64, &str)>,
) -> Result<(Vec<ReadingEntry>, bool), DbError> {
    // 50 rows plus one probe row, the same exact `has_more` the timeline uses.
    const PAGE_PLUS_ONE: i64 = 51;
    let escaped = query
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    let pattern = format!("%{escaped}%");
    let game_id = game.map(|id| id.to_string());
    let before_ts = before.map(|(ts, _)| ts);
    let before_id = before.map(|(_, id)| id);
    let rows = sqlx::query_as::<_, EntryRow>(
        "SELECT e.* FROM reading_entries e \
         JOIN games g ON g.id = e.game_id AND g.deleted = 0 \
         WHERE e.deleted = 0 \
           AND (e.note LIKE ? ESCAPE '\\' \
                OR e.route_name LIKE ? ESCAPE '\\' \
                OR e.chapter_name LIKE ? ESCAPE '\\') \
           AND (? IS NULL OR e.game_id = ?) \
           AND (? IS NULL OR e.created_at < ? OR (e.created_at = ? AND e.id < ?)) \
         ORDER BY e.created_at DESC, e.id DESC \
         LIMIT ?",
    )
    .bind(&pattern)
    .bind(&pattern)
    .bind(&pattern)
    .bind(&game_id)
    .bind(&game_id)
    .bind(before_ts)
    .bind(before_ts)
    .bind(before_ts)
    .bind(before_id)
    .bind(PAGE_PLUS_ONE)
    .fetch_all(pool)
    .await?;
    let has_more = rows.len() > 50;
    let entries = rows
        .into_iter()
        .take(50)
        .map(ReadingEntry::try_from)
        .collect::<Result<Vec<_>, _>>()?;
    Ok((entries, has_more))
}

/// Used inside the sync/restore transaction after games and sessions are stored.
/// Reject cross-game session references even though `SQLite`'s single-column FK
/// alone would accept them. Identity must not migrate to another parent.
pub async fn upsert<'e, E>(executor: E, entry: &ReadingEntry) -> Result<(), DbError>
where
    E: Executor<'e, Database = Sqlite>,
{
    entry.validate().map_err(DbError::Corrupt)?;
    let changed = sqlx::query(
        "INSERT INTO reading_entries
         (id, game_id, session_id, created_at, played_at, note, save_slot, route_name, chapter_name, photo_image_id, updated_at, device_id, deleted)
         SELECT ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?
         WHERE EXISTS (SELECT 1 FROM games WHERE id = ?)
           AND (? IS NULL OR EXISTS (SELECT 1 FROM play_sessions WHERE id = ? AND game_id = ?))
         ON CONFLICT(id) DO UPDATE SET note = excluded.note, save_slot = excluded.save_slot,
           route_name = excluded.route_name, chapter_name = excluded.chapter_name,
           photo_image_id = excluded.photo_image_id,
           updated_at = excluded.updated_at, device_id = excluded.device_id, deleted = excluded.deleted
         WHERE reading_entries.game_id = excluded.game_id
           AND reading_entries.session_id IS excluded.session_id
           AND reading_entries.created_at = excluded.created_at
           AND reading_entries.played_at IS excluded.played_at",
    ).bind(entry.meta.id.to_string()).bind(entry.game_id.to_string())
        .bind(entry.session_id.map(|id| id.to_string())).bind(entry.created_at.as_millis())
        .bind(entry.played_at.map(Timestamp::as_millis)).bind(&entry.note).bind(&entry.save_slot)
        .bind(&entry.route_name).bind(&entry.chapter_name).bind(&entry.photo_image_id)
        .bind(entry.meta.updated_at.as_millis())
        .bind(entry.meta.device_id_device.device().as_str()).bind(entry.meta.deleted)
        .bind(entry.game_id.to_string()).bind(entry.session_id.map(|id| id.to_string()))
        .bind(entry.session_id.map(|id| id.to_string())).bind(entry.game_id.to_string())
        .execute(executor).await?.rows_affected();
    if changed == 0 {
        return Err(DbError::Corrupt(
            "reading entry has an invalid parent or changed identity".to_owned(),
        ));
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
pub struct SaveReadingInput {
    pub id: Option<EntityId>,
    pub expected_updated_at: Option<i64>,
    pub game_id: EntityId,
    pub session_id: Option<EntityId>,
    pub note: String,
    pub save_slot: String,
    pub route_name: String,
    pub chapter_name: String,
    /// VNDB image id of a pinned screenshot, or empty for a text-only checkpoint.
    #[serde(default)]
    pub photo_image_id: String,
}

fn conflict() -> CommandError {
    CommandError::new(
        "reading_conflict",
        "This entry changed. Reload it before editing.",
    )
}

pub async fn save(
    pool: &SqlitePool,
    device: &DeviceId,
    input: SaveReadingInput,
) -> CommandResult<ReadingEntry> {
    let mut tx = pool.begin().await.map_err(DbError::from)?;
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM games WHERE id = ? AND deleted = 0)")
            .bind(input.game_id.to_string())
            .fetch_one(&mut *tx)
            .await
            .map_err(DbError::from)?;
    if !exists {
        return Err(CommandError::new("not_found", "game not found"));
    }
    let now = Timestamp::now()?;
    let mut entry = if let Some(id) = input.id {
        let row = sqlx::query_as::<_, EntryRow>(
            "SELECT * FROM reading_entries WHERE id = ? AND game_id = ? AND deleted = 0",
        )
        .bind(id.to_string())
        .bind(input.game_id.to_string())
        .fetch_optional(&mut *tx)
        .await
        .map_err(DbError::from)?
        .ok_or_else(conflict)?;
        let mut entry = ReadingEntry::try_from(row)?;
        if input.expected_updated_at != Some(entry.meta.updated_at.as_millis())
            || entry.session_id != input.session_id
        {
            return Err(conflict());
        }
        entry.meta.updated_at = Timestamp::from_millis(
            now.as_millis()
                .max(entry.meta.updated_at.as_millis().saturating_add(1)),
        );
        entry.meta.device_id_device = DeviceTag::new(device);
        entry
    } else {
        let played_at = if let Some(session) = input.session_id {
            Some(sqlx::query_scalar::<_, i64>("SELECT ended_at FROM play_sessions WHERE id = ? AND game_id = ? AND deleted = 0")
                .bind(session.to_string()).bind(input.game_id.to_string()).fetch_optional(&mut *tx)
                .await.map_err(DbError::from)?.ok_or_else(|| CommandError::new("invalid_session", "session does not belong to this game"))?)
        } else {
            None
        };
        ReadingEntry {
            meta: SyncMeta::new(device, now),
            game_id: input.game_id,
            session_id: input.session_id,
            created_at: now,
            played_at: played_at.map(Timestamp::from_millis),
            note: String::new(),
            save_slot: String::new(),
            route_name: String::new(),
            chapter_name: String::new(),
            photo_image_id: String::new(),
        }
    };
    entry.note = input.note.trim().to_owned();
    entry.save_slot = input.save_slot.trim().to_owned();
    entry.route_name = input.route_name.trim().to_owned();
    entry.chapter_name = input.chapter_name.trim().to_owned();
    entry.photo_image_id = input.photo_image_id.trim().to_owned();
    entry
        .validate()
        .map_err(|e| CommandError::new("invalid_reading_entry", e))?;
    upsert(&mut *tx, &entry).await?;
    tx.commit().await.map_err(DbError::from)?;
    Ok(entry)
}

pub async fn delete(
    pool: &SqlitePool,
    device: &DeviceId,
    game: EntityId,
    id: EntityId,
    expected: i64,
) -> CommandResult<()> {
    let now = Timestamp::now()?
        .as_millis()
        .max(expected.saturating_add(1));
    let changed = sqlx::query(
        "UPDATE reading_entries SET deleted = 1, updated_at = ?, device_id = ?
         WHERE id = ? AND game_id = ? AND deleted = 0 AND updated_at = ?
           AND EXISTS (SELECT 1 FROM games WHERE id = reading_entries.game_id AND deleted = 0)",
    )
    .bind(now)
    .bind(device.as_str())
    .bind(id.to_string())
    .bind(game.to_string())
    .bind(expected)
    .execute(pool)
    .await
    .map_err(DbError::from)?
    .rows_affected();
    if changed == 0 {
        return Err(conflict());
    }
    Ok(())
}
