//! Timestamp formatting and aggregation helpers.
//!
//! Formatting is locale-independent: the frontend owns locale-aware display via
//! `Intl`. These helpers produce the stable buckets (day, month) that stats and
//! the heatmap rely on, so the bucketing rules are testable in one place.

use crate::clock::Timestamp;

pub const MS_PER_DAY: i64 = 86_400_000;
pub const MS_PER_HOUR: i64 = 3_600_000;
pub const MS_PER_MINUTE: i64 = 60_000;
pub const MS_PER_SECOND: i64 = 1_000;

/// Floored whole minutes (clamped at 0).
#[must_use]
pub const fn minutes_of(ms: i64) -> i64 {
    if ms <= 0 { 0 } else { ms / MS_PER_MINUTE }
}

/// Floored whole seconds (clamped at 0).
#[must_use]
pub const fn seconds_of(ms: i64) -> i64 {
    if ms <= 0 { 0 } else { ms / MS_PER_SECOND }
}

/// True for non-zero durations under one minute (tracker records ~4s slices that floor to 0 min).
#[must_use]
pub const fn is_sub_minute(ms: i64) -> bool {
    ms > 0 && ms < MS_PER_MINUTE
}

/// Split a duration into `(hours, minutes)`, discarding seconds.
#[must_use]
pub const fn hours_minutes(ms: i64) -> (i64, i64) {
    let total_minutes = minutes_of(ms);
    (total_minutes / 60, total_minutes % 60)
}

/// Calendar day key in UTC (days since the Unix epoch).
/// Limitation: sessions near midnight can land in the adjacent UTC day for local timezones.
#[must_use]
pub const fn utc_day_index(ts: Timestamp) -> i64 {
    ts.as_millis().div_euclid(MS_PER_DAY)
}

/// UTC month key `(year, month)` with month in `1..=12`.
#[must_use]
pub fn utc_month_index(ts: Timestamp) -> (i32, u8) {
    let days = utc_day_index(ts);
    days_to_civil(days)
}

/// Convert days since the Unix epoch to `(year, month)`.
///
/// Howard Hinnant's `civil_from_days`, restricted to the fields we need.
fn days_to_civil(days: i64) -> (i32, u8) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let m = mp + if mp < 10 { 3 } else { -9 }; // [1, 12]
    let year = y + i64::from(m <= 2);
    (
        i32::try_from(year).unwrap_or(i32::MAX),
        u8::try_from(m).unwrap_or(1),
    )
}

/// Format a duration as `H:MM` for tabular display.
#[must_use]
pub fn format_duration(ms: i64) -> String {
    let (hours, minutes) = hours_minutes(ms);
    format!("{hours}:{minutes:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minutes_floor_and_clamp() {
        assert_eq!(minutes_of(0), 0);
        assert_eq!(minutes_of(-5_000), 0);
        assert_eq!(minutes_of(59_999), 0);
        assert_eq!(minutes_of(60_000), 1);
        assert_eq!(minutes_of(119_999), 1);
    }

    #[test]
    fn hours_minutes_split() {
        assert_eq!(hours_minutes(0), (0, 0));
        assert_eq!(hours_minutes(90 * MS_PER_MINUTE), (1, 30));
        assert_eq!(hours_minutes(10 * MS_PER_HOUR), (10, 0));
    }

    #[test]
    fn a_short_but_real_session_is_not_reported_as_zero() {
        // The rows that read "0 min": counted time between one and 59 seconds.
        assert!(is_sub_minute(4_000));
        assert!(is_sub_minute(59_999));
        assert!(!is_sub_minute(60_000));
        // Zero and negative values are not "short", they are absent.
        assert!(!is_sub_minute(0));
        assert!(!is_sub_minute(-5_000));
        assert_eq!(seconds_of(4_000), 4);
        assert_eq!(seconds_of(-1), 0);
    }

    #[test]
    fn format_duration_pads_minutes() {
        assert_eq!(format_duration(0), "0:00");
        assert_eq!(format_duration(5 * MS_PER_MINUTE), "0:05");
        assert_eq!(format_duration(3 * MS_PER_HOUR + 7 * MS_PER_MINUTE), "3:07");
    }

    #[test]
    fn epoch_is_day_zero() {
        assert_eq!(utc_day_index(Timestamp::EPOCH), 0);
        assert_eq!(utc_month_index(Timestamp::EPOCH), (1970, 1));
    }

    #[test]
    fn known_dates_resolve_correctly() {
        // 2024-02-29T12:00:00Z (leap day)
        let leap = Timestamp::from_millis(1_709_208_000_000);
        assert_eq!(utc_month_index(leap), (2024, 2));

        // 2025-12-31T23:59:59Z
        let end_of_year = Timestamp::from_millis(1_767_225_599_000);
        assert_eq!(utc_month_index(end_of_year), (2025, 12));

        // 2026-01-01T00:00:00Z
        let new_year = Timestamp::from_millis(1_767_225_600_000);
        assert_eq!(utc_month_index(new_year), (2026, 1));
        assert_ne!(utc_day_index(end_of_year), utc_day_index(new_year));
    }

    #[test]
    fn day_index_is_monotonic_across_a_month() {
        let start = utc_day_index(Timestamp::from_millis(1_767_225_600_000));
        let later = utc_day_index(Timestamp::from_millis(1_767_225_600_000 + 30 * MS_PER_DAY));
        assert_eq!(later - start, 30);
    }

    #[test]
    fn pre_epoch_times_do_not_panic() {
        let ancient = Timestamp::from_millis(-2_208_988_800_000); // 1900-01-01
        assert_eq!(utc_month_index(ancient), (1900, 1));
    }
}
