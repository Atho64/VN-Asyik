//! Custom reading goals and validation rules.
//!
//! A `ReadingGoal` tracks either an automatic metric from the local library
//! (`PlaytimeMinutes`, `VnsCompleted`) over a calendar period, or a user-managed
//! numeric target (`Manual`) with an optional unit label.

use serde::{Deserialize, Serialize};

use crate::clock::Timestamp;
use crate::model::SyncMeta;
use crate::sync::MetaFields;

/// Maximum length for a goal title, in Unicode scalar values.
pub const MAX_GOAL_TITLE_CHARS: usize = 120;

/// Maximum length for a custom unit label, in Unicode scalar values.
pub const MAX_GOAL_UNIT_CHARS: usize = 32;

/// Upper bound for a goal target or manual progress value.
pub const MAX_GOAL_VALUE: i64 = 1_000_000;

/// What a reading goal measures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalKind {
    /// Counted playtime in minutes (e.g. 120 = 2 hours).
    PlaytimeMinutes,
    /// Visual novels completed in the period (derived via `finish_ts_sql`).
    VnsCompleted,
    /// User-updated numeric target with an optional unit label.
    Manual,
}

impl GoalKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PlaytimeMinutes => "playtime_minutes",
            Self::VnsCompleted => "vns_completed",
            Self::Manual => "manual",
        }
    }

    /// Parse a stored kind string.
    ///
    /// # Errors
    /// Returns an error string for any unrecognized value.
    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw {
            "playtime_minutes" => Ok(Self::PlaytimeMinutes),
            "vns_completed" => Ok(Self::VnsCompleted),
            "manual" => Ok(Self::Manual),
            _ => Err(format!("unknown goal kind: {raw}")),
        }
    }
}

/// Calendar window over which an automatic goal is evaluated (or displayed for manual goals).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalPeriod {
    Daily,
    Weekly,
    Monthly,
    Yearly,
    AllTime,
}

impl GoalPeriod {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Daily => "daily",
            Self::Weekly => "weekly",
            Self::Monthly => "monthly",
            Self::Yearly => "yearly",
            Self::AllTime => "all_time",
        }
    }

    /// Parse a stored period string.
    ///
    /// # Errors
    /// Returns an error string for any unrecognized value.
    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw {
            "daily" => Ok(Self::Daily),
            "weekly" => Ok(Self::Weekly),
            "monthly" => Ok(Self::Monthly),
            "yearly" => Ok(Self::Yearly),
            "all_time" => Ok(Self::AllTime),
            _ => Err(format!("unknown goal period: {raw}")),
        }
    }
}

/// A synced custom reading goal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadingGoal {
    pub meta: SyncMeta,
    pub title: String,
    pub kind: GoalKind,
    pub period: GoalPeriod,
    /// Positive target: minutes for `PlaytimeMinutes`, count for `VnsCompleted` / `Manual`.
    pub target_value: i64,
    /// User-entered progress for `Manual` goals (ignored for automatic goals).
    pub manual_progress: i64,
    /// Optional unit label for `Manual` goals (e.g. "route", "chapter", "VN").
    pub unit: Option<String>,
    pub pinned: bool,
    /// Immutable creation timestamp in UTC milliseconds.
    pub created_at: Timestamp,
}

impl MetaFields for ReadingGoal {
    fn meta(&self) -> &SyncMeta {
        &self.meta
    }
}

/// Normalize and validate a `ReadingGoal` before writing or importing.
///
/// # Errors
/// Returns an error string if the title is empty/too long, target is out
/// of bounds, manual progress is negative/too large, or unit is too long.
pub fn validate_reading_goal(goal: &mut ReadingGoal) -> Result<(), String> {
    let trimmed_title = goal.title.trim();
    if trimmed_title.is_empty() {
        return Err("goal title cannot be empty".to_owned());
    }
    if trimmed_title.chars().count() > MAX_GOAL_TITLE_CHARS {
        return Err("goal title is too long".to_owned());
    }
    goal.title = trimmed_title.to_owned();

    if goal.target_value <= 0 || goal.target_value > MAX_GOAL_VALUE {
        return Err("goal target must be between 1 and 1,000,000".to_owned());
    }

    if goal.manual_progress < 0 || goal.manual_progress > MAX_GOAL_VALUE {
        return Err("goal progress must be between 0 and 1,000,000".to_owned());
    }

    if let Some(raw_unit) = goal.unit.as_deref() {
        let trimmed = raw_unit.trim();
        if trimmed.is_empty() {
            goal.unit = None;
        } else if trimmed.chars().count() > MAX_GOAL_UNIT_CHARS {
            return Err("goal unit is too long".to_owned());
        } else {
            goal.unit = Some(trimmed.to_owned());
        }
    }

    if goal.kind != GoalKind::Manual {
        goal.manual_progress = 0;
        goal.unit = None;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::DeviceId;

    fn sample_goal(kind: GoalKind) -> ReadingGoal {
        let device = DeviceId::parse("a").unwrap();
        let ts = Timestamp::from_millis(1_700_000_000_000);
        ReadingGoal {
            meta: SyncMeta::new(&device, ts),
            title: "  10 VN tahun ini  ".to_owned(),
            kind,
            period: GoalPeriod::Yearly,
            target_value: 10,
            manual_progress: 3,
            unit: Some("  route  ".to_owned()),
            pinned: false,
            created_at: ts,
        }
    }

    #[test]
    fn validates_and_normalizes_manual_and_auto_goals() {
        let mut manual = sample_goal(GoalKind::Manual);
        validate_reading_goal(&mut manual).expect("valid manual goal");
        assert_eq!(manual.title, "10 VN tahun ini");
        assert_eq!(manual.unit.as_deref(), Some("route"));
        assert_eq!(manual.manual_progress, 3);

        let mut auto = sample_goal(GoalKind::VnsCompleted);
        validate_reading_goal(&mut auto).expect("valid auto goal");
        assert_eq!(auto.manual_progress, 0);
        assert_eq!(auto.unit, None);
    }

    #[test]
    fn rejects_empty_title_or_non_positive_target() {
        let mut empty = sample_goal(GoalKind::PlaytimeMinutes);
        empty.title = "   ".to_owned();
        assert!(validate_reading_goal(&mut empty).is_err());

        let mut zero = sample_goal(GoalKind::PlaytimeMinutes);
        zero.target_value = 0;
        assert!(validate_reading_goal(&mut zero).is_err());
    }

    #[test]
    fn kind_and_period_roundtrip() {
        for kind in [
            GoalKind::PlaytimeMinutes,
            GoalKind::VnsCompleted,
            GoalKind::Manual,
        ] {
            assert_eq!(GoalKind::parse(kind.as_str()).unwrap(), kind);
        }
        for period in [
            GoalPeriod::Daily,
            GoalPeriod::Weekly,
            GoalPeriod::Monthly,
            GoalPeriod::Yearly,
            GoalPeriod::AllTime,
        ] {
            assert_eq!(GoalPeriod::parse(period.as_str()).unwrap(), period);
        }
    }
}
