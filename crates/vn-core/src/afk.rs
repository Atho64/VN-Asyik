//! AFK (away from keyboard) accounting.
//!
//! Rule: a session is a sequence of samples of "is the user active". Consecutive
//! inactive samples are only counted as AFK once they run longer than the
//! configured threshold. That grace window is what makes this usable for visual
//! novels: reading a long passage produces no input for tens of seconds, and
//! treating that as AFK would delete real reading time.
//!
//! The rule is expressed as a pure fold so it can be tested without any OS,
//! clock or window focus involvement.

use crate::clock::Timestamp;

/// Default grace before inactivity counts as AFK: 60 seconds.
///
/// Chosen because VN reading routinely produces 30-45 s of silence; a shorter
/// window would silently drop legitimate play. Configurable in Settings, clamped
/// by [`AfkConfig::MIN_THRESHOLD_MS`] and [`AfkConfig::MAX_THRESHOLD_MS`].
pub const DEFAULT_AFK_THRESHOLD_MS: i64 = 60_000;

/// Whether idle stretches are excluded from counted playtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AfkConfig {
    /// How long the user must be inactive before the stretch counts as AFK.
    pub threshold_ms: i64,
    /// When false, idle time is still measured but folded into active time.
    pub exclude_afk: bool,
}

impl Default for AfkConfig {
    fn default() -> Self {
        Self {
            threshold_ms: DEFAULT_AFK_THRESHOLD_MS,
            exclude_afk: true,
        }
    }
}

impl AfkConfig {
    /// Shortest accepted threshold.
    pub const MIN_THRESHOLD_MS: i64 = 15_000;
    /// Longest accepted threshold.
    pub const MAX_THRESHOLD_MS: i64 = 1_800_000;

    /// Clamp a threshold into the accepted range.
    #[must_use]
    pub fn clamp_threshold(ms: i64) -> i64 {
        ms.clamp(Self::MIN_THRESHOLD_MS, Self::MAX_THRESHOLD_MS)
    }

    /// Build a config, clamping the threshold rather than rejecting it.
    #[must_use]
    pub fn new(threshold_ms: i64, exclude_afk: bool) -> Self {
        Self {
            threshold_ms: Self::clamp_threshold(threshold_ms),
            exclude_afk,
        }
    }
}

/// One observation of user activity, taken at a point in time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sample {
    /// When the sample was taken.
    pub at: Timestamp,
    /// Whether the user produced input recently enough to be considered present.
    pub active: bool,
}

/// Active and idle totals for a session, in milliseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct ActivityTotals {
    /// Time attributed to the user being present.
    pub active_ms: i64,
    /// Time attributed to the user being away.
    pub afk_ms: i64,
}

impl ActivityTotals {
    /// Wall-clock total of the accounted stretch.
    #[must_use]
    pub const fn total_ms(&self) -> i64 {
        self.active_ms + self.afk_ms
    }
}

/// Fold a sequence of samples into [`ActivityTotals`].
///
/// Time between two consecutive samples is attributed according to the *earlier*
/// sample: if the user was already away, the gap is idle time.
///
/// The grace rule is applied to the accumulated idle run, not per gap: the run
/// stays as reading time until its total exceeds `config.threshold_ms`. Once it
/// does, the whole run so far becomes AFK and every further idle gap in the run
/// accrues to AFK too, so the user is not charged for the first minute of a long
/// break. A run that ends below the threshold is folded back into active time.
///
/// The final sample contributes no duration of its own.
#[must_use]
pub fn account(samples: &[Sample], config: AfkConfig) -> ActivityTotals {
    if samples.len() < 2 {
        return ActivityTotals::default();
    }

    let mut totals = ActivityTotals::default();
    // Active time and the current idle run are accumulated separately, then the
    // run is resolved once when the user becomes active again or the samples end.
    let mut active_ms: i64 = 0;
    let mut idle_run_ms: i64 = 0;

    for pair in samples.windows(2) {
        let [previous, next] = [pair[0], pair[1]];
        let gap = next.at.saturating_diff_ms(previous.at).max(0);

        // A gap is attributed to the state the user was in for its duration,
        // which is the state captured by this (earlier) sample.
        if previous.active {
            resolve_idle_run(&mut totals, &mut active_ms, &mut idle_run_ms, config);
            active_ms += gap;
        } else {
            idle_run_ms += gap;
        }
    }

    // The session's own stretch is covered by the last sample, so resolve the
    // trailing run and commit the active total.
    resolve_idle_run(&mut totals, &mut active_ms, &mut idle_run_ms, config);
    totals.active_ms += active_ms;

    finish(totals, config)
}

/// Commit an idle run: AFK if it outlasts the threshold, otherwise reading time.
///
/// This is the grace rule for a fold over sampled activity. The live tracker does
/// not accumulate runs this way: it reads the OS's own idle duration on every poll
/// and charges only the part past the threshold (`tracker::away_ms_in_gap`), so a
/// break is away from its first second past the grace and a session that is being
/// clicked through never becomes a break at all.
pub fn resolve_idle_run(
    totals: &mut ActivityTotals,
    active_ms: &mut i64,
    idle_run_ms: &mut i64,
    config: AfkConfig,
) {
    if *idle_run_ms > config.threshold_ms {
        totals.afk_ms += *idle_run_ms;
    } else {
        *active_ms += *idle_run_ms;
    }
    *idle_run_ms = 0;
}

/// Settle the buckets at the end of an accounting stretch.
///
/// With `exclude_afk` off the user still wants to know they were away, but the time
/// is credited as play, so the two buckets merge into active.
#[must_use]
pub fn finish(totals: ActivityTotals, config: AfkConfig) -> ActivityTotals {
    if config.exclude_afk {
        totals
    } else {
        ActivityTotals {
            active_ms: totals.total_ms(),
            afk_ms: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(ms: i64) -> Timestamp {
        Timestamp::from_millis(ms)
    }

    fn active(at: i64) -> Sample {
        Sample {
            at: t(at),
            active: true,
        }
    }

    fn idle(at: i64) -> Sample {
        Sample {
            at: t(at),
            active: false,
        }
    }

    const S: i64 = 1_000;

    #[test]
    fn default_threshold_is_sixty_seconds_and_excludes_afk() {
        let config = AfkConfig::default();
        assert_eq!(config.threshold_ms, 60_000);
        assert!(config.exclude_afk);
    }

    #[test]
    fn threshold_is_clamped_into_range() {
        assert_eq!(AfkConfig::clamp_threshold(1), AfkConfig::MIN_THRESHOLD_MS);
        assert_eq!(
            AfkConfig::clamp_threshold(i64::MAX),
            AfkConfig::MAX_THRESHOLD_MS
        );
        assert_eq!(AfkConfig::new(1, true).threshold_ms, 15_000);
    }

    #[test]
    fn empty_and_single_sample_account_to_zero() {
        assert_eq!(
            account(&[], AfkConfig::default()),
            ActivityTotals::default()
        );
        assert_eq!(
            account(&[active(0)], AfkConfig::default()),
            ActivityTotals::default()
        );
    }

    #[test]
    fn continuous_activity_is_all_active() {
        let samples = [active(0), active(S), active(2 * S), active(3 * S)];
        let totals = account(&samples, AfkConfig::default());
        assert_eq!(totals.active_ms, 3 * S);
        assert_eq!(totals.afk_ms, 0);
    }

    #[test]
    fn idle_under_threshold_counts_as_reading() {
        // 5 s of input, then 45 s without: a long passage, not a break.
        let samples = [active(0), idle(5 * S), active(46 * S)];
        let totals = account(&samples, AfkConfig::default());
        assert_eq!(totals.active_ms, 46 * S);
        assert_eq!(totals.afk_ms, 0);
    }

    #[test]
    fn idle_over_threshold_counts_whole_run_as_afk() {
        // 2 s of input, then away for 118 s before coming back.
        let samples = [active(0), idle(2 * S), active(120 * S)];
        let totals = account(&samples, AfkConfig::default());
        assert_eq!(totals.active_ms, 2 * S);
        assert_eq!(totals.afk_ms, 118 * S);
        assert_eq!(totals.total_ms(), 120 * S);
    }

    #[test]
    fn a_long_run_is_counted_from_its_start() {
        // The user walked away at t=10; the whole stretch is AFK even though the
        // threshold only crosses 60 s later.
        let samples = [active(0), idle(10 * S), active(200 * S)];
        let totals = account(&samples, AfkConfig::default());
        assert_eq!(totals.active_ms, 10 * S);
        assert_eq!(totals.afk_ms, 190 * S);
    }

    #[test]
    fn multiple_afk_runs_accumulate() {
        // 10 s active, then away for 100 s, 5 s active, away for 200 s, 5 s active.
        let samples = [
            active(0),
            idle(10 * S),
            active(110 * S),
            idle(115 * S),
            active(315 * S),
        ];
        let totals = account(&samples, AfkConfig::default());
        assert_eq!(totals.afk_ms, 100 * S + 200 * S);
        assert_eq!(totals.active_ms, 10 * S + 5 * S);
        assert_eq!(totals.total_ms(), 315 * S);
    }

    #[test]
    fn excluding_afk_off_folds_idle_into_active() {
        let samples = [active(0), idle(5 * S), active(100 * S)];
        let totals = account(&samples, AfkConfig::new(60_000, false));
        assert_eq!(totals.active_ms, 100 * S);
        assert_eq!(totals.afk_ms, 0);
    }

    #[test]
    fn a_run_exactly_at_the_threshold_is_not_afk() {
        // Strictly greater than the threshold is required, so a run of exactly
        // 60 s stays as reading time.
        let samples = [active(0), idle(S), active(61 * S)];
        let totals = account(&samples, AfkConfig::default());
        assert_eq!(totals.afk_ms, 0);
        assert_eq!(totals.active_ms, 61 * S);
    }

    #[test]
    fn non_monotonic_timestamps_do_not_go_negative() {
        let samples = [active(100 * S), active(50 * S), active(200 * S)];
        let totals = account(&samples, AfkConfig::default());
        assert_eq!(totals.afk_ms, 0);
        assert!(totals.active_ms >= 0);
    }

    #[test]
    fn totals_always_match_wallclock() {
        let samples = [
            active(0),
            idle(20 * S),
            active(120 * S),
            idle(130 * S),
            active(400 * S),
        ];
        let totals = account(&samples, AfkConfig::default());
        assert_eq!(totals.total_ms(), 400 * S);
    }

    #[test]
    fn a_shorter_threshold_catches_more_breaks() {
        // 20 s of reading, 30 s away, then back.
        let samples = [active(0), idle(20 * S), active(50 * S)];
        // Under the 60 s default the 30 s silence is reading; with a 15 s
        // threshold it is a break.
        assert_eq!(account(&samples, AfkConfig::default()).afk_ms, 0);
        assert_eq!(
            account(&samples, AfkConfig::new(15_000, true)).afk_ms,
            30 * S
        );
    }

    #[test]
    fn never_ends_in_afk_while_still_active() {
        let samples = [active(0), idle(10 * S), active(500 * S)];
        let totals = account(&samples, AfkConfig::default());
        assert!(totals.afk_ms > 0);
        // Wallclock is preserved even though most of the session was away.
        assert_eq!(totals.total_ms(), 500 * S);
    }
}
