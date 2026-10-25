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
    /// Milliseconds accumulated so far, excluding paused time.
    pub elapsed_ms: i64,
    /// When the current stretch of accumulation began, or `None` while paused.
    pub resumed_at: Option<Timestamp>,
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
        Self {
            chapter_id: Some(chapter_id),
            route_id: None,
            game_id,
            started_at: now,
            elapsed_ms: 0,
            resumed_at: Some(now),
        }
    }

    /// Start timing a route rather than a chapter.
    #[must_use]
    pub fn start_route(route_id: EntityId, game_id: EntityId, now: Timestamp) -> Self {
        Self {
            chapter_id: None,
            route_id: Some(route_id),
            game_id,
            started_at: now,
            elapsed_ms: 0,
            resumed_at: Some(now),
        }
    }

    /// Whether time is currently accumulating.
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.resumed_at.is_some()
    }

    /// Total elapsed time at `now`, including the current stretch.
    #[must_use]
    pub fn elapsed_at(&self, now: Timestamp) -> i64 {
        match self.resumed_at {
            Some(from) => self.elapsed_ms + (now.as_millis() - from.as_millis()).max(0),
            None => self.elapsed_ms,
        }
    }

    /// Stop accumulating without ending the run.
    ///
    /// # Errors
    /// Fails when the timer is already paused.
    pub fn pause(&mut self, now: Timestamp) -> Result<(), ManualError> {
        if self.resumed_at.is_none() {
            return Err(ManualError::AlreadyPaused);
        }
        self.elapsed_ms = self.elapsed_at(now);
        self.resumed_at = None;
        Ok(())
    }

    /// Resume accumulation.
    ///
    /// # Errors
    /// Fails when the timer is not paused.
    pub fn resume(&mut self, now: Timestamp) -> Result<(), ManualError> {
        if self.resumed_at.is_some() {
            return Err(ManualError::AlreadyRunning);
        }
        self.resumed_at = Some(now);
        Ok(())
    }

    /// Finish the run, returning the total to record.
    #[must_use]
    pub fn finish(&self, now: Timestamp) -> i64 {
        self.elapsed_at(now)
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

    fn t(ms: i64) -> Timestamp {
        Timestamp::from_millis(1_700_000_000_000 + ms)
    }

    fn ids() -> (EntityId, EntityId) {
        (EntityId::new(), EntityId::new())
    }

    #[test]
    fn starting_a_timer_accumulates_from_now() {
        let (chapter, game) = ids();
        let run = ManualRun::start(chapter, game, t(0));
        assert!(run.is_running());
        assert_eq!(run.elapsed_at(t(5_000)), 5_000);
    }

    #[test]
    fn a_running_timer_reports_a_later_total_as_time_passes() {
        // The UI polls this on an interval. If the answer did not grow with the
        // clock, the displayed timer would sit at 0:00 while time accrued.
        let (chapter, game) = ids();
        let run = ManualRun::start(chapter, game, t(0));
        assert_eq!(run.elapsed_at(t(1_000)), 1_000);
        assert_eq!(run.elapsed_at(t(9_000)), 9_000);
    }

    #[test]
    fn pausing_freezes_the_total() {
        let (chapter, game) = ids();
        let mut run = ManualRun::start(chapter, game, t(0));
        run.pause(t(4_000)).expect("pause");
        assert!(!run.is_running());
        // Time passing while paused must not count.
        assert_eq!(run.elapsed_at(t(90_000)), 4_000);
    }

    #[test]
    fn resuming_continues_from_the_paused_total() {
        let (chapter, game) = ids();
        let mut run = ManualRun::start(chapter, game, t(0));
        run.pause(t(4_000)).expect("pause");
        run.resume(t(10_000)).expect("resume");
        assert_eq!(run.elapsed_at(t(12_000)), 6_000);
    }

    #[test]
    fn finishing_reports_the_accumulated_total() {
        let (chapter, game) = ids();
        let mut run = ManualRun::start(chapter, game, t(0));
        run.pause(t(3_000)).expect("pause");
        run.resume(t(8_000)).expect("resume");
        assert_eq!(run.finish(t(9_000)), 4_000);
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
    fn a_backwards_clock_never_subtracts_time() {
        // A system clock change must not reduce recorded playtime.
        let (chapter, game) = ids();
        let mut run = ManualRun::start(chapter, game, t(10_000));
        run.pause(t(5_000)).expect("pause");
        assert_eq!(run.elapsed_ms, 0);
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
        let (route, game) = ids();
        let mut run = ManualRun::start_route(route, game, t(0));
        run.pause(t(4_000)).expect("pause");
        run.resume(t(10_000)).expect("resume");
        assert_eq!(run.finish(t(12_000)), 6_000);
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
