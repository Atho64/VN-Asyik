//! Mapping between VNDB list labels and local statuses, and conflict detection.
//!
//! Everything here is pure so the rules can be tested exhaustively. The two
//! behaviours that matter most:
//!
//! * **Nothing is overwritten silently.** When local and remote agree, sync is a
//!   no-op. When they disagree, a [`Conflict`] is produced and the user decides,
//!   unless they have explicitly picked a default policy.
//! * **Label sets are respected.** VNDB lets an entry carry several status labels
//!   at once (the website only prevents it in its own UI). Reading such an entry
//!   is resolved by a fixed precedence rather than by whichever label happens to
//!   come first in the array.

use serde::{Deserialize, Serialize};

use crate::vndb::ulist::UlistEntry;
use vn_core::model::GameStatus;

/// Well-known label ids documented by VNDB: ids below 10 are pre-defined.
pub mod label_id {
    /// "No label".
    pub const NONE: i64 = 0;
    /// "Playing".
    pub const PLAYING: i64 = 1;
    /// "Finished".
    pub const FINISHED: i64 = 2;
    /// "Stalled".
    pub const STALLED: i64 = 3;
    /// "Dropped".
    pub const DROPPED: i64 = 4;
    /// "Wishlist".
    pub const WISHLIST: i64 = 5;
    /// "Blacklist".
    pub const BLACKLIST: i64 = 6;
    /// "Voted", virtual: added and removed automatically with a vote.
    pub const VOTED: i64 = 7;
}

/// User-configurable mapping from local status to VNDB label ids.
///
/// The defaults mirror the website's own meaning. `wishlist` maps to the Wishlist
/// label; "on hold" maps to Stalled, which is what the website calls it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabelMapping {
    /// Label used for a game the user is reading.
    pub playing: i64,
    /// Label used for a completed game.
    pub completed: i64,
    /// Label used for a game on hold.
    pub on_hold: i64,
    /// Label used for a dropped game.
    pub dropped: i64,
    /// Label used for a wishlisted game.
    pub wishlist: i64,
}

impl Default for LabelMapping {
    fn default() -> Self {
        Self {
            playing: label_id::PLAYING,
            completed: label_id::FINISHED,
            on_hold: label_id::STALLED,
            dropped: label_id::DROPPED,
            wishlist: label_id::WISHLIST,
        }
    }
}

/// The status labels, in the precedence order used when several are present.
///
/// Order chosen deliberately: a finished game that still carries "Playing" is
/// finished; a dropped game that carries "Stalled" is dropped. Only the
/// wishlist stays lowest, because it is the least informative.
pub const STATUS_PRECEDENCE: [GameStatus; 5] = [
    GameStatus::Completed,
    GameStatus::Dropped,
    GameStatus::OnHold,
    GameStatus::Playing,
    GameStatus::Wishlist,
];

impl LabelMapping {
    /// Which label id represents `status`.
    #[must_use]
    pub const fn label_for(&self, status: GameStatus) -> i64 {
        match status {
            GameStatus::Playing => self.playing,
            GameStatus::Completed => self.completed,
            GameStatus::OnHold => self.on_hold,
            GameStatus::Dropped => self.dropped,
            GameStatus::Wishlist => self.wishlist,
        }
    }

    /// Which status a label id represents, if any.
    #[must_use]
    pub fn status_for(&self, label: i64) -> Option<GameStatus> {
        if label == self.playing {
            Some(GameStatus::Playing)
        } else if label == self.completed {
            Some(GameStatus::Completed)
        } else if label == self.on_hold {
            Some(GameStatus::OnHold)
        } else if label == self.dropped {
            Some(GameStatus::Dropped)
        } else if label == self.wishlist {
            Some(GameStatus::Wishlist)
        } else {
            None
        }
    }

    /// Resolve an entry's labels to one status by precedence.
    ///
    /// Returns `None` when the entry carries no status label at all (for example
    /// an entry that only has a vote), which must not be read as a status.
    #[must_use]
    pub fn status_from_labels(&self, labels: &[i64]) -> Option<GameStatus> {
        STATUS_PRECEDENCE
            .iter()
            .copied()
            .find(|status| labels.contains(&self.label_for(*status)))
    }

    /// The labels to set and unset to move from `current` to `target`.
    ///
    /// Uses set/unset rather than replacing the whole label array, so private and
    /// custom labels on the entry are preserved.
    #[must_use]
    pub fn label_edits(&self, current: &[i64], target: GameStatus) -> (Vec<i64>, Vec<i64>) {
        let wanted = self.label_for(target);
        let mut set = Vec::new();
        if !current.contains(&wanted) {
            set.push(wanted);
        }
        // The website clears the other status labels when one is chosen; the API
        // does not, so push does it explicitly to match website behaviour.
        let unset: Vec<i64> = STATUS_PRECEDENCE
            .iter()
            .map(|status| self.label_for(*status))
            .filter(|label| *label != wanted && current.contains(label))
            .collect();
        (set, unset)
    }

    /// True when the mapping sends two statuses to the same label, which would
    /// make a round trip lossy. The UI refuses to save such a mapping.
    #[must_use]
    pub fn collides(&self) -> Option<(GameStatus, GameStatus)> {
        let entries = [
            (GameStatus::Playing, self.playing),
            (GameStatus::Completed, self.completed),
            (GameStatus::OnHold, self.on_hold),
            (GameStatus::Dropped, self.dropped),
            (GameStatus::Wishlist, self.wishlist),
        ];
        for (index, (status_a, label_a)) in entries.iter().enumerate() {
            for (status_b, label_b) in entries.iter().skip(index + 1) {
                if label_a == label_b {
                    return Some((*status_a, *status_b));
                }
            }
        }
        None
    }
}

/// VNDB vote is 10..=100 decivotes, which is the app's scale exactly.
///
/// This is deliberately a clamp and not a rescale: both sides count tenths, so
/// no information is lost in either direction. It previously divided by ten to
/// reach whole stars, which turned a VNDB vote of 73 into 7 and threw away the
/// part the user had actually chosen.
#[must_use]
pub fn vote_to_rating(vote: Option<u32>) -> Option<u8> {
    vote.map(|value| u8::try_from(value.clamp(10, 100)).unwrap_or(10))
}

/// Convert a local rating to a VNDB vote.
///
/// A missing rating stays missing rather than becoming a 0 vote, which VNDB
/// would reject anyway since its floor is 10.
#[must_use]
pub fn rating_to_vote(rating: Option<u8>) -> Option<u32> {
    rating.map(|value| u32::from(value.clamp(10, 100)))
}

/// Turn a `YYYY-MM-DD` date into UTC milliseconds at midnight.
///
/// Returns `None` for an unparseable or out-of-range date rather than guessing.
#[must_use]
pub fn date_to_ms(date: Option<&str>) -> Option<i64> {
    let text = date?;
    let mut parts = text.split('-');
    let year: i32 = parts.next()?.parse().ok()?;
    let month: u32 = parts.next()?.parse().ok()?;
    let day: u32 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let date = time::Date::from_calendar_date(
        year,
        time::Month::try_from(u8::try_from(month).ok()?).ok()?,
        u8::try_from(day).ok()?,
    )
    .ok()?;
    let datetime = date.midnight().assume_utc();
    i64::try_from(datetime.unix_timestamp_nanos() / 1_000_000).ok()
}

/// Milliseconds to a `YYYY-MM-DD` date string.
#[must_use]
pub fn ms_to_date(ms: Option<i64>) -> Option<String> {
    let ms = ms?;
    let nanos = i128::from(ms) * 1_000_000;
    let datetime = time::OffsetDateTime::from_unix_timestamp_nanos(nanos).ok()?;
    let date = datetime.date();
    Some(format!(
        "{:04}-{:02}-{:02}",
        date.year(),
        u8::from(date.month()),
        date.day()
    ))
}

/// A field that differs between local and remote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Field {
    /// Reading status.
    Status,
    /// Star rating / VNDB vote.
    Rating,
    /// Free-form notes.
    Notes,
    /// Start date.
    Started,
    /// Finish date.
    Finished,
}

impl Field {
    /// Stable key used by the UI and the resolution map.
    #[must_use]
    pub const fn key(&self) -> &'static str {
        match self {
            Self::Status => "status",
            Self::Rating => "rating",
            Self::Notes => "notes",
            Self::Started => "started",
            Self::Finished => "finished",
        }
    }
}

/// One field conflict between the local row and the VNDB entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Conflict {
    /// Which field differs.
    pub field: Field,
    /// The local value, already stringified for display.
    pub local: Option<String>,
    /// The remote value, already stringified for display.
    pub remote: Option<String>,
}

/// The local side of a comparison.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LocalState {
    /// Local status.
    pub status: Option<GameStatus>,
    /// Local rating in decivotes, 10..=100.
    pub rating: Option<u8>,
    /// Local notes.
    pub notes: Option<String>,
    /// Local start date, UTC ms.
    pub started_at: Option<i64>,
    /// Local finish date, UTC ms.
    pub finished_at: Option<i64>,
    /// Last local modification, UTC ms.
    pub updated_at_ms: i64,
}

/// The remote side of a comparison.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RemoteState {
    /// Remote status derived from labels.
    pub status: Option<GameStatus>,
    /// Remote rating derived from the vote.
    pub rating: Option<u8>,
    /// Remote notes.
    pub notes: Option<String>,
    /// Remote start date, UTC ms.
    pub started_at: Option<i64>,
    /// Remote finish date, UTC ms.
    pub finished_at: Option<i64>,
    /// Last remote modification, UTC ms.
    pub lastmod_ms: i64,
}

impl RemoteState {
    /// Build from a raw list entry using the configured mapping.
    #[must_use]
    pub fn from_entry(entry: &UlistEntry, mapping: &LabelMapping) -> Self {
        Self {
            status: mapping.status_from_labels(&entry.label_ids()),
            rating: vote_to_rating(entry.vote),
            notes: entry.notes.clone().filter(|text| !text.trim().is_empty()),
            started_at: date_to_ms(entry.started.as_deref()),
            finished_at: date_to_ms(entry.finished.as_deref()),
            lastmod_ms: entry.lastmod_ms().unwrap_or(0),
        }
    }
}

/// How a conflict should be resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Resolution {
    /// Keep the local value; push it to VNDB.
    KeepLocal,
    /// Take the VNDB value; overwrite the local row.
    KeepRemote,
    /// Leave both sides untouched and skip this field.
    Skip,
}

/// The default policy applied when the user does not want to review each field.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictPolicy {
    /// Ask the user for every conflict.
    #[default]
    Ask,
    /// Local always wins.
    PreferLocal,
    /// VNDB always wins.
    PreferRemote,
    /// Newest change wins, by timestamp.
    Newest,
}

/// Compare the two sides and list every field that disagrees.
///
/// A field only conflicts when both sides actually have a value that differs, or
/// when one side has a value and the other explicitly has none **and** the other
/// side was modified more recently. That avoids treating "VNDB never had notes"
/// as a conflict every single sync.
#[must_use]
pub fn diff(local: &LocalState, remote: &RemoteState) -> Vec<Conflict> {
    let mut conflicts = Vec::new();

    if let Some(local_status) = local.status
        && let Some(remote_status) = remote.status
        && local_status != remote_status
    {
        conflicts.push(Conflict {
            field: Field::Status,
            local: Some(local_status.as_str().to_owned()),
            remote: Some(remote_status.as_str().to_owned()),
        });
    }

    if local.rating != remote.rating && (local.rating.is_some() || remote.rating.is_some()) {
        conflicts.push(Conflict {
            field: Field::Rating,
            local: local.rating.map(|value| value.to_string()),
            remote: remote.rating.map(|value| value.to_string()),
        });
    }

    let local_notes = local.notes.as_deref().unwrap_or("").trim();
    let remote_notes = remote.notes.as_deref().unwrap_or("").trim();
    if local_notes != remote_notes && (!local_notes.is_empty() || !remote_notes.is_empty()) {
        conflicts.push(Conflict {
            field: Field::Notes,
            local: local.notes.clone(),
            remote: remote.notes.clone(),
        });
    }

    if local.started_at != remote.started_at
        && (local.started_at.is_some() || remote.started_at.is_some())
    {
        conflicts.push(Conflict {
            field: Field::Started,
            local: ms_to_date(local.started_at),
            remote: ms_to_date(remote.started_at),
        });
    }

    if local.finished_at != remote.finished_at
        && (local.finished_at.is_some() || remote.finished_at.is_some())
    {
        conflicts.push(Conflict {
            field: Field::Finished,
            local: ms_to_date(local.finished_at),
            remote: ms_to_date(remote.finished_at),
        });
    }

    conflicts
}

/// Apply a policy to decide a conflict, falling back to [`Resolution::Skip`] when
/// the policy cannot decide.
#[must_use]
pub fn resolve(_conflict: &Conflict, policy: ConflictPolicy, local_newer: bool) -> Resolution {
    match policy {
        ConflictPolicy::Ask => Resolution::Skip,
        ConflictPolicy::PreferLocal => Resolution::KeepLocal,
        ConflictPolicy::PreferRemote => Resolution::KeepRemote,
        ConflictPolicy::Newest => {
            if local_newer {
                Resolution::KeepLocal
            } else {
                Resolution::KeepRemote
            }
        }
    }
}

/// Whether the local row was modified more recently than the remote entry.
#[must_use]
pub fn local_is_newer(local: &LocalState, remote: &RemoteState) -> bool {
    local.updated_at_ms > remote.lastmod_ms
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mapping() -> LabelMapping {
        LabelMapping::default()
    }

    fn entry(labels: &[i64], vote: Option<u32>) -> UlistEntry {
        UlistEntry {
            id: "v17".to_owned(),
            labels: labels
                .iter()
                .map(|id| crate::vndb::ulist::UlistLabel {
                    id: *id,
                    label: None,
                })
                .collect(),
            vote,
            lastmod: Some(1_600_000_000),
            ..UlistEntry::default()
        }
    }

    #[test]
    fn field_keys_survive_json_as_snake_case() {
        // The UI keys its decision map by `field.key()` and matches values that
        // came through JSON. If serialisation changes the casing, every decision
        // silently stops matching, so this is pinned by a test.
        let conflict = Conflict {
            field: Field::Status,
            local: Some("wishlist".to_owned()),
            remote: Some("playing".to_owned()),
        };
        let json = serde_json::to_string(&conflict).expect("serialize");
        assert!(json.contains(r#""field":"status""#), "got {json}");

        for field in [
            Field::Status,
            Field::Rating,
            Field::Notes,
            Field::Started,
            Field::Finished,
        ] {
            let encoded = serde_json::to_string(&field).expect("serialize");
            assert_eq!(encoded, format!("\"{}\"", field.key()));
        }
    }

    #[test]
    fn default_mapping_uses_the_documented_label_ids() {
        let m = mapping();
        assert_eq!(m.playing, 1);
        assert_eq!(m.completed, 2);
        assert_eq!(m.on_hold, 3);
        assert_eq!(m.dropped, 4);
        assert_eq!(m.wishlist, 5);
        assert_eq!(m.collides(), None);
    }

    #[test]
    fn mapping_round_trips_every_status() {
        let m = mapping();
        for status in STATUS_PRECEDENCE {
            assert_eq!(m.status_for(m.label_for(status)), Some(status));
        }
    }

    #[test]
    fn unknown_labels_are_ignored() {
        let m = mapping();
        assert_eq!(m.status_from_labels(&[7, 42]), None);
        assert_eq!(m.status_for(42), None);
    }

    #[test]
    fn several_status_labels_resolve_by_precedence() {
        let m = mapping();
        // Finished beats Playing beats Wishlist.
        assert_eq!(
            m.status_from_labels(&[1, 2, 5]),
            Some(GameStatus::Completed)
        );
        assert_eq!(m.status_from_labels(&[1, 5]), Some(GameStatus::Playing));
        assert_eq!(m.status_from_labels(&[5]), Some(GameStatus::Wishlist));
        assert_eq!(
            m.status_from_labels(&[3, 4]),
            Some(GameStatus::Dropped),
            "dropped outranks stalled"
        );
    }

    #[test]
    fn label_edits_set_the_target_and_clear_siblings() {
        let m = mapping();
        let (set, unset) = m.label_edits(&[1], GameStatus::Completed);
        assert_eq!(set, vec![2]);
        assert_eq!(unset, vec![1]);
    }

    #[test]
    fn label_edits_are_empty_when_already_correct() {
        let m = mapping();
        let (set, unset) = m.label_edits(&[2], GameStatus::Completed);
        assert!(set.is_empty());
        assert!(unset.is_empty());
    }

    #[test]
    fn label_edits_leave_custom_labels_alone() {
        let m = mapping();
        // 99 is a user label, 1 is Playing; moving to Finished must not touch 99.
        let (set, unset) = m.label_edits(&[1, 99, 7], GameStatus::Completed);
        assert_eq!(set, vec![2]);
        assert_eq!(unset, vec![1]);
        assert!(!unset.contains(&99));
        assert!(!unset.contains(&7));
    }

    #[test]
    fn colliding_mapping_is_detected() {
        let mut m = mapping();
        m.completed = m.playing;
        let pair = m.collides().expect("collision");
        assert!(
            pair == (GameStatus::Playing, GameStatus::Completed)
                || pair == (GameStatus::Completed, GameStatus::Playing)
        );
    }

    #[test]
    fn a_vote_survives_the_round_trip_unchanged() {
        // Both scales count tenths, so 73 must come back as 73. The old mapping
        // divided by ten and this assertion would have read Some(7).
        assert_eq!(vote_to_rating(None), None);
        assert_eq!(vote_to_rating(Some(100)), Some(100));
        assert_eq!(vote_to_rating(Some(10)), Some(10));
        assert_eq!(vote_to_rating(Some(75)), Some(75));
        assert_eq!(vote_to_rating(Some(73)), Some(73));
        assert_eq!(rating_to_vote(None), None);
        assert_eq!(rating_to_vote(Some(75)), Some(75));
        for decivote in 10..=100u32 {
            assert_eq!(
                rating_to_vote(vote_to_rating(Some(decivote))),
                Some(decivote),
                "{decivote} must survive the round trip"
            );
        }
    }

    #[test]
    fn an_out_of_range_vote_is_clamped_not_rejected() {
        // VNDB should never send these, but silently storing a 0 would make an
        // unrated game look unrated for a different reason than it is.
        assert_eq!(vote_to_rating(Some(0)), Some(10));
        assert_eq!(vote_to_rating(Some(500)), Some(100));
        assert_eq!(rating_to_vote(Some(0)), Some(10));
        assert_eq!(rating_to_vote(Some(200)), Some(100));
    }

    #[test]
    fn dates_convert_both_ways() {
        let ms = date_to_ms(Some("2020-09-01")).expect("date");
        assert_eq!(ms_to_date(Some(ms)).as_deref(), Some("2020-09-01"));
    }

    #[test]
    fn bad_dates_are_rejected_rather_than_guessed() {
        assert!(date_to_ms(Some("2020-13-01")).is_none());
        assert!(date_to_ms(Some("2020-02-31")).is_none());
        assert!(date_to_ms(Some("2020")).is_none());
        assert!(date_to_ms(Some("not a date")).is_none());
        assert!(date_to_ms(None).is_none());
    }

    #[test]
    fn remote_state_maps_an_entry() {
        let m = mapping();
        let mut e = entry(&[1, 7], Some(80));
        e.started = Some("2020-09-01".to_owned());
        e.notes = Some("hello".to_owned());
        let remote = RemoteState::from_entry(&e, &m);
        assert_eq!(remote.status, Some(GameStatus::Playing));
        assert_eq!(remote.rating, Some(80), "decivotes, not stars");
        assert_eq!(remote.notes.as_deref(), Some("hello"));
        assert_eq!(remote.started_at, date_to_ms(Some("2020-09-01")));
        assert_eq!(remote.lastmod_ms, 1_600_000_000_000);
    }

    #[test]
    fn blank_remote_notes_are_treated_as_absent() {
        let m = mapping();
        let mut e = entry(&[], None);
        e.notes = Some("   ".to_owned());
        assert!(RemoteState::from_entry(&e, &m).notes.is_none());
    }

    #[test]
    fn identical_sides_produce_no_conflicts() {
        let local = LocalState {
            status: Some(GameStatus::Playing),
            rating: Some(8),
            notes: Some("hello".to_owned()),
            started_at: date_to_ms(Some("2020-09-01")),
            finished_at: None,
            updated_at_ms: 1_000,
        };
        let remote = RemoteState {
            status: Some(GameStatus::Playing),
            rating: Some(8),
            notes: Some("hello".to_owned()),
            started_at: date_to_ms(Some("2020-09-01")),
            finished_at: None,
            lastmod_ms: 1_000,
        };
        assert!(diff(&local, &remote).is_empty());
    }

    #[test]
    fn each_diverging_field_is_reported() {
        let local = LocalState {
            status: Some(GameStatus::Playing),
            rating: Some(8),
            notes: Some("mine".to_owned()),
            started_at: None,
            finished_at: None,
            updated_at_ms: 2_000,
        };
        let remote = RemoteState {
            status: Some(GameStatus::Completed),
            rating: Some(5),
            notes: Some("theirs".to_owned()),
            started_at: date_to_ms(Some("2020-01-01")),
            finished_at: None,
            lastmod_ms: 1_000,
        };
        let conflicts = diff(&local, &remote);
        let fields: Vec<&str> = conflicts.iter().map(|c| c.field.key()).collect();
        assert!(fields.contains(&"status"));
        assert!(fields.contains(&"rating"));
        assert!(fields.contains(&"notes"));
        assert!(fields.contains(&"started"));
        assert!(!fields.contains(&"finished"));
    }

    #[test]
    fn a_missing_value_on_one_side_is_still_a_conflict_when_it_differs() {
        let local = LocalState {
            notes: Some("mine".to_owned()),
            updated_at_ms: 5_000,
            ..LocalState::default()
        };
        let remote = RemoteState {
            lastmod_ms: 1_000,
            ..RemoteState::default()
        };
        let conflicts = diff(&local, &remote);
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].field, Field::Notes);
        assert!(conflicts[0].remote.is_none());
    }

    #[test]
    fn both_sides_empty_is_not_a_conflict() {
        let local = LocalState {
            notes: Some("   ".to_owned()),
            ..LocalState::default()
        };
        let remote = RemoteState::default();
        assert!(diff(&local, &remote).is_empty());
    }

    #[test]
    fn policies_resolve_as_documented() {
        let conflict = Conflict {
            field: Field::Status,
            local: Some("playing".to_owned()),
            remote: Some("completed".to_owned()),
        };
        assert_eq!(
            resolve(&conflict, ConflictPolicy::Ask, true),
            Resolution::Skip,
            "Ask never decides on its own"
        );
        assert_eq!(
            resolve(&conflict, ConflictPolicy::PreferLocal, false),
            Resolution::KeepLocal
        );
        assert_eq!(
            resolve(&conflict, ConflictPolicy::PreferRemote, true),
            Resolution::KeepRemote
        );
        assert_eq!(
            resolve(&conflict, ConflictPolicy::Newest, true),
            Resolution::KeepLocal
        );
        assert_eq!(
            resolve(&conflict, ConflictPolicy::Newest, false),
            Resolution::KeepRemote
        );
    }

    #[test]
    fn default_policy_asks() {
        assert_eq!(ConflictPolicy::default(), ConflictPolicy::Ask);
    }

    #[test]
    fn newest_is_decided_by_timestamps() {
        let local = LocalState {
            updated_at_ms: 2_000,
            ..LocalState::default()
        };
        let remote = RemoteState {
            lastmod_ms: 1_000,
            ..RemoteState::default()
        };
        assert!(local_is_newer(&local, &remote));

        let older_local = LocalState {
            updated_at_ms: 500,
            ..LocalState::default()
        };
        assert!(!local_is_newer(&older_local, &remote));
    }
}
