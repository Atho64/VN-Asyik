//! Local memory policy. Text is untrusted context, never an instruction or tool grant.
use super::{Knowledge, SessionOptions};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const MAX_ITEMS: i64 = 1_000;
pub const MAX_PINS: i64 = 100;
pub const MAX_MARKERS: i64 = 10_000;
pub const MAX_BODY: usize = 2_048;
pub const MAX_EVIDENCE: usize = 4_096;
pub const MAX_EVIDENCE_TOTAL: i64 = 16 * 1024 * 1024;
pub const MAX_JOBS: i64 = 20;
pub const MAX_INPUT: usize = 65_536;
pub const MAX_OUTPUT: usize = 16_384;
pub const REPLAY_BUDGET: usize = 48_000;
pub const MEMORY_BUDGET: usize = 8_192;
pub const WEEK: i64 = 7 * 86_400_000;
pub const RECAP_LIFETIME: i64 = 90 * 86_400_000;
pub const POLICY_VERSION: i64 = 1;
pub const PROMPT_VERSION: i64 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    Shared,
    Companion,
    Reading,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Class {
    Preference,
    Continuity,
    Episode,
    Observation,
    Summary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Evidence {
    Reader,
    Passage,
    Inference,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Context {
    pub companion_id: String,
    pub game_id: String,
    pub playthrough_id: String,
    pub knowledge: Knowledge,
}

impl Context {
    pub fn new(options: &SessionOptions, playthrough: &str) -> Self {
        Self {
            companion_id: options.profile.as_ref().map_or_else(
                || {
                    super::profile::Preset::legacy(options.persona)
                        .snapshot()
                        .id
                },
                |profile| profile.id.clone(),
            ),
            game_id: options.game_id.clone(),
            playthrough_id: playthrough.into(),
            knowledge: options.knowledge,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Memory {
    pub id: String,
    pub version: i64,
    pub class: Class,
    pub scope: Scope,
    pub companion_id: Option<String>,
    pub game_id: Option<String>,
    pub playthrough_id: Option<String>,
    pub knowledge: Knowledge,
    pub body: String,
    pub evidence: Evidence,
    pub pinned: bool,
    pub origin_session: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub expires_at: Option<i64>,
}

pub fn body_valid(body: &str) -> bool {
    !body.trim().is_empty() && body.len() <= MAX_BODY && !body.contains('\0')
}

pub fn level(policy: Knowledge) -> u8 {
    match policy {
        Knowledge::Metadata => 0,
        Knowledge::Observed => 1,
        Knowledge::All => 2,
    }
}

impl Memory {
    pub fn valid(&self) -> bool {
        if !body_valid(&self.body) || self.version < 1 {
            return false;
        }
        match self.scope {
            Scope::Shared => {
                self.class == Class::Preference
                    && self.evidence == Evidence::Reader
                    && self.companion_id.is_none()
                    && self.game_id.is_none()
                    && self.playthrough_id.is_none()
                    && self.origin_session.is_none()
                    && self.knowledge == Knowledge::Metadata
            }
            Scope::Companion => {
                matches!(self.class, Class::Preference | Class::Continuity)
                    && self.evidence == Evidence::Reader
                    && self.companion_id.is_some()
                    && self.game_id.is_none()
                    && self.playthrough_id.is_none()
                    && self.origin_session.is_none()
                    && self.knowledge == Knowledge::Metadata
            }
            Scope::Reading => {
                matches!(
                    self.class,
                    Class::Episode | Class::Observation | Class::Summary
                ) && self.game_id.is_some()
                    && self.playthrough_id.is_some()
                    && self.companion_id.is_none()
                    && self.origin_session.is_some()
                    && self.knowledge != Knowledge::Metadata
            }
        }
    }

    pub fn eligible(&self, context: &Context, now: i64) -> bool {
        self.valid()
            && (self.pinned || self.expires_at.is_none_or(|expiry| expiry > now))
            && level(self.knowledge) <= level(context.knowledge)
            && match self.scope {
                Scope::Shared => true,
                Scope::Companion => self.companion_id.as_deref() == Some(&context.companion_id),
                Scope::Reading => {
                    self.game_id.as_deref() == Some(&context.game_id)
                        && self.playthrough_id.as_deref() == Some(&context.playthrough_id)
                }
            }
    }

    /// Include labels and JSON escaping in the budget, not just the raw body.
    pub fn prompt_record(&self) -> String {
        serde_json::json!({"memoryId":self.id,"class":self.class,"evidence":self.evidence,
            "scope":self.scope,"knowledge":self.knowledge,"text":self.body})
        .to_string()
    }
}

fn words(text: &str) -> BTreeSet<String> {
    text.split(|ch: char| !ch.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .take(256)
        .map(str::to_lowercase)
        .collect()
}

/// Filter before ranking. No cross-playthrough or identity fallback, even when pinned.
pub fn retrieve(items: &[Memory], context: &Context, query: &str, now: i64) -> Vec<Memory> {
    let query = words(query);
    let mut candidates: Vec<_> = items
        .iter()
        .filter(|item| item.eligible(context, now))
        .map(|item| (item, words(&item.body).intersection(&query).count()))
        .collect();
    candidates.sort_by(|(a, sa), (b, sb)| {
        b.pinned
            .cmp(&a.pinned)
            .then(sb.cmp(sa))
            .then(b.updated_at.cmp(&a.updated_at))
            .then(a.id.cmp(&b.id))
    });
    let mut bytes = 0;
    candidates
        .into_iter()
        .filter_map(|(item, _)| {
            let size = item.prompt_record().len() + 1;
            if bytes + size > MEMORY_BUDGET {
                return None;
            }
            bytes += size;
            Some(item.clone())
        })
        .take(12)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn item() -> Memory {
        Memory {
            id: "m".into(),
            version: 1,
            class: Class::Episode,
            scope: Scope::Reading,
            companion_id: None,
            game_id: Some("game".into()),
            playthrough_id: Some("run".into()),
            knowledge: Knowledge::Observed,
            body: "We met at the station".into(),
            evidence: Evidence::Inference,
            pinned: false,
            origin_session: Some("s".into()),
            created_at: 1,
            updated_at: 1,
            expires_at: None,
        }
    }
    fn context() -> Context {
        Context {
            companion_id: "p".into(),
            game_id: "game".into(),
            playthrough_id: "run".into(),
            knowledge: Knowledge::Observed,
        }
    }
    #[test]
    fn pinned_material_never_crosses_policy_game_or_playthrough() {
        let mut item = item();
        item.pinned = true;
        let mut ctx = context();
        assert!(item.eligible(&ctx, 2));
        ctx.playthrough_id = "earlier save".into();
        assert!(!item.eligible(&ctx, 2));
        ctx = context();
        ctx.game_id = "another game".into();
        assert!(!item.eligible(&ctx, 2));
        ctx = context();
        ctx.knowledge = Knowledge::Metadata;
        assert!(!item.eligible(&ctx, 2));
        ctx = context();
        item.knowledge = Knowledge::All;
        assert!(!item.eligible(&ctx, 2));
    }
    #[test]
    fn sharing_never_promotes_inference_to_preference() {
        let mut item = item();
        item.scope = Scope::Shared;
        item.class = Class::Preference;
        assert!(!item.valid());
        item.evidence = Evidence::Reader;
        item.game_id = None;
        item.playthrough_id = None;
        item.origin_session = None;
        item.knowledge = Knowledge::Metadata;
        assert!(item.eligible(&context(), 2));
        item.scope = Scope::Companion;
        item.companion_id = Some("other".into());
        assert!(!item.eligible(&context(), 2));
        item.companion_id = Some("p".into());
        assert!(item.eligible(&context(), 2));
    }
    #[test]
    fn ranking_counts_utf8_and_serialized_evidence_and_caps_items() {
        let mut items = vec![item(); 30];
        for (index, item) in items.iter_mut().enumerate() {
            item.id = index.to_string();
            item.body = "語\"".repeat(450);
        }
        let selected = retrieve(&items, &context(), "語", 3);
        assert!(selected.len() < 12);
        assert!(
            selected
                .iter()
                .map(|m| m.prompt_record().len() + 1)
                .sum::<usize>()
                <= MEMORY_BUDGET
        );
        assert!(!body_valid(&"語".repeat(683)));
    }
}
