//! Global and cross-game reading notes stored in `global_notes` (migration 0038).
//!
//! Unlike per-session checkpoints (`reading_entries`), a global note can exist
//! without any game (`game_id = NULL`) or optionally link to one visual novel,
//! and can be pinned to the top of the Timeline Notes tab.

use serde::{Deserialize, Serialize};
use sqlx::{Executor, FromRow, Sqlite, SqlitePool};
use vn_core::model::{DeviceTag, SyncMeta};
use vn_core::reading::GlobalNote;
use vn_core::{DeviceId, EntityId, Timestamp};

use super::DbError;
use crate::commands::{CommandError, CommandResult};

#[derive(Debug, FromRow)]
struct NoteRow {
    id: String,
    game_id: Option<String>,
    title: String,
    body: String,
    pinned: bool,
    created_at: i64,
    updated_at: i64,
    device_id: String,
    deleted: bool,
}

impl TryFrom<NoteRow> for GlobalNote {
    type Error = DbError;

    fn try_from(row: NoteRow) -> Result<Self, Self::Error> {
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
            game_id: row.game_id.as_deref().map(parse).transpose()?,
            created_at: Timestamp::from_millis(row.created_at),
            title: row.title,
            body: row.body,
            pinned: row.pinned,
        })
    }
}

#[derive(Debug, FromRow)]
struct NoteViewRow {
    #[sqlx(flatten)]
    note: NoteRow,
    live_game_id: Option<String>,
    game_title: Option<String>,
}

/// A global note paired with its live game's title, when linked to a non-deleted game.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GlobalNoteView {
    pub note: GlobalNote,
    pub game_title: Option<String>,
}

impl TryFrom<NoteViewRow> for GlobalNoteView {
    type Error = DbError;

    fn try_from(row: NoteViewRow) -> Result<Self, Self::Error> {
        let mut note = GlobalNote::try_from(row.note)?;
        // Hide links to soft-deleted games in the UI view without mutating the synced row.
        if row.live_game_id.is_none() {
            note.game_id = None;
        }
        Ok(Self {
            note,
            game_title: row.game_title,
        })
    }
}

/// List live global notes (pinned first, then newest updated), with optional text search and game filter.
pub async fn list(
    pool: &SqlitePool,
    query: Option<&str>,
    game_id: Option<EntityId>,
    global_only: bool,
) -> Result<Vec<GlobalNoteView>, DbError> {
    let trimmed = query.map(str::trim).filter(|q| !q.is_empty());
    let pattern = trimmed.map(|q| {
        let escaped = q
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        format!("%{escaped}%")
    });
    let game_str = game_id.map(|id| id.to_string());

    let rows = sqlx::query_as::<_, NoteViewRow>(
        "SELECT n.id, n.game_id, n.title, n.body, n.pinned, n.created_at, n.updated_at, n.device_id, n.deleted, \
                g.id AS live_game_id, g.title AS game_title \
         FROM global_notes n \
         LEFT JOIN games g ON g.id = n.game_id AND g.deleted = 0 \
         WHERE n.deleted = 0 \
           AND (? = 0 OR g.id IS NULL) \
           AND (? IS NULL OR g.id = ?) \
           AND (? IS NULL OR n.title LIKE ? ESCAPE '\\' \
                OR n.body LIKE ? ESCAPE '\\' \
                OR COALESCE(g.title, '') LIKE ? ESCAPE '\\') \
         ORDER BY n.pinned DESC, n.updated_at DESC, n.created_at DESC, n.id DESC",
    )
    .bind(i32::from(global_only))
    .bind(&game_str)
    .bind(&game_str)
    .bind(&pattern)
    .bind(&pattern)
    .bind(&pattern)
    .bind(&pattern)
    .fetch_all(pool)
    .await?;

    rows.into_iter().map(GlobalNoteView::try_from).collect()
}

/// Every global note row, including tombstones, for backup and folder sync.
pub async fn list_all_including_deleted<'e, E>(executor: E) -> Result<Vec<GlobalNote>, DbError>
where
    E: Executor<'e, Database = Sqlite>,
{
    let rows = sqlx::query_as::<_, NoteRow>(
        "SELECT id, game_id, title, body, pinned, created_at, updated_at, device_id, deleted \
         FROM global_notes ORDER BY created_at DESC, id DESC",
    )
    .fetch_all(executor)
    .await?;
    rows.into_iter().map(GlobalNote::try_from).collect()
}

/// Upsert one global note inside a sync, backup, or save transaction.
pub async fn upsert<'e, E>(executor: E, note: &GlobalNote) -> Result<(), DbError>
where
    E: Executor<'e, Database = Sqlite>,
{
    note.validate().map_err(DbError::Corrupt)?;
    let game_id = note.game_id.map(|id| id.to_string());
    sqlx::query(
        "INSERT INTO global_notes \
         (id, game_id, title, body, pinned, created_at, updated_at, device_id, deleted) \
         VALUES ( \
           ?, \
           CASE WHEN ? IS NOT NULL AND EXISTS (SELECT 1 FROM games WHERE id = ?) THEN ? ELSE NULL END, \
           ?, ?, ?, ?, ?, ?, ? \
         ) \
         ON CONFLICT(id) DO UPDATE SET \
           game_id = excluded.game_id, \
           title = excluded.title, \
           body = excluded.body, \
           pinned = excluded.pinned, \
           updated_at = excluded.updated_at, \
           device_id = excluded.device_id, \
           deleted = excluded.deleted",
    )
    .bind(note.meta.id.to_string())
    .bind(&game_id)
    .bind(&game_id)
    .bind(&game_id)
    .bind(&note.title)
    .bind(&note.body)
    .bind(note.pinned)
    .bind(note.created_at.as_millis())
    .bind(note.meta.updated_at.as_millis())
    .bind(note.meta.device_id_device.device().as_str())
    .bind(note.meta.deleted)
    .execute(executor)
    .await?;
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveGlobalNoteInput {
    pub id: Option<EntityId>,
    pub expected_updated_at: Option<i64>,
    pub game_id: Option<EntityId>,
    #[serde(default)]
    pub title: String,
    pub body: String,
    #[serde(default)]
    pub pinned: bool,
}

fn conflict() -> CommandError {
    CommandError::new(
        "note_conflict",
        "This note changed. Reload it before editing.",
    )
}

async fn load_view(pool: &SqlitePool, id: EntityId) -> Result<GlobalNoteView, DbError> {
    let row = sqlx::query_as::<_, NoteViewRow>(
        "SELECT n.id, n.game_id, n.title, n.body, n.pinned, n.created_at, n.updated_at, n.device_id, n.deleted, \
                g.id AS live_game_id, g.title AS game_title \
         FROM global_notes n \
         LEFT JOIN games g ON g.id = n.game_id AND g.deleted = 0 \
         WHERE n.id = ?",
    )
    .bind(id.to_string())
    .fetch_one(pool)
    .await?;
    GlobalNoteView::try_from(row)
}

/// Create or update a global note.
pub async fn save(
    pool: &SqlitePool,
    device: &DeviceId,
    input: SaveGlobalNoteInput,
) -> CommandResult<GlobalNoteView> {
    let mut tx = pool.begin().await.map_err(DbError::from)?;
    if let Some(game_id) = input.game_id {
        let exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM games WHERE id = ? AND deleted = 0)")
                .bind(game_id.to_string())
                .fetch_one(&mut *tx)
                .await
                .map_err(DbError::from)?;
        if !exists {
            return Err(CommandError::new("not_found", "linked game not found"));
        }
    }

    let now = Timestamp::now()?;
    let mut note = if let Some(id) = input.id {
        let row = sqlx::query_as::<_, NoteRow>(
            "SELECT id, game_id, title, body, pinned, created_at, updated_at, device_id, deleted \
             FROM global_notes WHERE id = ? AND deleted = 0",
        )
        .bind(id.to_string())
        .fetch_optional(&mut *tx)
        .await
        .map_err(DbError::from)?
        .ok_or_else(conflict)?;
        let mut existing = GlobalNote::try_from(row)?;
        if let Some(expected) = input.expected_updated_at
            && expected != existing.meta.updated_at.as_millis()
        {
            return Err(conflict());
        }
        existing.meta.updated_at = Timestamp::from_millis(
            now.as_millis()
                .max(existing.meta.updated_at.as_millis().saturating_add(1)),
        );
        existing.meta.device_id_device = DeviceTag::new(device);
        existing
    } else {
        GlobalNote {
            meta: SyncMeta::new(device, now),
            game_id: None,
            created_at: now,
            title: String::new(),
            body: String::new(),
            pinned: false,
        }
    };

    note.game_id = input.game_id;
    note.title = input.title.trim().to_owned();
    note.body = input.body.trim().to_owned();
    note.pinned = input.pinned;
    note.validate()
        .map_err(|err| CommandError::new("invalid_global_note", err))?;

    upsert(&mut *tx, &note).await?;
    tx.commit().await.map_err(DbError::from)?;
    Ok(load_view(pool, note.meta.id).await?)
}

/// Pin or unpin an existing live global note.
pub async fn set_pinned(
    pool: &SqlitePool,
    device: &DeviceId,
    id: EntityId,
    pinned: bool,
) -> CommandResult<GlobalNoteView> {
    let mut tx = pool.begin().await.map_err(DbError::from)?;
    let row = sqlx::query_as::<_, NoteRow>(
        "SELECT id, game_id, title, body, pinned, created_at, updated_at, device_id, deleted \
         FROM global_notes WHERE id = ? AND deleted = 0",
    )
    .bind(id.to_string())
    .fetch_optional(&mut *tx)
    .await
    .map_err(DbError::from)?
    .ok_or_else(|| CommandError::new("not_found", "note not found"))?;
    let mut note = GlobalNote::try_from(row)?;
    let now = Timestamp::now()?;
    note.pinned = pinned;
    note.meta.updated_at = Timestamp::from_millis(
        now.as_millis()
            .max(note.meta.updated_at.as_millis().saturating_add(1)),
    );
    note.meta.device_id_device = DeviceTag::new(device);
    upsert(&mut *tx, &note).await?;
    tx.commit().await.map_err(DbError::from)?;
    Ok(load_view(pool, id).await?)
}

/// Soft-delete a global note so the tombstone propagates across sync and backups.
pub async fn delete(
    pool: &SqlitePool,
    device: &DeviceId,
    id: EntityId,
    expected_updated_at: Option<i64>,
) -> CommandResult<()> {
    let current: Option<i64> =
        sqlx::query_scalar("SELECT updated_at FROM global_notes WHERE id = ? AND deleted = 0")
            .bind(id.to_string())
            .fetch_optional(pool)
            .await
            .map_err(DbError::from)?;
    let Some(current_ts) = current else {
        return Err(conflict());
    };
    if let Some(expected) = expected_updated_at
        && expected != current_ts
    {
        return Err(conflict());
    }
    let now = Timestamp::now()?
        .as_millis()
        .max(current_ts.saturating_add(1));
    let changed = sqlx::query(
        "UPDATE global_notes SET deleted = 1, updated_at = ?, device_id = ? \
         WHERE id = ? AND deleted = 0",
    )
    .bind(now)
    .bind(device.as_str())
    .bind(id.to_string())
    .execute(pool)
    .await
    .map_err(DbError::from)?
    .rows_affected();
    if changed == 0 {
        return Err(conflict());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{Db, queries};
    use vn_core::model::{Game, GameStatus};

    #[tokio::test]
    async fn global_and_game_linked_notes_crud_pin_and_search() {
        let db = Db::open_memory().await.expect("db");
        let device = queries::ensure_device_id(db.pool()).await.expect("device");
        let now = Timestamp::from_millis(1_000);
        let game = Game::new(&device, now, "Steins;Gate", GameStatus::Playing).expect("game");
        let game_id = game.meta.id;
        queries::upsert_game(db.pool(), &game)
            .await
            .expect("seed game");

        // Create a global note (no game linked).
        let global = save(
            db.pool(),
            &device,
            SaveGlobalNoteInput {
                id: None,
                expected_updated_at: None,
                game_id: None,
                title: "SciADV Reading Order".to_owned(),
                body: "Read Chaos;Head Noah before Steins;Gate and Robotics;Notes.".to_owned(),
                pinned: false,
            },
        )
        .await
        .expect("save global");
        assert_eq!(global.note.game_id, None);
        assert_eq!(global.game_title, None);

        // Create a game-linked note.
        let linked = save(
            db.pool(),
            &device,
            SaveGlobalNoteInput {
                id: None,
                expected_updated_at: None,
                game_id: Some(game_id),
                title: "Divergence Numbers".to_owned(),
                body: "Track worldline shifts across chapters.".to_owned(),
                pinned: false,
            },
        )
        .await
        .expect("save linked");
        assert_eq!(linked.note.game_id, Some(game_id));
        assert_eq!(linked.game_title.as_deref(), Some("Steins;Gate"));

        // Pin the global note and verify it sorts above the newer unpinned note.
        let pinned = set_pinned(db.pool(), &device, global.note.meta.id, true)
            .await
            .expect("pin");
        assert!(pinned.note.pinned);

        let all = list(db.pool(), None, None, false).await.expect("list all");
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].note.meta.id, global.note.meta.id);
        assert_eq!(all[1].note.meta.id, linked.note.meta.id);

        // Search by body text or game title.
        let search_body = list(db.pool(), Some("Chaos;Head"), None, false)
            .await
            .expect("search body");
        assert_eq!(search_body.len(), 1);
        assert_eq!(search_body[0].note.meta.id, global.note.meta.id);

        let search_game = list(db.pool(), Some("Steins;Gate"), None, false)
            .await
            .expect("search game");
        assert_eq!(search_game.len(), 2);

        // Filter by global_only and by game_id.
        let only_global = list(db.pool(), None, None, true)
            .await
            .expect("global only");
        assert_eq!(only_global.len(), 1);
        assert_eq!(only_global[0].note.meta.id, global.note.meta.id);

        let only_game = list(db.pool(), None, Some(game_id), false)
            .await
            .expect("game only");
        assert_eq!(only_game.len(), 1);
        assert_eq!(only_game[0].note.meta.id, linked.note.meta.id);

        // Edit and then soft-delete.
        let edited = save(
            db.pool(),
            &device,
            SaveGlobalNoteInput {
                id: Some(linked.note.meta.id),
                expected_updated_at: Some(linked.note.meta.updated_at.as_millis()),
                game_id: Some(game_id),
                title: "Divergence Meter".to_owned(),
                body: "Updated worldline notes.".to_owned(),
                pinned: false,
            },
        )
        .await
        .expect("edit");
        assert_eq!(edited.note.title, "Divergence Meter");

        delete(
            db.pool(),
            &device,
            edited.note.meta.id,
            Some(edited.note.meta.updated_at.as_millis()),
        )
        .await
        .expect("delete");

        let remaining = list(db.pool(), None, None, false)
            .await
            .expect("after delete");
        assert_eq!(remaining.len(), 1);
        let with_tombstones = list_all_including_deleted(db.pool())
            .await
            .expect("including deleted");
        assert_eq!(with_tombstones.len(), 2);
        assert!(with_tombstones.iter().any(|n| n.meta.deleted));
    }
}
