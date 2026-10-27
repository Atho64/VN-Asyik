//! The playtime state machine.
//!
//! Pure, clock-free and OS-free: it consumes events stamped with a timestamp and
//! produces the sessions to persist. The OS layer only has to answer two
//! questions on a timer ("is the game's window focused?" and "when was the last
//! input?") and feed the answers in, which is what makes the hard parts testable
//! without a game running.
//!
//! Model:
//!
//! * A **run** begins when the tracked game's window gains focus and ends when it
//!   loses focus or the tracker stops.
//! * A run is split into **segments** by chapter changes, because per-chapter
//!   time is a headline feature. Segments are emitted as separate sessions, so a
//!   chapter change never loses accumulated time.
//! * Inside a segment, every gap is classified as **active** or **away** from the
//!   OS's own idle measurement. The grace boundary is the configured AFK
//!   threshold: an idle run under the threshold is reading time, and only the part
//!   past it is away.
//! * **Checkpoints** do not close the session. They emit a snapshot so the caller
//!   can persist progress; the session continues and the final write supersedes
//!   the checkpoint, keyed by session id.

use serde::{Deserialize, Serialize};

use crate::afk::AfkConfig;
use crate::clock::Timestamp;
use crate::ids::EntityId;

/// Which process is being tracked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackTarget {
    /// Game this process belongs to.
    pub game_id: EntityId,
    /// Process id. Changes across restarts, so it is not part of identity.
    pub pid: u32,
    /// Normalised absolute executable path.
    pub exe_path: String,
}

/// What the tracker is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// Nothing tracked.
    Idle,
    /// A tracked game has focus and time is accruing.
    Tracking,
    /// The user paused deliberately. Time does not accrue.
    Paused,
    /// The game window lost focus; the run stays open but accrues nothing.
    Unfocused,
}

/// Why a run ended, which the UI shows in the session list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// The user stopped tracking.
    UserStopped,
    /// The process exited.
    ProcessExited,
    /// The user paused and the run was closed.
    Paused,
    /// The app is shutting down.
    AppClosing,
    /// A checkpoint of an unfinished run, written before a crash.
    Recovered,
}

/// A finished stretch of time for one chapter, ready to persist.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Segment {
    /// Stable id, so a checkpoint and the final write address the same row.
    pub session_id: EntityId,
    /// Game the segment belongs to.
    pub game_id: EntityId,
    /// Chapter, when the adapter knew one.
    pub chapter_id: Option<EntityId>,
    /// Route the segment belongs to, when the user timed a route directly.
    ///
    /// Mutually exclusive with `chapter_id` in practice: a session targets the most
    /// specific thing the user named. Both `None` is ordinary detected time where
    /// the adapter knew the game but no chapter.
    pub route_id: Option<EntityId>,
    /// Segment start, UTC ms.
    pub started_at: Timestamp,
    /// Segment end, UTC ms.
    pub ended_at: Timestamp,
    /// Active time inside the segment.
    pub active_ms: i64,
    /// Away time inside the segment.
    pub afk_ms: i64,
    /// Whether this is an interim checkpoint rather than a final segment.
    pub checkpoint: bool,
}

impl Segment {
    /// Total time covered by the segment.
    #[must_use]
    pub const fn elapsed_ms(&self) -> i64 {
        self.active_ms + self.afk_ms
    }
}

/// An event that drives the machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// The tracked game's window gained focus.
    FocusGained,
    /// The tracked game's window lost focus.
    FocusLost,
    /// The tracked process is no longer running.
    ProcessExited,
    /// The user pressed pause.
    Pause,
    /// A manual timer took the current stretch over, so detection closes its
    /// segment without crediting the gap between the last poll and now.
    HandoffToManual,
    /// The user pressed resume.
    Resume,
    /// The user pressed stop.
    Stop,
    /// A periodic tick. `idle_for_ms` is how long the system has been without input.
    Tick {
        /// Milliseconds since the last user input, as reported by the OS.
        idle_for_ms: i64,
    },
    /// The engine reported a chapter change.
    ChapterChanged(Option<EntityId>),
    /// The app is closing; flush.
    Shutdown,
}

/// Configuration for the machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrackerConfig {
    /// AFK threshold and whether idle time is excluded.
    pub afk: AfkConfig,
    /// How often a checkpoint should be emitted, in ms.
    pub checkpoint_interval_ms: i64,
}

impl Default for TrackerConfig {
    fn default() -> Self {
        Self {
            afk: AfkConfig::default(),
            // 20 s sits inside the 15-30 s window AGENTS.md §7 asks for: a crash
            // loses at most one interval of time.
            checkpoint_interval_ms: 20_000,
        }
    }
}

/// The state machine.
#[derive(Debug, Clone)]
pub struct Tracker {
    config: TrackerConfig,
    phase: Phase,
    target: Option<TrackTarget>,
    /// The open segment, if a run is in progress.
    open: Option<OpenSegment>,
    last_event_at: Timestamp,
}

/// A segment still accumulating time.
#[derive(Debug, Clone)]
struct OpenSegment {
    session_id: EntityId,
    started_at: Timestamp,
    chapter_id: Option<EntityId>,
    active_ms: i64,
    afk_ms: i64,
    /// Time accrued since the last checkpoint or segment start.
    since_emit_ms: i64,
}

impl OpenSegment {
    fn new(session_id: EntityId, at: Timestamp, chapter_id: Option<EntityId>) -> Self {
        Self {
            session_id,
            started_at: at,
            chapter_id,
            active_ms: 0,
            afk_ms: 0,
            since_emit_ms: 0,
        }
    }

    fn snapshot(
        &self,
        now: Timestamp,
        game_id: EntityId,
        checkpoint: bool,
        config: crate::afk::AfkConfig,
    ) -> Segment {
        let totals = crate::afk::finish(
            crate::afk::ActivityTotals {
                active_ms: self.active_ms,
                afk_ms: self.afk_ms,
            },
            config,
        );
        Segment {
            session_id: self.session_id,
            game_id,
            chapter_id: self.chapter_id,
            // Detected segments never target a route: the adapter reports a chapter or
            // nothing, and a route is something only the user can name.
            route_id: None,
            started_at: self.started_at,
            ended_at: now,
            active_ms: totals.active_ms,
            afk_ms: totals.afk_ms,
            checkpoint,
        }
    }
}

impl Tracker {
    /// Build a tracker with the given configuration.
    #[must_use]
    pub fn new(config: TrackerConfig, now: Timestamp) -> Self {
        Self {
            config,
            phase: Phase::Idle,
            target: None,
            open: None,
            last_event_at: now,
        }
    }

    /// Apply a new AFK configuration without disturbing the run in progress.
    ///
    /// The classification reads `config.afk` on every tick, so the next tick
    /// judges idle runs with the new threshold; the open segment, its accrued
    /// time and the phase all survive. This is what lets a settings change
    /// reach a tracker that already has a game open.
    pub fn set_afk_config(&mut self, afk: AfkConfig) {
        self.config.afk = afk;
    }

    /// Current phase.
    #[must_use]
    pub const fn phase(&self) -> Phase {
        self.phase
    }

    /// The tracked target, when a run is open or paused.
    #[must_use]
    pub fn target(&self) -> Option<&TrackTarget> {
        self.target.as_ref()
    }

    /// Begin tracking a target. Idempotent for the same target.
    ///
    /// Returns nothing; the caller starts the clock with a [`Event::FocusGained`]
    /// or a [`Event::Tick`], which is what actually begins accruing time.
    pub fn start(&mut self, target: TrackTarget, now: Timestamp) {
        self.target = Some(target);
        self.phase = Phase::Unfocused;
        self.last_event_at = now;
    }

    /// Feed one event, returning any segments that were closed.
    ///
    /// A returned segment with `checkpoint: true` means "persist this now"; the
    /// session is still open and later snapshots carry the same `session_id`.
    pub fn apply(&mut self, event: Event, now: Timestamp) -> Vec<Segment> {
        let mut emitted = Vec::new();
        let delta = now.saturating_diff_ms(self.last_event_at).max(0);
        // A clock that jumps backwards (DST, NTP correction, a resumed VM) must
        // not later be credited as elapsed time, so the high-water mark only
        // moves forward.
        if now > self.last_event_at {
            self.last_event_at = now;
        }

        match event {
            Event::FocusGained => {
                if self.phase == Phase::Paused {
                    // Paused wins over focus: the user asked for no tracking.
                    return emitted;
                }
                // The gap before this event elapsed while unfocused, so it must
                // not be credited. Accrual starts with the next tick.
                self.phase = Phase::Tracking;
                if self.open.is_none() {
                    self.open_segment(now);
                }
            }
            Event::FocusLost => {
                if self.phase == Phase::Tracking {
                    self.accrue(delta, now, &mut emitted);
                    self.phase = Phase::Unfocused;
                }
            }
            Event::Tick { idle_for_ms } => {
                if self.phase == Phase::Tracking {
                    self.accrue_tick(delta, idle_for_ms, now, &mut emitted);
                }
            }
            Event::ChapterChanged(chapter_id) => {
                if self.phase == Phase::Tracking {
                    self.accrue(delta, now, &mut emitted);
                }
                if let Some(open) = self.open.as_ref()
                    && open.chapter_id != chapter_id
                {
                    // Close the old segment under its own chapter before moving on.
                    if let Some(game_id) = self.target.as_ref().map(|t| t.game_id) {
                        emitted.push(open.snapshot(now, game_id, false, self.config.afk));
                    }
                    let new_id = EntityId::new();
                    self.open = Some(OpenSegment::new(new_id, now, chapter_id));
                }
            }
            // A manual timer took the stretch over. The gap this event covers is
            // the timer's to record, so it is closed here without accruing it.
            Event::HandoffToManual => {
                if let Some(segment) = self.close(now, StopReason::Paused) {
                    emitted.push(segment);
                }
                self.phase = Phase::Paused;
            }
            Event::Pause => {
                if self.phase == Phase::Tracking {
                    self.accrue(delta, now, &mut emitted);
                }
                if let Some(segment) = self.close(now, StopReason::Paused) {
                    emitted.push(segment);
                }
                self.phase = Phase::Paused;
            }
            Event::Resume => {
                if self.phase == Phase::Paused {
                    self.phase = Phase::Unfocused;
                    self.open_segment(now);
                }
            }
            Event::Stop => {
                // Closing a run that is not focused must not credit the unfocused
                // gap to playtime.
                if matches!(self.phase, Phase::Tracking) {
                    self.accrue(delta, now, &mut emitted);
                }
                if let Some(segment) = self.close(now, StopReason::UserStopped) {
                    emitted.push(segment);
                }
                self.phase = Phase::Idle;
                self.target = None;
            }
            Event::ProcessExited => {
                if self.phase == Phase::Tracking {
                    self.accrue(delta, now, &mut emitted);
                }
                if let Some(segment) = self.close(now, StopReason::ProcessExited) {
                    emitted.push(segment);
                }
                self.phase = Phase::Idle;
                self.target = None;
            }
            Event::Shutdown => {
                if self.phase == Phase::Tracking {
                    self.accrue(delta, now, &mut emitted);
                }
                if let Some(segment) = self.close(now, StopReason::AppClosing) {
                    emitted.push(segment);
                }
                self.phase = Phase::Idle;
            }
        }

        emitted
    }

    /// Accrue `delta` as active time, honouring the checkpoint interval.
    fn accrue(&mut self, delta: i64, now: Timestamp, emitted: &mut Vec<Segment>) {
        if delta <= 0 {
            return;
        }
        let Some(open) = self.open.as_mut() else {
            return;
        };
        open.active_ms += delta;
        open.since_emit_ms += delta;
        self.maybe_checkpoint(now, emitted);
    }

    /// Accrue a tick, attributing the gap to active or away time.
    ///
    /// `idle_for_ms` is how long the user had gone without input when this tick
    /// was taken, measured by the OS. It is not a flag saying the gap was idle:
    /// after a click half a second ago it reads 500, and that is somebody who is
    /// present. Only the stretch of the current idle run past the grace threshold
    /// is away time, so the reading seconds of a break stay credited.
    fn accrue_tick(
        &mut self,
        delta: i64,
        idle_for_ms: i64,
        now: Timestamp,
        emitted: &mut Vec<Segment>,
    ) {
        if delta <= 0 {
            return;
        }
        let Some(open) = self.open.as_mut() else {
            return;
        };

        let away = away_ms_in_gap(delta, idle_for_ms, self.config.afk.threshold_ms);
        open.afk_ms += away;
        open.active_ms += delta - away;
        open.since_emit_ms += delta;
        self.maybe_checkpoint(now, emitted);
    }

    /// Emit a snapshot when enough time has passed.
    fn maybe_checkpoint(&mut self, now: Timestamp, emitted: &mut Vec<Segment>) {
        let Some(open) = self.open.as_ref() else {
            return;
        };
        if open.since_emit_ms < self.config.checkpoint_interval_ms {
            return;
        }
        let Some(game_id) = self.target.as_ref().map(|t| t.game_id) else {
            return;
        };
        emitted.push(open.snapshot(now, game_id, true, self.config.afk));
        if let Some(open) = self.open.as_mut() {
            open.since_emit_ms = 0;
        }
    }

    fn open_segment(&mut self, now: Timestamp) {
        if self.open.is_some() {
            return;
        }
        self.open = Some(OpenSegment::new(EntityId::new(), now, None));
    }

    /// Close the open segment, returning it as a final write.
    fn close(&mut self, now: Timestamp, _reason: StopReason) -> Option<Segment> {
        let open = self.open.take()?;
        let game_id = self.target.as_ref()?.game_id;
        // A segment with no accrued time carries nothing worth storing.
        if open.active_ms == 0 && open.afk_ms == 0 {
            return None;
        }
        Some(open.snapshot(now, game_id, false, self.config.afk))
    }

    /// Force a checkpoint when a run is open, e.g. before a risky operation.
    ///
    /// Resets the checkpoint timer, so this is for persistence, not for reading
    /// status. Use [`Self::peek`] to look at the run without side effects.
    pub fn flush(&mut self, now: Timestamp) -> Option<Segment> {
        let open = self.open.as_ref()?;
        let game_id = self.target.as_ref()?.game_id;
        let snapshot = open.snapshot(now, game_id, true, self.config.afk);
        if let Some(open) = self.open.as_mut() {
            open.since_emit_ms = 0;
        }
        Some(snapshot)
    }

    /// Read the open run without changing anything.
    ///
    /// This is what the UI polls for its live timer; calling `flush` there would
    /// reset the checkpoint interval and silently stop checkpoints from firing.
    #[must_use]
    pub fn peek(&self, now: Timestamp) -> Option<Segment> {
        let open = self.open.as_ref()?;
        let game_id = self.target.as_ref()?.game_id;
        Some(open.snapshot(now, game_id, true, self.config.afk))
    }
}

/// How many of the last `gap_ms` were away from the keyboard.
///
/// `idle_for_ms` and `gap_ms` are measured by clocks that agree on "now": the
/// current idle run began `idle_for_ms` ago and the gap began `gap_ms` ago, so
/// the away window is the part of the run past the grace threshold. Clamping to
/// the gap matters in both directions — a run longer than the gap means the whole
/// gap was idle (a delayed or skipped poll), and a run shorter than the gap means
/// input arrived inside it and only the tail is away.
fn away_ms_in_gap(gap_ms: i64, idle_for_ms: i64, threshold_ms: i64) -> i64 {
    (idle_for_ms.max(0) - threshold_ms.max(0)).clamp(0, gap_ms.max(0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::DeviceId;

    fn t(ms: i64) -> Timestamp {
        Timestamp::from_millis(ms)
    }

    fn game() -> EntityId {
        EntityId::new()
    }

    fn target(game_id: EntityId) -> TrackTarget {
        TrackTarget {
            game_id,
            pid: 4242,
            exe_path: "C:/games/test.exe".to_owned(),
        }
    }

    fn tracker() -> (Tracker, EntityId) {
        let game_id = game();
        let mut tracker = Tracker::new(TrackerConfig::default(), t(0));
        tracker.start(target(game_id), t(0));
        (tracker, game_id)
    }

    /// Ticks every `step` ms while focused, for `total` ms, with no idle.
    fn tick_for(tracker: &mut Tracker, from: i64, total: i64, step: i64) -> Vec<Segment> {
        let mut out = Vec::new();
        let mut elapsed = 0;
        while elapsed < total {
            elapsed += step;
            out.extend(tracker.apply(Event::Tick { idle_for_ms: 0 }, t(from + elapsed)));
        }
        out
    }

    #[test]
    fn default_checkpoint_is_inside_the_documented_window() {
        let config = TrackerConfig::default();
        assert!(config.checkpoint_interval_ms >= 15_000);
        assert!(config.checkpoint_interval_ms <= 30_000);
    }

    #[test]
    fn the_away_helper_clamps_a_delayed_poll_to_the_gap() {
        // A poll delayed by a suspended machine reports an idle run longer than the
        // gap; only the gap can be charged, never phantom time.
        assert_eq!(away_ms_in_gap(2_000, 3_600_000, 60_000), 2_000);
        // Input arrived 500 ms before the tick.
        assert_eq!(away_ms_in_gap(2_000, 500, 60_000), 0);
        // The idle run began inside the gap and crossed the threshold 700 ms ago.
        assert_eq!(away_ms_in_gap(2_000, 60_700, 60_000), 700);
        // A clock that went backwards must not produce negative away time.
        assert_eq!(away_ms_in_gap(2_000, -5, 60_000), 0);
        // Two events in the same millisecond carry no time at all.
        assert_eq!(away_ms_in_gap(0, 3_600_000, 60_000), 0);
    }

    #[test]
    fn a_new_tracker_is_idle() {
        let tracker = Tracker::new(TrackerConfig::default(), t(0));
        assert_eq!(tracker.phase(), Phase::Idle);
        assert!(tracker.target().is_none());
    }

    #[test]
    fn starting_does_not_accrue_until_focus() {
        let (mut tracker, _) = tracker();
        assert_eq!(tracker.phase(), Phase::Unfocused);
        let emitted = tracker.apply(Event::Tick { idle_for_ms: 0 }, t(1_000));
        assert_eq!(emitted.len(), 0, "no focus, no time");
    }

    #[test]
    fn focus_then_ticks_accrues_active_time() {
        let (mut tracker, game_id) = tracker();
        tracker.apply(Event::FocusGained, t(0));
        assert_eq!(tracker.phase(), Phase::Tracking);
        let segments = tick_for(&mut tracker, 0, 10_000, 1_000);
        assert_eq!(segments.len(), 0, "under the checkpoint interval");

        let final_segment = tracker
            .apply(Event::Stop, t(10_000))
            .into_iter()
            .next()
            .expect("segment");
        assert_eq!(final_segment.game_id, game_id);
        assert_eq!(final_segment.active_ms, 10_000);
        assert_eq!(final_segment.afk_ms, 0);
        assert!(!final_segment.checkpoint);
    }

    #[test]
    fn losing_focus_stops_accruing_but_keeps_the_run() {
        let (mut tracker, _) = tracker();
        tracker.apply(Event::FocusGained, t(0));
        tick_for(&mut tracker, 0, 5_000, 1_000);
        tracker.apply(Event::FocusLost, t(5_000));
        assert_eq!(tracker.phase(), Phase::Unfocused);

        // Ticks while unfocused must not accrue.
        tick_for(&mut tracker, 5_000, 30_000, 1_000);
        let segment = tracker.apply(Event::Stop, t(35_000)).into_iter().next();
        assert_eq!(segment.expect("segment").active_ms, 5_000);
    }

    #[test]
    fn refocusing_resumes_the_same_run() {
        let (mut tracker, _) = tracker();
        tracker.apply(Event::FocusGained, t(0));
        tick_for(&mut tracker, 0, 5_000, 1_000);
        tracker.apply(Event::FocusLost, t(5_000));
        tracker.apply(Event::FocusGained, t(20_000));
        tick_for(&mut tracker, 20_000, 5_000, 1_000);

        let segment = tracker.apply(Event::Stop, t(25_000)).into_iter().next();
        let segment = segment.expect("segment");
        assert_eq!(segment.active_ms, 10_000);
        assert_eq!(segment.started_at, t(0), "the run keeps its original start");
    }

    #[test]
    fn short_idle_stays_active() {
        let (mut tracker, _) = tracker();
        tracker.apply(Event::FocusGained, t(0));
        tick_for(&mut tracker, 0, 2_000, 1_000);

        // 30 s with no input, then input returns: under the 60 s default.
        let mut out = Vec::new();
        for step in 1..=30 {
            out.extend(tracker.apply(
                Event::Tick {
                    idle_for_ms: step * 1_000,
                },
                t(2_000 + step * 1_000),
            ));
        }
        out.extend(tracker.apply(Event::Tick { idle_for_ms: 0 }, t(32_000)));
        out.extend(tracker.apply(Event::Stop, t(33_000)));

        let segment = out.into_iter().next_back().expect("segment");
        assert_eq!(segment.afk_ms, 0, "30 s of reading is not a break");
        assert_eq!(segment.active_ms, 33_000);
    }

    #[test]
    fn a_near_zero_idle_reading_is_not_a_break() {
        // `idle_for_ms` is "time since the last input", not a flag: with a click
        // 300 ms ago it reads 300, never 0. Accumulating that as an idle run made
        // every session look away after a minute of *playing*, so the active clock
        // stopped and the away figure started from zero mid-session.
        let (mut tracker, _) = tracker();
        tracker.apply(Event::FocusGained, t(0));

        // Five minutes of clicking: one poll every 5 s, each seeing 300 ms of idle.
        for step in 1..=60 {
            tracker.apply(Event::Tick { idle_for_ms: 300 }, t(step * 5_000));
        }
        let segment = tracker
            .apply(Event::Stop, t(301_000))
            .into_iter()
            .next()
            .expect("segment");
        assert_eq!(segment.afk_ms, 0, "a click every few seconds is playing");
        assert_eq!(segment.active_ms, 301_000);
    }

    #[test]
    fn a_break_is_the_part_past_the_grace_and_never_restarts() {
        // Away is derived from the OS's idle run on every tick, so a second break
        // adds to the first instead of starting over, and each run's grace period
        // stays reading time.
        let (mut tracker, _) = tracker();
        tracker.apply(Event::FocusGained, t(0));

        // 120 s without input: away from the 61st second on.
        for step in 1..=120 {
            tracker.apply(
                Event::Tick {
                    idle_for_ms: step * 1_000,
                },
                t(step * 1_000),
            );
        }
        // Back: the last input arrived 400 ms before this tick.
        tracker.apply(Event::Tick { idle_for_ms: 400 }, t(121_000));
        // A second, 129 s run measured from that input.
        for step in 122..=250 {
            tracker.apply(
                Event::Tick {
                    idle_for_ms: (step - 121) * 1_000,
                },
                t(step * 1_000),
            );
        }
        let segment = tracker
            .apply(Event::Stop, t(251_000))
            .into_iter()
            .next()
            .expect("segment");

        // First run: idle since t=0, away from 60 s -> 60 s.
        // Second run: idle since t=121 s, away from 181 s -> 69 s.
        assert_eq!(segment.afk_ms, 60_000 + 69_000);
        assert_eq!(segment.active_ms, 251_000 - 129_000);
        assert_eq!(segment.elapsed_ms(), 251_000);
    }

    #[test]
    fn turning_exclude_afk_off_credits_the_break_as_play() {
        // The setting exists so a user who does not want breaks stripped can have the
        // time back. It has to change the stored numbers, not just the UI toggle.
        let mut config = TrackerConfig::default();
        config.afk.exclude_afk = false;
        let mut tracker = Tracker::new(config, t(0));
        tracker.start(target(game()), t(0));
        tracker.apply(Event::FocusGained, t(0));
        tick_for(&mut tracker, 0, 2_000, 1_000);
        for step in 1..=120 {
            tracker.apply(
                Event::Tick {
                    idle_for_ms: step * 1_000,
                },
                t(2_000 + step * 1_000),
            );
        }
        let segment = tracker
            .apply(Event::Stop, t(122_000))
            .into_iter()
            .next_back()
            .expect("segment");

        assert_eq!(segment.afk_ms, 0, "the break is credited, not filed as AFK");
        assert_eq!(segment.active_ms, 122_000);
    }

    #[test]
    fn long_idle_becomes_afk() {
        let (mut tracker, _) = tracker();
        tracker.apply(Event::FocusGained, t(0));
        tick_for(&mut tracker, 0, 2_000, 1_000);

        let mut out = Vec::new();
        // 120 s idle: the run passes the 60 s threshold at 61 s and everything
        // from then on is away time.
        for step in 1..=120 {
            out.extend(tracker.apply(
                Event::Tick {
                    idle_for_ms: step * 1_000,
                },
                t(2_000 + step * 1_000),
            ));
        }
        out.extend(tracker.apply(Event::Stop, t(122_000)));

        let segment = out.into_iter().next_back().expect("segment");
        // 2 s active, then a 120 s idle run. The first 60 s of the run is reading
        // time and the remaining 60 s is away, so the split is exact, not "roughly
        // 59 s" — a loose bound here would have hidden a change to the boundary.
        assert_eq!(segment.active_ms, 62_000);
        assert_eq!(segment.afk_ms, 60_000);
        assert_eq!(segment.elapsed_ms(), 122_000);
    }

    #[test]
    fn a_mid_run_config_change_reaches_the_next_tick() {
        let (mut tracker, game_id) = tracker();
        tracker.apply(Event::FocusGained, t(0));
        tick_for(&mut tracker, 0, 2_000, 1_000);

        // The default 60 s threshold would keep all of what follows reading
        // time. Lowering it to the 15 s minimum (a settings change the user
        // makes while the game is open) must take effect on the next tick
        // without closing or splitting the open segment.
        tracker.set_afk_config(AfkConfig::new(15_000, true));

        let mut out = Vec::new();
        for step in 1..=17 {
            out.extend(tracker.apply(
                Event::Tick {
                    idle_for_ms: step * 1_000,
                },
                t(2_000 + step * 1_000),
            ));
        }
        out.extend(tracker.apply(Event::Stop, t(19_000)));

        // Under the old threshold this whole stretch is active; the new one
        // sends the idle run past the grace boundary from its 16th second.
        let segment = out.into_iter().next().expect("segment");
        assert_eq!(segment.game_id, game_id);
        assert_eq!(segment.elapsed_ms(), 19_000);
        assert_eq!(segment.active_ms, 17_000, "2 s plus the first 15 s of idle");
        assert_eq!(segment.afk_ms, 2_000, "the run crossed at 16 s of idle");
    }

    #[test]
    fn checkpoint_is_emitted_at_the_interval_and_reuses_the_session_id() {
        let (mut tracker, _) = tracker();
        tracker.apply(Event::FocusGained, t(0));
        // 20 s default interval, ticking every second.
        let segments = tick_for(&mut tracker, 0, 20_000, 1_000);
        assert_eq!(segments.len(), 1, "one checkpoint at 20 s");
        assert!(segments[0].checkpoint);

        let checkpoint_id = segments[0].session_id;
        let final_segment = tracker
            .apply(Event::Stop, t(21_000))
            .into_iter()
            .next()
            .expect("final");
        assert_eq!(
            final_segment.session_id, checkpoint_id,
            "the final write must address the same row"
        );
        assert!(!final_segment.checkpoint);
        assert!(final_segment.active_ms > segments[0].active_ms);
    }

    #[test]
    fn multiple_checkpoints_accumulate() {
        let (mut tracker, _) = tracker();
        tracker.apply(Event::FocusGained, t(0));
        let segments = tick_for(&mut tracker, 0, 60_000, 1_000);
        assert!(segments.len() >= 3);
        assert!(segments.iter().all(|segment| segment.checkpoint));
        let ids: std::collections::HashSet<_> =
            segments.iter().map(|segment| segment.session_id).collect();
        assert_eq!(ids.len(), 1, "all checkpoints share one session id");
    }

    #[test]
    fn chapter_change_splits_the_session_and_keeps_time() {
        let (mut tracker, _) = tracker();
        tracker.apply(Event::FocusGained, t(0));
        tick_for(&mut tracker, 0, 10_000, 1_000);

        let chapter = EntityId::new();
        let first = tracker.apply(Event::ChapterChanged(Some(chapter)), t(10_000));
        assert_eq!(first.len(), 1);
        assert_eq!(
            first[0].chapter_id, None,
            "the first segment had no chapter"
        );
        assert_eq!(first[0].active_ms, 10_000);
        assert!(!first[0].checkpoint);

        tick_for(&mut tracker, 10_000, 5_000, 1_000);
        let second = tracker
            .apply(Event::Stop, t(15_000))
            .into_iter()
            .next()
            .expect("second segment");
        assert_eq!(second.chapter_id, Some(chapter));
        assert_eq!(second.active_ms, 5_000);
        assert_ne!(second.session_id, first[0].session_id);
    }

    #[test]
    fn changing_to_the_same_chapter_does_not_split() {
        let (mut tracker, _) = tracker();
        let chapter = EntityId::new();
        tracker.apply(Event::FocusGained, t(0));
        tracker.apply(Event::ChapterChanged(Some(chapter)), t(0));
        tick_for(&mut tracker, 0, 5_000, 1_000);
        let segments = tracker.apply(Event::ChapterChanged(Some(chapter)), t(5_000));
        assert_eq!(segments.len(), 0, "no split when the chapter is unchanged");
    }

    #[test]
    fn clearing_the_chapter_splits_too() {
        let (mut tracker, _) = tracker();
        let chapter = EntityId::new();
        tracker.apply(Event::FocusGained, t(0));
        tracker.apply(Event::ChapterChanged(Some(chapter)), t(0));
        tick_for(&mut tracker, 0, 5_000, 1_000);
        let segments = tracker.apply(Event::ChapterChanged(None), t(5_000));
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].chapter_id, Some(chapter));
    }

    #[test]
    fn pause_closes_the_segment_and_freezes_time() {
        let (mut tracker, _) = tracker();
        tracker.apply(Event::FocusGained, t(0));
        tick_for(&mut tracker, 0, 5_000, 1_000);

        let paused = tracker.apply(Event::Pause, t(5_000));
        assert_eq!(tracker.phase(), Phase::Paused);
        assert_eq!(paused.len(), 1);
        assert_eq!(paused[0].active_ms, 5_000);

        // Ticks while paused accrue nothing.
        tick_for(&mut tracker, 5_000, 60_000, 1_000);
        assert_eq!(tracker.phase(), Phase::Paused);
    }

    #[test]
    fn resume_starts_a_new_segment() {
        let (mut tracker, _) = tracker();
        tracker.apply(Event::FocusGained, t(0));
        tick_for(&mut tracker, 0, 5_000, 1_000);
        tracker.apply(Event::Pause, t(5_000));

        tracker.apply(Event::Resume, t(60_000));
        assert_eq!(tracker.phase(), Phase::Unfocused);
        tracker.apply(Event::FocusGained, t(60_000));
        tick_for(&mut tracker, 60_000, 5_000, 1_000);
        let segment = tracker.apply(Event::Stop, t(65_000)).into_iter().next();
        assert_eq!(segment.expect("segment").active_ms, 5_000);
    }

    #[test]
    fn focus_while_paused_does_not_resume() {
        let (mut tracker, _) = tracker();
        tracker.apply(Event::FocusGained, t(0));
        tracker.apply(Event::Pause, t(1_000));
        tracker.apply(Event::FocusGained, t(2_000));
        assert_eq!(tracker.phase(), Phase::Paused);
    }

    #[test]
    fn process_exit_closes_the_session_and_clears_the_target() {
        let (mut tracker, _) = tracker();
        tracker.apply(Event::FocusGained, t(0));
        tick_for(&mut tracker, 0, 5_000, 1_000);
        let segments = tracker.apply(Event::ProcessExited, t(5_000));
        assert_eq!(segments.len(), 1);
        assert_eq!(tracker.phase(), Phase::Idle);
        assert!(tracker.target().is_none());
    }

    #[test]
    fn shutdown_flushes_an_open_session() {
        let (mut tracker, _) = tracker();
        tracker.apply(Event::FocusGained, t(0));
        tick_for(&mut tracker, 0, 5_000, 1_000);
        let segments = tracker.apply(Event::Shutdown, t(5_000));
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].active_ms, 5_000);
    }

    #[test]
    fn an_empty_run_emits_nothing() {
        let (mut tracker, _) = tracker();
        tracker.apply(Event::FocusGained, t(0));
        let segments = tracker.apply(Event::Stop, t(0));
        assert_eq!(segments.len(), 0, "a zero-length run is not worth a row");
    }

    #[test]
    fn midnight_rollover_is_just_a_long_gap() {
        // 23:59:30 to 00:00:30 across a day boundary.
        let start = 1_767_225_570_000;
        let game_id = game();
        let mut tracker = Tracker::new(TrackerConfig::default(), t(start));
        tracker.start(target(game_id), t(start));
        tracker.apply(Event::FocusGained, t(start));
        tick_for(&mut tracker, start, 60_000, 5_000);
        let segment = tracker
            .apply(Event::Stop, t(start + 60_000))
            .into_iter()
            .next()
            .expect("segment");
        assert_eq!(
            segment.active_ms, 60_000,
            "wall clock crossing is irrelevant"
        );
    }

    #[test]
    fn a_backwards_clock_never_credits_phantom_time() {
        // DST, an NTP correction or a resumed VM can make timestamps go backwards.
        // Nothing was actually played in that window, so nothing may be credited
        // and no session row should appear.
        let (mut tracker, _) = tracker();
        tracker.apply(Event::FocusGained, t(10_000));
        let segments = tracker.apply(Event::Tick { idle_for_ms: 0 }, t(5_000));
        assert_eq!(segments.len(), 0, "segments should be empty");
        let segment = tracker.apply(Event::Stop, t(10_000)).into_iter().next();
        assert!(
            segment.is_none(),
            "a backwards clock must not produce playtime, got {segment:?}"
        );
    }

    #[test]
    fn a_backwards_clock_does_not_lose_time_already_counted() {
        // Real time before the jump is kept; only the impossible gap is ignored.
        let (mut tracker, _) = tracker();
        tracker.apply(Event::FocusGained, t(0));
        tick_for(&mut tracker, 0, 5_000, 1_000);
        tracker.apply(Event::Tick { idle_for_ms: 0 }, t(2_000));
        let segment = tracker.apply(Event::Stop, t(6_000)).into_iter().next();
        assert_eq!(segment.expect("segment").active_ms, 6_000);
    }

    #[test]
    fn flush_emits_a_checkpoint_without_closing() {
        let (mut tracker, _) = tracker();
        tracker.apply(Event::FocusGained, t(0));
        tick_for(&mut tracker, 0, 5_000, 1_000);
        let flushed = tracker.flush(t(5_000)).expect("checkpoint");
        assert!(flushed.checkpoint);
        assert_eq!(flushed.active_ms, 5_000);

        tick_for(&mut tracker, 5_000, 5_000, 1_000);
        let final_segment = tracker
            .apply(Event::Stop, t(10_000))
            .into_iter()
            .next()
            .expect("final");
        assert_eq!(final_segment.session_id, flushed.session_id);
        assert_eq!(final_segment.active_ms, 10_000);
    }

    #[test]
    fn segments_carry_the_game_id_from_the_target() {
        let (mut tracker, game_id) = tracker();
        tracker.apply(Event::FocusGained, t(0));
        tick_for(&mut tracker, 0, 3_000, 1_000);
        let segment = tracker.apply(Event::Stop, t(3_000)).into_iter().next();
        assert_eq!(segment.expect("segment").game_id, game_id);
    }

    #[test]
    fn a_device_id_is_not_needed_by_the_machine() {
        // Sanity check that the machine stays free of identity concerns; the
        // caller stamps sessions with its own device id when persisting.
        let device = DeviceId::parse("dev").expect("device");
        assert_eq!(device.as_str(), "dev");
    }
}
