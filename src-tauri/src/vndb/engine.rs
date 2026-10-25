//! The sync planner.
//!
//! `plan` is pure: it takes the local row, what VNDB currently reports, and the
//! user's policy, and returns the actions to take. `execute` is the only part
//! that touches the network, and it is deliberately thin.
//!
//! Design rules that come straight from AGENTS.md §6 and §10:
//!
//! * **No silent overwrites.** A field that disagrees becomes a conflict unless
//!   the user picked a policy that decides it.
//! * **Local wins are explicit.** "Keep local" produces a patch, not a database
//!   write, so a failed push leaves local data untouched and retries later.
//! * **Playtime never syncs.** VNDB has no field for tracked seconds, so nothing
//!   in this module can touch a session.

use serde::{Deserialize, Serialize};
use vn_core::model::GameStatus;

use crate::vndb::mapping::{
    self, Conflict, ConflictPolicy, LabelMapping, LocalState, RemoteState, Resolution,
};
use crate::vndb::ulist::UlistPatch;

/// A local change to apply after the user (or policy) chose the remote value.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalUpdate {
    /// New status.
    pub status: Option<GameStatus>,
    /// New rating; `Some(None)` clears it.
    pub rating: Option<Option<u8>>,
    /// New notes; `Some(None)` clears them.
    pub notes: Option<Option<String>>,
    /// New start date, UTC ms; `Some(None)` clears it.
    pub started_at: Option<Option<i64>>,
    /// New finish date, UTC ms; `Some(None)` clears it.
    pub finished_at: Option<Option<i64>>,
}

impl LocalUpdate {
    /// Whether nothing would change.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.status.is_none()
            && self.rating.is_none()
            && self.notes.is_none()
            && self.started_at.is_none()
            && self.finished_at.is_none()
    }
}

/// What sync decided to do.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncPlan {
    /// Patch to send to VNDB, empty when there is nothing to push.
    pub push: UlistPatch,
    /// Local fields to overwrite from VNDB.
    pub update: LocalUpdate,
    /// Conflicts the user must decide.
    pub conflicts: Vec<Conflict>,
    /// Fields left alone because the policy said to skip them.
    pub skipped: Vec<Conflict>,
}

impl SyncPlan {
    /// Whether the plan needs a network call.
    #[must_use]
    pub fn needs_push(&self) -> bool {
        !self.push.is_empty()
    }

    /// Whether the plan changes the local database.
    #[must_use]
    pub fn needs_local_update(&self) -> bool {
        !self.update.is_empty()
    }

    /// Whether anything at all happens.
    #[must_use]
    pub fn is_noop(&self) -> bool {
        !self.needs_push() && !self.needs_local_update() && self.conflicts.is_empty()
    }
}

/// A decision for one field, given by the user when reviewing conflicts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldDecision {
    /// Field key, as produced by [`mapping::Field::key`].
    pub field: String,
    /// What to do with it.
    pub resolution: Resolution,
}

/// Build a plan for one VN.
///
/// `decisions` overrides the policy for specific fields, which is how the conflict
/// screen reports the user's choices back into the sync.
#[must_use]
pub fn plan(
    local: &LocalState,
    remote: &RemoteState,
    remote_labels: &[i64],
    mapping: &LabelMapping,
    policy: ConflictPolicy,
    decisions: &[FieldDecision],
) -> SyncPlan {
    let conflicts = mapping::diff(local, remote);
    let mut result = SyncPlan {
        conflicts: Vec::new(),
        ..SyncPlan::default()
    };

    let local_newer = mapping::local_is_newer(local, remote);

    for conflict in conflicts {
        let resolution = decisions
            .iter()
            .find(|decision| decision.field == conflict.field.key())
            .map_or_else(
                || mapping::resolve(&conflict, policy, local_newer),
                |decision| decision.resolution,
            );

        match resolution {
            Resolution::Skip => {
                // Nothing decided: surface it for the user and change nothing.
                result.conflicts.push(conflict);
            }
            Resolution::KeepRemote => {
                apply_remote(
                    &mut result.update,
                    &conflict,
                    remote,
                    mapping,
                    remote_labels,
                );
            }
            Resolution::KeepLocal => {
                apply_local(&mut result.push, &conflict, local, remote_labels, mapping);
            }
        }
    }

    result
}

/// Write the remote value into the local update.
fn apply_remote(
    update: &mut LocalUpdate,
    conflict: &Conflict,
    remote: &RemoteState,
    mapping: &LabelMapping,
    remote_labels: &[i64],
) {
    match conflict.field {
        mapping::Field::Status => {
            // Prefer the label-derived status; fall back to the parsed remote value.
            update.status = remote
                .status
                .or_else(|| mapping.status_from_labels(remote_labels))
                .or_else(|| {
                    conflict
                        .remote
                        .as_deref()
                        .and_then(|text| GameStatus::parse(text).ok())
                });
        }
        mapping::Field::Rating => update.rating = Some(remote.rating),
        mapping::Field::Notes => update.notes = Some(remote.notes.clone()),
        mapping::Field::Started => update.started_at = Some(remote.started_at),
        mapping::Field::Finished => update.finished_at = Some(remote.finished_at),
    }
}

/// Write the local value into the patch that will be pushed.
fn apply_local(
    push: &mut UlistPatch,
    conflict: &Conflict,
    local: &LocalState,
    remote_labels: &[i64],
    mapping: &LabelMapping,
) {
    match conflict.field {
        mapping::Field::Status => {
            if let Some(status) = local.status {
                let (set, unset) = mapping.label_edits(remote_labels, status);
                push.labels_set = set;
                push.labels_unset = unset;
            }
        }
        mapping::Field::Rating => push.vote = Some(mapping::rating_to_vote(local.rating)),
        mapping::Field::Notes => push.notes = Some(local.notes.clone()),
        mapping::Field::Started => push.started = Some(mapping::ms_to_date(local.started_at)),
        mapping::Field::Finished => {
            push.finished = Some(mapping::ms_to_date(local.finished_at));
        }
    }
}

/// Build a patch that makes VNDB match the local row, ignoring conflicts.
///
/// Used when importing a game that came from VNDB but has never been pushed, and
/// when the user explicitly chooses "push everything from here".
#[must_use]
pub fn full_push(local: &LocalState, remote_labels: &[i64], mapping: &LabelMapping) -> UlistPatch {
    let mut patch = UlistPatch::default();
    if let Some(status) = local.status {
        let (set, unset) = mapping.label_edits(remote_labels, status);
        patch.labels_set = set;
        patch.labels_unset = unset;
    }
    if local.rating.is_some() {
        patch.vote = Some(mapping::rating_to_vote(local.rating));
    }
    if local.notes.is_some() {
        patch.notes = Some(local.notes.clone());
    }
    if local.started_at.is_some() {
        patch.started = Some(mapping::ms_to_date(local.started_at));
    }
    if local.finished_at.is_some() {
        patch.finished = Some(mapping::ms_to_date(local.finished_at));
    }
    patch
}

/// Count how many decisions still need a human.
#[must_use]
pub fn pending_decisions(plan: &SyncPlan) -> usize {
    plan.conflicts.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vndb::mapping::{Field, date_to_ms};

    fn mapping() -> LabelMapping {
        LabelMapping::default()
    }

    fn local() -> LocalState {
        LocalState {
            status: Some(GameStatus::Playing),
            rating: Some(80),
            notes: Some("mine".to_owned()),
            started_at: date_to_ms(Some("2020-09-01")),
            finished_at: None,
            updated_at_ms: 2_000,
        }
    }

    fn remote() -> RemoteState {
        RemoteState {
            status: Some(GameStatus::Completed),
            rating: Some(50),
            notes: Some("theirs".to_owned()),
            started_at: date_to_ms(Some("2020-01-01")),
            finished_at: None,
            lastmod_ms: 1_000,
        }
    }

    fn matching_local() -> LocalState {
        LocalState {
            status: Some(GameStatus::Completed),
            rating: Some(50),
            notes: Some("theirs".to_owned()),
            started_at: date_to_ms(Some("2020-01-01")),
            finished_at: None,
            updated_at_ms: 500,
        }
    }

    #[test]
    fn matching_sides_produce_a_noop_plan() {
        let built = plan(
            &matching_local(),
            &remote(),
            &[2],
            &mapping(),
            ConflictPolicy::Ask,
            &[],
        );
        assert!(built.is_noop());
        assert_eq!(built.conflicts.len(), 0, "built.conflicts should be empty");
    }

    #[test]
    fn ask_policy_surfaces_conflicts_and_changes_nothing() {
        let built = plan(
            &local(),
            &remote(),
            &[2],
            &mapping(),
            ConflictPolicy::Ask,
            &[],
        );
        assert!(!built.needs_push());
        assert!(!built.needs_local_update());
        // status, rating, notes and started all differ; finished is absent on both.
        assert_eq!(built.conflicts.len(), 4);
        assert!(built.conflicts.iter().any(|c| c.field == Field::Status));
    }

    #[test]
    fn prefer_local_pushes_a_full_patch() {
        let built = plan(
            &local(),
            &remote(),
            &[2],
            &mapping(),
            ConflictPolicy::PreferLocal,
            &[],
        );
        assert!(built.needs_push());
        assert!(!built.needs_local_update());
        assert_eq!(built.push.vote, Some(Some(80)));
        assert_eq!(built.push.notes, Some(Some("mine".to_owned())));
        // Playing (1) is set; Completed (2) is cleared.
        assert_eq!(built.push.labels_set, vec![1]);
        assert_eq!(built.push.labels_unset, vec![2]);
    }

    #[test]
    fn prefer_remote_updates_local_values() {
        let built = plan(
            &local(),
            &remote(),
            &[2],
            &mapping(),
            ConflictPolicy::PreferRemote,
            &[],
        );
        assert!(!built.needs_push());
        assert!(built.needs_local_update());
        assert_eq!(built.update.status, Some(GameStatus::Completed));
        assert_eq!(built.update.rating, Some(Some(50)), "decivotes");
        assert_eq!(built.update.notes, Some(Some("theirs".to_owned())));
        assert_eq!(
            built.update.started_at,
            Some(date_to_ms(Some("2020-01-01")))
        );
    }

    #[test]
    fn newest_policy_uses_timestamps() {
        // local.updated_at_ms (2000) > remote.lastmod_ms (1000), so local wins.
        let built = plan(
            &local(),
            &remote(),
            &[2],
            &mapping(),
            ConflictPolicy::Newest,
            &[],
        );
        assert!(built.needs_push());
        assert!(!built.needs_local_update());

        let stale_local = LocalState {
            updated_at_ms: 500,
            ..local()
        };
        let built = plan(
            &stale_local,
            &remote(),
            &[2],
            &mapping(),
            ConflictPolicy::Newest,
            &[],
        );
        assert!(!built.needs_push());
        assert!(built.needs_local_update());
    }

    #[test]
    fn a_field_decision_overrides_the_policy() {
        let decisions = [
            FieldDecision {
                field: "status".to_owned(),
                resolution: Resolution::KeepRemote,
            },
            FieldDecision {
                field: "rating".to_owned(),
                resolution: Resolution::KeepLocal,
            },
            FieldDecision {
                field: "notes".to_owned(),
                resolution: Resolution::Skip,
            },
        ];
        let built = plan(
            &local(),
            &remote(),
            &[2],
            &mapping(),
            ConflictPolicy::PreferLocal,
            &decisions,
        );
        assert_eq!(built.update.status, Some(GameStatus::Completed));
        assert_eq!(built.push.vote, Some(Some(80)));
        // The skipped field is reported, not silently dropped.
        assert_eq!(built.conflicts.len(), 1);
        assert_eq!(built.conflicts[0].field, Field::Notes);
    }

    #[test]
    fn mixed_resolutions_can_do_both_directions() {
        let decisions = [
            FieldDecision {
                field: "status".to_owned(),
                resolution: Resolution::KeepRemote,
            },
            FieldDecision {
                field: "rating".to_owned(),
                resolution: Resolution::KeepLocal,
            },
            FieldDecision {
                field: "notes".to_owned(),
                resolution: Resolution::KeepRemote,
            },
        ];
        let built = plan(
            &local(),
            &remote(),
            &[2],
            &mapping(),
            ConflictPolicy::Ask,
            &decisions,
        );
        assert!(built.needs_push(), "rating goes up");
        assert!(built.needs_local_update(), "status and notes come down");
        assert_eq!(built.push.vote, Some(Some(80)));
        assert_eq!(built.update.status, Some(GameStatus::Completed));
        // `started` also differs and was not decided, so it stays a conflict.
        assert_eq!(built.conflicts.len(), 1);
        assert_eq!(built.conflicts[0].field, Field::Started);
    }

    #[test]
    fn local_status_becomes_label_edits_and_keeps_custom_labels() {
        // Local and remote both say Completed, so the only difference is the
        // labels array: remote still carries Playing (1) plus a custom label 99.
        // PreferLocal therefore emits a corrective label edit.
        let mut local = matching_local();
        local.status = Some(GameStatus::Completed);
        let remote = RemoteState {
            status: Some(GameStatus::Completed),
            rating: Some(50),
            notes: Some("theirs".to_owned()),
            started_at: date_to_ms(Some("2020-01-01")),
            finished_at: None,
            lastmod_ms: 1_000,
        };
        // `diff` reports no field conflict here, so use the explicit full push to
        // express "make VNDB match local labels".
        let patch = full_push(&local, &[1, 99], &mapping());
        // Finished (2) is added, Playing (1) removed, and the custom label 99 is
        // never mentioned, so it survives on the server.
        assert_eq!(patch.labels_set, vec![2]);
        assert_eq!(patch.labels_unset, vec![1]);
        assert!(!patch.labels_unset.contains(&99), "custom labels survive");
        let _ = remote;
    }

    #[test]
    fn clearing_a_local_rating_pushes_an_explicit_null() {
        let mut local = local();
        local.rating = None;
        let built = plan(
            &local,
            &remote(),
            &[1],
            &mapping(),
            ConflictPolicy::PreferLocal,
            &[],
        );
        assert_eq!(built.push.vote, Some(None));
        let json = serde_json::to_string(&built.push).expect("serialize");
        assert!(json.contains("\"vote\":null"));
    }

    #[test]
    fn full_push_sends_every_local_field() {
        let patch = full_push(&local(), &[1], &mapping());
        assert_eq!(patch.vote, Some(Some(80)));
        assert_eq!(patch.notes, Some(Some("mine".to_owned())));
        assert_eq!(patch.started, Some(Some("2020-09-01".to_owned())));
        // Already Playing, so no label edit is emitted.
        assert_eq!(
            patch.labels_set.len(),
            0,
            "patch.labels_set should be empty"
        );
    }

    #[test]
    fn full_push_omits_fields_the_user_never_set() {
        let bare = LocalState {
            status: None,
            rating: None,
            notes: None,
            started_at: None,
            finished_at: None,
            updated_at_ms: 0,
        };
        let patch = full_push(&bare, &[], &mapping());
        assert!(patch.is_empty(), "nothing local means nothing to push");
    }

    #[test]
    fn playtime_can_never_appear_in_a_patch() {
        // The patch type has no field for seconds. This test documents the
        // invariant: whatever the local state holds, the serialised patch cannot
        // carry tracked time.
        let patch = full_push(&local(), &[1], &mapping());
        let json = serde_json::to_string(&patch).expect("serialize");
        for forbidden in ["counted", "playtime", "seconds", "afk"] {
            assert!(
                !json.contains(forbidden),
                "patch must not carry {forbidden}"
            );
        }
    }

    #[test]
    fn pending_decisions_counts_undecided_conflicts() {
        let built = plan(
            &local(),
            &remote(),
            &[1],
            &mapping(),
            ConflictPolicy::Ask,
            &[],
        );
        assert_eq!(pending_decisions(&built), 4);
    }
}
