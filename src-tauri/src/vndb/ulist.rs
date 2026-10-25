//! User-list (`ulist`) request and response shapes.
//!
//! Documented at <https://api.vndb.org/kana#post-ulist>. Notes that shaped the
//! code here:
//!
//! * `POST /ulist` requires the `"user"` parameter, unlike `POST /vn`.
//! * `vote` is an integer 10..=100 (i.e. decivotes), `null` when unvoted.
//! * `started` / `finished` are `YYYY-MM-DD` strings or null.
//! * `labels` is an array of `{id, label}`; private labels appear only with the
//!   `listread` permission.
//! * `lastmod` is a Unix timestamp in **seconds**, unlike everything else here.

use serde::{Deserialize, Serialize};

/// A `POST /ulist` request body.
#[derive(Debug, Clone, Serialize)]
pub struct UlistQuery {
    /// User whose list to fetch. Required by the endpoint.
    pub user: String,
    /// Comma-separated field list.
    pub fields: String,
    /// Filter predicates.
    pub filters: serde_json::Value,
    /// Results per page, max 100.
    pub results: u32,
    /// Page number, starting at 1.
    pub page: u32,
    /// Sort key: `id`, `title`, `voted`, `vote`, `added`, `lastmod`, `started`,
    /// `finished`, and the other documented values.
    pub sort: String,
    /// Descending when true.
    pub reverse: bool,
}

impl UlistQuery {
    /// A page of a user's whole list, in list-modification order.
    #[must_use]
    pub fn page(user: &str, fields: impl Into<String>, page: u32) -> Self {
        Self {
            user: user.to_owned(),
            fields: fields.into(),
            filters: serde_json::json!([]),
            results: 100,
            page,
            sort: "id".to_owned(),
            reverse: false,
        }
    }
}

/// A `POST /ulist` response envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UlistResponse {
    /// The list entries.
    #[serde(default)]
    pub results: Vec<UlistEntry>,
    /// Whether another page exists.
    #[serde(default)]
    pub more: bool,
}

/// One entry on the user's list.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UlistEntry {
    /// VN id the entry refers to.
    pub id: String,
    /// Unix seconds when the entry was added.
    pub added: Option<i64>,
    /// Unix seconds when the user voted, or null.
    pub voted: Option<i64>,
    /// Unix seconds of the last list modification.
    pub lastmod: Option<i64>,
    /// Vote in decivotes, 10..=100, or null.
    pub vote: Option<u32>,
    /// Start date, `YYYY-MM-DD`.
    pub started: Option<String>,
    /// Finish date, `YYYY-MM-DD`.
    pub finished: Option<String>,
    /// Notes.
    pub notes: Option<String>,
    /// Labels applied to this entry.
    #[serde(default)]
    pub labels: Vec<UlistLabel>,
    /// Nested VN fields, when requested.
    pub vn: Option<serde_json::Value>,
}

impl UlistEntry {
    /// Label ids applied to this entry.
    #[must_use]
    pub fn label_ids(&self) -> Vec<i64> {
        self.labels.iter().map(|label| label.id).collect()
    }

    /// Whether a specific label is applied.
    #[must_use]
    pub fn has_label(&self, id: i64) -> bool {
        self.labels.iter().any(|label| label.id == id)
    }

    /// `lastmod` as milliseconds, converted from the documented Unix seconds.
    #[must_use]
    pub fn lastmod_ms(&self) -> Option<i64> {
        self.lastmod.map(|seconds| seconds.saturating_mul(1_000))
    }
}

/// A label applied to a list entry.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UlistLabel {
    /// Label id.
    pub id: i64,
    /// Label name.
    pub label: Option<String>,
}

/// Response from `GET /ulist_labels`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LabelsResponse {
    /// The user's labels.
    #[serde(default)]
    pub labels: Vec<UlistLabelInfo>,
}

/// One label definition.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UlistLabelInfo {
    /// Label id.
    pub id: i64,
    /// Whether the label is private.
    pub private: Option<bool>,
    /// Label name.
    pub label: Option<String>,
    /// Number of entries carrying it.
    pub count: Option<u32>,
}

/// A `PATCH /ulist/<id>` body.
///
/// Every member is optional and missing members are not modified. `null` unsets a
/// field. `labels_set` / `labels_unset` are used instead of `labels` so unrelated
/// labels (including private ones) are never destroyed by a push.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UlistPatch {
    // `skip_serializing_if` keeps the wire format minimal (the API treats a
    // missing member as "do not modify"), so every field also needs
    // `serde(default)` to survive a round trip through the push queue.
    /// Vote in decivotes, or null to remove the vote.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vote: Option<Option<u32>>,
    /// Notes, or null to clear.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<Option<String>>,
    /// Start date, or null to clear.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started: Option<Option<String>>,
    /// Finish date, or null to clear.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished: Option<Option<String>>,
    /// Labels to add.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels_set: Vec<i64>,
    /// Labels to remove.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels_unset: Vec<i64>,
}

impl UlistPatch {
    /// Whether this patch would change nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.vote.is_none()
            && self.notes.is_none()
            && self.started.is_none()
            && self.finished.is_none()
            && self.labels_set.is_empty()
            && self.labels_unset.is_empty()
    }
}

/// Fields requested when pulling the list: what the mapping needs, plus the VN
/// title and image so a newly discovered entry can be shown without a second call.
pub const ULIST_FIELDS: &str = "id,added,voted,lastmod,vote,started,finished,notes,labels.id,labels.label,vn.title,vn.alttitle,vn.image.url,vn.image.sexual,vn.image.violence,vn.length_minutes";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_requires_the_user_and_defaults_to_a_full_page() {
        let query = UlistQuery::page("u3", ULIST_FIELDS, 1);
        assert_eq!(query.user, "u3");
        assert_eq!(query.results, 100);
        assert_eq!(query.page, 1);
        assert_eq!(query.sort, "id");
    }

    #[test]
    fn decodes_a_realistic_ulist_response() {
        let json = r#"{
          "results": [{
            "id": "v17",
            "added": 1600000000,
            "voted": 1600000100,
            "lastmod": 1600000200,
            "vote": 80,
            "started": "2020-09-01",
            "finished": null,
            "notes": "reading route A",
            "labels": [{"id": 1, "label": "Playing"}, {"id": 7, "label": "Voted"}],
            "vn": {"title": "Ever17", "alttitle": "Ever17"}
          }],
          "more": true
        }"#;
        let parsed: UlistResponse = serde_json::from_str(json).expect("decode");
        assert!(parsed.more);
        let entry = &parsed.results[0];
        assert_eq!(entry.id, "v17");
        assert_eq!(entry.vote, Some(80));
        assert_eq!(entry.started.as_deref(), Some("2020-09-01"));
        assert!(entry.finished.is_none());
        assert_eq!(entry.label_ids(), vec![1, 7]);
        assert!(entry.has_label(1));
        assert!(!entry.has_label(2));
    }

    #[test]
    fn lastmod_seconds_become_milliseconds() {
        let entry = UlistEntry {
            lastmod: Some(1_600_000_200),
            ..UlistEntry::default()
        };
        assert_eq!(entry.lastmod_ms(), Some(1_600_000_200_000));
    }

    #[test]
    fn an_unvoted_entry_has_no_vote() {
        let entry = UlistEntry::default();
        assert!(entry.vote.is_none());
        assert!(entry.label_ids().is_empty());
    }

    #[test]
    fn patch_skips_untouched_fields() {
        let patch = UlistPatch::default();
        assert!(patch.is_empty());
        let json = serde_json::to_string(&patch).expect("serialize");
        assert_eq!(json, "{}");
    }

    #[test]
    fn patch_distinguishes_unset_from_untouched() {
        // `vote: None` (untouched) must not appear; `vote: Some(None)` must be
        // serialised as explicit null so the API clears the vote.
        let untouched = UlistPatch {
            notes: None,
            ..UlistPatch::default()
        };
        assert!(
            !serde_json::to_string(&untouched)
                .expect("serialize")
                .contains("notes")
        );

        let clear_vote = UlistPatch {
            vote: Some(None),
            ..UlistPatch::default()
        };
        let json = serde_json::to_string(&clear_vote).expect("serialize");
        assert_eq!(json, r#"{"vote":null}"#);
    }

    #[test]
    fn patch_carries_label_edits_without_touching_other_labels() {
        let patch = UlistPatch {
            labels_set: vec![2],
            labels_unset: vec![1],
            ..UlistPatch::default()
        };
        let json = serde_json::to_string(&patch).expect("serialize");
        assert!(json.contains("\"labels_set\":[2]"));
        assert!(json.contains("\"labels_unset\":[1]"));
        // The destructive `labels` field is never used.
        assert!(!json.contains("\"labels\":"));
    }

    #[test]
    fn patch_survives_a_json_round_trip() {
        // The push queue stores patches as JSON, so a skipped field must decode
        // back to its default rather than failing.
        let original = UlistPatch {
            vote: Some(Some(90)),
            ..UlistPatch::default()
        };
        let json = serde_json::to_string(&original).expect("serialize");
        let decoded: UlistPatch = serde_json::from_str(&json).expect("decode");
        assert_eq!(decoded, original);
    }

    #[test]
    fn empty_patch_round_trips() {
        let json = serde_json::to_string(&UlistPatch::default()).expect("serialize");
        let decoded: UlistPatch = serde_json::from_str(&json).expect("decode");
        assert!(decoded.is_empty());
    }

    #[test]
    fn labels_response_decodes() {
        let json = r#"{"labels":[
            {"id":0,"private":false,"label":"No label","count":3},
            {"id":1,"private":false,"label":"Playing","count":2},
            {"id":7,"private":false,"label":"Voted","count":5}
        ]}"#;
        let parsed: LabelsResponse = serde_json::from_str(json).expect("decode");
        assert_eq!(parsed.labels.len(), 3);
        assert_eq!(parsed.labels[1].label.as_deref(), Some("Playing"));
    }
}
