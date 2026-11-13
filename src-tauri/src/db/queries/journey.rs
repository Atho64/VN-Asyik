//! Cross-game journey reads: the Timeline feed and the Year in Review.
//!
//! Both are read-only aggregates over child tables, so every branch joins live
//! `games` — the orphan rule (`AGENTS.md` §5): `PRAGMA foreign_keys` is per
//! connection and hard-deleted parents leave orphan children behind, which once
//! reported 8.8 hours of playtime from games that no longer existed.
//!
//! Bucketing follows the `playtime_by_day` convention, not `stats_derived`'s:
//! these figures describe the calendar the reader experienced, so `SQLite`'s
//! `'localtime'` modifier buckets by the machine's calendar day/year. The two
//! conventions differ near midnight and on DST boundaries; this page sits next
//! to the heatmap, so it keeps the heatmap's unit.

use std::collections::HashMap;

use sqlx::SqlitePool;
use vn_core::ids::EntityId;
use vn_core::model::{Game, SessionSource};

use crate::commands::journey::{
    DayMoment, FirstGameMoment, LongestSessionMoment, MemoryEvent, MonthMoment, RouteMoment,
    TimelineCursor, TimelineEvent, TimelinePage, YearReview, YearReviewGame,
};
use crate::db::models::GameRow;
use crate::db::pool::DbError;

pub(crate) fn corrupt(err: String) -> DbError {
    DbError::Corrupt(err)
}

/// One timeline event before the game row is attached. The `UNION ALL` branches
/// in [`timeline_page`] all produce these columns.
#[derive(sqlx::FromRow)]
struct EventRow {
    ts: i64,
    event_id: String,
    kind: String,
    game_id: String,
    counted_ms: Option<i64>,
    afk_ms: Option<i64>,
    source: Option<String>,
    note: Option<String>,
    route_name: Option<String>,
    chapter_name: Option<String>,
    ending_name: Option<String>,
}

/// Every branch already joins live `games` (orphan rule), so the ids collected
/// from a page are all live; this loads their rows for the embedded `Game`.
pub(crate) async fn games_by_ids(
    pool: &SqlitePool,
    ids: &[String],
) -> Result<HashMap<String, Game>, DbError> {
    let ids_json = serde_json::to_string(ids).map_err(|err| corrupt(err.to_string()))?;
    let rows: Vec<GameRow> = sqlx::query_as(
        "SELECT id, title, title_original, vndb_id, status, rating, notes, length_minutes, \
                manual_playtime_ms, accent_color, cover_url, cover_sexual, cover_violence, \
                custom_cover_path, custom_cover_source_vndb, started_at, finished_at, \
                updated_at, device_id, deleted \
         FROM games \
         WHERE deleted = 0 AND id IN (SELECT value FROM json_each(?))",
    )
    .bind(ids_json)
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|row| {
            // Key by the stored text, not a re-canonicalized id: both sides of
            // the lookup read the same column, so both see the same bytes.
            let key = row.id.clone();
            Game::try_from(row).map(|game| (key, game))
        })
        .collect::<Result<HashMap<_, _>, _>>()
        .map_err(|err| corrupt(err.to_string()))
}

/// Cross-game Timeline feed, newest first, in bounded keyset pages.
///
/// The page is one `UNION ALL` over four event sources, each joining live
/// `games` so a deleted game's history cannot surface:
///
/// - `session` — a reading sitting. Zero-time sessions are the game page's
///   business: the latest-reading card already hides them, and a feed full of
///   "opened the menu for a minute" rows is noise, not a journey.
/// - `journal` — a reading entry; route/chapter names are historical snapshots,
///   kept exactly as they were written (0029).
/// - `ending` — an achieved ending with a date ("'Achieved' is a date, not a
///   flag" is enforced on write).
/// - `finished` — one per live game, dated by `finished_at` or, because that
///   column is written only by VNDB sync, by the first achieved ending.
///
/// The note preview is capped at the same 240 chars + ellipsis the session
/// history uses; the full note lives on the game page.
///
/// # Errors
/// `db_error` when a query fails, `corrupt` for an unknown event kind or an
/// unparseable id.
pub async fn timeline_page(
    pool: &SqlitePool,
    before: Option<TimelineCursor>,
) -> Result<TimelinePage, DbError> {
    // 50 events plus one probe row, the way `has_more` stays exact without a
    // second count over the union.
    const PAGE_PLUS_ONE: i64 = 51;
    let before_ts = before.as_ref().map(|cursor| cursor.ts);
    let before_id = before.map(|cursor| cursor.id.to_string());
    let rows: Vec<EventRow> = sqlx::query_as(
        "SELECT ts, event_id, kind, game_id, counted_ms, afk_ms, source, note, route_name, \
                chapter_name, ending_name \
         FROM ( \
             SELECT s.started_at AS ts, s.id AS event_id, 'session' AS kind, \
                    s.game_id AS game_id, s.counted_ms AS counted_ms, s.afk_ms AS afk_ms, \
                    s.source AS source, \
                    (SELECT substr(e.note, 1, 241) FROM reading_entries e \
                     WHERE e.game_id = s.game_id AND e.session_id = s.id AND e.deleted = 0 \
                       AND trim(e.note, char(9, 10, 13, 32)) <> '' \
                     ORDER BY e.created_at DESC, e.id DESC LIMIT 1) AS note, \
                    NULL AS route_name, NULL AS chapter_name, NULL AS ending_name \
             FROM play_sessions s JOIN games g ON g.id = s.game_id AND g.deleted = 0 \
             WHERE s.deleted = 0 AND s.counted_ms > 0 \
             UNION ALL \
             SELECT e.created_at, e.id, 'journal', \
                    e.game_id, NULL, NULL, NULL, \
                    substr(e.note, 1, 241), \
                    e.route_name, e.chapter_name, NULL \
             FROM reading_entries e JOIN games g ON g.id = e.game_id AND g.deleted = 0 \
             WHERE e.deleted = 0 \
             UNION ALL \
             SELECT en.achieved_at, en.id, 'ending', \
                    en.game_id, NULL, NULL, NULL, NULL, \
                    COALESCE((SELECT r.name FROM routes r \
                              WHERE r.id = en.route_id AND r.deleted = 0), en.route), \
                    NULL, en.name \
             FROM endings en JOIN games g ON g.id = en.game_id AND g.deleted = 0 \
             WHERE en.deleted = 0 AND en.achieved = 1 AND en.achieved_at IS NOT NULL \
             UNION ALL \
             SELECT COALESCE(g.finished_at, \
                             (SELECT MIN(en.achieved_at) FROM endings en \
                              WHERE en.game_id = g.id AND en.deleted = 0 AND en.achieved = 1 \
                                AND en.achieved_at IS NOT NULL)), \
                    g.id, 'finished', \
                    g.id, NULL, NULL, NULL, NULL, NULL, NULL, NULL \
             FROM games g \
             WHERE g.deleted = 0 \
               AND COALESCE(g.finished_at, \
                            (SELECT MIN(en.achieved_at) FROM endings en \
                             WHERE en.game_id = g.id AND en.deleted = 0 AND en.achieved = 1 \
                               AND en.achieved_at IS NOT NULL)) IS NOT NULL \
         ) \
         WHERE (? IS NULL OR ts < ? OR (ts = ? AND event_id < ?)) \
         ORDER BY ts DESC, event_id DESC \
         LIMIT ?",
    )
    .bind(before_ts)
    .bind(before_ts)
    .bind(before_ts)
    .bind(before_id)
    .bind(PAGE_PLUS_ONE)
    .fetch_all(pool)
    .await?;

    let has_more = rows.len() > 50;
    let page_rows: Vec<EventRow> = rows.into_iter().take(50).collect();
    let game_ids: Vec<String> = page_rows.iter().map(|row| row.game_id.clone()).collect();
    let games = games_by_ids(pool, &game_ids).await?;

    let mut events = Vec::with_capacity(page_rows.len());
    for row in page_rows {
        let Some(game) = games.get(&row.game_id) else {
            // Both queries filter `deleted = 0`, so this is only reachable when
            // the game was removed between them. A feed must not advertise an
            // event its game no longer backs; dropping the row is the honest
            // read of that race.
            continue;
        };
        let kind = match row.kind.as_str() {
            "session" | "journal" | "ending" | "finished" => row.kind,
            other => return Err(corrupt(format!("unknown timeline kind: {other}"))),
        };
        let source = row
            .source
            .map(|raw| {
                SessionSource::parse(&raw)
                    .map_err(|other| corrupt(format!("unknown source: {other}")))
            })
            .transpose()?;
        events.push(TimelineEvent {
            kind,
            ts: row.ts,
            id: row.event_id,
            game: game.clone(),
            counted_ms: row.counted_ms,
            afk_ms: row.afk_ms,
            source,
            note: row.note,
            route_name: row.route_name,
            chapter_name: row.chapter_name,
            ending_name: row.ending_name,
        });
    }
    Ok(TimelinePage { events, has_more })
}

/// How many memories "On this day" returns at most.
///
/// A card on the Stats page, not a feed: ten rows is already more than anyone
/// reads there, and the Timeline is where the whole history lives.
pub const ON_THIS_DAY_LIMIT: i64 = 10;

/// One past moment that shares today's month and day.
#[derive(sqlx::FromRow)]
struct MemoryRow {
    ts: i64,
    event_id: String,
    kind: String,
    game_id: String,
    note: Option<String>,
    route_name: Option<String>,
    chapter_name: Option<String>,
    ending_name: Option<String>,
    years_ago: i64,
}

/// Moments from earlier years that happened on today's date.
///
/// Two sources, both of which carry something worth re-reading: a journal
/// checkpoint and an achieved ending. Sessions are deliberately absent — a card
/// that resurfaces "you read for 40 minutes" says nothing, while "you wrote this"
/// is the point of the feature.
///
/// The match is on the reader's local month and day (`'localtime'`, the heatmap's
/// calendar), and today's own rows are excluded: this is a memory, not a second
/// view of the present. `years_ago` is computed in `SQLite` from the same
/// calendar, so a checkpoint written at 00:30 local is not credited to yesterday.
///
/// # Errors
/// `db_error` when a query fails, `corrupt` for an unknown kind or a bad id.
pub async fn on_this_day(pool: &SqlitePool) -> Result<Vec<MemoryEvent>, DbError> {
    let rows: Vec<MemoryRow> = sqlx::query_as(
        "SELECT ts, event_id, kind, game_id, note, route_name, chapter_name, ending_name, \
                years_ago \
         FROM ( \
             SELECT e.created_at AS ts, e.id AS event_id, 'journal' AS kind, \
                    e.game_id AS game_id, substr(e.note, 1, 241) AS note, \
                    e.route_name AS route_name, e.chapter_name AS chapter_name, \
                    NULL AS ending_name, \
                    CAST(strftime('%Y', 'now', 'localtime') AS INTEGER) \
                      - CAST(strftime('%Y', e.created_at / 1000, 'unixepoch', 'localtime') AS INTEGER) \
                      AS years_ago \
             FROM reading_entries e JOIN games g ON g.id = e.game_id AND g.deleted = 0 \
             WHERE e.deleted = 0 \
               AND strftime('%m-%d', e.created_at / 1000, 'unixepoch', 'localtime') \
                   = strftime('%m-%d', 'now', 'localtime') \
               AND date(e.created_at / 1000, 'unixepoch', 'localtime') < date('now', 'localtime') \
             UNION ALL \
             SELECT en.achieved_at, en.id, 'ending', en.game_id, NULL, \
                    COALESCE((SELECT r.name FROM routes r \
                              WHERE r.id = en.route_id AND r.deleted = 0), en.route), \
                    NULL, en.name, \
                    CAST(strftime('%Y', 'now', 'localtime') AS INTEGER) \
                      - CAST(strftime('%Y', en.achieved_at / 1000, 'unixepoch', 'localtime') AS INTEGER) \
             FROM endings en JOIN games g ON g.id = en.game_id AND g.deleted = 0 \
             WHERE en.deleted = 0 AND en.achieved = 1 AND en.achieved_at IS NOT NULL \
               AND strftime('%m-%d', en.achieved_at / 1000, 'unixepoch', 'localtime') \
                   = strftime('%m-%d', 'now', 'localtime') \
               AND date(en.achieved_at / 1000, 'unixepoch', 'localtime') < date('now', 'localtime') \
         ) \
         ORDER BY ts DESC, event_id DESC \
         LIMIT ?",
    )
    .bind(ON_THIS_DAY_LIMIT)
    .fetch_all(pool)
    .await?;

    let game_ids: Vec<String> = rows.iter().map(|row| row.game_id.clone()).collect();
    let games = games_by_ids(pool, &game_ids).await?;
    let mut memories = Vec::with_capacity(rows.len());
    for row in rows {
        let Some(game) = games.get(&row.game_id) else {
            continue;
        };
        let kind = match row.kind.as_str() {
            "journal" | "ending" => row.kind,
            other => return Err(corrupt(format!("unknown memory kind: {other}"))),
        };
        memories.push(MemoryEvent {
            kind,
            ts: row.ts,
            id: row.event_id,
            game: game.clone(),
            note: row.note,
            route_name: row.route_name,
            chapter_name: row.chapter_name,
            ending_name: row.ending_name,
            years_ago: row.years_ago,
        });
    }
    Ok(memories)
}

// The Year in Review is two aggregates: everything sessions can answer on
// their own, and everything the reading records (endings, journal, game
// start/finish) answer. Each query is auditable on its own, and these tables
// are bounded by reading history, so the extra round trips cost nothing a
// user could notice.

/// Everything the review derives from sessions alone: totals, the longest
/// sitting, the first game of the year, the top list.
struct SessionFigures {
    total_ms: i64,
    afk_ms: i64,
    session_count: i64,
    active_days: i64,
    longest_session: Option<LongestSessionMoment>,
    first_game: Option<FirstGameMoment>,
    top_games: Vec<YearReviewGame>,
    busiest_month: Option<MonthMoment>,
    busiest_day: Option<DayMoment>,
    average_session_ms: Option<i64>,
    longest_streak_days: i64,
    top_route: Option<RouteMoment>,
    games_continued: i64,
    weekday_ms: Vec<i64>,
    hour_ms: Vec<i64>,
}

/// A small SQL bucket index, checked rather than cast: a corrupt row must not
/// silently land in the wrong weekday or hour.
pub(crate) fn bucket_index(value: i64, len: usize, label: &str) -> Result<usize, DbError> {
    let index =
        usize::try_from(value).map_err(|_| corrupt(format!("{label} out of range: {value}")))?;
    if index >= len {
        return Err(corrupt(format!("{label} out of range: {value}")));
    }
    Ok(index)
}

/// # Errors
/// `db_error` when a query fails, `corrupt` for an unparseable id.
async fn session_figures(pool: &SqlitePool, year_text: &str) -> Result<SessionFigures, DbError> {
    let (total_ms, afk_ms, session_count): (i64, i64, i64) = sqlx::query_as(
        "SELECT COALESCE(SUM(ps.counted_ms), 0), COALESCE(SUM(ps.afk_ms), 0), COUNT(*) \
         FROM play_sessions ps JOIN games g ON g.id = ps.game_id AND g.deleted = 0 \
         WHERE ps.deleted = 0 \
           AND strftime('%Y', ps.started_at / 1000, 'unixepoch', 'localtime') = ?",
    )
    .bind(year_text)
    .fetch_one(pool)
    .await?;

    let (active_days,): (i64,) = sqlx::query_as(
        "SELECT COUNT(DISTINCT CAST(strftime('%s', ps.started_at / 1000, 'unixepoch', \
                                   'localtime', 'start of day') AS INTEGER) / 86400) \
         FROM play_sessions ps JOIN games g ON g.id = ps.game_id AND g.deleted = 0 \
         WHERE ps.deleted = 0 \
           AND strftime('%Y', ps.started_at / 1000, 'unixepoch', 'localtime') = ?",
    )
    .bind(year_text)
    .fetch_one(pool)
    .await?;

    let (longest_session, first_game, top_games) = session_highlights(pool, year_text).await?;
    let rhythm = session_rhythm(pool, year_text).await?;
    let (top_route, games_continued) = session_routes(pool, year_text).await?;

    Ok(SessionFigures {
        total_ms,
        afk_ms,
        session_count,
        active_days,
        longest_session,
        first_game,
        top_games,
        busiest_month: rhythm.busiest_month,
        busiest_day: rhythm.busiest_day,
        average_session_ms: rhythm.average_session_ms,
        longest_streak_days: rhythm.longest_streak_days,
        top_route,
        games_continued,
        weekday_ms: rhythm.weekday_ms,
        hour_ms: rhythm.hour_ms,
    })
}

/// The longest session, the first game, and the top five games of the year.
///
/// # Errors
/// `db_error` when a query fails, `corrupt` for an unparseable id.
async fn session_highlights(
    pool: &SqlitePool,
    year_text: &str,
) -> Result<
    (
        Option<LongestSessionMoment>,
        Option<FirstGameMoment>,
        Vec<YearReviewGame>,
    ),
    DbError,
> {
    let longest: Option<(i64, String, String)> = sqlx::query_as(
        "SELECT ps.counted_ms, ps.game_id, g.title \
         FROM play_sessions ps JOIN games g ON g.id = ps.game_id AND g.deleted = 0 \
         WHERE ps.deleted = 0 \
           AND strftime('%Y', ps.started_at / 1000, 'unixepoch', 'localtime') = ? \
         ORDER BY ps.counted_ms DESC, ps.id ASC LIMIT 1",
    )
    .bind(year_text)
    .fetch_optional(pool)
    .await?;
    let longest_session = longest
        .map(
            |(counted_ms, game_id, title)| -> Result<LongestSessionMoment, DbError> {
                Ok(LongestSessionMoment {
                    game_id: EntityId::parse(&game_id).map_err(|err| corrupt(err.to_string()))?,
                    title,
                    counted_ms,
                })
            },
        )
        .transpose()?;

    let first: Option<(i64, String, String)> = sqlx::query_as(
        "SELECT ps.started_at, ps.game_id, g.title \
         FROM play_sessions ps JOIN games g ON g.id = ps.game_id AND g.deleted = 0 \
         WHERE ps.deleted = 0 \
           AND strftime('%Y', ps.started_at / 1000, 'unixepoch', 'localtime') = ? \
         ORDER BY ps.started_at ASC, ps.id ASC LIMIT 1",
    )
    .bind(year_text)
    .fetch_optional(pool)
    .await?;
    let first_game = first
        .map(
            |(started_at, game_id, title)| -> Result<FirstGameMoment, DbError> {
                Ok(FirstGameMoment {
                    game_id: EntityId::parse(&game_id).map_err(|err| corrupt(err.to_string()))?,
                    title,
                    started_at,
                })
            },
        )
        .transpose()?;

    let top_rows: Vec<(String, String, i64)> = sqlx::query_as(
        "SELECT ps.game_id, g.title, SUM(ps.counted_ms) \
         FROM play_sessions ps JOIN games g ON g.id = ps.game_id AND g.deleted = 0 \
         WHERE ps.deleted = 0 \
           AND strftime('%Y', ps.started_at / 1000, 'unixepoch', 'localtime') = ? \
         GROUP BY ps.game_id, g.title \
         ORDER BY SUM(ps.counted_ms) DESC, g.title ASC \
         LIMIT 5",
    )
    .bind(year_text)
    .fetch_all(pool)
    .await?;
    let top_games = top_rows
        .into_iter()
        .map(|(game_id, title, counted_ms)| {
            Ok(YearReviewGame {
                game_id: EntityId::parse(&game_id).map_err(|err| corrupt(err.to_string()))?,
                title,
                counted_ms,
            })
        })
        .collect::<Result<Vec<_>, DbError>>()?;

    Ok((longest_session, first_game, top_games))
}

/// The "when did this year happen" figures: busiest month and day, mean session,
/// local-day streak, and the weekday/hour distributions.
struct SessionRhythm {
    busiest_month: Option<MonthMoment>,
    busiest_day: Option<DayMoment>,
    average_session_ms: Option<i64>,
    longest_streak_days: i64,
    weekday_ms: Vec<i64>,
    hour_ms: Vec<i64>,
}

/// The busiest month and day, or `None` for a quiet year.
///
/// # Errors
/// `db_error` when a query fails, `corrupt` for an out-of-range month.
async fn busiest_moments(
    pool: &SqlitePool,
    year_text: &str,
) -> Result<(Option<MonthMoment>, Option<DayMoment>), DbError> {
    // These answer "when did this year happen" with the same local calendar the
    // rest of the card uses. Both are `None` for a quiet year rather than
    // January 1 / day zero, which would look like a real answer.
    let busiest_month: Option<(i64, i64)> = sqlx::query_as(
        "SELECT CAST(strftime('%m', ps.started_at / 1000, 'unixepoch', 'localtime') AS INTEGER), \
                SUM(ps.counted_ms) \
         FROM play_sessions ps JOIN games g ON g.id = ps.game_id AND g.deleted = 0 \
         WHERE ps.deleted = 0 \
           AND strftime('%Y', ps.started_at / 1000, 'unixepoch', 'localtime') = ? \
         GROUP BY 1 \
         ORDER BY SUM(ps.counted_ms) DESC, 1 ASC LIMIT 1",
    )
    .bind(year_text)
    .fetch_optional(pool)
    .await?;
    let busiest_month = busiest_month
        .map(|(month, counted_ms)| -> Result<MonthMoment, DbError> {
            let month = i32::try_from(month)
                .map_err(|_| corrupt(format!("month out of range: {month}")))?;
            Ok(MonthMoment { month, counted_ms })
        })
        .transpose()?;

    let busiest_day: Option<(i64, i64)> = sqlx::query_as(
        "SELECT CAST(strftime('%s', ps.started_at / 1000, 'unixepoch', 'localtime', \
                                'start of day') AS INTEGER) / 86400, \
                SUM(ps.counted_ms) \
         FROM play_sessions ps JOIN games g ON g.id = ps.game_id AND g.deleted = 0 \
         WHERE ps.deleted = 0 \
           AND strftime('%Y', ps.started_at / 1000, 'unixepoch', 'localtime') = ? \
         GROUP BY 1 \
         ORDER BY SUM(ps.counted_ms) DESC, 1 ASC LIMIT 1",
    )
    .bind(year_text)
    .fetch_optional(pool)
    .await?;
    let busiest_day = busiest_day.map(|(day, counted_ms)| DayMoment { day, counted_ms });
    Ok((busiest_month, busiest_day))
}

/// Mean session, local-day streak, and the weekday/hour distributions.
///
/// # Errors
/// `db_error` when a query fails, `corrupt` for an out-of-range bucket.
async fn rhythm_distributions(
    pool: &SqlitePool,
    year_text: &str,
) -> Result<(Option<i64>, i64, Vec<i64>, Vec<i64>), DbError> {
    // Zero-length rows are excluded: a session created and immediately cancelled
    // would otherwise pull the mean toward a duration nobody read.
    let average_session_ms: Option<i64> = sqlx::query_scalar(
        "SELECT SUM(ps.counted_ms) / COUNT(*) \
         FROM play_sessions ps JOIN games g ON g.id = ps.game_id AND g.deleted = 0 \
         WHERE ps.deleted = 0 AND ps.counted_ms > 0 \
           AND strftime('%Y', ps.started_at / 1000, 'unixepoch', 'localtime') = ?",
    )
    .bind(year_text)
    .fetch_one(pool)
    .await?;

    let day_rows: Vec<i64> = sqlx::query_scalar(
        "SELECT DISTINCT CAST(strftime('%s', ps.started_at / 1000, 'unixepoch', 'localtime', \
                                      'start of day') AS INTEGER) / 86400 \
         FROM play_sessions ps JOIN games g ON g.id = ps.game_id AND g.deleted = 0 \
         WHERE ps.deleted = 0 AND ps.counted_ms > 0 \
           AND strftime('%Y', ps.started_at / 1000, 'unixepoch', 'localtime') = ? \
         ORDER BY 1",
    )
    .bind(year_text)
    .fetch_all(pool)
    .await?;
    let longest_streak_days = vn_core::stats::longest_day_streak(&day_rows)
        .and_then(|days| i64::try_from(days).ok())
        .unwrap_or(0);

    let weekday_rows: Vec<(i64, i64)> = sqlx::query_as(
        "SELECT (CAST(strftime('%w', ps.started_at / 1000, 'unixepoch', 'localtime') AS INTEGER) + 6) % 7, \
                SUM(ps.counted_ms) \
         FROM play_sessions ps JOIN games g ON g.id = ps.game_id AND g.deleted = 0 \
         WHERE ps.deleted = 0 \
           AND strftime('%Y', ps.started_at / 1000, 'unixepoch', 'localtime') = ? \
         GROUP BY 1",
    )
    .bind(year_text)
    .fetch_all(pool)
    .await?;
    let mut weekday_ms = vec![0i64; 7];
    for (index, counted_ms) in weekday_rows {
        weekday_ms[bucket_index(index, 7, "weekday")?] = counted_ms;
    }

    let hour_rows: Vec<(i64, i64)> = sqlx::query_as(
        "SELECT CAST(strftime('%H', ps.started_at / 1000, 'unixepoch', 'localtime') AS INTEGER), \
                SUM(ps.counted_ms) \
         FROM play_sessions ps JOIN games g ON g.id = ps.game_id AND g.deleted = 0 \
         WHERE ps.deleted = 0 \
           AND strftime('%Y', ps.started_at / 1000, 'unixepoch', 'localtime') = ? \
         GROUP BY 1",
    )
    .bind(year_text)
    .fetch_all(pool)
    .await?;
    let mut hour_ms = vec![0i64; 24];
    for (index, counted_ms) in hour_rows {
        hour_ms[bucket_index(index, 24, "hour")?] = counted_ms;
    }

    Ok((average_session_ms, longest_streak_days, weekday_ms, hour_ms))
}

/// # Errors
/// `db_error` when a query fails, `corrupt` for an out-of-range bucket.
async fn session_rhythm(pool: &SqlitePool, year_text: &str) -> Result<SessionRhythm, DbError> {
    let (busiest_month, busiest_day) = busiest_moments(pool, year_text).await?;
    let (average_session_ms, longest_streak_days, weekday_ms, hour_ms) =
        rhythm_distributions(pool, year_text).await?;
    Ok(SessionRhythm {
        busiest_month,
        busiest_day,
        average_session_ms,
        longest_streak_days,
        weekday_ms,
        hour_ms,
    })
}

/// The top route and how many games continued from an earlier year.
///
/// # Errors
/// `db_error` when a query fails.
async fn session_routes(
    pool: &SqlitePool,
    year_text: &str,
) -> Result<(Option<RouteMoment>, i64), DbError> {
    // A session can name its route directly (new rows) or only its chapter (old
    // rows); the chapter's linked route is the same fact one join away, and the
    // engine's `route_name` is the last fallback when no user route exists. All
    // three joins are live-row joins, so a deleted route cannot win the year.
    let top_route: Option<(String, i64)> = sqlx::query_as(
        "SELECT COALESCE(r.name, cr.name, c.route_name) AS name, \
                SUM(ps.counted_ms) AS counted_ms \
         FROM play_sessions ps \
         JOIN games g ON g.id = ps.game_id AND g.deleted = 0 \
         LEFT JOIN routes r ON r.id = ps.route_id AND r.deleted = 0 \
         LEFT JOIN chapters c ON c.id = ps.chapter_id AND c.deleted = 0 \
         LEFT JOIN routes cr ON cr.id = c.route_id AND cr.deleted = 0 \
         WHERE ps.deleted = 0 \
           AND strftime('%Y', ps.started_at / 1000, 'unixepoch', 'localtime') = ? \
           AND COALESCE(r.name, cr.name, c.route_name) IS NOT NULL \
           AND TRIM(COALESCE(r.name, cr.name, c.route_name)) <> '' \
         GROUP BY COALESCE(r.name, cr.name, c.route_name) \
         ORDER BY counted_ms DESC, name ASC LIMIT 1",
    )
    .bind(year_text)
    .fetch_optional(pool)
    .await?;
    let top_route = top_route.map(|(name, counted_ms)| RouteMoment { name, counted_ms });

    // Continued means the first session predates the selected year but a session
    // in the selected year exists. A game read only in 2023 is not "continued in
    // 2024" just because its row still exists.
    let (games_continued,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM ( \
             SELECT ps.game_id, \
                    MIN(strftime('%Y', ps.started_at / 1000, 'unixepoch', 'localtime')) AS first_year, \
                    MAX(CASE WHEN strftime('%Y', ps.started_at / 1000, 'unixepoch', 'localtime') = ? \
                             THEN 1 ELSE 0 END) AS in_year \
             FROM play_sessions ps JOIN games g ON g.id = ps.game_id AND g.deleted = 0 \
             WHERE ps.deleted = 0 \
             GROUP BY ps.game_id \
             HAVING in_year = 1 AND CAST(first_year AS INTEGER) < CAST(? AS INTEGER) \
         )",
    )
    .bind(year_text)
    .bind(year_text)
    .fetch_one(pool)
    .await?;

    Ok((top_route, games_continued))
}

/// # Errors
/// `db_error` when a query fails.
async fn record_figures(
    pool: &SqlitePool,
    year_text: &str,
) -> Result<(i64, i64, i64, i64, Option<i64>), DbError> {
    let (endings_reached,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) \
         FROM endings en JOIN games g ON g.id = en.game_id AND g.deleted = 0 \
         WHERE en.deleted = 0 AND en.achieved = 1 AND en.achieved_at IS NOT NULL \
           AND strftime('%Y', en.achieved_at / 1000, 'unixepoch', 'localtime') = ?",
    )
    .bind(year_text)
    .fetch_one(pool)
    .await?;

    let (journal_entries,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) \
         FROM reading_entries e JOIN games g ON g.id = e.game_id AND g.deleted = 0 \
         WHERE e.deleted = 0 \
           AND strftime('%Y', e.created_at / 1000, 'unixepoch', 'localtime') = ?",
    )
    .bind(year_text)
    .fetch_one(pool)
    .await?;

    // A game "starts" when its first session does — `games.started_at` is
    // sparse (written by VNDB sync only), while sessions exist for every
    // tracked game and are the same dated source the timeline uses.
    let (games_started,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM ( \
             SELECT MIN(ps.started_at) AS first_at \
             FROM play_sessions ps JOIN games g ON g.id = ps.game_id AND g.deleted = 0 \
             WHERE ps.deleted = 0 \
             GROUP BY ps.game_id \
         ) \
         WHERE strftime('%Y', first_at / 1000, 'unixepoch', 'localtime') = ?",
    )
    .bind(year_text)
    .fetch_one(pool)
    .await?;

    // Finished and average rating in one pass over the same derived finish
    // date, so counting and rating describe the same set of games: a separate
    // rating query could pair "9 finished" with an average over 5 of them.
    // `AVG` skips NULL ratings on its own, and is NULL when no finished game
    // carries one.
    let (games_finished, avg_rating): (i64, Option<f64>) = sqlx::query_as(
        "SELECT COUNT(*), AVG(g.rating) \
         FROM games g \
         JOIN ( \
             SELECT g2.id AS id, \
                    COALESCE(g2.finished_at, \
                             (SELECT MIN(en.achieved_at) FROM endings en \
                              WHERE en.game_id = g2.id AND en.deleted = 0 AND en.achieved = 1 \
                                AND en.achieved_at IS NOT NULL)) AS finish_ts \
             FROM games g2 WHERE g2.deleted = 0 \
         ) f ON f.id = g.id \
         WHERE f.finish_ts IS NOT NULL \
           AND strftime('%Y', f.finish_ts / 1000, 'unixepoch', 'localtime') = ?",
    )
    .bind(year_text)
    .fetch_one(pool)
    .await?;
    // Rounding a mean to a whole decivote is the point; the fractional part it
    // can carry is not displayable here.
    #[allow(
        clippy::cast_possible_truncation,
        reason = "the mean is rounded first; a decivote is a whole number"
    )]
    let avg_rating_decivotes = avg_rating.map(|value| value.round() as i64);

    Ok((
        endings_reached,
        journal_entries,
        games_started,
        games_finished,
        avg_rating_decivotes,
    ))
}

/// The Year in Review for one calendar year, aggregated in SQL over live rows.
///
/// # Errors
/// `db_error` when a query fails, `corrupt` for an unparseable id.
pub async fn year_review(pool: &SqlitePool, year: i32) -> Result<YearReview, DbError> {
    let year_text = year.to_string();
    let sessions = session_figures(pool, &year_text).await?;
    let (endings_reached, journal_entries, games_started, games_finished, avg_rating_decivotes) =
        record_figures(pool, &year_text).await?;

    Ok(YearReview {
        year,
        total_ms: sessions.total_ms,
        afk_ms: sessions.afk_ms,
        session_count: sessions.session_count,
        active_days: sessions.active_days,
        endings_reached,
        journal_entries,
        games_started,
        games_finished,
        avg_rating_decivotes,
        longest_session: sessions.longest_session,
        first_game: sessions.first_game,
        top_games: sessions.top_games,
        busiest_month: sessions.busiest_month,
        busiest_day: sessions.busiest_day,
        average_session_ms: sessions.average_session_ms,
        longest_streak_days: sessions.longest_streak_days,
        top_route: sessions.top_route,
        games_continued: sessions.games_continued,
        weekday_ms: sessions.weekday_ms,
        hour_ms: sessions.hour_ms,
    })
}
