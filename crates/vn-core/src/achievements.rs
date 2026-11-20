//! Achievement catalog, pure evaluation and monotonic unlock merge.
//!
//! The catalog is the approved 65-item list from `docs/PLAN-ACHIEVEMENTS.md`.
//! Every slug matches an icon file in `src/assets/achievements/<slug>.svg`.
//!
//! Evaluation is pure: the app layer builds one [`AchievementInput`] of shared
//! aggregates (local-day buckets, live-parent joins, `finish_ts_sql`) and this
//! module decides unlocked/progress/target for every item. No Tauri, no OS, no
//! clock reads: `now` is passed in so tests are deterministic.
//!
//! Unlocks are monotonic. Once earned they are never revoked, and the sync
//! merge ([`merge_unlocks`]) keeps the earliest `unlocked_at` for a slug, with
//! deterministic tiebreaks so two devices converge. A tombstone never beats a
//! live unlock.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::clock::Timestamp;
use crate::model::SyncMeta;
use crate::sync::{self, MetaFields};

/// Milliseconds in one hour of counted playtime.
pub const HOUR_MS: i64 = 3_600_000;
/// Milliseconds in one local day (used for gap/anniversary arithmetic only;
/// day bucketing itself comes from the SQL `'localtime'` calendar).
pub const DAY_MS: i64 = 86_400_000;

/// Effort tier of an achievement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    Bronze,
    Silver,
    Gold,
}

impl Tier {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Bronze => "bronze",
            Self::Silver => "silver",
            Self::Gold => "gold",
        }
    }

    /// Parse a stored tier string.
    ///
    /// # Errors
    /// Returns an error string for any unrecognized value.
    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw {
            "bronze" => Ok(Self::Bronze),
            "silver" => Ok(Self::Silver),
            "gold" => Ok(Self::Gold),
            _ => Err(format!("unknown achievement tier: {raw}")),
        }
    }
}

/// One catalog entry. Names and descriptions are i18n keys owned by the
/// frontend (phase P2); the backend only carries the stable slug, tier, group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AchievementDef {
    pub slug: &'static str,
    pub tier: Tier,
    pub group: &'static str,
}

// Group letters are the wire contract: the frontend zod schema and the
// `achievements.group.*` i18n keys both use exactly "A".."F". Keep them in sync.
/// Reading time and consistency (A1..A20).
const GROUP_TIME: &str = "A";
/// Collection and finishes (B1..B14).
const GROUP_COLLECTION: &str = "B";
/// Routes, chapters, endings (C1..C9).
const GROUP_ROUTES: &str = "C";
/// Journal, notes, goals (D1..D8).
const GROUP_JOURNAL: &str = "D";
/// Game screenshots (E1..E5). Windows/Android capture only.
const GROUP_SHOTS: &str = "E";
/// Exploration and habits (F1..F9).
const GROUP_EXPLORATION: &str = "F";

/// The final approved catalog, in plan order (A, B, C, D, E, F).
static CATALOG: [AchievementDef; 65] = [
    // A. Waktu baca dan konsistensi.
    AchievementDef {
        slug: "first-session",
        tier: Tier::Bronze,
        group: GROUP_TIME,
    },
    AchievementDef {
        slug: "playtime-10h",
        tier: Tier::Bronze,
        group: GROUP_TIME,
    },
    AchievementDef {
        slug: "playtime-100h",
        tier: Tier::Silver,
        group: GROUP_TIME,
    },
    AchievementDef {
        slug: "playtime-500h",
        tier: Tier::Gold,
        group: GROUP_TIME,
    },
    AchievementDef {
        slug: "playtime-1000h",
        tier: Tier::Gold,
        group: GROUP_TIME,
    },
    AchievementDef {
        slug: "day-2h",
        tier: Tier::Bronze,
        group: GROUP_TIME,
    },
    AchievementDef {
        slug: "day-5h",
        tier: Tier::Silver,
        group: GROUP_TIME,
    },
    AchievementDef {
        slug: "day-10h",
        tier: Tier::Gold,
        group: GROUP_TIME,
    },
    AchievementDef {
        slug: "marathon-10h-5d",
        tier: Tier::Gold,
        group: GROUP_TIME,
    },
    AchievementDef {
        slug: "streak-2h-7d",
        tier: Tier::Gold,
        group: GROUP_TIME,
    },
    AchievementDef {
        slug: "streak-3",
        tier: Tier::Bronze,
        group: GROUP_TIME,
    },
    AchievementDef {
        slug: "streak-7",
        tier: Tier::Silver,
        group: GROUP_TIME,
    },
    AchievementDef {
        slug: "streak-30",
        tier: Tier::Gold,
        group: GROUP_TIME,
    },
    AchievementDef {
        slug: "streak-100",
        tier: Tier::Gold,
        group: GROUP_TIME,
    },
    AchievementDef {
        slug: "session-4h",
        tier: Tier::Silver,
        group: GROUP_TIME,
    },
    AchievementDef {
        slug: "session-8h",
        tier: Tier::Gold,
        group: GROUP_TIME,
    },
    AchievementDef {
        slug: "night-owl",
        tier: Tier::Silver,
        group: GROUP_TIME,
    },
    AchievementDef {
        slug: "early-bird",
        tier: Tier::Bronze,
        group: GROUP_TIME,
    },
    AchievementDef {
        slug: "sessions-100",
        tier: Tier::Silver,
        group: GROUP_TIME,
    },
    AchievementDef {
        slug: "comeback",
        tier: Tier::Bronze,
        group: GROUP_TIME,
    },
    // B. Koleksi dan tamat.
    AchievementDef {
        slug: "first-game",
        tier: Tier::Bronze,
        group: GROUP_COLLECTION,
    },
    AchievementDef {
        slug: "library-10",
        tier: Tier::Bronze,
        group: GROUP_COLLECTION,
    },
    AchievementDef {
        slug: "library-50",
        tier: Tier::Silver,
        group: GROUP_COLLECTION,
    },
    AchievementDef {
        slug: "library-100",
        tier: Tier::Gold,
        group: GROUP_COLLECTION,
    },
    AchievementDef {
        slug: "first-finish",
        tier: Tier::Bronze,
        group: GROUP_COLLECTION,
    },
    AchievementDef {
        slug: "finished-5",
        tier: Tier::Silver,
        group: GROUP_COLLECTION,
    },
    AchievementDef {
        slug: "finished-25",
        tier: Tier::Gold,
        group: GROUP_COLLECTION,
    },
    AchievementDef {
        slug: "finished-100",
        tier: Tier::Gold,
        group: GROUP_COLLECTION,
    },
    AchievementDef {
        slug: "finish-long",
        tier: Tier::Silver,
        group: GROUP_COLLECTION,
    },
    AchievementDef {
        slug: "rating-first",
        tier: Tier::Bronze,
        group: GROUP_COLLECTION,
    },
    AchievementDef {
        slug: "rating-10",
        tier: Tier::Silver,
        group: GROUP_COLLECTION,
    },
    AchievementDef {
        slug: "rated-25",
        tier: Tier::Silver,
        group: GROUP_COLLECTION,
    },
    AchievementDef {
        slug: "wishlist-10",
        tier: Tier::Bronze,
        group: GROUP_COLLECTION,
    },
    AchievementDef {
        slug: "anniversary-1y",
        tier: Tier::Gold,
        group: GROUP_COLLECTION,
    },
    // C. Rute, chapter, ending.
    AchievementDef {
        slug: "first-route",
        tier: Tier::Bronze,
        group: GROUP_ROUTES,
    },
    AchievementDef {
        slug: "first-ending",
        tier: Tier::Bronze,
        group: GROUP_ROUTES,
    },
    AchievementDef {
        slug: "endings-10",
        tier: Tier::Silver,
        group: GROUP_ROUTES,
    },
    AchievementDef {
        slug: "endings-50",
        tier: Tier::Gold,
        group: GROUP_ROUTES,
    },
    AchievementDef {
        slug: "endings-vn-3",
        tier: Tier::Silver,
        group: GROUP_ROUTES,
    },
    AchievementDef {
        slug: "endings-vn-5",
        tier: Tier::Gold,
        group: GROUP_ROUTES,
    },
    AchievementDef {
        slug: "chapters-50",
        tier: Tier::Silver,
        group: GROUP_ROUTES,
    },
    AchievementDef {
        slug: "route-10h",
        tier: Tier::Silver,
        group: GROUP_ROUTES,
    },
    AchievementDef {
        slug: "endings-vn-10",
        tier: Tier::Gold,
        group: GROUP_ROUTES,
    },
    // D. Jurnal, catatan, goal.
    AchievementDef {
        slug: "first-note",
        tier: Tier::Bronze,
        group: GROUP_JOURNAL,
    },
    AchievementDef {
        slug: "notes-25",
        tier: Tier::Silver,
        group: GROUP_JOURNAL,
    },
    AchievementDef {
        slug: "notes-100",
        tier: Tier::Gold,
        group: GROUP_JOURNAL,
    },
    AchievementDef {
        slug: "save-slot-10",
        tier: Tier::Silver,
        group: GROUP_JOURNAL,
    },
    AchievementDef {
        slug: "journal-streak-7",
        tier: Tier::Gold,
        group: GROUP_JOURNAL,
    },
    AchievementDef {
        slug: "global-note-first",
        tier: Tier::Bronze,
        group: GROUP_JOURNAL,
    },
    AchievementDef {
        slug: "goal-first",
        tier: Tier::Bronze,
        group: GROUP_JOURNAL,
    },
    AchievementDef {
        slug: "goal-hit",
        tier: Tier::Silver,
        group: GROUP_JOURNAL,
    },
    // E. Screenshot (Windows/Android capture).
    AchievementDef {
        slug: "shot-first",
        tier: Tier::Bronze,
        group: GROUP_SHOTS,
    },
    AchievementDef {
        slug: "shots-25",
        tier: Tier::Silver,
        group: GROUP_SHOTS,
    },
    AchievementDef {
        slug: "shots-100",
        tier: Tier::Gold,
        group: GROUP_SHOTS,
    },
    AchievementDef {
        slug: "caption-10",
        tier: Tier::Silver,
        group: GROUP_SHOTS,
    },
    AchievementDef {
        slug: "shot-games-5",
        tier: Tier::Silver,
        group: GROUP_SHOTS,
    },
    // F. Jelajah dan kebiasaan.
    AchievementDef {
        slug: "walkthrough-first",
        tier: Tier::Bronze,
        group: GROUP_EXPLORATION,
    },
    AchievementDef {
        slug: "guide-checks-20",
        tier: Tier::Silver,
        group: GROUP_EXPLORATION,
    },
    AchievementDef {
        slug: "manual-translate",
        tier: Tier::Silver,
        group: GROUP_EXPLORATION,
    },
    AchievementDef {
        slug: "discover-import",
        tier: Tier::Bronze,
        group: GROUP_EXPLORATION,
    },
    AchievementDef {
        slug: "studios-5",
        tier: Tier::Silver,
        group: GROUP_EXPLORATION,
    },
    AchievementDef {
        slug: "tags-10",
        tier: Tier::Silver,
        group: GROUP_EXPLORATION,
    },
    AchievementDef {
        slug: "backup-first",
        tier: Tier::Bronze,
        group: GROUP_EXPLORATION,
    },
    AchievementDef {
        slug: "sync-first",
        tier: Tier::Bronze,
        group: GROUP_EXPLORATION,
    },
    AchievementDef {
        slug: "cross-device",
        tier: Tier::Silver,
        group: GROUP_EXPLORATION,
    },
];

/// The full approved catalog (65 items, plan order).
#[must_use]
pub fn catalog() -> &'static [AchievementDef] {
    &CATALOG
}

/// Look up one catalog entry by slug.
#[must_use]
pub fn def(slug: &str) -> Option<&'static AchievementDef> {
    CATALOG.iter().find(|entry| entry.slug == slug)
}

/// Shared aggregates the app layer computes once for the whole catalog.
///
/// Every value already respects the orphan rule (child aggregates join their
/// live parent) and the local-day definition (`'localtime'` buckets). Manual
/// playtime corrections are included in `total_playtime_ms` only: they carry no
/// date, so they never enter daily/streak aggregates.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AchievementInput {
    /// All-time counted playtime including per-game `manual_playtime_ms`, floored at 0 per game.
    pub total_playtime_ms: i64,
    /// Sessions with `counted_ms > 0` over live games.
    pub session_count: i64,
    /// Earliest counted session start (the calendar's "started reading" rule).
    pub first_session_at: Option<Timestamp>,
    /// Largest single-session `counted_ms`.
    pub longest_session_ms: i64,
    /// Largest gap in whole days between consecutive counted sessions
    /// (previous `ended_at` to next `started_at`), across all games.
    pub max_gap_days: i64,
    /// Start of the first counted session that followed a gap of >= 30 days.
    pub comeback_at: Option<Timestamp>,
    /// Local-day playtime buckets `(day_index, counted_ms)` from `playtime_by_day`.
    pub day_playtime: Vec<(i64, i64)>,
    /// Counted ms of sessions starting in the local 00:00-04:00 window.
    pub night_ms: i64,
    /// Counted ms of sessions starting in the local 05:00-08:00 window.
    pub morning_ms: i64,
    /// Distinct `device_id` values among counted sessions.
    pub distinct_session_devices: i64,
    /// Live games in the collection, any status.
    pub library_count: i64,
    /// Live games with status `wishlist`.
    pub wishlist_count: i64,
    /// Live games with a rating.
    pub rated_count: i64,
    /// Live games rated exactly 10.0 (100 decivotes).
    pub perfect_rating_count: i64,
    /// Live games with a finish timestamp (`finish_ts_sql`).
    pub finished_count: i64,
    /// Earliest finish timestamp across live games.
    pub first_finish_at: Option<Timestamp>,
    /// Finished live games whose VNDB `length_minutes` >= 3000.
    pub long_finish_count: i64,
    /// Earliest finish timestamp among long finished games.
    pub first_long_finish_at: Option<Timestamp>,
    /// Live routes over live games.
    pub routes_count: i64,
    /// Largest counted total attributed to a single live route.
    pub max_route_ms: i64,
    /// Achieved endings across live games.
    pub endings_achieved_total: i64,
    /// Most achieved endings recorded within one live game.
    pub endings_max_per_game: i64,
    /// Earliest `achieved_at` among achieved endings.
    pub first_ending_at: Option<Timestamp>,
    /// Chapters marked reached over live games.
    pub chapters_reached: i64,
    /// Live journal entries over live games.
    pub journal_count: i64,
    /// Earliest journal `created_at`.
    pub first_journal_at: Option<Timestamp>,
    /// Journal entries with a nonempty `save_slot`.
    pub save_slot_count: i64,
    /// Distinct local day indexes carrying at least one journal entry.
    pub journal_days: Vec<i64>,
    /// Live global notes.
    pub global_note_count: i64,
    /// Earliest global note `created_at`.
    pub first_global_note_at: Option<Timestamp>,
    /// Live reading goals.
    pub goal_count: i64,
    /// Earliest reading goal `created_at`.
    pub first_goal_at: Option<Timestamp>,
    /// Automatic goals whose live progress reached their target.
    pub goal_hit_count: i64,
    /// Live game screenshots over live games.
    pub screenshot_count: i64,
    /// Earliest screenshot `captured_at`.
    pub first_screenshot_at: Option<Timestamp>,
    /// Screenshots with a nonempty caption.
    pub caption_count: i64,
    /// Distinct games with at least one screenshot.
    pub screenshot_game_count: i64,
    /// Walkthroughs fetched for live games.
    pub walkthrough_count: i64,
    /// Earliest walkthrough `created_at`.
    pub first_walkthrough_at: Option<Timestamp>,
    /// Accumulated checked guide steps across live-game walkthroughs.
    pub guide_checks: i64,
    /// Manual walkthrough translations stored for live games.
    pub manual_translation_count: i64,
    /// Earliest manual translation `updated_at`.
    pub first_manual_translation_at: Option<Timestamp>,
    /// Marker: a game was imported from the Discover screen (F4).
    pub discover_imported_at: Option<Timestamp>,
    /// Marker: the first backup/library export was made (F7).
    pub backup_made_at: Option<Timestamp>,
    /// Distinct VNDB developers across played games (counted sessions).
    pub studio_count: i64,
    /// Distinct non-spoiler, non-lie VNDB tags across played games.
    pub tag_count: i64,
    /// Sync runs with outcome `ok`.
    pub sync_ok_count: i64,
    /// Earliest successful sync run finish time.
    pub first_sync_ok_at: Option<Timestamp>,
}

/// Evaluation result for one catalog item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AchievementEval {
    pub slug: &'static str,
    pub unlocked: bool,
    pub progress: i64,
    pub target: i64,
}

/// Longest run of consecutive day indexes (difference of exactly 1).
/// Input order does not matter; duplicates are collapsed.
#[must_use]
pub fn longest_consecutive_run(days: &[i64]) -> i64 {
    let mut sorted: Vec<i64> = days.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    let mut best = 0_i64;
    let mut current = 0_i64;
    let mut previous: Option<i64> = None;
    for day in sorted {
        current = match previous {
            Some(prev) if day == prev + 1 => current + 1,
            _ => 1,
        };
        best = best.max(current);
        previous = Some(day);
    }
    best
}

/// Longest consecutive run of days whose counted ms reached `min_ms`.
#[must_use]
pub fn longest_run_at_least(day_playtime: &[(i64, i64)], min_ms: i64) -> i64 {
    let days: Vec<i64> = day_playtime
        .iter()
        .filter(|(_, ms)| *ms >= min_ms)
        .map(|(day, _)| *day)
        .collect();
    longest_consecutive_run(&days)
}

/// Progress and target for one slug. Both are in the same unit so the UI can
/// render a bar without knowing the predicate: ms for time, counts otherwise.
fn progress_and_target(slug: &str, input: &AchievementInput, now: Timestamp) -> (i64, i64) {
    let max_day_ms = input
        .day_playtime
        .iter()
        .map(|(_, ms)| *ms)
        .max()
        .unwrap_or(0);
    let reading_days: Vec<i64> = input
        .day_playtime
        .iter()
        .filter(|(_, ms)| *ms > 0)
        .map(|(day, _)| *day)
        .collect();
    let anniversary_days = input.first_session_at.map_or(0, |first| {
        (now.as_millis() - first.as_millis())
            .div_euclid(DAY_MS)
            .max(0)
    });
    match slug {
        "first-session" => (input.session_count, 1),
        "playtime-10h" => (input.total_playtime_ms, 10 * HOUR_MS),
        "playtime-100h" => (input.total_playtime_ms, 100 * HOUR_MS),
        "playtime-500h" => (input.total_playtime_ms, 500 * HOUR_MS),
        "playtime-1000h" => (input.total_playtime_ms, 1000 * HOUR_MS),
        "day-2h" => (max_day_ms, 2 * HOUR_MS),
        "day-5h" => (max_day_ms, 5 * HOUR_MS),
        "day-10h" => (max_day_ms, 10 * HOUR_MS),
        "marathon-10h-5d" => (longest_run_at_least(&input.day_playtime, 10 * HOUR_MS), 5),
        "streak-2h-7d" => (longest_run_at_least(&input.day_playtime, 2 * HOUR_MS), 7),
        "streak-3" => (longest_consecutive_run(&reading_days), 3),
        "streak-7" => (longest_consecutive_run(&reading_days), 7),
        "streak-30" => (longest_consecutive_run(&reading_days), 30),
        "streak-100" => (longest_consecutive_run(&reading_days), 100),
        "session-4h" => (input.longest_session_ms, 4 * HOUR_MS),
        "session-8h" => (input.longest_session_ms, 8 * HOUR_MS),
        "night-owl" => (input.night_ms, HOUR_MS),
        "early-bird" => (input.morning_ms, HOUR_MS),
        "sessions-100" => (input.session_count, 100),
        "comeback" => (input.max_gap_days, 30),
        "first-game" => (input.library_count, 1),
        "library-10" => (input.library_count, 10),
        "library-50" => (input.library_count, 50),
        "library-100" => (input.library_count, 100),
        "first-finish" => (input.finished_count, 1),
        "finished-5" => (input.finished_count, 5),
        "finished-25" => (input.finished_count, 25),
        "finished-100" => (input.finished_count, 100),
        "finish-long" => (input.long_finish_count, 1),
        "rating-first" => (input.rated_count, 1),
        "rating-10" => (input.perfect_rating_count, 1),
        "rated-25" => (input.rated_count, 25),
        "wishlist-10" => (input.wishlist_count, 10),
        "anniversary-1y" => (anniversary_days, 365),
        "first-route" => (input.routes_count, 1),
        "first-ending" => (input.endings_achieved_total, 1),
        "endings-10" => (input.endings_achieved_total, 10),
        "endings-50" => (input.endings_achieved_total, 50),
        "endings-vn-3" => (input.endings_max_per_game, 3),
        "endings-vn-5" => (input.endings_max_per_game, 5),
        "chapters-50" => (input.chapters_reached, 50),
        "route-10h" => (input.max_route_ms, 10 * HOUR_MS),
        "endings-vn-10" => (input.endings_max_per_game, 10),
        "first-note" => (input.journal_count, 1),
        "notes-25" => (input.journal_count, 25),
        "notes-100" => (input.journal_count, 100),
        "save-slot-10" => (input.save_slot_count, 10),
        "journal-streak-7" => (longest_consecutive_run(&input.journal_days), 7),
        "global-note-first" => (input.global_note_count, 1),
        "goal-first" => (input.goal_count, 1),
        "goal-hit" => (input.goal_hit_count, 1),
        "shot-first" => (input.screenshot_count, 1),
        "shots-25" => (input.screenshot_count, 25),
        "shots-100" => (input.screenshot_count, 100),
        "caption-10" => (input.caption_count, 10),
        "shot-games-5" => (input.screenshot_game_count, 5),
        "walkthrough-first" => (input.walkthrough_count, 1),
        "guide-checks-20" => (input.guide_checks, 20),
        "manual-translate" => (input.manual_translation_count, 1),
        "discover-import" => (i64::from(input.discover_imported_at.is_some()), 1),
        "studios-5" => (input.studio_count, 5),
        "tags-10" => (input.tag_count, 10),
        "backup-first" => (i64::from(input.backup_made_at.is_some()), 1),
        "sync-first" => (input.sync_ok_count, 1),
        "cross-device" => (input.distinct_session_devices, 2),
        // Unknown slugs never unlock; the catalog is the only source of truth.
        _ => (0, 1),
    }
}

/// Evaluate every catalog item against the shared aggregates.
///
/// Boundaries are inclusive (`progress >= target`): exactly 10 hours unlocks
/// `playtime-10h`. Results follow catalog order.
#[must_use]
pub fn evaluate(input: &AchievementInput, now: Timestamp) -> Vec<AchievementEval> {
    CATALOG
        .iter()
        .map(|entry| {
            let (progress, target) = progress_and_target(entry.slug, input, now);
            AchievementEval {
                slug: entry.slug,
                unlocked: progress >= target,
                progress,
                target,
            }
        })
        .collect()
}

/// The honest unlock timestamp for a slug, or `None` while it is locked.
///
/// Event-backed items use the real recorded date (first session, first finish,
/// markers, ...). Cumulative thresholds use `now`, the scan time: the history
/// does not record when a running total first crossed a line, and inventing a
/// historical date would be a false claim (R-17/R-36). The anniversary is the
/// one derived exception: its moment is exactly first session + 365 days.
#[must_use]
pub fn unlocked_at_for(slug: &str, input: &AchievementInput, now: Timestamp) -> Option<Timestamp> {
    let (progress, target) = progress_and_target(slug, input, now);
    if progress < target {
        return None;
    }
    let event = match slug {
        "first-session" => input.first_session_at,
        "comeback" => input.comeback_at,
        "first-finish" => input.first_finish_at,
        "finish-long" => input.first_long_finish_at,
        "first-ending" => input.first_ending_at,
        "first-note" => input.first_journal_at,
        "global-note-first" => input.first_global_note_at,
        "goal-first" => input.first_goal_at,
        "shot-first" => input.first_screenshot_at,
        "walkthrough-first" => input.first_walkthrough_at,
        "manual-translate" => input.first_manual_translation_at,
        "discover-import" => input.discover_imported_at,
        "backup-first" => input.backup_made_at,
        "sync-first" => input.first_sync_ok_at,
        "anniversary-1y" => input
            .first_session_at
            .map(|first| Timestamp::from_millis(first.as_millis() + 365 * DAY_MS)),
        _ => None,
    };
    Some(event.unwrap_or(now))
}

/// A synced achievement unlock. Storage identity is the slug (`achievement_id`
/// is UNIQUE); the UUID in `meta.id` travels with the row but is not the key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AchievementUnlock {
    pub meta: SyncMeta,
    /// Catalog slug, e.g. `first-session`.
    pub achievement_id: String,
    /// When the achievement was earned, UTC ms (event date or scan time).
    pub unlocked_at: Timestamp,
}

impl MetaFields for AchievementUnlock {
    fn meta(&self) -> &SyncMeta {
        &self.meta
    }
}

impl AchievementUnlock {
    /// Validate before writing or importing: the slug must exist in the catalog.
    ///
    /// # Errors
    /// Returns an error string for an unknown or empty slug.
    pub fn validate(&self) -> Result<(), String> {
        if def(&self.achievement_id).is_none() {
            return Err(format!("unknown achievement: {}", self.achievement_id));
        }
        Ok(())
    }
}

/// Total order deciding which of two copies of the same slug wins.
///
/// `Less` means `a` wins. Keys, in order: live beats deleted (unlocks are
/// monotonic, a tombstone never revokes one), earliest `unlocked_at`, newest
/// `updated_at`, highest `device_id`, and finally highest row id so even two
/// installs sharing a device id converge on one stored row.
fn unlock_precedence(a: &AchievementUnlock, b: &AchievementUnlock) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    // A live unlock outranks a tombstone regardless of timestamps.
    match (a.meta.deleted, b.meta.deleted) {
        (false, true) => return Ordering::Less,
        (true, false) => return Ordering::Greater,
        _ => {}
    }
    a.unlocked_at
        .cmp(&b.unlocked_at)
        .then_with(|| b.meta.updated_at.cmp(&a.meta.updated_at))
        .then_with(|| {
            b.meta
                .device_id_device
                .device()
                .as_str()
                .cmp(a.meta.device_id_device.device().as_str())
        })
        .then_with(|| b.meta.id.cmp(&a.meta.id))
}

/// Monotonic union of two devices' unlocks, keyed by slug.
///
/// Both sides keep every slug either had; for a shared slug the earliest
/// `unlocked_at` wins, with deterministic tiebreaks. Unlocks are never deleted:
/// a deleted copy loses to a live one. Mirrors [`sync::merge_rows`] ordering
/// (local rows first, then remote-only) and conflict noise rules.
#[must_use]
pub fn merge_unlocks(
    local: &[AchievementUnlock],
    remote: &[AchievementUnlock],
) -> (Vec<AchievementUnlock>, Vec<sync::Conflict>) {
    let mut out: Vec<AchievementUnlock> = Vec::with_capacity(local.len() + remote.len());
    let mut conflicts: Vec<sync::Conflict> = Vec::new();
    let remote_by_slug: HashMap<&str, &AchievementUnlock> = remote
        .iter()
        .map(|row| (row.achievement_id.as_str(), row))
        .collect();
    let mut seen: HashSet<&str> = HashSet::with_capacity(local.len());

    for row in local {
        let slug = row.achievement_id.as_str();
        seen.insert(slug);
        match remote_by_slug.get(slug) {
            None => out.push(row.clone()),
            Some(theirs) => {
                let theirs = *theirs;
                let decision = unlock_precedence(row, theirs);
                let (winner, kept, discarded) = match decision {
                    std::cmp::Ordering::Less => (row, sync::Side::Local, theirs),
                    std::cmp::Ordering::Greater => (theirs, sync::Side::Remote, row),
                    // Same revision on both sides: keep local, log nothing.
                    std::cmp::Ordering::Equal => {
                        out.push(row.clone());
                        continue;
                    }
                };
                out.push(winner.clone());
                // Only log when the copies actually differed.
                if row != theirs {
                    conflicts.push(sync::Conflict {
                        id: slug.to_owned(),
                        table: "achievement_unlocks",
                        kept,
                        reason: sync::Reason::UnlockWins,
                        discarded_updated_at_ms: discarded.meta.updated_at.as_millis(),
                    });
                }
            }
        }
    }

    for row in remote {
        if !seen.contains(row.achievement_id.as_str()) {
            out.push(row.clone());
        }
    }

    (out, conflicts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{DeviceId, EntityId};
    use crate::model::DeviceTag;

    const DEV_A: &str = "01a00000-0000-7000-8000-00000000000a";
    const DEV_B: &str = "01a00000-0000-7000-8000-00000000000b";
    const NOW: i64 = 1_700_000_000_000;

    fn now() -> Timestamp {
        Timestamp::from_millis(NOW)
    }

    fn unlock(slug: &str, device: &str, unlocked_at: i64, updated_at: i64) -> AchievementUnlock {
        unlock_with_deleted(slug, device, unlocked_at, updated_at, false)
    }

    fn unlock_with_deleted(
        slug: &str,
        device: &str,
        unlocked_at: i64,
        updated_at: i64,
        deleted: bool,
    ) -> AchievementUnlock {
        AchievementUnlock {
            meta: SyncMeta {
                id: EntityId::new(),
                updated_at: Timestamp::from_millis(updated_at),
                device_id_device: DeviceTag::new(&DeviceId::parse(device).expect("device")),
                deleted,
            },
            achievement_id: slug.to_owned(),
            unlocked_at: Timestamp::from_millis(unlocked_at),
        }
    }

    fn eval(slug: &str, input: &AchievementInput) -> AchievementEval {
        evaluate(input, now())
            .into_iter()
            .find(|item| item.slug == slug)
            .expect("catalog slug")
    }

    fn empty_input() -> AchievementInput {
        AchievementInput::default()
    }

    // ---- Catalog -----------------------------------------------------------

    #[test]
    fn catalog_has_65_unique_slugs_matching_the_approved_icons() {
        // The exact approved slug set; each name is an existing
        // src/assets/achievements/<slug>.svg file.
        let expected = [
            "first-session",
            "playtime-10h",
            "playtime-100h",
            "playtime-500h",
            "playtime-1000h",
            "day-2h",
            "day-5h",
            "day-10h",
            "marathon-10h-5d",
            "streak-2h-7d",
            "streak-3",
            "streak-7",
            "streak-30",
            "streak-100",
            "session-4h",
            "session-8h",
            "night-owl",
            "early-bird",
            "sessions-100",
            "comeback",
            "first-game",
            "library-10",
            "library-50",
            "library-100",
            "first-finish",
            "finished-5",
            "finished-25",
            "finished-100",
            "finish-long",
            "rating-first",
            "rating-10",
            "rated-25",
            "wishlist-10",
            "anniversary-1y",
            "first-route",
            "first-ending",
            "endings-10",
            "endings-50",
            "endings-vn-3",
            "endings-vn-5",
            "chapters-50",
            "route-10h",
            "endings-vn-10",
            "first-note",
            "notes-25",
            "notes-100",
            "save-slot-10",
            "journal-streak-7",
            "global-note-first",
            "goal-first",
            "goal-hit",
            "shot-first",
            "shots-25",
            "shots-100",
            "caption-10",
            "shot-games-5",
            "walkthrough-first",
            "guide-checks-20",
            "manual-translate",
            "discover-import",
            "studios-5",
            "tags-10",
            "backup-first",
            "sync-first",
            "cross-device",
        ];
        assert_eq!(catalog().len(), 65);
        assert_eq!(expected.len(), 65);
        let mut slugs: Vec<&str> = catalog().iter().map(|entry| entry.slug).collect();
        slugs.sort_unstable();
        let mut wanted = expected.to_vec();
        wanted.sort_unstable();
        assert_eq!(slugs, wanted, "catalog must match the approved icon slugs");
        assert_eq!(
            slugs.iter().collect::<HashSet<_>>().len(),
            65,
            "slugs must be unique"
        );
    }

    #[test]
    fn tier_distribution_is_the_approved_pyramid() {
        let count = |tier: Tier| catalog().iter().filter(|entry| entry.tier == tier).count();
        assert_eq!(count(Tier::Bronze), 21);
        assert_eq!(count(Tier::Silver), 26);
        assert_eq!(count(Tier::Gold), 18);
    }

    #[test]
    fn tier_roundtrips() {
        for tier in [Tier::Bronze, Tier::Silver, Tier::Gold] {
            assert_eq!(Tier::parse(tier.as_str()).unwrap(), tier);
        }
        assert!(Tier::parse("platinum").is_err());
    }

    #[test]
    fn evaluate_covers_every_catalog_item_in_order() {
        let results = evaluate(&AchievementInput::default(), now());
        assert_eq!(results.len(), 65);
        for (entry, result) in catalog().iter().zip(&results) {
            assert_eq!(entry.slug, result.slug);
            assert!(!result.unlocked, "empty history unlocks nothing");
        }
    }

    #[test]
    fn catalog_groups_are_the_wire_letters_a_to_f() {
        // The frontend zod schema and i18n keys use exactly these letters; a
        // descriptive slug here once broke achievements_list at runtime.
        let mut counts = std::collections::BTreeMap::new();
        for entry in catalog() {
            assert!(
                ["A", "B", "C", "D", "E", "F"].contains(&entry.group),
                "{} has non-wire group {:?}",
                entry.slug,
                entry.group,
            );
            *counts.entry(entry.group).or_insert(0usize) += 1;
        }
        assert_eq!(counts.get("A"), Some(&20));
        assert_eq!(counts.get("B"), Some(&14));
        assert_eq!(counts.get("C"), Some(&9));
        assert_eq!(counts.get("D"), Some(&8));
        assert_eq!(counts.get("E"), Some(&5));
        assert_eq!(counts.get("F"), Some(&9));
        // Anchor one slug per group so a remap cannot silently pass.
        let group_of = |slug: &str| catalog().iter().find(|e| e.slug == slug).unwrap().group;
        assert_eq!(group_of("first-session"), "A");
        assert_eq!(group_of("first-game"), "B");
        assert_eq!(group_of("first-route"), "C");
        assert_eq!(group_of("first-note"), "D");
        assert_eq!(group_of("shot-first"), "E");
        assert_eq!(group_of("walkthrough-first"), "F");
    }

    // ---- Predicate boundaries ----------------------------------------------

    #[test]
    fn playtime_boundary_is_inclusive_at_exactly_ten_hours() {
        let mut input = empty_input();
        input.total_playtime_ms = 10 * HOUR_MS - 1;
        assert!(!eval("playtime-10h", &input).unlocked);
        input.total_playtime_ms = 10 * HOUR_MS;
        let exact = eval("playtime-10h", &input);
        assert!(exact.unlocked, "exactly 10h must unlock");
        assert_eq!(exact.progress, 10 * HOUR_MS);
        assert!(!eval("playtime-100h", &input).unlocked);
    }

    #[test]
    fn daily_boundary_uses_the_best_local_day() {
        let mut input = empty_input();
        input.day_playtime = vec![(100, HOUR_MS), (101, 2 * HOUR_MS)];
        assert!(eval("day-2h", &input).unlocked);
        assert!(!eval("day-5h", &input).unlocked);
        input.day_playtime.push((200, 10 * HOUR_MS));
        assert!(eval("day-10h", &input).unlocked);
    }

    #[test]
    fn streak_counts_consecutive_day_indexes_across_midnight() {
        // day_index is the local calendar day; a session spanning midnight is
        // bucketed by its start day by the shared SQL, and consecutive indexes
        // are what makes a streak.
        let mut input = empty_input();
        input.day_playtime = vec![(100, 1), (101, 1), (102, 1)];
        assert!(eval("streak-3", &input).unlocked);
        assert!(!eval("streak-7", &input).unlocked);
        // A gap breaks the run even though three days were read in total.
        input.day_playtime = vec![(100, 1), (102, 1), (103, 1)];
        assert!(!eval("streak-3", &input).unlocked);
        // Zero-ms days do not count as reading.
        input.day_playtime = vec![(100, 0), (101, 1), (102, 1)];
        assert!(!eval("streak-3", &input).unlocked);
    }

    #[test]
    fn marathon_needs_five_consecutive_ten_hour_days() {
        let mut input = empty_input();
        input.day_playtime = (0..5).map(|day| (day, 10 * HOUR_MS)).collect();
        assert!(eval("marathon-10h-5d", &input).unlocked);
        // Four days is not five.
        input.day_playtime = (0..4).map(|day| (day, 10 * HOUR_MS)).collect();
        assert!(!eval("marathon-10h-5d", &input).unlocked);
        // A sub-10h day breaks the run even with six days present.
        input.day_playtime = vec![
            (0, 10 * HOUR_MS),
            (1, 10 * HOUR_MS),
            (2, 9 * HOUR_MS),
            (3, 10 * HOUR_MS),
            (4, 10 * HOUR_MS),
            (5, 10 * HOUR_MS),
        ];
        assert!(!eval("marathon-10h-5d", &input).unlocked);
    }

    #[test]
    fn full_week_needs_seven_consecutive_two_hour_days() {
        let mut input = empty_input();
        input.day_playtime = (0..7).map(|day| (day, 2 * HOUR_MS)).collect();
        assert!(eval("streak-2h-7d", &input).unlocked);
        input.day_playtime[3] = (3, 2 * HOUR_MS - 1);
        assert!(!eval("streak-2h-7d", &input).unlocked);
    }

    #[test]
    fn single_session_boundaries() {
        let mut input = empty_input();
        input.longest_session_ms = 4 * HOUR_MS;
        assert!(eval("session-4h", &input).unlocked);
        assert!(!eval("session-8h", &input).unlocked);
        input.longest_session_ms = 8 * HOUR_MS;
        assert!(eval("session-8h", &input).unlocked);
    }

    #[test]
    fn night_and_morning_windows_accumulate_one_hour() {
        let mut input = empty_input();
        input.night_ms = HOUR_MS - 1;
        input.morning_ms = HOUR_MS;
        assert!(!eval("night-owl", &input).unlocked);
        assert!(eval("early-bird", &input).unlocked);
        input.night_ms = HOUR_MS;
        assert!(eval("night-owl", &input).unlocked);
    }

    #[test]
    fn comeback_boundary_is_thirty_days_and_dates_the_return() {
        let mut input = empty_input();
        input.max_gap_days = 29;
        assert!(!eval("comeback", &input).unlocked);
        input.max_gap_days = 30;
        let returned = Timestamp::from_millis(NOW - DAY_MS);
        input.comeback_at = Some(returned);
        assert!(eval("comeback", &input).unlocked);
        assert_eq!(unlocked_at_for("comeback", &input, now()), Some(returned));
    }

    #[test]
    fn anniversary_unlocks_at_365_days_with_the_derived_date() {
        let mut input = empty_input();
        let first = Timestamp::from_millis(NOW - 365 * DAY_MS);
        input.first_session_at = Some(first);
        let exact = eval("anniversary-1y", &input);
        assert!(exact.unlocked, "exactly 365 days must unlock");
        assert_eq!(
            unlocked_at_for("anniversary-1y", &input, now()),
            Some(Timestamp::from_millis(first.as_millis() + 365 * DAY_MS)),
            "the anniversary moment is first session + 365 days, not the scan time"
        );
        input.first_session_at = Some(Timestamp::from_millis(NOW - 365 * DAY_MS + 1));
        assert!(!eval("anniversary-1y", &input).unlocked);
    }

    #[test]
    fn per_vn_ending_ladder_uses_the_max_within_one_game() {
        let mut input = empty_input();
        // Ten endings spread over ten games is not three in one VN.
        input.endings_achieved_total = 10;
        input.endings_max_per_game = 1;
        assert!(eval("endings-10", &input).unlocked);
        assert!(!eval("endings-vn-3", &input).unlocked);
        input.endings_max_per_game = 3;
        assert!(eval("endings-vn-3", &input).unlocked);
        assert!(!eval("endings-vn-5", &input).unlocked);
        input.endings_max_per_game = 10;
        assert!(eval("endings-vn-10", &input).unlocked);
    }

    #[test]
    fn route_ten_hours_uses_the_max_single_route_total() {
        let mut input = empty_input();
        input.max_route_ms = 10 * HOUR_MS - 1;
        assert!(!eval("route-10h", &input).unlocked);
        input.max_route_ms = 10 * HOUR_MS;
        assert!(eval("route-10h", &input).unlocked);
    }

    #[test]
    fn journal_streak_counts_consecutive_entry_days() {
        let mut input = empty_input();
        input.journal_days = vec![5, 6, 7, 8, 9, 10, 11];
        assert!(eval("journal-streak-7", &input).unlocked);
        input.journal_days = vec![5, 6, 7, 9, 10, 11, 12];
        assert!(!eval("journal-streak-7", &input).unlocked);
    }

    #[test]
    fn markers_and_counts_gate_their_items() {
        let mut input = empty_input();
        assert!(!eval("discover-import", &input).unlocked);
        assert!(!eval("backup-first", &input).unlocked);
        let marked = Timestamp::from_millis(NOW - 1000);
        input.discover_imported_at = Some(marked);
        input.backup_made_at = Some(marked);
        assert!(eval("discover-import", &input).unlocked);
        assert!(eval("backup-first", &input).unlocked);
        assert_eq!(unlocked_at_for("backup-first", &input, now()), Some(marked));

        input.distinct_session_devices = 1;
        assert!(!eval("cross-device", &input).unlocked);
        input.distinct_session_devices = 2;
        assert!(eval("cross-device", &input).unlocked);

        input.goal_hit_count = 1;
        assert!(eval("goal-hit", &input).unlocked);
        input.studio_count = 5;
        input.tag_count = 10;
        assert!(eval("studios-5", &input).unlocked);
        assert!(eval("tags-10", &input).unlocked);
    }

    #[test]
    fn event_dates_are_used_when_available_and_scan_time_otherwise() {
        let mut input = empty_input();
        let first = Timestamp::from_millis(NOW - 10 * DAY_MS);
        input.first_session_at = Some(first);
        input.session_count = 1;
        input.total_playtime_ms = 10 * HOUR_MS;
        assert_eq!(unlocked_at_for("first-session", &input, now()), Some(first));
        // A cumulative threshold has no recorded crossing date: scan time.
        assert_eq!(unlocked_at_for("playtime-10h", &input, now()), Some(now()));
        // Locked items have no date at all.
        assert_eq!(unlocked_at_for("playtime-100h", &input, now()), None);
        assert_eq!(unlocked_at_for("not-a-slug", &input, now()), None);
    }

    #[test]
    fn finish_long_requires_a_long_finished_game() {
        let mut input = empty_input();
        input.finished_count = 3;
        input.long_finish_count = 0;
        assert!(eval("first-finish", &input).unlocked);
        assert!(!eval("finish-long", &input).unlocked);
        let finish = Timestamp::from_millis(NOW - DAY_MS);
        input.long_finish_count = 1;
        input.first_long_finish_at = Some(finish);
        assert!(eval("finish-long", &input).unlocked);
        assert_eq!(unlocked_at_for("finish-long", &input, now()), Some(finish));
    }

    #[test]
    fn longest_run_helpers_handle_unsorted_and_duplicate_days() {
        assert_eq!(longest_consecutive_run(&[3, 1, 2, 2, 5]), 3);
        assert_eq!(longest_consecutive_run(&[]), 0);
        assert_eq!(longest_consecutive_run(&[7]), 1);
        let days = vec![(2, HOUR_MS), (0, 2 * HOUR_MS), (1, 2 * HOUR_MS)];
        assert_eq!(longest_run_at_least(&days, 2 * HOUR_MS), 2);
        assert_eq!(longest_run_at_least(&days, HOUR_MS), 3);
    }

    // ---- Unlock validation ---------------------------------------------------

    #[test]
    fn validate_accepts_catalog_slugs_and_rejects_unknown_ones() {
        let good = unlock("first-session", DEV_A, NOW, NOW);
        assert!(good.validate().is_ok());
        let bad = unlock("not-in-catalog", DEV_A, NOW, NOW);
        assert!(bad.validate().is_err());
    }

    // ---- Monotonic merge -----------------------------------------------------

    #[test]
    fn earliest_unlock_wins_even_against_a_newer_edit() {
        let local = vec![unlock("streak-3", DEV_A, 1_000, 9_000)];
        let remote = vec![unlock("streak-3", DEV_B, 2_000, 1_000)];
        let (merged, conflicts) = merge_unlocks(&local, &remote);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].unlocked_at.as_millis(), 1_000);
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].reason, sync::Reason::UnlockWins);
        assert_eq!(conflicts[0].table, "achievement_unlocks");
        assert_eq!(conflicts[0].id, "streak-3");
        // The other device reaches the same answer.
        let (reverse, _) = merge_unlocks(&remote, &local);
        assert_eq!(reverse[0].unlocked_at.as_millis(), 1_000);
    }

    #[test]
    fn equal_unlock_times_break_by_newer_update_then_higher_device() {
        // Same unlocked_at, newer updated_at wins.
        let older = unlock("day-2h", DEV_A, 1_000, 1_000);
        let newer = unlock("day-2h", DEV_B, 1_000, 2_000);
        let (merged, _) = merge_unlocks(&[older], &[newer]);
        assert_eq!(merged[0].meta.updated_at.as_millis(), 2_000);

        // Same unlocked_at and updated_at: higher device id wins on both sides.
        let a = unlock("day-2h", DEV_A, 1_000, 1_000);
        let b = unlock("day-2h", DEV_B, 1_000, 1_000);
        let (from_a, _) = merge_unlocks(std::slice::from_ref(&a), std::slice::from_ref(&b));
        let (from_b, _) = merge_unlocks(&[b], &[a]);
        assert_eq!(
            from_a[0].meta.device_id_device.device().as_str(),
            DEV_B,
            "higher device id wins the tie"
        );
        assert_eq!(
            from_b[0].meta.device_id_device.device().as_str(),
            from_a[0].meta.device_id_device.device().as_str(),
            "both devices must converge"
        );
    }

    #[test]
    fn a_deleted_copy_never_revokes_a_live_unlock() {
        // Monotonic: even a newer tombstone loses to a live unlock.
        let live = unlock("night-owl", DEV_A, 1_000, 1_000);
        let dead = unlock_with_deleted("night-owl", DEV_B, 500, 9_000, true);
        let (merged, _) = merge_unlocks(std::slice::from_ref(&live), std::slice::from_ref(&dead));
        assert!(!merged[0].meta.deleted);
        assert_eq!(merged[0].unlocked_at.as_millis(), 1_000);
        let (reverse, _) = merge_unlocks(&[dead], &[live]);
        assert!(!reverse[0].meta.deleted, "direction must not matter");
    }

    #[test]
    fn merge_keeps_slugs_only_one_side_had() {
        let local = vec![unlock("first-session", DEV_A, 1_000, 1_000)];
        let remote = vec![unlock("first-finish", DEV_B, 2_000, 2_000)];
        let (merged, conflicts) = merge_unlocks(&local, &remote);
        assert_eq!(merged.len(), 2);
        assert!(conflicts.is_empty(), "no overlap means no conflict");
    }

    #[test]
    fn merge_is_symmetric_and_convergent() {
        let local = vec![
            unlock("streak-3", DEV_A, 1_000, 5_000),
            unlock("day-2h", DEV_A, 3_000, 3_000),
            unlock("first-note", DEV_A, 2_000, 2_000),
        ];
        let remote = vec![
            unlock("streak-3", DEV_B, 2_000, 1_000),
            unlock("day-2h", DEV_B, 3_000, 4_000),
            unlock("first-finish", DEV_B, 9_000, 9_000),
        ];
        let (from_local, _) = merge_unlocks(&local, &remote);
        let (from_remote, _) = merge_unlocks(&remote, &local);
        let mut a: Vec<(String, i64, i64)> = from_local
            .iter()
            .map(|row| {
                (
                    row.achievement_id.clone(),
                    row.unlocked_at.as_millis(),
                    row.meta.updated_at.as_millis(),
                )
            })
            .collect();
        let mut b: Vec<(String, i64, i64)> = from_remote
            .iter()
            .map(|row| {
                (
                    row.achievement_id.clone(),
                    row.unlocked_at.as_millis(),
                    row.meta.updated_at.as_millis(),
                )
            })
            .collect();
        a.sort();
        b.sort();
        assert_eq!(a, b, "both devices must land on the same unlocks");
        assert_eq!(a.len(), 4);
    }

    #[test]
    fn re_merging_an_already_merged_set_is_a_quiet_no_op() {
        let local = vec![unlock("streak-3", DEV_A, 2_000, 2_000)];
        let remote = vec![unlock("streak-3", DEV_B, 1_000, 1_000)];
        let (once, _) = merge_unlocks(&local, &remote);
        let (twice, conflicts) = merge_unlocks(&once, &remote);
        assert_eq!(once, twice);
        assert!(conflicts.is_empty(), "re-merging must not log conflicts");
    }

    #[test]
    fn identical_unlocks_do_not_produce_conflict_noise() {
        let row = unlock("first-session", DEV_A, 1_000, 1_000);
        let (_, conflicts) = merge_unlocks(std::slice::from_ref(&row), std::slice::from_ref(&row));
        assert!(conflicts.is_empty());
    }
}
