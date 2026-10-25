//! Turning Android's usage event log into playtime intervals.
//!
//! # What Android gives us, and what it does not
//!
//! Android cannot tell us that a *game* is in the foreground. It reports activity
//! lifecycle events per package: `ACTIVITY_RESUMED` (1) and `ACTIVITY_PAUSED` (2),
//! plus screen state. From those we can only say "this package was on screen from
//! t1 to t2". A visual novel on Android is one package, so that is the game's
//! playtime, with no chapter information at all.
//!
//! Everything here is pure: it takes a slice of events and produces intervals, so
//! the awkward parts are unit-tested without a device. The rules come from the
//! API documentation rather than from guesswork:
//!
//! - `ACTIVITY_RESUMED` is 1 and `ACTIVITY_PAUSED` is 2. The older
//!   `MOVE_TO_FOREGROUND`/`MOVE_TO_BACKGROUND` constants were deprecated in API 29
//!   *in favour of* these and kept the same values, so reading the numeric values
//!   works on both old and new devices.
//! - Events are only retained for a few days, so a gap longer than the retention
//!   window is not evidence of anything.
//! - `DEVICE_SHUTDOWN` and `DEVICE_STARTUP` exist precisely so that an interval
//!   left open across them can be discarded: the documentation says any open
//!   events between them "should be ignored because the closing time is unknown".
//!   Crediting that gap would invent hours of playtime for a phone that was off.
//! - The screen going non-interactive ends the interval. A game left running with
//!   the screen off is not being played.

/// An event type as it appears in `UsageEvents.Event.getEventType()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsageEventKind {
    /// The package's activity came to the foreground (value 1).
    Resumed,
    /// The package's activity left the foreground (value 2).
    Paused,
    /// The activity became invisible (value 23). Not required for our purposes,
    /// but it can close an interval that never saw a pause.
    Stopped,
    /// Screen became interactive (value 15).
    ScreenInteractive,
    /// Screen stopped being interactive (value 16).
    ScreenNonInteractive,
    /// Either shutdown or startup; both mean "any open interval is void".
    DeviceBoundary,
    /// Anything else, which is ignored.
    Other,
}

impl UsageEventKind {
    /// Classify the raw integer from the platform.
    ///
    /// Deliberately matches numeric values rather than the constants' names,
    /// because the deprecated and current names share values and the numbers are
    /// what cross the JNI boundary.
    pub const fn from_code(code: i32) -> Self {
        match code {
            // ACTIVITY_RESUMED / MOVE_TO_FOREGROUND
            1 => UsageEventKind::Resumed,
            // ACTIVITY_PAUSED / MOVE_TO_BACKGROUND
            2 => UsageEventKind::Paused,
            // SCREEN_INTERACTIVE / KEYGUARD_HIDDEN
            15 | 18 => UsageEventKind::ScreenInteractive,
            // SCREEN_NON_INTERACTIVE / KEYGUARD_SHOWN
            16 | 17 => UsageEventKind::ScreenNonInteractive,
            // ACTIVITY_STOPPED
            23 => UsageEventKind::Stopped,
            // DEVICE_SHUTDOWN (26), DEVICE_STARTUP (27)
            26 | 27 => UsageEventKind::DeviceBoundary,
            _ => UsageEventKind::Other,
        }
    }
}

/// One event, reduced to what the interval builder needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageEvent {
    /// Package that produced the event. Screen and device events have none.
    pub package: Option<String>,
    /// Activity class, when available, to distinguish transitions within one app.
    pub activity: Option<String>,
    /// What happened.
    pub kind: UsageEventKind,
    /// When, in Unix epoch milliseconds.
    pub at_ms: i64,
}

/// A span during which one package was on screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageInterval {
    pub package: String,
    pub start_ms: i64,
    pub end_ms: i64,
}

impl UsageInterval {
    pub fn duration_ms(&self) -> i64 {
        (self.end_ms - self.start_ms).max(0)
    }
}

/// Build intervals for events belonging to `packages` only.
///
/// `end_cap_ms` is the exclusive end of the queried window. An interval still
/// open at that point is closed there: the alternative is to drop it, which
/// would lose the game the user is playing *right now*.
///
/// Returns intervals ordered by start time. Zero-length intervals are dropped;
/// they carry no playtime and would only add noise to the session table.
pub fn intervals_for(
    packages: &[String],
    events: &[UsageEvent],
    end_cap_ms: i64,
) -> Vec<UsageInterval> {
    let mut open: Option<(String, Option<String>, i64)> = None;
    let mut out: Vec<UsageInterval> = Vec::new();
    let mut screen_on = true;

    let close = |open: &mut Option<(String, Option<String>, i64)>,
                 at_ms: i64,
                 out: &mut Vec<UsageInterval>| {
        if let Some((package, _, start)) = open.take()
            && at_ms > start
        {
            out.push(UsageInterval {
                package,
                start_ms: start,
                end_ms: at_ms,
            });
        }
    };

    for event in events {
        if event.at_ms >= end_cap_ms {
            break;
        }
        match event.kind {
            UsageEventKind::Resumed => {
                let Some(package) = event.package.as_deref() else {
                    continue;
                };
                // A resume for any other package closes the open one, including
                // untracked apps: a missing outgoing pause must not count a browser.
                // Android reports the new activity resuming before, or in the same
                // millisecond as, the old one pausing, and holding both open would
                // double-count.
                if open
                    .as_ref()
                    .is_some_and(|(current, _, _)| current != package)
                {
                    close(&mut open, event.at_ms, &mut out);
                }
                if !screen_on || !packages.iter().any(|wanted| wanted == package) {
                    continue;
                }
                if open.is_none() {
                    open = Some((package.to_owned(), event.activity.clone(), event.at_ms));
                } else if let Some((_, activity, _)) = &mut open {
                    // Android reports each activity separately. If B resumes
                    // before A pauses, A's later pause must not stop B's time.
                    activity.clone_from(&event.activity);
                }
            }
            UsageEventKind::Paused | UsageEventKind::Stopped => {
                if let Some((current, activity, _)) = &open {
                    let matches = event
                        .package
                        .as_deref()
                        .is_some_and(|package| package == current);
                    let same_activity = activity.is_none()
                        || event.activity.is_none()
                        || activity == &event.activity;
                    if matches && same_activity {
                        close(&mut open, event.at_ms, &mut out);
                    }
                }
            }
            UsageEventKind::ScreenNonInteractive => {
                screen_on = false;
                // Turning the screen off ends play even without a pause event.
                close(&mut open, event.at_ms, &mut out);
            }
            UsageEventKind::ScreenInteractive => {
                screen_on = true;
            }
            UsageEventKind::DeviceBoundary => {
                // Documented as making any open interval unclosable: the actual
                // end is unknown, so it is discarded rather than guessed.
                open = None;
            }
            UsageEventKind::Other => {}
        }
    }

    // An interval still open at the end of the window is real playtime up to
    // `end_cap_ms`. When the screen is off the interval was already closed at the
    // screen-off event, so there is nothing left to close.
    if screen_on {
        close(&mut open, end_cap_ms, &mut out);
    }

    out
}

/// Total milliseconds per package across `intervals`.
pub fn totals_by_package(intervals: &[UsageInterval]) -> Vec<(String, i64)> {
    let mut totals: Vec<(String, i64)> = Vec::new();
    for interval in intervals {
        match totals
            .iter_mut()
            .find(|(package, _)| package == &interval.package)
        {
            Some((_, total)) => *total += interval.duration_ms(),
            None => totals.push((interval.package.clone(), interval.duration_ms())),
        }
    }
    totals
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(kind: UsageEventKind, at_ms: i64, package: Option<&str>) -> UsageEvent {
        UsageEvent {
            package: package.map(str::to_owned),
            activity: None,
            kind,
            at_ms,
        }
    }

    fn pkg() -> Vec<String> {
        vec!["com.example.vn".to_owned()]
    }

    #[test]
    fn the_numeric_codes_match_the_platform_constants() {
        // ACTIVITY_RESUMED / ACTIVITY_PAUSED / ACTIVITY_STOPPED /
        // SCREEN_INTERACTIVE / SCREEN_NON_INTERACTIVE / DEVICE_SHUTDOWN /
        // DEVICE_STARTUP, per the UsageEvents.Event reference.
        assert_eq!(UsageEventKind::from_code(1), UsageEventKind::Resumed);
        assert_eq!(UsageEventKind::from_code(2), UsageEventKind::Paused);
        assert_eq!(UsageEventKind::from_code(23), UsageEventKind::Stopped);
        assert_eq!(
            UsageEventKind::from_code(15),
            UsageEventKind::ScreenInteractive
        );
        assert_eq!(
            UsageEventKind::from_code(16),
            UsageEventKind::ScreenNonInteractive
        );
        assert_eq!(
            UsageEventKind::from_code(26),
            UsageEventKind::DeviceBoundary
        );
        assert_eq!(
            UsageEventKind::from_code(27),
            UsageEventKind::DeviceBoundary
        );
        assert_eq!(UsageEventKind::from_code(7), UsageEventKind::Other);
    }

    #[test]
    fn a_resume_then_pause_is_one_interval() {
        let events = vec![
            ev(UsageEventKind::Resumed, 1_000, Some("com.example.vn")),
            ev(UsageEventKind::Paused, 61_000, Some("com.example.vn")),
        ];
        let intervals = intervals_for(&pkg(), &events, 100_000);
        assert_eq!(intervals.len(), 1);
        assert_eq!(intervals[0].duration_ms(), 60_000);
    }

    #[test]
    fn keyguard_locks_close_time_until_a_new_activity_resume() {
        let event = |code, at| ev(UsageEventKind::from_code(code), at, Some("com.example.vn"));
        let events = vec![
            event(1, 1_000),
            event(17, 11_000),
            event(18, 21_000),
            event(1, 22_000),
            event(2, 32_000),
        ];
        let intervals = intervals_for(&pkg(), &events, 40_000);
        assert_eq!(
            intervals
                .iter()
                .map(UsageInterval::duration_ms)
                .sum::<i64>(),
            20_000
        );
    }

    #[test]
    fn other_packages_are_ignored() {
        // The phone is full of apps. Counting them would make the library's
        // playtime meaningless.
        let events = vec![
            ev(UsageEventKind::Resumed, 1_000, Some("com.android.chrome")),
            ev(UsageEventKind::Paused, 50_000, Some("com.android.chrome")),
        ];
        assert_eq!(
            intervals_for(&pkg(), &events, 100_000).len(),
            0,
            "intervals_for(&pkg(), &events, 100_000) should be empty"
        );
    }

    #[test]
    fn an_interval_open_at_the_window_end_is_closed_there() {
        // This is the game the user is playing right now. Dropping it would make
        // the tracker appear to do nothing until they stop playing.
        let events = vec![ev(UsageEventKind::Resumed, 1_000, Some("com.example.vn"))];
        let intervals = intervals_for(&pkg(), &events, 31_000);
        assert_eq!(intervals.len(), 1);
        assert_eq!(intervals[0].duration_ms(), 30_000);
    }

    #[test]
    fn the_screen_going_off_ends_the_interval() {
        // A visual novel left open with the screen off is not being played.
        let events = vec![
            ev(UsageEventKind::Resumed, 1_000, Some("com.example.vn")),
            ev(UsageEventKind::ScreenNonInteractive, 21_000, None),
            ev(UsageEventKind::ScreenInteractive, 60_000, None),
        ];
        let intervals = intervals_for(&pkg(), &events, 90_000);
        assert_eq!(intervals.len(), 1);
        assert_eq!(intervals[0].end_ms, 21_000);
        assert_eq!(intervals[0].duration_ms(), 20_000);
    }

    #[test]
    fn a_device_restart_voids_an_open_interval() {
        // The docs are explicit: an open event with no matching close across a
        // shutdown has an unknown end, so it must be ignored. Crediting it would
        // invent however long the phone was off.
        let events = vec![
            ev(UsageEventKind::Resumed, 1_000, Some("com.example.vn")),
            ev(UsageEventKind::DeviceBoundary, 5_000, None),
            ev(UsageEventKind::DeviceBoundary, 3_600_000, None),
        ];
        let intervals = intervals_for(&pkg(), &events, 4_000_000);
        assert_eq!(
            intervals.len(),
            0,
            "an interval across a restart must be dropped, got {intervals:?}",
        );
    }

    #[test]
    fn switching_between_tracked_packages_does_not_double_count() {
        // Android reports the incoming resume before the outgoing pause. Holding
        // both open would credit the same wall time to two games.
        let two = vec!["com.example.a".to_owned(), "com.example.b".to_owned()];
        let events = vec![
            ev(UsageEventKind::Resumed, 1_000, Some("com.example.a")),
            ev(UsageEventKind::Resumed, 11_000, Some("com.example.b")),
            ev(UsageEventKind::Paused, 11_000, Some("com.example.a")),
            ev(UsageEventKind::Paused, 21_000, Some("com.example.b")),
        ];
        let intervals = intervals_for(&two, &events, 100_000);
        let total: i64 = intervals.iter().map(UsageInterval::duration_ms).sum();
        assert_eq!(total, 20_000, "10s per package, not 10s counted twice");
    }

    #[test]
    fn a_pause_for_an_untracked_package_does_not_close_ours() {
        let events = vec![
            ev(UsageEventKind::Resumed, 1_000, Some("com.example.vn")),
            ev(UsageEventKind::Paused, 5_000, Some("com.android.chrome")),
            ev(UsageEventKind::Paused, 31_000, Some("com.example.vn")),
        ];
        let intervals = intervals_for(&pkg(), &events, 100_000);
        assert_eq!(intervals.len(), 1);
        assert_eq!(intervals[0].duration_ms(), 30_000);
    }

    #[test]
    fn zero_length_intervals_are_dropped() {
        let events = vec![
            ev(UsageEventKind::Resumed, 1_000, Some("com.example.vn")),
            ev(UsageEventKind::Paused, 1_000, Some("com.example.vn")),
        ];
        assert_eq!(
            intervals_for(&pkg(), &events, 100_000).len(),
            0,
            "intervals_for(&pkg(), &events, 100_000) should be empty"
        );
    }

    #[test]
    fn totals_add_up_per_package() {
        let intervals = vec![
            UsageInterval {
                package: "a".to_owned(),
                start_ms: 0,
                end_ms: 10_000,
            },
            UsageInterval {
                package: "b".to_owned(),
                start_ms: 10_000,
                end_ms: 15_000,
            },
            UsageInterval {
                package: "a".to_owned(),
                start_ms: 20_000,
                end_ms: 25_000,
            },
        ];
        let totals = totals_by_package(&intervals);
        assert_eq!(
            totals,
            vec![("a".to_owned(), 15_000), ("b".to_owned(), 5_000)]
        );
    }

    #[test]
    fn an_empty_log_produces_nothing() {
        assert_eq!(
            intervals_for(&pkg(), &[], 100_000).len(),
            0,
            "intervals_for(&pkg(), &[], 100_000) should be empty"
        );
    }

    #[test]
    fn switching_to_an_untracked_app_closes_a_missing_pause() {
        let events = vec![
            ev(UsageEventKind::Resumed, 1_000, Some("com.example.vn")),
            ev(UsageEventKind::Resumed, 9_000, Some("com.android.chrome")),
        ];
        let intervals = intervals_for(&pkg(), &events, 60_000);
        assert_eq!(intervals[0].duration_ms(), 8_000);
    }

    #[test]
    fn an_old_activity_pause_does_not_stop_the_new_activity() {
        let event = |kind, at_ms, activity: &str| UsageEvent {
            activity: Some(activity.to_owned()),
            ..ev(kind, at_ms, Some("com.example.vn"))
        };
        let events = vec![
            event(UsageEventKind::Resumed, 1_000, "Main"),
            event(UsageEventKind::Resumed, 5_000, "Game"),
            event(UsageEventKind::Paused, 5_010, "Main"),
            event(UsageEventKind::Paused, 31_000, "Game"),
        ];
        let intervals = intervals_for(&pkg(), &events, 60_000);
        assert_eq!(intervals.len(), 1);
        assert_eq!(intervals[0].duration_ms(), 30_000);
    }

    #[test]
    fn resume_while_screen_is_off_does_not_count() {
        let events = vec![
            ev(UsageEventKind::ScreenNonInteractive, 1_000, None),
            ev(UsageEventKind::Resumed, 2_000, Some("com.example.vn")),
            ev(UsageEventKind::ScreenInteractive, 10_000, None),
        ];
        assert_eq!(
            intervals_for(&pkg(), &events, 20_000).len(),
            0,
            "intervals_for(&pkg(), &events, 20_000) should be empty"
        );
    }
}
