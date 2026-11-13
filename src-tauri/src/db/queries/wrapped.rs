//! Library-wide aggregates for the all-time wrapped card.
//!
//! Every aggregate joins live `games` (the orphan rule), and every calendar
//! bucket uses `SQLite`'s `'localtime'` modifier so the card agrees with the
//! heatmap and the per-year review. The per-year queries live in `journey.rs`;
//! these are the same shapes without a year filter, kept as static SQL so each
//! figure remains auditable on its own.

use sqlx::SqlitePool;
use vn_core::ids::EntityId;

use super::journey::{bucket_index, corrupt};
use crate::commands::journey::{
    DayMoment, FirstGameMoment, LibraryWrapped, LongestSessionMoment, MonthMoment, RouteMoment,
    YearMoment, YearReviewGame,
};
use crate::db::pool::DbError;

fn parse_game_id(value: &str) -> Result<EntityId, DbError> {
    EntityId::parse(value).map_err(|err| corrupt(err.to_string()))
}

fn parse_year(value: i64) -> Result<i32, DbError> {
    i32::try_from(value).map_err(|_| corrupt(format!("year out of range: {value}")))
}

fn parse_month(value: i64) -> Result<i32, DbError> {
    i32::try_from(value).map_err(|_| corrupt(format!("month out of range: {value}")))
}

struct LibraryCounts {
    session_count: i64,
    active_days: i64,
    games_tracked: i64,
    games_in_library: i64,
    games_finished: i64,
    endings_reached: i64,
    journal_entries: i64,
}

struct LibraryTime {
    total_ms: i64,
    afk_ms: i64,
    average_session_ms: Option<i64>,
    average_rating_decivotes: Option<i64>,
}

struct LibraryHighlights {
    longest_session: Option<LongestSessionMoment>,
    first_game: Option<FirstGameMoment>,
    first_year: Option<i32>,
    top_games: Vec<YearReviewGame>,
}

struct LibraryMoments {
    busiest_month: Option<MonthMoment>,
    busiest_day: Option<DayMoment>,
    most_read_year: Option<YearMoment>,
    longest_streak_days: i64,
}

/// Counts that do not need session time.
///
/// # Errors
/// `db_error` when a query fails.
async fn library_counts(pool: &SqlitePool) -> Result<LibraryCounts, DbError> {
    let (session_count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM play_sessions ps \
         JOIN games g ON g.id = ps.game_id AND g.deleted = 0 WHERE ps.deleted = 0",
    )
    .fetch_one(pool)
    .await?;

    let (active_days,): (i64,) = sqlx::query_as(
        "SELECT COUNT(DISTINCT CAST(strftime('%s', ps.started_at / 1000, 'unixepoch', \
                                   'localtime', 'start of day') AS INTEGER) / 86400) \
         FROM play_sessions ps JOIN games g ON g.id = ps.game_id AND g.deleted = 0 \
         WHERE ps.deleted = 0",
    )
    .fetch_one(pool)
    .await?;

    let (games_tracked,): (i64,) = sqlx::query_as(
        "SELECT COUNT(DISTINCT ps.game_id) FROM play_sessions ps \
         JOIN games g ON g.id = ps.game_id AND g.deleted = 0 WHERE ps.deleted = 0",
    )
    .fetch_one(pool)
    .await?;

    let (games_in_library,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM games WHERE deleted = 0")
            .fetch_one(pool)
            .await?;

    let (games_finished,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM games WHERE deleted = 0 AND status = 'completed'")
            .fetch_one(pool)
            .await?;

    let (endings_reached,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM endings en JOIN games g ON g.id = en.game_id AND g.deleted = 0 \
         WHERE en.deleted = 0 AND en.achieved = 1",
    )
    .fetch_one(pool)
    .await?;

    let (journal_entries,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM reading_entries e \
         JOIN games g ON g.id = e.game_id AND g.deleted = 0 WHERE e.deleted = 0",
    )
    .fetch_one(pool)
    .await?;

    Ok(LibraryCounts {
        session_count,
        active_days,
        games_tracked,
        games_in_library,
        games_finished,
        endings_reached,
        journal_entries,
    })
}

/// Time totals and the two means.
///
/// # Errors
/// `db_error` when a query fails.
async fn library_time(pool: &SqlitePool) -> Result<LibraryTime, DbError> {
    let (total_ms, afk_ms): (i64, i64) = sqlx::query_as(
        "SELECT COALESCE(SUM(ps.counted_ms), 0), COALESCE(SUM(ps.afk_ms), 0) \
         FROM play_sessions ps JOIN games g ON g.id = ps.game_id AND g.deleted = 0 \
         WHERE ps.deleted = 0",
    )
    .fetch_one(pool)
    .await?;

    // Zero-length rows are excluded: a session created and immediately cancelled
    // would otherwise pull the mean toward a duration nobody read.
    let average_session_ms: Option<i64> = sqlx::query_scalar(
        "SELECT SUM(ps.counted_ms) / COUNT(*) \
         FROM play_sessions ps JOIN games g ON g.id = ps.game_id AND g.deleted = 0 \
         WHERE ps.deleted = 0 AND ps.counted_ms > 0",
    )
    .fetch_one(pool)
    .await?;

    let average_rating: Option<f64> = sqlx::query_scalar(
        "SELECT AVG(rating) FROM games WHERE deleted = 0 AND rating IS NOT NULL",
    )
    .fetch_one(pool)
    .await?;
    // Rounding a mean to a whole decivote is the point; the fractional part it
    // can carry is not displayable here.
    #[allow(
        clippy::cast_possible_truncation,
        reason = "the mean is rounded first; a decivote is a whole number"
    )]
    let average_rating_decivotes = average_rating.map(|value| value.round() as i64);

    Ok(LibraryTime {
        total_ms,
        afk_ms,
        average_session_ms,
        average_rating_decivotes,
    })
}

/// The longest session, first game, first year, and top five games.
///
/// # Errors
/// `db_error` when a query fails, `corrupt` for an unparseable id or year.
async fn library_highlights(pool: &SqlitePool) -> Result<LibraryHighlights, DbError> {
    let longest: Option<(i64, String, String)> = sqlx::query_as(
        "SELECT ps.counted_ms, ps.game_id, g.title \
         FROM play_sessions ps JOIN games g ON g.id = ps.game_id AND g.deleted = 0 \
         WHERE ps.deleted = 0 \
         ORDER BY ps.counted_ms DESC, ps.id ASC LIMIT 1",
    )
    .fetch_optional(pool)
    .await?;
    let longest_session = longest
        .map(
            |(counted_ms, game_id, title)| -> Result<LongestSessionMoment, DbError> {
                Ok(LongestSessionMoment {
                    game_id: parse_game_id(&game_id)?,
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
         ORDER BY ps.started_at ASC, ps.id ASC LIMIT 1",
    )
    .fetch_optional(pool)
    .await?;
    let first_game = first
        .map(
            |(started_at, game_id, title)| -> Result<FirstGameMoment, DbError> {
                Ok(FirstGameMoment {
                    game_id: parse_game_id(&game_id)?,
                    title,
                    started_at,
                })
            },
        )
        .transpose()?;

    let first_year: Option<i64> = sqlx::query_scalar(
        "SELECT CAST(strftime('%Y', ps.started_at / 1000, 'unixepoch', 'localtime') AS INTEGER) \
         FROM play_sessions ps JOIN games g ON g.id = ps.game_id AND g.deleted = 0 \
         WHERE ps.deleted = 0 \
         ORDER BY ps.started_at ASC, ps.id ASC LIMIT 1",
    )
    .fetch_optional(pool)
    .await?;
    let first_year = first_year.map(parse_year).transpose()?;

    let top_rows: Vec<(String, String, i64)> = sqlx::query_as(
        "SELECT ps.game_id, g.title, SUM(ps.counted_ms) \
         FROM play_sessions ps JOIN games g ON g.id = ps.game_id AND g.deleted = 0 \
         WHERE ps.deleted = 0 \
         GROUP BY ps.game_id, g.title \
         ORDER BY SUM(ps.counted_ms) DESC, g.title ASC LIMIT 5",
    )
    .fetch_all(pool)
    .await?;
    let top_games = top_rows
        .into_iter()
        .map(|(game_id, title, counted_ms)| {
            Ok(YearReviewGame {
                game_id: parse_game_id(&game_id)?,
                title,
                counted_ms,
            })
        })
        .collect::<Result<Vec<_>, DbError>>()?;

    Ok(LibraryHighlights {
        longest_session,
        first_game,
        first_year,
        top_games,
    })
}

/// The busiest month and day, most read year, and longest local-day streak.
///
/// # Errors
/// `db_error` when a query fails, `corrupt` for an out-of-range month or year.
async fn library_moments(pool: &SqlitePool) -> Result<LibraryMoments, DbError> {
    let busiest_month: Option<(i64, i64)> = sqlx::query_as(
        "SELECT CAST(strftime('%m', ps.started_at / 1000, 'unixepoch', 'localtime') AS INTEGER), \
                SUM(ps.counted_ms) \
         FROM play_sessions ps JOIN games g ON g.id = ps.game_id AND g.deleted = 0 \
         WHERE ps.deleted = 0 \
         GROUP BY 1 \
         ORDER BY SUM(ps.counted_ms) DESC, 1 ASC LIMIT 1",
    )
    .fetch_optional(pool)
    .await?;
    let busiest_month = busiest_month
        .map(|(month, counted_ms)| -> Result<MonthMoment, DbError> {
            Ok(MonthMoment {
                month: parse_month(month)?,
                counted_ms,
            })
        })
        .transpose()?;

    let busiest_day: Option<(i64, i64)> = sqlx::query_as(
        "SELECT CAST(strftime('%s', ps.started_at / 1000, 'unixepoch', 'localtime', \
                                'start of day') AS INTEGER) / 86400, \
                SUM(ps.counted_ms) \
         FROM play_sessions ps JOIN games g ON g.id = ps.game_id AND g.deleted = 0 \
         WHERE ps.deleted = 0 \
         GROUP BY 1 \
         ORDER BY SUM(ps.counted_ms) DESC, 1 ASC LIMIT 1",
    )
    .fetch_optional(pool)
    .await?;
    let busiest_day = busiest_day.map(|(day, counted_ms)| DayMoment { day, counted_ms });

    let most_read_year: Option<(i64, i64)> = sqlx::query_as(
        "SELECT CAST(strftime('%Y', ps.started_at / 1000, 'unixepoch', 'localtime') AS INTEGER) \
                AS year, SUM(ps.counted_ms) AS counted_ms \
         FROM play_sessions ps JOIN games g ON g.id = ps.game_id AND g.deleted = 0 \
         WHERE ps.deleted = 0 \
         GROUP BY year \
         ORDER BY counted_ms DESC, year ASC LIMIT 1",
    )
    .fetch_optional(pool)
    .await?;
    let most_read_year = most_read_year
        .map(|(year, counted_ms)| -> Result<YearMoment, DbError> {
            Ok(YearMoment {
                year: parse_year(year)?,
                counted_ms,
            })
        })
        .transpose()?;

    let day_rows: Vec<i64> = sqlx::query_scalar(
        "SELECT DISTINCT CAST(strftime('%s', ps.started_at / 1000, 'unixepoch', 'localtime', \
                                      'start of day') AS INTEGER) / 86400 \
         FROM play_sessions ps JOIN games g ON g.id = ps.game_id AND g.deleted = 0 \
         WHERE ps.deleted = 0 AND ps.counted_ms > 0 \
         ORDER BY 1",
    )
    .fetch_all(pool)
    .await?;
    let longest_streak_days = vn_core::stats::longest_day_streak(&day_rows)
        .and_then(|days| i64::try_from(days).ok())
        .unwrap_or(0);

    Ok(LibraryMoments {
        busiest_month,
        busiest_day,
        most_read_year,
        longest_streak_days,
    })
}

/// Reading time per local weekday and hour.
///
/// # Errors
/// `db_error` when a query fails, `corrupt` for an out-of-range bucket.
async fn library_distributions(pool: &SqlitePool) -> Result<(Vec<i64>, Vec<i64>), DbError> {
    let weekday_rows: Vec<(i64, i64)> = sqlx::query_as(
        "SELECT (CAST(strftime('%w', ps.started_at / 1000, 'unixepoch', 'localtime') AS INTEGER) + 6) % 7, \
                SUM(ps.counted_ms) \
         FROM play_sessions ps JOIN games g ON g.id = ps.game_id AND g.deleted = 0 \
         WHERE ps.deleted = 0 \
         GROUP BY 1",
    )
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
         GROUP BY 1",
    )
    .fetch_all(pool)
    .await?;
    let mut hour_ms = vec![0i64; 24];
    for (index, counted_ms) in hour_rows {
        hour_ms[bucket_index(index, 24, "hour")?] = counted_ms;
    }

    Ok((weekday_ms, hour_ms))
}

/// The route with the most time across the whole library.
///
/// # Errors
/// `db_error` when a query fails.
async fn library_route(pool: &SqlitePool) -> Result<Option<RouteMoment>, DbError> {
    let top_route: Option<(String, i64)> = sqlx::query_as(
        "SELECT COALESCE(r.name, cr.name, c.route_name) AS name, \
                SUM(ps.counted_ms) AS counted_ms \
         FROM play_sessions ps \
         JOIN games g ON g.id = ps.game_id AND g.deleted = 0 \
         LEFT JOIN routes r ON r.id = ps.route_id AND r.deleted = 0 \
         LEFT JOIN chapters c ON c.id = ps.chapter_id AND c.deleted = 0 \
         LEFT JOIN routes cr ON cr.id = c.route_id AND cr.deleted = 0 \
         WHERE ps.deleted = 0 \
           AND COALESCE(r.name, cr.name, c.route_name) IS NOT NULL \
           AND TRIM(COALESCE(r.name, cr.name, c.route_name)) <> '' \
         GROUP BY COALESCE(r.name, cr.name, c.route_name) \
         ORDER BY counted_ms DESC, name ASC LIMIT 1",
    )
    .fetch_optional(pool)
    .await?;
    Ok(top_route.map(|(name, counted_ms)| RouteMoment { name, counted_ms }))
}

/// The all-time wrapped aggregates for the Stats page.
///
/// # Errors
/// `db_error` when a query fails, `corrupt` for an unparseable id or bucket.
pub async fn library_wrapped(pool: &SqlitePool) -> Result<LibraryWrapped, DbError> {
    let counts = library_counts(pool).await?;
    let time = library_time(pool).await?;
    let highlights = library_highlights(pool).await?;
    let moments = library_moments(pool).await?;
    let (weekday_ms, hour_ms) = library_distributions(pool).await?;
    let top_route = library_route(pool).await?;

    Ok(LibraryWrapped {
        total_ms: time.total_ms,
        afk_ms: time.afk_ms,
        session_count: counts.session_count,
        active_days: counts.active_days,
        games_tracked: counts.games_tracked,
        games_finished: counts.games_finished,
        games_in_library: counts.games_in_library,
        endings_reached: counts.endings_reached,
        journal_entries: counts.journal_entries,
        average_rating_decivotes: time.average_rating_decivotes,
        average_session_ms: time.average_session_ms,
        longest_session: highlights.longest_session,
        longest_streak_days: moments.longest_streak_days,
        first_game: highlights.first_game,
        first_year: highlights.first_year,
        top_games: highlights.top_games,
        top_route,
        busiest_month: moments.busiest_month,
        busiest_day: moments.busiest_day,
        most_read_year: moments.most_read_year,
        weekday_ms,
        hour_ms,
    })
}
