//! `SQLite` persistence and live progress evaluation for custom reading goals.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use sqlx::{Executor, FromRow, Sqlite, SqlitePool};
use vn_core::goals::{GoalKind, GoalPeriod, ReadingGoal, validate_reading_goal};
use vn_core::model::{DeviceTag, SyncMeta};
use vn_core::{DeviceId, EntityId, Timestamp};

use crate::commands::{CommandError, CommandResult};
use crate::db::pool::DbError;
use crate::db::queries::journey::finish_ts_sql;

/// Goal row hydrated with its live progress value for the UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadingGoalView {
    pub goal: ReadingGoal,
    /// Current progress in the same unit as `goal.target_value`:
    /// minutes for `playtime_minutes`, count for `vns_completed` and `manual`.
    pub current_value: i64,
}

/// Payload for creating or updating a reading goal.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveReadingGoalInput {
    pub id: Option<EntityId>,
    pub title: String,
    pub kind: GoalKind,
    pub period: GoalPeriod,
    pub target_value: i64,
    #[serde(default)]
    pub manual_progress: i64,
    pub unit: Option<String>,
    #[serde(default)]
    pub pinned: bool,
    pub expected_updated_at: Option<i64>,
}

#[derive(Debug, FromRow)]
struct ReadingGoalRow {
    id: String,
    title: String,
    kind: String,
    period: String,
    target_value: i64,
    manual_progress: i64,
    unit: Option<String>,
    pinned: bool,
    created_at: i64,
    updated_at: i64,
    device_id: String,
    deleted: bool,
}

impl TryFrom<ReadingGoalRow> for ReadingGoal {
    type Error = DbError;

    fn try_from(row: ReadingGoalRow) -> Result<Self, Self::Error> {
        Ok(Self {
            meta: SyncMeta {
                id: EntityId::parse(&row.id).map_err(|e| DbError::Corrupt(e.to_string()))?,
                updated_at: Timestamp::from_millis(row.updated_at),
                device_id_device: DeviceTag::new(
                    &DeviceId::parse(&row.device_id)
                        .map_err(|e| DbError::Corrupt(e.to_string()))?,
                ),
                deleted: row.deleted,
            },
            title: row.title,
            kind: GoalKind::parse(&row.kind).map_err(DbError::Corrupt)?,
            period: GoalPeriod::parse(&row.period).map_err(DbError::Corrupt)?,
            target_value: row.target_value,
            manual_progress: row.manual_progress,
            unit: row.unit,
            pinned: row.pinned,
            created_at: Timestamp::from_millis(row.created_at),
        })
    }
}

fn period_predicate(column: &str, period: GoalPeriod) -> String {
    match period {
        GoalPeriod::Daily => {
            format!("date({column} / 1000, 'unixepoch', 'localtime') = date('now', 'localtime')")
        }
        GoalPeriod::Weekly => format!(
            "date({column} / 1000, 'unixepoch', 'localtime') >= \
             date('now', 'localtime', '-' || ((CAST(strftime('%w', 'now', 'localtime') AS INTEGER) + 6) % 7) || ' days') \
             AND date({column} / 1000, 'unixepoch', 'localtime') < \
             date('now', 'localtime', '-' || ((CAST(strftime('%w', 'now', 'localtime') AS INTEGER) + 6) % 7) || ' days', '+7 days')"
        ),
        GoalPeriod::Monthly => format!(
            "strftime('%Y-%m', {column} / 1000, 'unixepoch', 'localtime') = strftime('%Y-%m', 'now', 'localtime')"
        ),
        GoalPeriod::Yearly => format!(
            "strftime('%Y', {column} / 1000, 'unixepoch', 'localtime') = strftime('%Y', 'now', 'localtime')"
        ),
        GoalPeriod::AllTime => "1 = 1".to_owned(),
    }
}

async fn playtime_minutes_for_period(
    pool: &SqlitePool,
    period: GoalPeriod,
) -> Result<i64, DbError> {
    if period == GoalPeriod::AllTime {
        let total_ms: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(MAX(0, COALESCE(s.ms, 0) + g.manual_playtime_ms)), 0) \
             FROM games g \
             LEFT JOIN ( \
                 SELECT game_id, SUM(counted_ms) AS ms \
                 FROM play_sessions \
                 WHERE deleted = 0 AND counted_ms > 0 \
                 GROUP BY game_id \
             ) s ON s.game_id = g.id \
             WHERE g.deleted = 0",
        )
        .fetch_one(pool)
        .await?;
        return Ok(total_ms / 60_000);
    }

    let predicate = period_predicate("ps.started_at", period);
    let total_ms: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT COALESCE(SUM(ps.counted_ms), 0) \
         FROM play_sessions ps \
         JOIN games g ON g.id = ps.game_id AND g.deleted = 0 \
         WHERE ps.deleted = 0 AND ps.counted_ms > 0 AND {predicate}"
    )))
    .fetch_one(pool)
    .await?;
    Ok(total_ms / 60_000)
}

async fn completed_vns_for_period(pool: &SqlitePool, period: GoalPeriod) -> Result<i64, DbError> {
    let finish_ts = finish_ts_sql("g2");
    if period == GoalPeriod::AllTime {
        let count: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT COUNT(*) \
             FROM games g \
             JOIN ( \
                 SELECT g2.id AS id, {finish_ts} AS finish_ts \
                 FROM games g2 WHERE g2.deleted = 0 \
             ) f ON f.id = g.id \
             WHERE g.deleted = 0 AND (g.status = 'completed' OR f.finish_ts IS NOT NULL)"
        )))
        .fetch_one(pool)
        .await?;
        return Ok(count);
    }

    let predicate = period_predicate("f.finish_ts", period);
    let count: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT COUNT(*) \
         FROM games g \
         JOIN ( \
             SELECT g2.id AS id, {finish_ts} AS finish_ts \
             FROM games g2 WHERE g2.deleted = 0 \
         ) f ON f.id = g.id \
         WHERE g.deleted = 0 AND f.finish_ts IS NOT NULL AND {predicate}"
    )))
    .fetch_one(pool)
    .await?;
    Ok(count)
}

async fn compute_progress(
    pool: &SqlitePool,
    goal: &ReadingGoal,
    cache: &mut HashMap<(GoalKind, GoalPeriod), i64>,
) -> Result<i64, DbError> {
    if goal.kind == GoalKind::Manual {
        return Ok(goal.manual_progress);
    }
    let key = (goal.kind, goal.period);
    if let Some(&cached) = cache.get(&key) {
        return Ok(cached);
    }
    let value = match goal.kind {
        GoalKind::PlaytimeMinutes => playtime_minutes_for_period(pool, goal.period).await?,
        GoalKind::VnsCompleted => completed_vns_for_period(pool, goal.period).await?,
        GoalKind::Manual => goal.manual_progress,
    };
    cache.insert(key, value);
    Ok(value)
}

/// List live reading goals ordered by pinned first, then most recently updated.
///
/// # Errors
/// Returns [`DbError`] on query failure or corrupt stored rows.
pub async fn list(pool: &SqlitePool) -> Result<Vec<ReadingGoalView>, DbError> {
    let rows: Vec<ReadingGoalRow> = sqlx::query_as(
        "SELECT id, title, kind, period, target_value, manual_progress, unit, \
                pinned, created_at, updated_at, device_id, deleted \
         FROM reading_goals \
         WHERE deleted = 0 \
         ORDER BY pinned DESC, updated_at DESC, id DESC",
    )
    .fetch_all(pool)
    .await?;

    let mut cache = HashMap::new();
    let mut views = Vec::with_capacity(rows.len());
    for row in rows {
        let goal = ReadingGoal::try_from(row)?;
        let current_value = compute_progress(pool, &goal, &mut cache).await?;
        views.push(ReadingGoalView {
            goal,
            current_value,
        });
    }
    Ok(views)
}

/// Count live automatic goals whose computed progress reached their target.
///
/// Reuses [`list`]'s live progress definitions so achievements and the goals
/// screen never disagree about what "hit" means. Manual goals are excluded:
/// their progress is user-entered, not an automatic metric.
///
/// # Errors
/// Returns [`DbError`] on query failure or corrupt stored rows.
pub async fn automatic_goal_hits(pool: &SqlitePool) -> Result<i64, DbError> {
    let views = list(pool).await?;
    let hits = views
        .iter()
        .filter(|view| {
            view.goal.kind != GoalKind::Manual && view.current_value >= view.goal.target_value
        })
        .count();
    Ok(i64::try_from(hits).unwrap_or(i64::MAX))
}

/// Return all reading goals, including tombstones, for sync and backup export.
///
/// # Errors
/// Returns [`DbError`] on query failure or corrupt rows.
pub async fn list_all_including_deleted<'e, E>(executor: E) -> Result<Vec<ReadingGoal>, DbError>
where
    E: Executor<'e, Database = Sqlite>,
{
    let rows: Vec<ReadingGoalRow> = sqlx::query_as(
        "SELECT id, title, kind, period, target_value, manual_progress, unit, \
                pinned, created_at, updated_at, device_id, deleted \
         FROM reading_goals \
         ORDER BY created_at ASC, id ASC",
    )
    .fetch_all(executor)
    .await?;

    rows.into_iter().map(ReadingGoal::try_from).collect()
}

/// Insert or replace a reading goal row (used by save, sync, and backup import).
///
/// # Errors
/// Returns [`DbError`] on query failure.
pub async fn upsert<'e, E>(executor: E, goal: &ReadingGoal) -> Result<(), DbError>
where
    E: Executor<'e, Database = Sqlite>,
{
    sqlx::query(
        "INSERT INTO reading_goals \
            (id, title, kind, period, target_value, manual_progress, unit, \
             pinned, created_at, updated_at, device_id, deleted) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT(id) DO UPDATE SET \
            title = excluded.title, \
            kind = excluded.kind, \
            period = excluded.period, \
            target_value = excluded.target_value, \
            manual_progress = excluded.manual_progress, \
            unit = excluded.unit, \
            pinned = excluded.pinned, \
            created_at = excluded.created_at, \
            updated_at = excluded.updated_at, \
            device_id = excluded.device_id, \
            deleted = excluded.deleted",
    )
    .bind(goal.meta.id.to_string())
    .bind(&goal.title)
    .bind(goal.kind.as_str())
    .bind(goal.period.as_str())
    .bind(goal.target_value)
    .bind(goal.manual_progress)
    .bind(&goal.unit)
    .bind(goal.pinned)
    .bind(goal.created_at.as_millis())
    .bind(goal.meta.updated_at.as_millis())
    .bind(goal.meta.device_id_device.0.as_str())
    .bind(goal.meta.deleted)
    .execute(executor)
    .await?;
    Ok(())
}

async fn load_live(pool: &SqlitePool, id: EntityId) -> Result<Option<ReadingGoal>, DbError> {
    let row: Option<ReadingGoalRow> = sqlx::query_as(
        "SELECT id, title, kind, period, target_value, manual_progress, unit, \
                pinned, created_at, updated_at, device_id, deleted \
         FROM reading_goals WHERE id = ? AND deleted = 0",
    )
    .bind(id.to_string())
    .fetch_optional(pool)
    .await?;
    row.map(ReadingGoal::try_from).transpose()
}

/// Create or update a reading goal and return its hydrated view.
///
/// # Errors
/// Returns [`CommandError`] on validation failure, stale revision, missing id, or db error.
pub async fn save(
    pool: &SqlitePool,
    device_id: &DeviceId,
    input: SaveReadingGoalInput,
) -> CommandResult<ReadingGoalView> {
    let now = Timestamp::now()?;
    let mut goal = if let Some(id) = input.id {
        let Some(existing) = load_live(pool, id).await? else {
            return Err(CommandError::new("not_found", "Reading goal not found"));
        };
        if let Some(expected) = input.expected_updated_at
            && existing.meta.updated_at.as_millis() != expected
        {
            return Err(CommandError::new(
                "conflict",
                "Reading goal was updated elsewhere",
            ));
        }
        let next_ms = now
            .as_millis()
            .max(existing.meta.updated_at.as_millis() + 1);
        ReadingGoal {
            meta: SyncMeta {
                id,
                updated_at: Timestamp::from_millis(next_ms),
                device_id_device: DeviceTag::new(device_id),
                deleted: false,
            },
            title: input.title,
            kind: input.kind,
            period: input.period,
            target_value: input.target_value,
            manual_progress: input.manual_progress,
            unit: input.unit,
            pinned: input.pinned,
            created_at: existing.created_at,
        }
    } else {
        ReadingGoal {
            meta: SyncMeta::new(device_id, now),
            title: input.title,
            kind: input.kind,
            period: input.period,
            target_value: input.target_value,
            manual_progress: input.manual_progress,
            unit: input.unit,
            pinned: input.pinned,
            created_at: now,
        }
    };

    validate_reading_goal(&mut goal).map_err(|err| CommandError::new("invalid_goal", err))?;
    upsert(pool, &goal).await?;

    let mut cache = HashMap::new();
    let current_value = compute_progress(pool, &goal, &mut cache).await?;
    Ok(ReadingGoalView {
        goal,
        current_value,
    })
}

/// Toggle the pinned flag on a live reading goal.
///
/// # Errors
/// Returns [`CommandError`] if the goal does not exist or was deleted.
pub async fn set_pinned(
    pool: &SqlitePool,
    device_id: &DeviceId,
    id: EntityId,
    pinned: bool,
) -> CommandResult<ReadingGoalView> {
    let Some(mut goal) = load_live(pool, id).await? else {
        return Err(CommandError::new("not_found", "Reading goal not found"));
    };
    let now = Timestamp::now()?;
    let next_ms = now.as_millis().max(goal.meta.updated_at.as_millis() + 1);
    goal.pinned = pinned;
    goal.meta.updated_at = Timestamp::from_millis(next_ms);
    goal.meta.device_id_device = DeviceTag::new(device_id);
    upsert(pool, &goal).await?;

    let mut cache = HashMap::new();
    let current_value = compute_progress(pool, &goal, &mut cache).await?;
    Ok(ReadingGoalView {
        goal,
        current_value,
    })
}

/// Update `manual_progress` on a live manual reading goal.
///
/// # Errors
/// Returns [`CommandError`] if missing, not `Manual`, or out of bounds.
pub async fn set_manual_progress(
    pool: &SqlitePool,
    device_id: &DeviceId,
    id: EntityId,
    progress: i64,
) -> CommandResult<ReadingGoalView> {
    let Some(mut goal) = load_live(pool, id).await? else {
        return Err(CommandError::new("not_found", "Reading goal not found"));
    };
    if goal.kind != GoalKind::Manual {
        return Err(CommandError::new(
            "invalid_goal",
            "Progress can only be edited on manual goals",
        ));
    }
    let now = Timestamp::now()?;
    let next_ms = now.as_millis().max(goal.meta.updated_at.as_millis() + 1);
    goal.manual_progress = progress;
    goal.meta.updated_at = Timestamp::from_millis(next_ms);
    goal.meta.device_id_device = DeviceTag::new(device_id);
    validate_reading_goal(&mut goal).map_err(|err| CommandError::new("invalid_goal", err))?;
    upsert(pool, &goal).await?;
    let current_value = goal.manual_progress;
    Ok(ReadingGoalView {
        goal,
        current_value,
    })
}

/// Soft-delete a reading goal so folder sync propagates the tombstone.
///
/// # Errors
/// Returns [`CommandError`] if the goal does not exist or is already deleted.
pub async fn delete(pool: &SqlitePool, device_id: &DeviceId, id: EntityId) -> CommandResult<()> {
    let Some(mut goal) = load_live(pool, id).await? else {
        return Err(CommandError::new("not_found", "Reading goal not found"));
    };
    let now = Timestamp::now()?;
    let next_ms = now.as_millis().max(goal.meta.updated_at.as_millis() + 1);
    goal.meta.deleted = true;
    goal.meta.updated_at = Timestamp::from_millis(next_ms);
    goal.meta.device_id_device = DeviceTag::new(device_id);
    upsert(pool, &goal).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{Db, queries};
    use vn_core::model::{Game, GameStatus, PlaySession, SessionSource};

    #[tokio::test]
    async fn goals_auto_playtime_vns_and_manual_crud() {
        let db = Db::open_memory().await.expect("in-memory db");
        let dev = queries::ensure_device_id(db.pool()).await.expect("device");
        let now = Timestamp::now().expect("now");

        // Insert a live completed game with a 90-minute session today.
        let mut game = Game::new(&dev, now, "Clannad", GameStatus::Completed).expect("game");
        game.finished_at = Some(now);
        queries::upsert_game(db.pool(), &game).await.expect("save");

        let started = Timestamp::from_millis(now.as_millis() - 5_400_000);
        let session = PlaySession::new(
            &dev,
            now,
            game.meta.id,
            started,
            now,
            5_400_000,
            SessionSource::Manual,
        )
        .expect("session");
        queries::insert_session(db.pool(), &session)
            .await
            .expect("session");

        // 1. Daily playtime goal (target 120m -> current 90m).
        let daily = save(
            db.pool(),
            &dev,
            SaveReadingGoalInput {
                id: None,
                title: "2 jam sehari".to_owned(),
                kind: GoalKind::PlaytimeMinutes,
                period: GoalPeriod::Daily,
                target_value: 120,
                manual_progress: 0,
                unit: None,
                pinned: false,
                expected_updated_at: None,
            },
        )
        .await
        .expect("daily goal");
        assert_eq!(daily.current_value, 90);

        // 2. Yearly VN goal (target 10 -> current 1).
        let yearly = save(
            db.pool(),
            &dev,
            SaveReadingGoalInput {
                id: None,
                title: "10 VN tahun ini".to_owned(),
                kind: GoalKind::VnsCompleted,
                period: GoalPeriod::Yearly,
                target_value: 10,
                manual_progress: 0,
                unit: None,
                pinned: false,
                expected_updated_at: None,
            },
        )
        .await
        .expect("yearly goal");
        assert_eq!(yearly.current_value, 1);

        // 3. Manual goal with pin + progress step + delete.
        let manual = save(
            db.pool(),
            &dev,
            SaveReadingGoalInput {
                id: None,
                title: "Route Key".to_owned(),
                kind: GoalKind::Manual,
                period: GoalPeriod::AllTime,
                target_value: 5,
                manual_progress: 2,
                unit: Some("route".to_owned()),
                pinned: false,
                expected_updated_at: None,
            },
        )
        .await
        .expect("manual goal");
        assert_eq!(manual.current_value, 2);

        let stepped = set_manual_progress(db.pool(), &dev, manual.goal.meta.id, 4)
            .await
            .expect("step");
        assert_eq!(stepped.current_value, 4);

        let pinned = set_pinned(db.pool(), &dev, manual.goal.meta.id, true)
            .await
            .expect("pin");
        assert!(pinned.goal.pinned);

        let all = list(db.pool()).await.expect("list");
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].goal.meta.id, manual.goal.meta.id);

        delete(db.pool(), &dev, manual.goal.meta.id)
            .await
            .expect("delete");
        assert_eq!(list(db.pool()).await.expect("list after delete").len(), 2);
        assert_eq!(
            list_all_including_deleted(db.pool())
                .await
                .expect("including deleted")
                .len(),
            3
        );
    }
}
