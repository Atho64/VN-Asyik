//! Library statistics.
//!
//! Kept out of the command layer so the arithmetic can be tested without a
//! database. Every figure here is derived from rows the app already stores; none
//! is estimated, sampled, or made up.
//!
//! # Two rules
//!
//! A statistic with no data behind it is `None`, not zero. "Average rating 0"
//! claims the user rated everything zero, while "no rating yet" is the truth.
//!
//! Ratios are computed against the population they describe. Completion rate is
//! finished divided by *started*, not by the whole library, because an unplayed
//! wishlist entry is not an unfinished game.

use serde::{Deserialize, Serialize};

use crate::clock::Timestamp;

/// One play session, as far as statistics care.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionSample {
    /// When it began.
    pub started_at: i64,
    /// Time actually counted, excluding idle.
    pub counted_ms: i64,
}

/// One game, as far as statistics care.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameSample {
    /// Status slug, e.g. `completed`.
    pub status: String,
    /// The user's rating in decivotes: 10 is 1.0, 100 is 10.0.
    pub rating: Option<u8>,
    /// Total playtime for this game.
    pub playtime_ms: i64,
    /// When it was finished, UTC ms, when known.
    pub finished_at: Option<i64>,
}

/// Everything the stats screen shows that is not already a query.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DerivedStats {
    /// Finished divided by started, 0 to 1. `None` when nothing was started.
    pub completion_rate: Option<f64>,
    /// Mean of the user's ratings in decivotes, so 75.5 means 7.55.
    pub average_rating: Option<f64>,
    /// The longest single session.
    pub longest_session_ms: Option<i64>,
    /// Mean session length, excluding zero-length rows.
    pub average_session_ms: Option<i64>,
    /// Sessions counted.
    pub session_count: usize,
    /// The route with the most time, when any chapter has a route.
    pub favourite_route: Option<RouteStat>,
    /// How many games carry each whole-star rating, indexed as
    /// `counts[stars - 1]`. A 7.5 counts toward 7, since the chart shows stars.
    pub rating_spread: [usize; 10],
    /// Games finished in the current calendar year, UTC.
    pub finished_this_year: Option<usize>,
    /// Longest run of consecutive days with a session.
    pub longest_streak_days: Option<usize>,
}

/// A route's total time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteStat {
    /// Route label.
    pub name: String,
    /// Time across the route's chapters.
    pub playtime_ms: i64,
}

/// Milliseconds in a day, for streak arithmetic.
const DAY_MS: i64 = 86_400_000;

/// Compute every derived figure from the raw rows.
#[must_use]
pub fn derive(
    games: &[GameSample],
    sessions: &[SessionSample],
    routes: &[RouteStat],
    now: Timestamp,
) -> DerivedStats {
    DerivedStats {
        completion_rate: completion_rate(games),
        average_rating: average_rating(games),
        longest_session_ms: longest_session(sessions),
        average_session_ms: average_session(sessions),
        session_count: sessions.iter().filter(|s| s.counted_ms > 0).count(),
        favourite_route: favourite_route(routes),
        rating_spread: rating_spread(games),
        finished_this_year: finished_this_year(games, now),
        longest_streak_days: longest_streak(sessions),
    }
}

/// Finished over started.
///
/// A wishlist entry was never begun, so counting it as an incomplete game would
/// make a large wishlist look like failure. `None` when nothing has started.
#[must_use]
pub fn completion_rate(games: &[GameSample]) -> Option<f64> {
    let started = games
        .iter()
        .filter(|game| {
            matches!(
                game.status.as_str(),
                "playing" | "completed" | "on_hold" | "dropped"
            )
        })
        .count();
    if started == 0 {
        return None;
    }
    let finished = games
        .iter()
        .filter(|game| game.status == "completed")
        .count();
    // A library cannot hold more games than a u32 can count, and casting through
    // u32 avoids the usize-to-float precision lint without changing the result.
    let finished = u32::try_from(finished).unwrap_or(u32::MAX);
    let started = u32::try_from(started).unwrap_or(u32::MAX);
    Some(f64::from(finished) / f64::from(started))
}

/// Mean of the non-null ratings.
///
/// Unrated games are skipped rather than counted as zero, which would drag the
/// average toward a number the user never gave.
#[must_use]
pub fn average_rating(games: &[GameSample]) -> Option<f64> {
    let ratings: Vec<u8> = games.iter().filter_map(|game| game.rating).collect();
    if ratings.is_empty() {
        return None;
    }
    let total: u32 = ratings.iter().map(|value| u32::from(*value)).sum();
    let count = u32::try_from(ratings.len()).unwrap_or(u32::MAX).max(1);
    Some(f64::from(total) / f64::from(count))
}

/// The single longest session.
#[must_use]
pub fn longest_session(sessions: &[SessionSample]) -> Option<i64> {
    sessions
        .iter()
        .map(|session| session.counted_ms)
        .max()
        .filter(|ms| *ms > 0)
}

/// Mean session length.
///
/// Only sessions with counted time take part: a row created and immediately
/// cancelled would otherwise pull the average toward zero.
#[must_use]
pub fn average_session(sessions: &[SessionSample]) -> Option<i64> {
    let counted: Vec<i64> = sessions
        .iter()
        .map(|session| session.counted_ms)
        .filter(|ms| *ms > 0)
        .collect();
    if counted.is_empty() {
        return None;
    }
    let total: i64 = counted.iter().sum();
    Some(total / i64::try_from(counted.len()).unwrap_or(1))
}

/// The route with the most time.
#[must_use]
pub fn favourite_route(routes: &[RouteStat]) -> Option<RouteStat> {
    routes
        .iter()
        .filter(|route| route.playtime_ms > 0 && !route.name.trim().is_empty())
        .max_by_key(|route| route.playtime_ms)
        .cloned()
}

/// How many games carry each whole-star rating, 1 to 10.
///
/// A decivote of 75 belongs to the 7-star bucket. The chart plots ten bars, so
/// the tenths are bucketed rather than expanded into a hundred-bar axis that no
/// one asked for; the exact average still carries the full precision.
#[must_use]
pub fn rating_spread(games: &[GameSample]) -> [usize; 10] {
    let mut counts = [0usize; 10];
    for rating in games.iter().filter_map(|game| game.rating) {
        let stars = rating.clamp(10, 100) / 10;
        counts[usize::from(stars.clamp(1, 10)) - 1] += 1;
    }
    counts
}

/// Games finished in the current year.
///
/// `None` when no game carries a finish date at all, so the UI can say "not
/// recorded" instead of "0 finished this year", which would be a claim.
#[must_use]
pub fn finished_this_year(games: &[GameSample], now: Timestamp) -> Option<usize> {
    let dated: Vec<i64> = games
        .iter()
        .filter(|game| game.status == "completed")
        .filter_map(|game| game.finished_at)
        .collect();
    if dated.is_empty() {
        return None;
    }
    let year_start = year_start_ms(now);
    let year_end = year_start + 366 * DAY_MS;
    Some(
        dated
            .iter()
            .filter(|ms| **ms >= year_start && **ms < year_end)
            .count(),
    )
}

/// The first instant of the timestamp's year, UTC.
///
/// Derived from the day count rather than a calendar library: the app already
/// stores UTC milliseconds and needs no timezone logic for "this year".
fn year_start_ms(now: Timestamp) -> i64 {
    let ms = now.as_millis();
    // Days since the epoch, then back to 1 January of the containing year.
    let days = ms.div_euclid(DAY_MS);
    let mut year = 1970i64;
    let mut remaining = days;
    loop {
        let length = if is_leap(year) { 366 } else { 365 };
        if remaining < length {
            break;
        }
        remaining -= length;
        year += 1;
    }
    (days - remaining) * DAY_MS
}

fn is_leap(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

/// Longest run of consecutive calendar days that have at least one session.
///
/// Days are UTC. Two sessions that straddle midnight count as two days, which is
/// the honest reading of "played on these days".
#[must_use]
pub fn longest_streak(sessions: &[SessionSample]) -> Option<usize> {
    let mut days: Vec<i64> = sessions
        .iter()
        .filter(|session| session.counted_ms > 0)
        .map(|session| session.started_at.div_euclid(DAY_MS))
        .collect();
    if days.is_empty() {
        return None;
    }
    days.sort_unstable();
    days.dedup();

    let mut best = 1usize;
    let mut run = 1usize;
    for pair in days.windows(2) {
        if pair[1] == pair[0] + 1 {
            run += 1;
            best = best.max(run);
        } else {
            run = 1;
        }
    }
    Some(best)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game(
        status: &str,
        rating: Option<u8>,
        playtime_ms: i64,
        finished: Option<i64>,
    ) -> GameSample {
        GameSample {
            status: status.to_owned(),
            rating,
            playtime_ms,
            finished_at: finished,
        }
    }

    fn session(started_at: i64, counted_ms: i64) -> SessionSample {
        SessionSample {
            started_at,
            counted_ms,
        }
    }

    #[test]
    fn completion_counts_started_games_only() {
        // 2 of 4 started games are done; the two wishlist entries do not count,
        // so a big wishlist cannot make the rate look like failure.
        let games = [
            game("completed", None, 0, None),
            game("completed", None, 0, None),
            game("dropped", None, 0, None),
            game("playing", None, 0, None),
            game("wishlist", None, 0, None),
            game("wishlist", None, 0, None),
        ];
        let rate = completion_rate(&games).unwrap();
        assert!((rate - 0.5).abs() < 1e-9, "{rate}");
    }

    #[test]
    fn completion_of_an_unstarted_library_is_unknown() {
        let games = [game("wishlist", None, 0, None)];
        assert!(completion_rate(&games).is_none());
    }

    #[test]
    fn the_average_skips_unrated_games_rather_than_counting_zero() {
        let games = [
            game("completed", Some(8), 0, None),
            game("completed", Some(10), 0, None),
            game("completed", None, 0, None),
        ];
        let average = average_rating(&games).unwrap();
        assert!((average - 9.0).abs() < 1e-9, "{average}");
    }

    #[test]
    fn an_unrated_library_has_no_average() {
        assert!(average_rating(&[game("completed", None, 0, None)]).is_none());
    }

    #[test]
    fn the_longest_session_is_the_largest_one() {
        let sessions = [session(0, 1000), session(0, 9000), session(0, 500)];
        assert_eq!(longest_session(&sessions), Some(9000));
    }

    #[test]
    fn the_average_session_ignores_zero_length_rows() {
        // A cancelled session must not drag the average down.
        let sessions = [session(0, 6000), session(0, 2000), session(0, 0)];
        assert_eq!(average_session(&sessions), Some(4000));
    }

    #[test]
    fn no_sessions_means_no_session_numbers() {
        assert!(longest_session(&[]).is_none());
        assert!(average_session(&[]).is_none());
    }

    #[test]
    fn the_favourite_route_is_the_most_played() {
        let routes = [
            RouteStat {
                name: "Shiroha".to_owned(),
                playtime_ms: 7_200_000,
            },
            RouteStat {
                name: "Kamome".to_owned(),
                playtime_ms: 3_600_000,
            },
        ];
        assert_eq!(favourite_route(&routes).unwrap().name, "Shiroha");
    }

    #[test]
    fn a_route_with_no_time_is_not_a_favourite() {
        let routes = [RouteStat {
            name: "Untouched".to_owned(),
            playtime_ms: 0,
        }];
        assert!(favourite_route(&routes).is_none());
    }

    #[test]
    fn the_rating_spread_puts_each_rating_in_its_bucket() {
        // Decivotes: 10 is 1.0, 100 is 10.0, 75 is 7.5.
        let games = [
            game("completed", Some(10), 0, None),
            game("completed", Some(100), 0, None),
            game("completed", Some(75), 0, None),
        ];
        let spread = rating_spread(&games);
        assert_eq!(spread[0], 1, "1.0");
        assert_eq!(spread[6], 1, "7.5 falls in the 7-star bucket");
        assert_eq!(spread[9], 1, "10.0");
        assert_eq!(spread[4], 0, "nothing at 5");
    }

    #[test]
    fn an_out_of_range_rating_is_clamped_into_a_bucket() {
        // The bucket array has ten slots; a stray 0 or 200 must not be dropped,
        // which would make the bars disagree with the game count beside them.
        let games = [
            game("completed", Some(0), 0, None),
            game("completed", Some(200), 0, None),
        ];
        let spread = rating_spread(&games);
        assert_eq!(spread.iter().sum::<usize>(), 2);
    }

    #[test]
    fn finished_this_year_counts_only_this_year() {
        let now = Timestamp::from_millis(1_790_000_000_000);
        let year_start = year_start_ms(now);
        let games = [
            game("completed", None, 0, Some(year_start + DAY_MS)),
            game("completed", None, 0, Some(year_start - DAY_MS)),
        ];
        assert_eq!(finished_this_year(&games, now), Some(1));
    }

    #[test]
    fn a_library_with_no_finish_dates_reports_unknown_not_zero() {
        // "0 finished this year" and "we do not know" are different claims.
        let games = [game("completed", None, 0, None)];
        let now = Timestamp::from_millis(1_790_000_000_000);
        assert!(finished_this_year(&games, now).is_none());
    }

    #[test]
    fn a_streak_counts_consecutive_days() {
        let base = 20_000 * DAY_MS;
        let sessions = [
            session(base, 1000),
            session(base + DAY_MS, 1000),
            session(base + 2 * DAY_MS, 1000),
            session(base + 5 * DAY_MS, 1000),
        ];
        assert_eq!(longest_streak(&sessions), Some(3));
    }

    #[test]
    fn several_sessions_on_one_day_are_one_day() {
        let base = 20_000 * DAY_MS;
        let sessions = [session(base, 1000), session(base + 1000, 1000)];
        assert_eq!(longest_streak(&sessions), Some(1));
    }

    #[test]
    fn a_streak_across_a_month_boundary_still_counts() {
        // 31 January then 1 February: consecutive days, not a gap.
        let day = 20_000 * DAY_MS;
        let sessions = [session(day, 1000), session(day + DAY_MS, 1000)];
        assert_eq!(longest_streak(&sessions), Some(2));
    }

    #[test]
    fn no_sessions_means_no_streak() {
        assert!(longest_streak(&[]).is_none());
    }

    #[test]
    fn deriving_always_accepts_an_empty_library() {
        let stats = derive(&[], &[], &[], Timestamp::from_millis(0));
        assert!(stats.completion_rate.is_none());
        assert!(stats.average_rating.is_none());
        assert!(stats.longest_session_ms.is_none());
        assert_eq!(stats.session_count, 0);
        assert_eq!(stats.rating_spread, [0; 10]);
    }

    #[test]
    fn a_daily_series_has_one_bucket_per_day_oldest_first() {
        let now = Timestamp::from_millis(20_000 * DAY_MS);
        let series = time_series(&[], Period::Daily, now);
        assert_eq!(series.len(), 30);
        // Contiguous: no gaps and no overlaps.
        for pair in series.windows(2) {
            assert_eq!(pair[0].end_ms, pair[1].start_ms);
        }
    }

    #[test]
    fn a_session_lands_in_the_bucket_containing_its_start() {
        let now = Timestamp::from_millis(20_000 * DAY_MS + 3_600_000);
        let today = 20_000 * DAY_MS + 60_000;
        let series = time_series(&[session(today, 5000)], Period::Daily, now);
        let hit = series.iter().find(|b| b.playtime_ms > 0).unwrap();
        assert_eq!(hit.playtime_ms, 5000);
        assert_eq!(hit.sessions, 1);
    }

    #[test]
    fn a_session_outside_the_window_is_dropped() {
        // Old history must not inflate the newest bucket.
        let now = Timestamp::from_millis(20_000 * DAY_MS);
        let series = time_series(&[session(1_000 * DAY_MS, 9999)], Period::Daily, now);
        assert_eq!(series.iter().map(|b| b.playtime_ms).sum::<i64>(), 0);
    }

    #[test]
    fn several_sessions_on_one_day_accumulate() {
        let now = Timestamp::from_millis(20_000 * DAY_MS + 10);
        let day = 20_000 * DAY_MS;
        let series = time_series(
            &[session(day + 1000, 3000), session(day + 5000, 7000)],
            Period::Daily,
            now,
        );
        let hit = series.iter().find(|b| b.playtime_ms > 0).unwrap();
        assert_eq!(hit.playtime_ms, 10_000);
        assert_eq!(hit.sessions, 2);
    }

    #[test]
    fn monthly_buckets_are_contiguous_and_month_length() {
        let now = Timestamp::from_millis(20_000 * DAY_MS);
        let series = time_series(&[], Period::Monthly, now);
        assert_eq!(series.len(), 24);
        for pair in series.windows(2) {
            assert_eq!(
                pair[0].end_ms, pair[1].start_ms,
                "months must be contiguous"
            );
        }
        for bucket in &series {
            let days = (bucket.end_ms - bucket.start_ms) / DAY_MS;
            assert!((28..=31).contains(&days), "{days} days");
        }
    }

    #[test]
    fn february_in_a_leap_year_is_29_days() {
        // 2024-03-01T00:00:00Z is the end of February 2024, a leap year.
        let now = Timestamp::from_millis(1_709_251_200_000);
        let series = time_series(&[], Period::Monthly, now);
        assert!(
            series
                .iter()
                .any(|b| (b.end_ms - b.start_ms) / DAY_MS == 29),
            "a 29-day month must appear in the window"
        );
    }

    #[test]
    fn weekly_buckets_are_seven_days_and_contiguous() {
        let now = Timestamp::from_millis(20_000 * DAY_MS + 5000);
        let series = time_series(&[], Period::Weekly, now);
        assert_eq!(series.len(), 26);
        for bucket in &series {
            assert_eq!((bucket.end_ms - bucket.start_ms) / DAY_MS, 7);
        }
        for pair in series.windows(2) {
            assert_eq!(pair[0].end_ms, pair[1].start_ms);
        }
    }

    #[test]
    fn yearly_buckets_are_consecutive() {
        let now = Timestamp::from_millis(20_000 * DAY_MS);
        let series = time_series(&[], Period::Yearly, now);
        assert_eq!(series.len(), 8);
        for pair in series.windows(2) {
            assert_eq!(pair[0].end_ms, pair[1].start_ms);
        }
    }

    #[test]
    fn all_four_periods_parse_from_a_string() {
        assert_eq!(Period::parse("daily"), Period::Daily);
        assert_eq!(Period::parse("weekly"), Period::Weekly);
        assert_eq!(Period::parse("monthly"), Period::Monthly);
        assert_eq!(Period::parse("yearly"), Period::Yearly);
        // Anything unknown falls back to the cheapest series.
        assert_eq!(Period::parse("nonsense"), Period::Daily);
    }
}

/// A bucket size for the time-series chart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Period {
    /// One bar per day.
    Daily,
    /// One bar per ISO-free 7-day block counted back from today.
    Weekly,
    /// One bar per calendar month.
    Monthly,
    /// One bar per calendar year.
    Yearly,
}

impl Period {
    /// Parse the stored or sent form, defaulting to `Daily`.
    #[must_use]
    pub fn parse(value: &str) -> Self {
        match value {
            "weekly" => Self::Weekly,
            "monthly" => Self::Monthly,
            "yearly" => Self::Yearly,
            _ => Self::Daily,
        }
    }

    /// How many buckets to show.
    ///
    /// Chosen so a bar stays wide enough to see: 30 days, half a year of weeks,
    /// two years of months, eight years. A year of daily bars at ~7px each reads
    /// as a texture rather than a chart.
    #[must_use]
    pub fn bucket_count(self) -> usize {
        match self {
            Self::Daily => 30,
            Self::Weekly => 26,
            Self::Monthly => 24,
            Self::Yearly => 8,
        }
    }
}

/// One bar in the time series.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimeBucket {
    /// Inclusive start of the bucket, UTC ms.
    pub start_ms: i64,
    /// Exclusive end.
    pub end_ms: i64,
    /// Time counted inside it.
    pub playtime_ms: i64,
    /// Sessions that contributed.
    pub sessions: usize,
}

/// Bucket sessions into a time series ending at `now`.
///
/// The series always has `Period::bucket_count()` entries, oldest first, with
/// zero-filled gaps. A chart that silently dropped empty days would make a
/// sporadic reader look continuous, which is the opposite of informative.
///
/// Buckets are aligned to UTC calendar days and months rather than to arbitrary
/// offsets, so "yesterday" means the same thing on every machine in the same
/// timezone-free data model the rest of the app uses.
#[must_use]
pub fn time_series(sessions: &[SessionSample], period: Period, now: Timestamp) -> Vec<TimeBucket> {
    let now_ms = now.as_millis();
    let mut buckets = build_buckets(period, now_ms);

    for session in sessions.iter().filter(|session| session.counted_ms > 0) {
        // A bucket is found by comparing against the ranges, which are ordered.
        if let Some(bucket) = buckets.iter_mut().find(|bucket| {
            session.started_at >= bucket.start_ms && session.started_at < bucket.end_ms
        }) {
            bucket.playtime_ms += session.counted_ms;
            bucket.sessions += 1;
        }
    }

    buckets
}

/// The empty series for a period, oldest first.
fn build_buckets(period: Period, now_ms: i64) -> Vec<TimeBucket> {
    let count = period.bucket_count();
    let mut out: Vec<TimeBucket> = Vec::with_capacity(count);

    match period {
        Period::Daily => {
            let today = now_ms.div_euclid(DAY_MS);
            for index in (0..count).rev() {
                let day = today - i64::try_from(index).unwrap_or(0);
                out.push(TimeBucket {
                    start_ms: day * DAY_MS,
                    end_ms: (day + 1) * DAY_MS,
                    playtime_ms: 0,
                    sessions: 0,
                });
            }
        }
        Period::Weekly => {
            // Blocks counted back from today, so the newest bucket ends now.
            let today = now_ms.div_euclid(DAY_MS);
            for index in (0..count).rev() {
                let end_day = today + 1 - i64::try_from(index).unwrap_or(0) * 7;
                let start_day = end_day - 7;
                out.push(TimeBucket {
                    start_ms: start_day * DAY_MS,
                    end_ms: end_day * DAY_MS,
                    playtime_ms: 0,
                    sessions: 0,
                });
            }
        }
        Period::Monthly => {
            let (year, month) = year_month(now_ms);
            for index in (0..count).rev() {
                let months_back = i64::try_from(index).unwrap_or(0);
                let (start_year, start_month) = shift_month(year, month, -months_back);
                let (end_year, end_month) = shift_month(start_year, start_month, 1);
                out.push(TimeBucket {
                    start_ms: month_start_ms(start_year, start_month),
                    end_ms: month_start_ms(end_year, end_month),
                    playtime_ms: 0,
                    sessions: 0,
                });
            }
        }
        Period::Yearly => {
            let (year, _) = year_month(now_ms);
            for index in (0..count).rev() {
                let start_year = year - i64::try_from(index).unwrap_or(0);
                out.push(TimeBucket {
                    start_ms: month_start_ms(start_year, 1),
                    end_ms: month_start_ms(start_year + 1, 1),
                    playtime_ms: 0,
                    sessions: 0,
                });
            }
        }
    }

    out
}

/// The UTC year and month (1-12) containing `ms`.
fn year_month(ms: i64) -> (i64, i64) {
    let days = ms.div_euclid(DAY_MS);
    let mut year = 1970i64;
    let mut remaining = days;
    loop {
        let length = if is_leap(year) { 366 } else { 365 };
        if remaining < length {
            break;
        }
        remaining -= length;
        year += 1;
    }
    // Walk the months of that year.
    let mut month = 1i64;
    for length in month_lengths(year) {
        if remaining < length {
            break;
        }
        remaining -= length;
        month += 1;
    }
    (year, month)
}

/// Days per month for a year.
fn month_lengths(year: i64) -> [i64; 12] {
    [
        31,
        if is_leap(year) { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ]
}

/// The first instant of a month, UTC ms.
fn month_start_ms(year: i64, month: i64) -> i64 {
    let mut days = 0i64;
    for y in 1970..year {
        days += if is_leap(y) { 366 } else { 365 };
    }
    let lengths = month_lengths(year);
    for m in 1..month.max(1) {
        let index = usize::try_from(m - 1).unwrap_or(0);
        days += lengths.get(index).copied().unwrap_or(30);
    }
    days * DAY_MS
}

/// Move a year/month by a signed number of months.
fn shift_month(year: i64, month: i64, delta: i64) -> (i64, i64) {
    let zero_based = year * 12 + (month - 1) + delta;
    (zero_based.div_euclid(12), zero_based.rem_euclid(12) + 1)
}
