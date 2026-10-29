//! Timing a chapter by hand, when the engine cannot be detected.
//!
//! # Why this exists
//!
//! Detection is not available for every engine. `KiriKiri` keeps no chapter in its
//! window title and its script engine is statically linked, so there is no
//! verified way to read the current scene. Showing "no chapter" forever is honest
//! but useless.
//!
//! A manual timer is the fallback that always works: the user says which chapter
//! they are playing and the app times it exactly as it times everything else. The
//! output is a normal play session, so reports, sync and exports treat it like any
//! detected chapter.
//!
//! # What it deliberately does not do
//!
//! It does not run alongside detection. If a manual timer is running, detected
//! chapter changes are ignored for that game: two sources writing segments for the
//! same stretch of time would double-count it.

use serde::{Deserialize, Serialize};
use vn_core::clock::Timestamp;
use vn_core::ids::EntityId;

/// A running manual timer.
///
/// The target is either a chapter or a route: a user who starts "the Shiroha route"
/// should not have to invent a chapter to blame the hour on. Exactly one is set,
/// which the constructors enforce.
///
/// # Counting
///
/// Time is not wall clock. The timer consumes the same samples the automatic
/// tracker does — the OS idle duration and, while a game window is being tracked,
/// whether that window is in front — and splits them with
/// `vn_core::tracker::away_ms_in_gap`, the exact rule playtime uses. So a manual
/// chapter, an automatic session and the statistics all describe the same stretch
/// the same way, and a manual timer left running while the window is behind the
/// browser does not buy minutes nobody read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManualRun {
    /// The chapter being timed, when the target is a chapter.
    pub chapter_id: Option<EntityId>,
    /// The route being timed, when the target is a route rather than a chapter.
    pub route_id: Option<EntityId>,
    /// The game it belongs to, so a second game cannot start a competing timer.
    pub game_id: EntityId,
    /// When the user pressed start.
    pub started_at: Timestamp,
    /// Counted time, classified the way playtime is.
    pub active_ms: i64,
    /// Time the timer was running while the user was away.
    pub afk_ms: i64,
    /// When the current accrual window opened, or `None` while paused.
    last_tick_at: Option<Timestamp>,
    /// The idle reading of the last sample, so pausing and stopping can close the
    /// open window with the classification it actually had instead of guessing.
    last_idle_for_ms: i64,
    /// The grace threshold in force at that sample.
    last_threshold_ms: i64,
}

/// How one sample of the manual timer should be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManualSample {
    /// The stretch counts, split by the idle reading.
    Count {
        /// How long the system had been without input, as the OS reports it.
        idle_for_ms: i64,
        /// One away-window ends here, so everything counts as play.
        all_active: bool,
        /// Grace threshold for the idle reading.
        threshold_ms: i64,
    },
    /// The game window was not in front. The timer keeps its total and the gap is
    /// not counted, exactly as automatic playtime pauses when the game loses focus.
    Skip,
}

/// Why a manual timer request was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ManualError {
    /// A timer is already running for a different chapter.
    #[error("a manual timer is already running for another chapter")]
    AlreadyRunning,
    /// Nothing is running to stop or pause.
    #[error("no manual timer is running")]
    NotRunning,
    /// The timer is already paused.
    #[error("the manual timer is already paused")]
    AlreadyPaused,
}

impl ManualRun {
    /// Start timing a chapter at `now`.
    #[must_use]
    pub fn start(chapter_id: EntityId, game_id: EntityId, now: Timestamp) -> Self {
        Self::new(Some(chapter_id), None, game_id, now)
    }

    /// Start timing a route rather than a chapter.
    #[must_use]
    pub fn start_route(route_id: EntityId, game_id: EntityId, now: Timestamp) -> Self {
        Self::new(None, Some(route_id), game_id, now)
    }

    fn new(
        chapter_id: Option<EntityId>,
        route_id: Option<EntityId>,
        game_id: EntityId,
        now: Timestamp,
    ) -> Self {
        Self {
            chapter_id,
            route_id,
            game_id,
            started_at: now,
            active_ms: 0,
            afk_ms: 0,
            last_tick_at: Some(now),
            last_idle_for_ms: 0,
            last_threshold_ms: 0,
        }
    }

    /// Whether time is currently accumulating.
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.last_tick_at.is_some()
    }

    /// Total counted time, the number the clock shows.
    #[must_use]
    pub fn total_ms(&self) -> i64 {
        self.active_ms + self.afk_ms
    }

    /// Counted time plus the open window, for display.
    ///
    /// The timer only classifies a stretch when a sample arrives, so reading
    /// `total_ms` between polls would freeze the clock for up to a poll interval.
    /// The open window is added whole: both of its buckets end up in the total, and
    /// the split is decided when the sample that closes it arrives.
    #[must_use]
    pub fn projected_total(&self, now: Timestamp) -> i64 {
        match self.last_tick_at {
            Some(from) => self.total_ms() + (now.as_millis() - from.as_millis()).max(0),
            None => self.total_ms(),
        }
    }

    /// Consume one sample, measured from the previous one.
    ///
    /// `idle_for_ms` is the OS's own reading, so the split matches the automatic
    /// tracker: the part of the gap past the grace threshold is away, the rest is
    /// play. `all_active` is the wall-clock mode, where nothing is away.
    pub fn tick(&mut self, now: Timestamp, sample: ManualSample) {
        let Some(from) = self.last_tick_at else {
            return;
        };
        let gap = (now.as_millis() - from.as_millis()).max(0);
        match sample {
            ManualSample::Skip => {}
            ManualSample::Count {
                idle_for_ms,
                all_active,
                threshold_ms,
            } => {
                let away = if all_active {
                    0
                } else {
                    vn_core::tracker::away_ms_in_gap(gap, idle_for_ms, threshold_ms)
                };
                self.afk_ms += away;
                self.active_ms += gap - away;
                self.last_idle_for_ms = idle_for_ms;
                self.last_threshold_ms = threshold_ms;
            }
        }
        self.last_tick_at = Some(now);
    }

    /// Stop accumulating without ending the run.
    ///
    /// # Errors
    /// Fails when the timer is already paused.
    pub fn pause(&mut self, now: Timestamp) -> Result<(), ManualError> {
        if self.last_tick_at.is_none() {
            return Err(ManualError::AlreadyPaused);
        }
        // Close the open window with the last classification seen, rather than
        // dropping or inventing it.
        self.tick(
            now,
            ManualSample::Count {
                idle_for_ms: self.last_idle_for_ms,
                all_active: false,
                threshold_ms: self.last_threshold_ms,
            },
        );
        self.last_tick_at = None;
        Ok(())
    }

    /// Resume accumulation.
    ///
    /// # Errors
    /// Fails when the timer is not paused.
    pub fn resume(&mut self, now: Timestamp) -> Result<(), ManualError> {
        if self.last_tick_at.is_some() {
            return Err(ManualError::AlreadyRunning);
        }
        self.last_tick_at = Some(now);
        Ok(())
    }

    /// Finish the run, returning `(counted, away)` to record.
    ///
    /// The trailing window is closed with the same classification as the last
    /// sample, and `exclude_afk = false` folds away time into play exactly as it
    /// does for a detected session.
    pub fn finish(&mut self, now: Timestamp, afk: vn_core::afk::AfkConfig) -> (i64, i64) {
        self.tick(
            now,
            ManualSample::Count {
                idle_for_ms: self.last_idle_for_ms,
                all_active: false,
                threshold_ms: self.last_threshold_ms,
            },
        );
        self.last_tick_at = None;
        let totals = vn_core::afk::finish(
            vn_core::afk::ActivityTotals {
                active_ms: self.active_ms,
                afk_ms: self.afk_ms,
            },
            afk,
        );
        (totals.active_ms, totals.afk_ms)
    }
}

/// Which chapter the tracker should attribute time to.
///
/// A manual run wins over detection because the user stated it explicitly. `None`
/// means detection decides. A route-timed run yields `None` here on purpose: it does
/// not name a chapter, so detection is not overridden by a chapter it never picked.
/// A timer belonging to another game never attributes this game's time.
#[must_use]
pub fn manual_override(run: Option<&ManualRun>, game_id: EntityId) -> Option<EntityId> {
    run.filter(|run| run.game_id == game_id)
        .and_then(|run| run.chapter_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vn_core::afk::AfkConfig;

    fn t(ms: i64) -> Timestamp {
        Timestamp::from_millis(1_700_000_000_000 + ms)
    }

    fn ids() -> (EntityId, EntityId) {
        (EntityId::new(), EntityId::new())
    }

    /// One sample where the user has been present for the whole gap.
    fn present() -> ManualSample {
        ManualSample::Count {
            idle_for_ms: 0,
            all_active: false,
            threshold_ms: 60_000,
        }
    }

    /// One sample where the OS says the user has been idle for `idle_for_ms`.
    fn idle(idle_for_ms: i64) -> ManualSample {
        ManualSample::Count {
            idle_for_ms,
            all_active: false,
            threshold_ms: 60_000,
        }
    }

    #[test]
    fn a_present_sample_counts_as_play() {
        let (chapter, game) = ids();
        let mut run = ManualRun::start(chapter, game, t(0));
        assert!(run.is_running());
        run.tick(t(5_000), present());
        assert_eq!(run.active_ms, 5_000);
        assert_eq!(run.afk_ms, 0);
    }

    #[test]
    fn the_clock_reports_counted_time_not_wall_clock() {
        // The point of the change: the number in the UI is what playtime would
        // record, so a manual timer cannot outrun a detected session.
        let (chapter, game) = ids();
        let mut run = ManualRun::start(chapter, game, t(0));
        run.tick(t(10_000), idle(130_000)); // idle past the grace: away
        assert_eq!(run.total_ms(), 10_000, "the sample is still counted");
        assert_eq!(run.active_ms, 0);
        assert_eq!(run.afk_ms, 10_000);
    }

    #[test]
    fn only_the_part_past_the_grace_threshold_is_away() {
        // Identical to `away_ms_in_gap` for detected playtime: 30 s idle against a
        // 60 s threshold is reading time, 90 s idle over a 60 s gap is 30 s away.
        let (chapter, game) = ids();
        let mut run = ManualRun::start(chapter, game, t(0));
        run.tick(t(60_000), idle(30_000));
        assert_eq!(run.active_ms, 60_000);
        assert_eq!(run.afk_ms, 0);

        run.tick(t(120_000), idle(90_000));
        assert_eq!(run.afk_ms, 30_000);
        assert_eq!(run.active_ms, 90_000);
    }

    #[test]
    fn a_skipped_sample_counts_nothing() {
        // The game window was not in front, which is where automatic playtime
        // pauses instead of billing the gap.
        let (chapter, game) = ids();
        let mut run = ManualRun::start(chapter, game, t(0));
        run.tick(t(30_000), ManualSample::Skip);
        assert_eq!(run.total_ms(), 0);
        run.tick(t(35_000), present());
        assert_eq!(run.total_ms(), 5_000);
    }

    #[test]
    fn pausing_freezes_the_total() {
        let (chapter, game) = ids();
        let mut run = ManualRun::start(chapter, game, t(0));
        run.tick(t(4_000), present());
        run.pause(t(4_000)).expect("pause");
        assert!(!run.is_running());
        // Time passing while paused must not count.
        run.tick(t(90_000), present());
        assert_eq!(run.total_ms(), 4_000);
    }

    #[test]
    fn resuming_continues_from_the_paused_total() {
        let (chapter, game) = ids();
        let mut run = ManualRun::start(chapter, game, t(0));
        run.tick(t(4_000), present());
        run.pause(t(5_000)).expect("pause");
        run.resume(t(10_000)).expect("resume");
        run.tick(t(12_000), present());
        // 4 s before the pause, plus the 1 s open window closed by it, plus 2 s.
        assert_eq!(run.total_ms(), 7_000);
    }

    #[test]
    fn finishing_reports_both_buckets() {
        let (chapter, game) = ids();
        let mut run = ManualRun::start(chapter, game, t(0));
        run.tick(t(10_000), present());
        run.tick(t(20_000), idle(130_000));
        let (active, away) = run.finish(t(21_000), AfkConfig::default());
        // 10 s present, then 11 s idle past the grace (the last second closes the
        // open window with the classification the sample had).
        assert_eq!(active, 10_000);
        assert_eq!(away, 11_000);
    }

    #[test]
    fn finishing_with_afk_exclusion_off_folds_away_into_play() {
        // Same rule as a detected session: the user still sees they were away,
        // but the time is credited as reading.
        let (chapter, game) = ids();
        let mut run = ManualRun::start(chapter, game, t(0));
        run.tick(t(10_000), idle(130_000));
        let (active, away) = run.finish(t(10_000), AfkConfig::new(60_000, false));
        assert_eq!(active, 10_000);
        assert_eq!(away, 0);
    }

    #[test]
    fn a_backwards_clock_never_subtracts_time() {
        // A system clock change must not reduce recorded playtime.
        let (chapter, game) = ids();
        let mut run = ManualRun::start(chapter, game, t(10_000));
        run.tick(t(5_000), present());
        assert_eq!(run.total_ms(), 0);
    }

    #[test]
    fn pausing_twice_is_refused() {
        let (chapter, game) = ids();
        let mut run = ManualRun::start(chapter, game, t(0));
        run.pause(t(1_000)).expect("first pause");
        assert_eq!(run.pause(t(2_000)), Err(ManualError::AlreadyPaused));
    }

    #[test]
    fn resuming_a_running_timer_is_refused() {
        let (chapter, game) = ids();
        let mut run = ManualRun::start(chapter, game, t(0));
        assert_eq!(run.resume(t(1_000)), Err(ManualError::AlreadyRunning));
    }

    #[test]
    fn a_route_run_names_no_chapter() {
        // Timing a route must not silently claim a chapter: `manual_override` feeds
        // detection, and returning a chapter here would attribute detected time to a
        // chapter the user never picked.
        let (route, game) = ids();
        let run = ManualRun::start_route(route, game, t(0));
        assert_eq!(run.route_id, Some(route));
        assert_eq!(run.chapter_id, None);
        assert_eq!(manual_override(Some(&run), game), None);
    }

    #[test]
    fn a_chapter_run_names_no_route() {
        // The mirror of the above: exactly one target, so the segment writer cannot
        // produce a row claiming both.
        let (chapter, game) = ids();
        let run = ManualRun::start(chapter, game, t(0));
        assert_eq!(run.chapter_id, Some(chapter));
        assert_eq!(run.route_id, None);
    }

    #[test]
    fn a_route_run_accumulates_like_a_chapter_run() {
        // A route timer is the same timer with a different target: the counting
        // rules must not depend on which one was started.
        let (route, game) = ids();
        let mut run = ManualRun::start_route(route, game, t(0));
        run.tick(t(4_000), present());
        run.pause(t(4_000)).expect("pause");
        run.resume(t(10_000)).expect("resume");
        run.tick(t(12_000), present());
        assert_eq!(run.total_ms(), 6_000);
    }

    #[test]
    fn a_timer_for_another_game_never_attributes_this_one() {
        // Two games open, a timer running on the other one: this game must fall back
        // to detection rather than inherit a chapter it has nothing to do with.
        let (chapter, game) = ids();
        let other_game = EntityId::new();
        let run = ManualRun::start(chapter, other_game, t(0));
        assert_eq!(manual_override(Some(&run), game), None);
    }

    #[test]
    fn a_manual_run_overrides_detection() {
        let (chapter, game) = ids();
        let run = ManualRun::start(chapter, game, t(0));
        assert_eq!(manual_override(Some(&run), game), Some(chapter));
        assert_eq!(manual_override(None, game), None);
    }
}
