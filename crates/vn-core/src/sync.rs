//! Merging two devices' copies of the same rows.
//!
//! # Why this is a pure module
//!
//! Sync bugs are the ones that destroy data, and they are also the hardest to
//! reproduce: they need two devices, a clock skew, and a sequence of edits that
//! nobody planned. So the decision of "which copy wins" is separated from all
//! transport and storage, and tested exhaustively here with hand-built rows.
//!
//! # The rules
//!
//! Two rules apply to every synced table:
//!
//! 1. **Last write wins, by `updated_at`, with `device_id` as the tiebreak.**
//!    Wall clocks are not perfectly synchronised, so identical timestamps are a
//!    real case and must resolve deterministically on both devices rather than
//!    each keeping its own copy.
//! 2. **A delete beats an edit unless the edit is strictly newer.** Without this,
//!    deleting a game on one device and touching it on another resurrects it,
//!    which is the most confusing possible outcome: the row comes back on its
//!    own. Treating a tombstone as an ordinary value under rule 1 would already
//!    handle most cases, but making it explicit documents the intent and lets the
//!    conflict log name it.
//!
//! Sessions are separate: they are append-only playtime records, so they merge by
//! identity and are never overwritten (see [`merge_sessions`]).

use crate::model::SyncMeta;

/// Anything that carries sync metadata.
///
/// Implemented for the four synced tables rather than derived from a field, so a
/// new table has to opt in explicitly and cannot silently arrive in a merge with
/// metadata the merge does not know how to read.
pub trait MetaFields {
    /// Row identity and sync metadata.
    fn meta(&self) -> &SyncMeta;
}

impl MetaFields for crate::model::Game {
    fn meta(&self) -> &SyncMeta {
        &self.meta
    }
}

impl MetaFields for crate::model::Chapter {
    fn meta(&self) -> &SyncMeta {
        &self.meta
    }
}

impl MetaFields for crate::model::PlaySession {
    fn meta(&self) -> &SyncMeta {
        &self.meta
    }
}

impl MetaFields for crate::model::Ending {
    fn meta(&self) -> &SyncMeta {
        &self.meta
    }
}

impl MetaFields for crate::model::Route {
    fn meta(&self) -> &SyncMeta {
        &self.meta
    }
}

/// Which side supplied the value that won.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// The copy already on this device.
    Local,
    /// The copy that arrived from elsewhere.
    Remote,
}

impl Side {
    /// The opposite side.
    #[must_use]
    pub const fn flip(self) -> Self {
        match self {
            Side::Local => Side::Remote,
            Side::Remote => Side::Local,
        }
    }
}

/// Why a merge picked the side it did.
///
/// Recorded so the conflict log can explain an overwrite instead of just
/// reporting that one happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// Only one side had the row.
    OnlyOneSide,
    /// Both sides were identical, so nothing moved.
    Identical,
    /// The winning side had a later `updated_at`.
    Newer,
    /// Timestamps matched; the higher `device_id` won to break the tie.
    TieBreak,
    /// One side deleted the row and the other only edited it.
    DeleteWins,
}

/// The outcome of merging one row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decision {
    /// Which side's value should be stored, or `None` when the two are equal and
    /// no write is needed at all.
    pub winner: Option<Side>,
    /// Why.
    pub reason: Reason,
}

impl Decision {
    /// Whether the local row has to be rewritten.
    #[must_use]
    pub const fn needs_write(&self) -> bool {
        matches!(self.winner, Some(Side::Remote))
    }
}

/// Compare two revisions of the same row and decide which to keep.
///
/// Both arguments must describe the same row identity; comparing different ids is
/// meaningless and is not defended against here because the caller indexes by id.
#[must_use]
pub fn decide<T: MetaFields>(local: &T, remote: &T) -> Decision {
    let local_meta = local.meta();
    let remote_meta = remote.meta();

    let forward = decide_one_way(local_meta, remote_meta);
    let backward = decide_one_way(remote_meta, local_meta);

    // A broken clock can make each side look newer than the other (equal
    // timestamps but different device ids is the normal case; genuinely
    // contradictory ones are not). Falling back to a deterministic comparison
    // keeps two devices converging on the same answer.
    forward.or(backward).unwrap_or(Decision {
        winner: None,
        reason: Reason::Identical,
    })
}

/// One-directional comparison, returning a decision when one side clearly wins.
fn decide_one_way(local: &SyncMeta, remote: &SyncMeta) -> Option<Decision> {
    if local.id != remote.id {
        return None;
    }
    // Deletion outranks an edit at the same or older timestamp. An edit that is
    // strictly newer wins, because the user acted afterwards.
    if local.deleted && !remote.deleted {
        return Some(if local.updated_at >= remote.updated_at {
            Decision {
                winner: Some(Side::Local),
                reason: Reason::DeleteWins,
            }
        } else {
            Decision {
                winner: Some(Side::Remote),
                reason: Reason::Newer,
            }
        });
    }
    if remote.deleted && !local.deleted {
        return Some(if remote.updated_at >= local.updated_at {
            Decision {
                winner: Some(Side::Remote),
                reason: Reason::DeleteWins,
            }
        } else {
            Decision {
                winner: Some(Side::Local),
                reason: Reason::Newer,
            }
        });
    }
    if remote.updated_at > local.updated_at {
        return Some(Decision {
            winner: Some(Side::Remote),
            reason: Reason::Newer,
        });
    }
    if local.updated_at > remote.updated_at {
        return Some(Decision {
            winner: Some(Side::Local),
            reason: Reason::Newer,
        });
    }
    // Equal timestamps. This happens whenever two devices write in the same
    // millisecond, and must not be resolved by "keep mine" on both sides.
    let local_tag = local.device_id_device.device().as_str();
    let remote_tag = remote.device_id_device.device().as_str();
    match remote_tag.cmp(local_tag) {
        std::cmp::Ordering::Greater => Some(Decision {
            winner: Some(Side::Remote),
            reason: Reason::TieBreak,
        }),
        std::cmp::Ordering::Less => Some(Decision {
            winner: Some(Side::Local),
            reason: Reason::TieBreak,
        }),
        // Same device id and same timestamp: the rows are the same revision. Two
        // installs sharing a device id is a configuration error, but it must not
        // produce a ping-pong of writes.
        std::cmp::Ordering::Equal => None,
    }
}

/// A conflict worth telling the user about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    /// Row id.
    pub id: String,
    /// Table the row belongs to, e.g. `games`.
    pub table: &'static str,
    /// Which copy was kept.
    pub kept: Side,
    /// Why.
    pub reason: Reason,
    /// The `updated_at` of the copy that lost, so the log shows what was dropped.
    pub discarded_updated_at_ms: i64,
}

/// Merge one table's rows by identity.
///
/// Returns the winning rows in the order the local side had them, followed by
/// rows only the remote side had. Both devices must call this with their own
/// collection as `local` to converge, which is the property the tests check.
#[must_use]
pub fn merge_rows<T: MetaFields + Clone>(
    table: &'static str,
    local: &[T],
    remote: &[T],
) -> (Vec<T>, Vec<Conflict>) {
    let mut out: Vec<T> = Vec::with_capacity(local.len() + remote.len());
    let mut conflicts: Vec<Conflict> = Vec::new();
    let mut seen: std::collections::HashSet<String> =
        std::collections::HashSet::with_capacity(local.len());

    // Walk local rows in order, replacing where remote wins.
    for row in local {
        let id = row.meta().id.to_string();
        seen.insert(id.clone());
        match remote
            .iter()
            .find(|candidate| candidate.meta().id.to_string() == id)
        {
            None => out.push(row.clone()),
            Some(theirs) => {
                let decision = decide(row, theirs);
                match decision.winner {
                    Some(Side::Remote) => {
                        out.push(theirs.clone());
                        conflicts.push(Conflict {
                            id,
                            table,
                            kept: Side::Remote,
                            reason: decision.reason,
                            discarded_updated_at_ms: row.meta().updated_at.as_millis(),
                        });
                    }
                    Some(Side::Local) => {
                        out.push(row.clone());
                        // Only log when the two rows actually differed; logging
                        // identical revisions would bury the real conflicts.
                        if decision.reason != Reason::Identical {
                            conflicts.push(Conflict {
                                id,
                                table,
                                kept: Side::Local,
                                reason: decision.reason,
                                discarded_updated_at_ms: theirs.meta().updated_at.as_millis(),
                            });
                        }
                    }
                    None => out.push(row.clone()),
                }
            }
        }
    }

    // Rows only the remote side has.
    for row in remote {
        let id = row.meta().id.to_string();
        if !seen.contains(&id) {
            out.push(row.clone());
        }
    }

    (out, conflicts)
}

/// Merge append-only sessions.
///
/// Sessions are playtime records, so a session that exists on either side is
/// kept as-is and never overwritten: overwriting one would silently rewrite
/// history. A correction is a new row, not an edit of an old one.
///
/// Returns the union, local first, and the ids that were only on the remote side.
#[must_use]
pub fn merge_sessions<T: MetaFields + Clone>(local: &[T], remote: &[T]) -> (Vec<T>, Vec<String>) {
    let mut out: Vec<T> = local.to_vec();
    let existing: std::collections::HashSet<String> =
        local.iter().map(|row| row.meta().id.to_string()).collect();

    let mut added = Vec::new();
    for row in remote {
        let id = row.meta().id.to_string();
        if !existing.contains(&id) {
            out.push(row.clone());
            added.push(id);
        }
    }
    (out, added)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::Timestamp;
    use crate::ids::{DeviceId, EntityId};
    use crate::model::SyncMeta;

    /// Minimal row type for merge tests.
    #[derive(Debug, Clone, PartialEq, Eq)]
    struct Row {
        id: String,
        meta: SyncMeta,
        value: &'static str,
    }

    impl Row {
        fn new(id: &str, device: &str, updated_at: i64, value: &'static str) -> Self {
            Self::with_deleted(id, device, updated_at, value, false)
        }

        fn deleted_row(id: &str, device: &str, updated_at: i64) -> Self {
            Self::with_deleted(id, device, updated_at, "gone", true)
        }

        fn with_deleted(
            id: &str,
            device: &str,
            updated_at: i64,
            value: &'static str,
            deleted: bool,
        ) -> Self {
            Self {
                id: id.to_owned(),
                meta: SyncMeta {
                    id: EntityId::parse(id).expect("id"),
                    updated_at: Timestamp::from_millis(updated_at),
                    device_id_device: crate::model::DeviceTag::new(
                        &DeviceId::parse(device).expect("device"),
                    ),
                    deleted,
                },
                value,
            }
        }
    }

    impl MetaFields for Row {
        fn meta(&self) -> &SyncMeta {
            &self.meta
        }
    }

    // Fixed UUIDs so the tests are readable and deterministic.
    const ROW: &str = "01a151d6-eca1-76f0-961d-a7a586661ecd";
    const ROW2: &str = "01a151d6-eca1-76f0-961d-a7a586661ece";
    const DEV_A: &str = "01a00000-0000-7000-8000-00000000000a";
    const DEV_B: &str = "01a00000-0000-7000-8000-00000000000b";

    #[test]
    fn the_newer_update_wins() {
        let local = Row::new(ROW, DEV_A, 1_000, "old");
        let remote = Row::new(ROW, DEV_B, 2_000, "new");
        let decision = decide(&local, &remote);
        assert_eq!(decision.winner, Some(Side::Remote));
        assert_eq!(decision.reason, Reason::Newer);
        assert!(decision.needs_write());
        // The same comparison from the other device's point of view.
        let reverse = decide(&remote, &local);
        assert_eq!(reverse.winner, Some(Side::Local));
    }

    #[test]
    fn an_older_remote_does_not_overwrite_local() {
        let local = Row::new(ROW, DEV_A, 5_000, "new");
        let remote = Row::new(ROW, DEV_B, 1_000, "old");
        let decision = decide(&local, &remote);
        assert_eq!(decision.winner, Some(Side::Local));
        assert!(!decision.needs_write());
    }

    #[test]
    fn equal_timestamps_break_the_tie_the_same_way_on_both_devices() {
        // Wall clocks are not synchronised, so this is a real case. If each device
        // kept its own copy, they would never converge.
        let a = Row::new(ROW, DEV_A, 1_000, "a");
        let b = Row::new(ROW, DEV_B, 1_000, "b");

        let from_a = decide(&a, &b);
        let from_b = decide(&b, &a);
        assert_eq!(from_a.reason, Reason::TieBreak);
        assert_eq!(from_b.reason, Reason::TieBreak);

        // Both must name the same winner: the higher device id.
        let winner_from_a = match from_a.winner {
            Some(Side::Local) => &a,
            Some(Side::Remote) => &b,
            None => panic!("expected a winner"),
        };
        let winner_from_b = match from_b.winner {
            Some(Side::Local) => &b,
            Some(Side::Remote) => &a,
            None => panic!("expected a winner"),
        };
        assert_eq!(winner_from_a, winner_from_b, "devices must converge");
        assert_eq!(winner_from_a.value, "b", "higher device id wins the tie");
    }

    #[test]
    fn a_delete_beats_an_edit_at_the_same_time() {
        // Resurrecting a deleted game on its own is the most confusing outcome a
        // sync can produce.
        let local = Row::deleted_row(ROW, DEV_A, 1_000);
        let remote = Row::new(ROW, DEV_B, 1_000, "edited");
        let decision = decide(&local, &remote);
        assert_eq!(decision.winner, Some(Side::Local));
        assert_eq!(decision.reason, Reason::DeleteWins);
    }

    #[test]
    fn a_delete_beats_an_older_edit() {
        let local = Row::deleted_row(ROW, DEV_A, 2_000);
        let remote = Row::new(ROW, DEV_B, 1_000, "edited");
        assert_eq!(decide(&local, &remote).reason, Reason::DeleteWins);
    }

    #[test]
    fn a_strictly_newer_edit_beats_a_delete() {
        // The user edited after deleting, so they meant to keep it.
        let local = Row::deleted_row(ROW, DEV_A, 1_000);
        let remote = Row::new(ROW, DEV_B, 5_000, "edited");
        let decision = decide(&local, &remote);
        assert_eq!(decision.winner, Some(Side::Remote));
        assert_eq!(decision.reason, Reason::Newer);
    }

    #[test]
    fn identical_rows_need_no_write() {
        let local = Row::new(ROW, DEV_A, 1_000, "same");
        let remote = Row::new(ROW, DEV_A, 1_000, "same");
        let decision = decide(&local, &remote);
        assert_eq!(decision.winner, None);
        assert_eq!(decision.reason, Reason::Identical);
        assert!(!decision.needs_write());
    }

    #[test]
    fn merging_keeps_rows_only_one_side_had() {
        let local = vec![Row::new(ROW, DEV_A, 1_000, "local")];
        let remote = vec![Row::new(ROW2, DEV_B, 1_000, "remote")];
        let (merged, conflicts) = merge_rows("games", &local, &remote);
        assert_eq!(merged.len(), 2);
        assert!(conflicts.is_empty(), "no overlap means no conflict");
    }

    #[test]
    fn merging_is_symmetric_which_is_what_makes_devices_converge() {
        // The property that matters: whichever device runs the merge, both end up
        // with the same set of rows.
        let local = vec![
            Row::new(ROW, DEV_A, 1_000, "a"),
            Row::new(ROW2, DEV_A, 3_000, "keep"),
        ];
        let remote = vec![
            Row::new(ROW, DEV_B, 2_000, "b"),
            Row::new(ROW2, DEV_B, 1_000, "older"),
        ];

        let (from_local, _) = merge_rows("games", &local, &remote);
        let (from_remote, _) = merge_rows("games", &remote, &local);

        let mut a: Vec<(String, &'static str)> = from_local
            .iter()
            .map(|row| (row.id.clone(), row.value))
            .collect();
        let mut b: Vec<(String, &'static str)> = from_remote
            .iter()
            .map(|row| (row.id.clone(), row.value))
            .collect();
        a.sort();
        b.sort();
        assert_eq!(a, b, "both devices must land on the same rows");
    }

    #[test]
    fn a_losing_overwrite_is_recorded_as_a_conflict() {
        let local = vec![Row::new(ROW, DEV_A, 1_000, "old")];
        let remote = vec![Row::new(ROW, DEV_B, 2_000, "new")];
        let (merged, conflicts) = merge_rows("games", &local, &remote);
        assert_eq!(merged[0].value, "new");
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].kept, Side::Remote);
        assert_eq!(conflicts[0].reason, Reason::Newer);
        assert_eq!(conflicts[0].discarded_updated_at_ms, 1_000);
        assert_eq!(conflicts[0].table, "games");
    }

    #[test]
    fn identical_rows_do_not_produce_conflict_noise() {
        let local = vec![Row::new(ROW, DEV_A, 1_000, "same")];
        let remote = vec![Row::new(ROW, DEV_A, 1_000, "same")];
        let (_, conflicts) = merge_rows("games", &local, &remote);
        assert!(
            conflicts.is_empty(),
            "unchanged rows must not fill the log, got {conflicts:?}",
        );
    }

    #[test]
    fn sessions_are_append_only_and_never_overwritten() {
        // Overwriting a session would silently rewrite playtime history.
        let local = vec![Row::new(ROW, DEV_A, 1_000, "local-time")];
        let remote = vec![Row::new(ROW, DEV_B, 9_000, "remote-time")];
        let (merged, added) = merge_sessions(&local, &remote);
        assert_eq!(merged.len(), 1);
        assert_eq!(
            merged[0].value, "local-time",
            "an existing session keeps its own value",
        );
        assert_eq!(added.len(), 0, "added should be empty");
    }

    #[test]
    fn sessions_from_the_other_device_are_added() {
        let local = vec![Row::new(ROW, DEV_A, 1_000, "mine")];
        let remote = vec![Row::new(ROW2, DEV_B, 2_000, "theirs")];
        let (merged, added) = merge_sessions(&local, &remote);
        assert_eq!(merged.len(), 2);
        assert_eq!(added, vec![ROW2.to_owned()]);
    }

    #[test]
    fn a_deadlock_free_merge_needs_no_second_pass() {
        // Merging an already-merged collection must be a no-op. If it were not,
        // repeated syncs would keep producing conflicts.
        let local = vec![Row::new(ROW, DEV_A, 1_000, "a")];
        let remote = vec![Row::new(ROW, DEV_B, 2_000, "b")];
        let (once, _) = merge_rows("games", &local, &remote);
        let (twice, conflicts) = merge_rows("games", &once, &remote);
        assert_eq!(once, twice);
        assert!(conflicts.is_empty(), "re-merging must be quiet");
    }

    #[test]
    fn a_row_deleted_locally_stays_deleted_after_merging_an_older_copy() {
        // The classic resurrection bug: pull an old backup and the deleted game
        // reappears.
        let local = vec![Row::deleted_row(ROW, DEV_A, 5_000)];
        let remote = vec![Row::new(ROW, DEV_B, 1_000, "old")];
        let (merged, _) = merge_rows("games", &local, &remote);
        assert_eq!(merged.len(), 1);
        assert!(merged[0].meta.deleted, "a stale copy must not undeaden it");
    }
}
