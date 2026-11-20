//! `SQLite` aggregates and persistence for the achievement catalog.
//!
//! One shared [`AchievementInput`] is built from bounded aggregate queries
//! (local-day buckets, live-parent joins, `finish_ts_sql`) and evaluated by the
//! pure `vn_core::achievements` rules. Every child aggregate joins its live
//! parent game (AGENTS.md §5 orphan rule); manual playtime corrections enter
//! the all-time total only, never the daily/streak buckets.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use sqlx::{Executor, FromRow, Sqlite, SqlitePool};
use vn_core::achievements::{
    self, AchievementDef, AchievementInput, AchievementUnlock, Tier, evaluate, unlocked_at_for,
};
use vn_core::model::{DeviceTag, SyncMeta};
use vn_core::{DeviceId, EntityId, Timestamp};

use crate::db::pool::DbError;
use crate::db::queries;
use crate::db::queries::journey::finish_ts_sql;
use crate::vndb::dto;

/// Comeback gap threshold: 30 days in milliseconds.
const COMEBACK_GAP_MS: i64 = 30 * 86_400_000;

/// One catalog item hydrated with its current progress and unlock state.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AchievementView {
    pub slug: String,
    pub tier: Tier,
    pub group: String,
    pub unlocked: bool,
    pub progress: i64,
    pub target: i64,
    /// UTC ms, present once unlocked (stored date wins over a fresh evaluation).
    pub unlocked_at: Option<i64>,
}

#[derive(Debug, FromRow)]
struct AchievementUnlockRow {
    id: String,
    achievement_id: String,
    unlocked_at: i64,
    updated_at: i64,
    device_id: String,
    deleted: bool,
}

impl TryFrom<AchievementUnlockRow> for AchievementUnlock {
    type Error = DbError;
    fn try_from(row: AchievementUnlockRow) -> Result<Self, Self::Error> {
        if achievements::def(&row.achievement_id).is_none() {
            return Err(DbError::Corrupt(format!(
                "unknown achievement slug: {}",
                row.achievement_id
            )));
        }
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
            achievement_id: row.achievement_id,
            unlocked_at: Timestamp::from_millis(row.unlocked_at),
        })
    }
}

/// Build the shared aggregates every predicate reads.
///
/// # Errors
/// Returns [`DbError`] on query failure or corrupt stored rows.
pub async fn build_input(pool: &SqlitePool) -> Result<AchievementInput, DbError> {
    let mut input = AchievementInput::default();
    session_aggregates(pool, &mut input).await?;
    collection_aggregates(pool, &mut input).await?;
    route_ending_aggregates(pool, &mut input).await?;
    journal_goal_aggregates(pool, &mut input).await?;
    capture_walkthrough_aggregates(pool, &mut input).await?;
    marker_vndb_aggregates(pool, &mut input).await?;
    sync_aggregates(pool, &mut input).await?;
    Ok(input)
}

/// Counted-session totals, gaps, local-day buckets and time-of-day windows.
async fn session_aggregates(
    pool: &SqlitePool,
    input: &mut AchievementInput,
) -> Result<(), DbError> {
    // All-time counted playtime including manual corrections, floored per game
    // (the same definition the all-time reading goal uses).
    input.total_playtime_ms = sqlx::query_scalar(
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

    // Counted-session stats over live games.
    let (session_count, first_session, longest_session): (i64, Option<i64>, i64) = sqlx::query_as(
        "SELECT COUNT(*), MIN(s.started_at), COALESCE(MAX(s.counted_ms), 0) \
         FROM play_sessions s JOIN games g ON g.id = s.game_id AND g.deleted = 0 \
         WHERE s.deleted = 0 AND s.counted_ms > 0",
    )
    .fetch_one(pool)
    .await?;
    input.session_count = session_count;
    input.first_session_at = first_session.map(Timestamp::from_millis);
    input.longest_session_ms = longest_session;

    // Comeback: largest gap between consecutive counted sessions (previous
    // ended_at to next started_at) and the first return after a >= 30 day gap.
    let (max_gap_ms, comeback_at): (i64, Option<i64>) = sqlx::query_as(
        "SELECT COALESCE(MAX(gap_ms), 0), \
                MIN(CASE WHEN gap_ms >= ? THEN started_at END) \
         FROM ( \
             SELECT s.started_at AS started_at, \
                    s.started_at - LAG(s.ended_at) OVER (ORDER BY s.started_at, s.id) AS gap_ms \
             FROM play_sessions s JOIN games g ON g.id = s.game_id AND g.deleted = 0 \
             WHERE s.deleted = 0 AND s.counted_ms > 0 \
         )",
    )
    .bind(COMEBACK_GAP_MS)
    .fetch_one(pool)
    .await?;
    input.max_gap_days = (max_gap_ms / 86_400_000).max(0);
    input.comeback_at = comeback_at.map(Timestamp::from_millis);

    // Local-day buckets: the exact definition the Stats heatmap uses.
    input.day_playtime = queries::playtime_by_day(pool).await?;

    // Night (00:00-04:00) and morning (05:00-08:00) windows, attributed by the
    // session's local start hour, consistent with the daily bucket rule.
    let (night_ms, morning_ms): (i64, i64) = sqlx::query_as(
        "SELECT COALESCE(SUM(CASE WHEN h < 4 THEN counted_ms ELSE 0 END), 0), \
                COALESCE(SUM(CASE WHEN h >= 5 AND h < 8 THEN counted_ms ELSE 0 END), 0) \
         FROM ( \
             SELECT s.counted_ms AS counted_ms, \
                    CAST(strftime('%H', s.started_at / 1000, 'unixepoch', 'localtime') AS INTEGER) AS h \
             FROM play_sessions s JOIN games g ON g.id = s.game_id AND g.deleted = 0 \
             WHERE s.deleted = 0 AND s.counted_ms > 0 \
         )",
    )
    .fetch_one(pool)
    .await?;
    input.night_ms = night_ms;
    input.morning_ms = morning_ms;

    input.distinct_session_devices = sqlx::query_scalar(
        "SELECT COUNT(DISTINCT s.device_id) \
         FROM play_sessions s JOIN games g ON g.id = s.game_id AND g.deleted = 0 \
         WHERE s.deleted = 0 AND s.counted_ms > 0",
    )
    .fetch_one(pool)
    .await?;
    Ok(())
}

/// Collection counts, ratings and finish timestamps.
async fn collection_aggregates(
    pool: &SqlitePool,
    input: &mut AchievementInput,
) -> Result<(), DbError> {
    // The finish timestamp has one SQL definition shared with the timeline,
    // year review and goals.
    let finish_ts = finish_ts_sql("g2");
    let (
        library_count,
        wishlist_count,
        rated_count,
        perfect_rating_count,
        finished_count,
        first_finish,
        long_finish_count,
        first_long_finish,
    ): (i64, i64, i64, i64, i64, Option<i64>, i64, Option<i64>) =
        sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT COUNT(*), \
                    COALESCE(SUM(CASE WHEN g.status = 'wishlist' THEN 1 ELSE 0 END), 0), \
                    COALESCE(SUM(CASE WHEN g.rating IS NOT NULL THEN 1 ELSE 0 END), 0), \
                    COALESCE(SUM(CASE WHEN g.rating = 100 THEN 1 ELSE 0 END), 0), \
                    COALESCE(SUM(CASE WHEN f.finish_ts IS NOT NULL THEN 1 ELSE 0 END), 0), \
                    MIN(f.finish_ts), \
                    COALESCE(SUM(CASE WHEN f.finish_ts IS NOT NULL AND g.length_minutes >= 3000 THEN 1 ELSE 0 END), 0), \
                    MIN(CASE WHEN f.finish_ts IS NOT NULL AND g.length_minutes >= 3000 THEN f.finish_ts END) \
             FROM games g \
             JOIN ( \
                 SELECT g2.id AS id, {finish_ts} AS finish_ts \
                 FROM games g2 WHERE g2.deleted = 0 \
             ) f ON f.id = g.id \
             WHERE g.deleted = 0"
        )))
        .fetch_one(pool)
        .await?;
    input.library_count = library_count;
    input.wishlist_count = wishlist_count;
    input.rated_count = rated_count;
    input.perfect_rating_count = perfect_rating_count;
    input.finished_count = finished_count;
    input.first_finish_at = first_finish.map(Timestamp::from_millis);
    input.long_finish_count = long_finish_count;
    input.first_long_finish_at = first_long_finish.map(Timestamp::from_millis);
    Ok(())
}

/// Routes, per-route playtime, endings and reached chapters.
async fn route_ending_aggregates(
    pool: &SqlitePool,
    input: &mut AchievementInput,
) -> Result<(), DbError> {
    input.routes_count = sqlx::query_scalar(
        "SELECT COUNT(*) FROM routes r \
         JOIN games g ON g.id = r.game_id AND g.deleted = 0 \
         WHERE r.deleted = 0",
    )
    .fetch_one(pool)
    .await?;

    // Counted time per route: a session attributes to its own route_id, or to
    // its chapter's route when only a chapter was recorded. Routes and their
    // games must both be live.
    input.max_route_ms = sqlx::query_scalar(
        "SELECT COALESCE(MAX(total_ms), 0) \
         FROM ( \
             SELECT COALESCE(SUM(sr.counted_ms), 0) AS total_ms \
             FROM ( \
                 SELECT s.counted_ms AS counted_ms, COALESCE(s.route_id, c.route_id) AS route_id \
                 FROM play_sessions s \
                 JOIN games g ON g.id = s.game_id AND g.deleted = 0 \
                 LEFT JOIN chapters c ON c.id = s.chapter_id AND c.deleted = 0 \
                 WHERE s.deleted = 0 AND s.counted_ms > 0 \
             ) sr \
             JOIN routes r ON r.id = sr.route_id AND r.deleted = 0 \
             JOIN games g2 ON g2.id = r.game_id AND g2.deleted = 0 \
             GROUP BY sr.route_id \
         )",
    )
    .fetch_one(pool)
    .await?;

    let (endings_total, first_ending): (i64, Option<i64>) = sqlx::query_as(
        "SELECT COUNT(*), MIN(e.achieved_at) \
         FROM endings e JOIN games g ON g.id = e.game_id AND g.deleted = 0 \
         WHERE e.deleted = 0 AND e.achieved = 1",
    )
    .fetch_one(pool)
    .await?;
    input.endings_achieved_total = endings_total;
    input.first_ending_at = first_ending.map(Timestamp::from_millis);

    input.endings_max_per_game = sqlx::query_scalar(
        "SELECT COALESCE(MAX(cnt), 0) \
         FROM ( \
             SELECT COUNT(*) AS cnt \
             FROM endings e JOIN games g ON g.id = e.game_id AND g.deleted = 0 \
             WHERE e.deleted = 0 AND e.achieved = 1 \
             GROUP BY e.game_id \
         )",
    )
    .fetch_one(pool)
    .await?;

    input.chapters_reached = sqlx::query_scalar(
        "SELECT COUNT(*) FROM chapters c \
         JOIN games g ON g.id = c.game_id AND g.deleted = 0 \
         WHERE c.deleted = 0 AND c.reached = 1",
    )
    .fetch_one(pool)
    .await?;
    Ok(())
}

/// Journal entries, global notes and reading goals.
async fn journal_goal_aggregates(
    pool: &SqlitePool,
    input: &mut AchievementInput,
) -> Result<(), DbError> {
    let (journal_count, first_journal, save_slot_count): (i64, Option<i64>, i64) = sqlx::query_as(
        "SELECT COUNT(*), MIN(e.created_at), \
                COALESCE(SUM(CASE WHEN trim(e.save_slot, char(9, 10, 13, 32)) <> '' THEN 1 ELSE 0 END), 0) \
         FROM reading_entries e JOIN games g ON g.id = e.game_id AND g.deleted = 0 \
         WHERE e.deleted = 0",
    )
    .fetch_one(pool)
    .await?;
    input.journal_count = journal_count;
    input.first_journal_at = first_journal.map(Timestamp::from_millis);
    input.save_slot_count = save_slot_count;

    let journal_days: Vec<(i64,)> = sqlx::query_as(
        "SELECT DISTINCT CAST(strftime('%s', e.created_at / 1000, 'unixepoch', 'localtime', 'start of day') AS INTEGER) / 86400 \
         FROM reading_entries e JOIN games g ON g.id = e.game_id AND g.deleted = 0 \
         WHERE e.deleted = 0 ORDER BY 1 ASC",
    )
    .fetch_all(pool)
    .await?;
    input.journal_days = journal_days.into_iter().map(|(day,)| day).collect();

    let (global_note_count, first_global_note): (i64, Option<i64>) =
        sqlx::query_as("SELECT COUNT(*), MIN(created_at) FROM global_notes WHERE deleted = 0")
            .fetch_one(pool)
            .await?;
    input.global_note_count = global_note_count;
    input.first_global_note_at = first_global_note.map(Timestamp::from_millis);

    let (goal_count, first_goal): (i64, Option<i64>) =
        sqlx::query_as("SELECT COUNT(*), MIN(created_at) FROM reading_goals WHERE deleted = 0")
            .fetch_one(pool)
            .await?;
    input.goal_count = goal_count;
    input.first_goal_at = first_goal.map(Timestamp::from_millis);
    input.goal_hit_count = crate::db::goals::automatic_goal_hits(pool).await?;
    Ok(())
}

/// Game screenshots and walkthrough activity.
async fn capture_walkthrough_aggregates(
    pool: &SqlitePool,
    input: &mut AchievementInput,
) -> Result<(), DbError> {
    let (screenshot_count, first_screenshot, caption_count, screenshot_game_count): (
        i64,
        Option<i64>,
        i64,
        i64,
    ) = sqlx::query_as(
        "SELECT COUNT(*), MIN(s.captured_at), \
                COALESCE(SUM(CASE WHEN trim(s.caption, char(9, 10, 13, 32)) <> '' THEN 1 ELSE 0 END), 0), \
                COUNT(DISTINCT s.game_id) \
         FROM game_screenshots s JOIN games g ON g.id = s.game_id AND g.deleted = 0 \
         WHERE s.deleted = 0",
    )
    .fetch_one(pool)
    .await?;
    input.screenshot_count = screenshot_count;
    input.first_screenshot_at = first_screenshot.map(Timestamp::from_millis);
    input.caption_count = caption_count;
    input.screenshot_game_count = screenshot_game_count;

    let (walkthrough_count, first_walkthrough): (i64, Option<i64>) = sqlx::query_as(
        "SELECT COUNT(*), MIN(w.created_at) \
         FROM game_walkthroughs w JOIN games g ON g.id = w.game_id AND g.deleted = 0",
    )
    .fetch_one(pool)
    .await?;
    input.walkthrough_count = walkthrough_count;
    input.first_walkthrough_at = first_walkthrough.map(Timestamp::from_millis);

    // Accumulated checked guide steps; malformed JSON degrades to empty.
    let checked_rows: Vec<(String,)> = sqlx::query_as(
        "SELECT w.checked_items_json \
         FROM game_walkthroughs w JOIN games g ON g.id = w.game_id AND g.deleted = 0",
    )
    .fetch_all(pool)
    .await?;
    let mut guide_checks: i64 = 0;
    for (json,) in checked_rows {
        let items: Vec<String> = serde_json::from_str(&json).unwrap_or_default();
        guide_checks = guide_checks.saturating_add(i64::try_from(items.len()).unwrap_or(0));
    }
    input.guide_checks = guide_checks;

    // Manual translations are stored with provider 'manual' (the view keys
    // manual_en/manual_id combine provider and target language).
    let (manual_translation_count, first_manual_translation): (i64, Option<i64>) = sqlx::query_as(
        "SELECT COUNT(*), MIN(t.updated_at) \
         FROM walkthrough_translations t JOIN games g ON g.id = t.game_id AND g.deleted = 0 \
         WHERE t.provider = 'manual'",
    )
    .fetch_one(pool)
    .await?;
    input.manual_translation_count = manual_translation_count;
    input.first_manual_translation_at = first_manual_translation.map(Timestamp::from_millis);
    Ok(())
}

/// Event markers and VNDB studio/tag breadth across played games.
async fn marker_vndb_aggregates(
    pool: &SqlitePool,
    input: &mut AchievementInput,
) -> Result<(), DbError> {
    // Event markers written by the export/import commands.
    let markers: Vec<(String, String)> = sqlx::query_as(
        "SELECT key, value FROM local_prefs \
         WHERE key IN ('achievements.discoverImported', 'achievements.backupMade')",
    )
    .fetch_all(pool)
    .await?;
    for (key, value) in markers {
        let Ok(ms) = value.parse::<i64>() else {
            continue;
        };
        match key.as_str() {
            "achievements.discoverImported" => {
                input.discover_imported_at = Some(Timestamp::from_millis(ms));
            }
            "achievements.backupMade" => input.backup_made_at = Some(Timestamp::from_millis(ms)),
            _ => {}
        }
    }

    // Studios and tags across played games (a counted session is the evidence
    // of play). Cached VNDB JSON degrades to empty when missing or malformed.
    let detail_rows: Vec<(Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT d.developers_json, d.tags_json \
         FROM games g LEFT JOIN vndb_details d ON d.vn_id = g.vndb_id \
         WHERE g.deleted = 0 \
           AND EXISTS ( \
               SELECT 1 FROM play_sessions s \
               WHERE s.game_id = g.id AND s.deleted = 0 AND s.counted_ms > 0 \
           )",
    )
    .fetch_all(pool)
    .await?;
    let mut studios: HashSet<String> = HashSet::new();
    let mut tags: HashSet<String> = HashSet::new();
    for (developers_json, tags_json) in detail_rows {
        let developers: Vec<dto::Producer> = developers_json
            .as_deref()
            .and_then(|text| serde_json::from_str(text).ok())
            .unwrap_or_default();
        for producer in developers {
            if let Some(id) = producer.id {
                studios.insert(id);
            }
        }
        let vn_tags: Vec<dto::Tag> = tags_json
            .as_deref()
            .and_then(|text| serde_json::from_str(text).ok())
            .unwrap_or_default();
        for tag in vn_tags {
            // Non-spoiler and not a lie; rows cached before the field existed
            // decode as None = safe.
            if tag.spoiler.unwrap_or(0) != 0 || tag.lie.unwrap_or(false) {
                continue;
            }
            if let Some(key) = tag.id.or(tag.name) {
                tags.insert(key);
            }
        }
    }
    input.studio_count = i64::try_from(studios.len()).unwrap_or(i64::MAX);
    input.tag_count = i64::try_from(tags.len()).unwrap_or(i64::MAX);
    Ok(())
}

/// Successful folder-sync runs.
async fn sync_aggregates(pool: &SqlitePool, input: &mut AchievementInput) -> Result<(), DbError> {
    let (sync_ok_count, first_sync_ok): (i64, Option<i64>) =
        sqlx::query_as("SELECT COUNT(*), MIN(finished_at) FROM sync_runs WHERE outcome = 'ok'")
            .fetch_one(pool)
            .await?;
    input.sync_ok_count = sync_ok_count;
    input.first_sync_ok_at = first_sync_ok.map(Timestamp::from_millis);
    Ok(())
}

/// Live unlock rows keyed by slug.
async fn stored_unlocks(pool: &SqlitePool) -> Result<HashMap<String, i64>, DbError> {
    let rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT achievement_id, unlocked_at FROM achievement_unlocks WHERE deleted = 0",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().collect())
}

/// Every catalog item with its current progress, for the achievements screen.
///
/// Locked items still show progress. An unlock already stored stays unlocked
/// even if the live aggregates later drop below the target (monotonic).
///
/// # Errors
/// Returns [`DbError`] on query failure or corrupt stored rows.
pub async fn list(pool: &SqlitePool) -> Result<Vec<AchievementView>, DbError> {
    let input = build_input(pool).await?;
    let now = Timestamp::now().map_err(|err| DbError::Corrupt(err.to_string()))?;
    let stored = stored_unlocks(pool).await?;
    let mut views = Vec::with_capacity(achievements::catalog().len());
    for (entry, eval) in achievements::catalog().iter().zip(evaluate(&input, now)) {
        let stored_at = stored.get(entry.slug).copied();
        let unlocked = eval.unlocked || stored_at.is_some();
        let unlocked_at = stored_at.or_else(|| {
            if eval.unlocked {
                unlocked_at_for(entry.slug, &input, now).map(Timestamp::as_millis)
            } else {
                None
            }
        });
        views.push(view_from(entry, &eval, unlocked, unlocked_at));
    }
    Ok(views)
}

fn view_from(
    entry: &AchievementDef,
    eval: &achievements::AchievementEval,
    unlocked: bool,
    unlocked_at: Option<i64>,
) -> AchievementView {
    AchievementView {
        slug: entry.slug.to_owned(),
        tier: entry.tier,
        group: entry.group.to_owned(),
        unlocked,
        progress: eval.progress,
        target: eval.target,
        unlocked_at,
    }
}

/// Return all unlock rows, including tombstones, for sync and backup export.
///
/// # Errors
/// Returns [`DbError`] on query failure or corrupt rows.
pub async fn list_all_including_deleted<'e, E>(
    executor: E,
) -> Result<Vec<AchievementUnlock>, DbError>
where
    E: Executor<'e, Database = Sqlite>,
{
    let rows: Vec<AchievementUnlockRow> = sqlx::query_as(
        "SELECT id, achievement_id, unlocked_at, updated_at, device_id, deleted \
         FROM achievement_unlocks ORDER BY achievement_id ASC",
    )
    .fetch_all(executor)
    .await?;
    rows.into_iter().map(AchievementUnlock::try_from).collect()
}

/// Insert or update an unlock row by its natural key (the slug).
///
/// Used by sync and backup import, where the merged winner must replace the
/// stored copy. The UUID travels with the row but the slug is the identity.
///
/// # Errors
/// Returns [`DbError`] on query failure.
pub async fn upsert<'e, E>(executor: E, unlock: &AchievementUnlock) -> Result<(), DbError>
where
    E: Executor<'e, Database = Sqlite>,
{
    sqlx::query(
        "INSERT INTO achievement_unlocks \
            (id, achievement_id, unlocked_at, updated_at, device_id, deleted) \
         VALUES (?, ?, ?, ?, ?, ?) \
         ON CONFLICT(achievement_id) DO UPDATE SET \
            id = excluded.id, \
            unlocked_at = excluded.unlocked_at, \
            updated_at = excluded.updated_at, \
            device_id = excluded.device_id, \
            deleted = excluded.deleted",
    )
    .bind(unlock.meta.id.to_string())
    .bind(&unlock.achievement_id)
    .bind(unlock.unlocked_at.as_millis())
    .bind(unlock.meta.updated_at.as_millis())
    .bind(unlock.meta.device_id_device.device().as_str())
    .bind(unlock.meta.deleted)
    .execute(executor)
    .await?;
    Ok(())
}

/// Insert an unlock only when its slug has no row yet; returns whether it wrote.
///
/// The evaluation path uses this instead of [`upsert`] so a concurrent scan can
/// never move an existing unlock's date forward.
///
/// # Errors
/// Returns [`DbError`] on query failure.
async fn insert_if_absent<'e, E>(executor: E, unlock: &AchievementUnlock) -> Result<bool, DbError>
where
    E: Executor<'e, Database = Sqlite>,
{
    let result = sqlx::query(
        "INSERT INTO achievement_unlocks \
            (id, achievement_id, unlocked_at, updated_at, device_id, deleted) \
         VALUES (?, ?, ?, ?, ?, ?) \
         ON CONFLICT(achievement_id) DO NOTHING",
    )
    .bind(unlock.meta.id.to_string())
    .bind(&unlock.achievement_id)
    .bind(unlock.unlocked_at.as_millis())
    .bind(unlock.meta.updated_at.as_millis())
    .bind(unlock.meta.device_id_device.device().as_str())
    .bind(unlock.meta.deleted)
    .execute(executor)
    .await?;
    Ok(result.rows_affected() > 0)
}

/// Evaluate the catalog and store only genuinely new unlocks.
///
/// Idempotent: an existing unlock is never rewritten, so its original date and
/// device survive every later scan. Returns the rows written by this call.
///
/// # Errors
/// Returns [`DbError`] on query failure.
pub async fn evaluate_and_store(
    pool: &SqlitePool,
    device: &DeviceId,
    now: Timestamp,
) -> Result<Vec<AchievementUnlock>, DbError> {
    let input = build_input(pool).await?;
    let existing: HashSet<String> = stored_unlocks(pool).await?.into_keys().collect();
    let mut fresh = Vec::new();
    for eval in evaluate(&input, now) {
        if !eval.unlocked || existing.contains(eval.slug) {
            continue;
        }
        let unlocked_at = unlocked_at_for(eval.slug, &input, now).unwrap_or(now);
        let unlock = AchievementUnlock {
            meta: SyncMeta::new(device, now),
            achievement_id: eval.slug.to_owned(),
            unlocked_at,
        };
        if insert_if_absent(pool, &unlock).await? {
            fresh.push(unlock);
        }
    }
    Ok(fresh)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;
    use vn_core::model::{Game, GameStatus, PlaySession, SessionSource};

    async fn seed_game(db: &Db, dev: &DeviceId, now: Timestamp, title: &str) -> Game {
        let game = Game::new(dev, now, title, GameStatus::Playing).expect("game");
        queries::upsert_game(db.pool(), &game)
            .await
            .expect("save game");
        game
    }

    async fn seed_session(
        db: &Db,
        dev: &DeviceId,
        game: &Game,
        started: Timestamp,
        counted_ms: i64,
    ) {
        let ended = Timestamp::from_millis(started.as_millis() + counted_ms);
        let session = PlaySession::new(
            dev,
            started,
            game.meta.id,
            started,
            ended,
            counted_ms,
            SessionSource::Manual,
        )
        .expect("session");
        queries::insert_session(db.pool(), &session)
            .await
            .expect("session");
    }

    fn slugs(unlocks: &[AchievementUnlock]) -> Vec<&str> {
        unlocks.iter().map(|u| u.achievement_id.as_str()).collect()
    }

    #[tokio::test]
    async fn a_90_minute_session_unlocks_first_session_but_not_day_2h() {
        let db = Db::open_memory().await.expect("db");
        let dev = queries::ensure_device_id(db.pool()).await.expect("device");
        let now = Timestamp::now().expect("now");
        let game = seed_game(&db, &dev, now, "Clannad").await;
        let started = Timestamp::from_millis(now.as_millis() - 5_400_000);
        seed_session(&db, &dev, &game, started, 5_400_000).await;

        let fresh = evaluate_and_store(db.pool(), &dev, now)
            .await
            .expect("scan");
        assert!(slugs(&fresh).contains(&"first-session"));
        assert!(
            !slugs(&fresh).contains(&"day-2h"),
            "90 minutes is under 2 hours"
        );

        let views = list(db.pool()).await.expect("list");
        assert_eq!(views.len(), 65);
        let first = views
            .iter()
            .find(|v| v.slug == "first-session")
            .expect("view");
        assert!(first.unlocked);
        assert_eq!(
            first.unlocked_at,
            Some(started.as_millis()),
            "event date is the session start"
        );
        let day = views.iter().find(|v| v.slug == "day-2h").expect("view");
        assert!(!day.unlocked);
        assert_eq!(day.progress, 5_400_000);
        assert_eq!(day.target, 2 * 3_600_000);
    }

    #[tokio::test]
    async fn manual_correction_counts_for_total_playtime_but_not_daily() {
        let db = Db::open_memory().await.expect("db");
        let dev = queries::ensure_device_id(db.pool()).await.expect("device");
        let now = Timestamp::now().expect("now");
        let mut game = seed_game(&db, &dev, now, "Ever17").await;
        // 1 hour counted today + a 9 hour manual correction = 10 hours total.
        game.manual_playtime_ms = 9 * 3_600_000;
        queries::upsert_game(db.pool(), &game)
            .await
            .expect("update");
        let started = Timestamp::from_millis(now.as_millis() - 3_600_000);
        seed_session(&db, &dev, &game, started, 3_600_000).await;

        let input = build_input(db.pool()).await.expect("input");
        assert_eq!(input.total_playtime_ms, 10 * 3_600_000);
        let fresh = evaluate_and_store(db.pool(), &dev, now)
            .await
            .expect("scan");
        assert!(
            slugs(&fresh).contains(&"playtime-10h"),
            "exact 10h boundary"
        );
        assert!(
            !slugs(&fresh).contains(&"day-2h"),
            "a correction has no date and must not enter the daily bucket"
        );
    }

    #[tokio::test]
    async fn three_consecutive_local_days_unlock_streak_3() {
        let db = Db::open_memory().await.expect("db");
        let dev = queries::ensure_device_id(db.pool()).await.expect("device");
        let now = Timestamp::now().expect("now");
        let game = seed_game(&db, &dev, now, "Kanon").await;
        for offset in [2, 1, 0] {
            let started = Timestamp::from_millis(now.as_millis() - offset * 86_400_000);
            seed_session(&db, &dev, &game, started, 600_000).await;
        }
        let fresh = evaluate_and_store(db.pool(), &dev, now)
            .await
            .expect("scan");
        assert!(slugs(&fresh).contains(&"streak-3"));
        assert!(!slugs(&fresh).contains(&"streak-7"));
    }

    #[tokio::test]
    async fn endings_ladder_counts_per_game_and_dates_the_first() {
        let db = Db::open_memory().await.expect("db");
        let dev = queries::ensure_device_id(db.pool()).await.expect("device");
        let now = Timestamp::now().expect("now");
        let game = seed_game(&db, &dev, now, "Air").await;
        let first_at = now.as_millis() - 3 * 86_400_000;
        for index in 0..3 {
            sqlx::query(
                "INSERT INTO endings (id, game_id, name, achieved, achieved_at, updated_at, device_id, deleted) \
                 VALUES (?, ?, ?, 1, ?, ?, ?, 0)",
            )
            .bind(format!("e{index}"))
            .bind(game.meta.id.to_string())
            .bind(format!("Ending {index}"))
            .bind(first_at + index * 1000)
            .bind(now.as_millis())
            .bind(dev.as_str())
            .execute(db.pool())
            .await
            .expect("ending");
        }
        let fresh = evaluate_and_store(db.pool(), &dev, now)
            .await
            .expect("scan");
        assert!(slugs(&fresh).contains(&"first-ending"));
        assert!(slugs(&fresh).contains(&"endings-vn-3"));
        assert!(!slugs(&fresh).contains(&"endings-vn-5"));
        assert!(!slugs(&fresh).contains(&"endings-10"));
        let first = fresh
            .iter()
            .find(|u| u.achievement_id == "first-ending")
            .expect("unlock");
        assert_eq!(first.unlocked_at.as_millis(), first_at);
    }

    #[tokio::test]
    async fn evaluate_and_store_is_idempotent_and_keeps_the_original_date() {
        let db = Db::open_memory().await.expect("db");
        let dev = queries::ensure_device_id(db.pool()).await.expect("device");
        let now = Timestamp::now().expect("now");
        let game = seed_game(&db, &dev, now, "Tomoyo").await;
        let started = Timestamp::from_millis(now.as_millis() - 600_000);
        seed_session(&db, &dev, &game, started, 600_000).await;

        let first = evaluate_and_store(db.pool(), &dev, now)
            .await
            .expect("first scan");
        assert!(!first.is_empty());
        let later = Timestamp::from_millis(now.as_millis() + 60_000);
        let second = evaluate_and_store(db.pool(), &dev, later)
            .await
            .expect("second scan");
        assert!(second.is_empty(), "no new unlocks on a re-scan");

        let all = list_all_including_deleted(db.pool()).await.expect("rows");
        assert_eq!(all.len(), first.len());
        let stored = all
            .iter()
            .find(|u| u.achievement_id == "first-session")
            .expect("row");
        assert_eq!(stored.unlocked_at.as_millis(), started.as_millis());
        assert_eq!(
            stored.meta.updated_at.as_millis(),
            now.as_millis(),
            "date must not drift"
        );
    }

    #[tokio::test]
    async fn upsert_keys_by_slug_not_uuid() {
        let db = Db::open_memory().await.expect("db");
        let dev = queries::ensure_device_id(db.pool()).await.expect("device");
        let now = Timestamp::now().expect("now");
        let first = AchievementUnlock {
            meta: SyncMeta::new(&dev, now),
            achievement_id: "first-session".to_owned(),
            unlocked_at: now,
        };
        upsert(db.pool(), &first).await.expect("insert");
        // A different UUID for the same slug must update, not duplicate.
        let second = AchievementUnlock {
            meta: SyncMeta::new(&dev, Timestamp::from_millis(now.as_millis() + 1)),
            achievement_id: "first-session".to_owned(),
            unlocked_at: Timestamp::from_millis(now.as_millis() - 1000),
        };
        upsert(db.pool(), &second).await.expect("upsert");
        let all = list_all_including_deleted(db.pool()).await.expect("rows");
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].meta.id, second.meta.id);
        assert_eq!(all[0].unlocked_at.as_millis(), now.as_millis() - 1000);
    }

    #[tokio::test]
    async fn orphan_sessions_never_enter_the_aggregates() {
        let db = Db::open_memory().await.expect("db");
        let dev = queries::ensure_device_id(db.pool()).await.expect("device");
        let now = Timestamp::now().expect("now");
        let game = seed_game(&db, &dev, now, "Live").await;
        seed_session(&db, &dev, &game, now, 600_000).await;

        // Seed an orphan with foreign keys off on one connection: the per-connection
        // pragma is exactly how real orphans appear (AGENTS.md §5). A hard DELETE
        // would cascade and prove nothing.
        {
            let mut conn = db.pool().acquire().await.expect("conn");
            sqlx::query("PRAGMA foreign_keys = OFF")
                .execute(&mut *conn)
                .await
                .expect("fk off");
            sqlx::query(
                "INSERT INTO play_sessions (id, game_id, started_at, ended_at, counted_ms, source, updated_at, device_id, deleted) \
                 VALUES ('orphan', 'gone-game', ?, ?, 360000000, 'manual', ?, ?, 0)",
            )
            .bind(now.as_millis())
            .bind(now.as_millis() + 360_000_000)
            .bind(now.as_millis())
            .bind(dev.as_str())
            .execute(&mut *conn)
            .await
            .expect("orphan");
        }

        let input = build_input(db.pool()).await.expect("input");
        assert_eq!(
            input.total_playtime_ms, 600_000,
            "100 orphan hours must not count"
        );
        assert_eq!(input.session_count, 1);
        assert_eq!(input.longest_session_ms, 600_000);
    }

    #[tokio::test]
    async fn markers_unlock_discover_import_and_backup_first_with_their_dates() {
        let db = Db::open_memory().await.expect("db");
        let dev = queries::ensure_device_id(db.pool()).await.expect("device");
        let now = Timestamp::now().expect("now");
        let marked = Timestamp::from_millis(now.as_millis() - 5000);
        queries::set_pref(
            db.pool(),
            "achievements.discoverImported",
            &marked.as_millis().to_string(),
        )
        .await
        .expect("marker");
        queries::set_pref(
            db.pool(),
            "achievements.backupMade",
            &marked.as_millis().to_string(),
        )
        .await
        .expect("marker");
        let fresh = evaluate_and_store(db.pool(), &dev, now)
            .await
            .expect("scan");
        assert!(slugs(&fresh).contains(&"discover-import"));
        assert!(slugs(&fresh).contains(&"backup-first"));
        for slug in ["discover-import", "backup-first"] {
            let row = fresh
                .iter()
                .find(|u| u.achievement_id == slug)
                .expect("row");
            assert_eq!(row.unlocked_at.as_millis(), marked.as_millis());
        }
    }

    #[tokio::test]
    async fn a_tombstoned_unlock_is_exported_but_not_shown() {
        let db = Db::open_memory().await.expect("db");
        let dev = queries::ensure_device_id(db.pool()).await.expect("device");
        let now = Timestamp::now().expect("now");
        let mut row = AchievementUnlock {
            meta: SyncMeta::new(&dev, now),
            achievement_id: "sync-first".to_owned(),
            unlocked_at: now,
        };
        row.meta.deleted = true;
        upsert(db.pool(), &row).await.expect("insert tombstone");
        let views = list(db.pool()).await.expect("list");
        let view = views.iter().find(|v| v.slug == "sync-first").expect("view");
        assert!(!view.unlocked, "no successful sync and the row is deleted");
        let all = list_all_including_deleted(db.pool()).await.expect("export");
        assert_eq!(all.len(), 1);
        assert!(all[0].meta.deleted);
    }
}
