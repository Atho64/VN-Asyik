//! Local scene scheduling. Times are monotonic milliseconds, never wall-clock.
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

pub const STABLE_MS: u64 = 2_000;
pub const COOLDOWN_MS: u64 = 60_000;
pub const HOUR_MS: u64 = 3_600_000;
pub const HOURLY_CAP: usize = 12;
pub const SESSION_CAP: u32 = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Crop {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}
impl Default for Crop {
    fn default() -> Self {
        Self {
            x: 0,
            y: 0,
            width: 1000,
            height: 1000,
        }
    }
}
impl Crop {
    pub fn valid(self) -> bool {
        self.width > 0
            && self.height > 0
            && self.width <= 1000
            && self.height <= 1000
            && self.x <= 1000 - self.width
            && self.y <= 1000 - self.height
    }
}

#[derive(Debug, Default)]
pub struct Budget {
    calls: VecDeque<u64>,
    pub total: u32,
}
impl Budget {
    pub fn hourly(&self, now: u64) -> usize {
        self.calls
            .iter()
            .filter(|t| now.saturating_sub(**t) < HOUR_MS)
            .count()
    }
    pub fn available(&self, now: u64) -> bool {
        self.total < SESSION_CAP
            && self.hourly(now) < HOURLY_CAP
            && self
                .calls
                .back()
                .is_none_or(|last| now.saturating_sub(*last) >= COOLDOWN_MS)
    }
    /// Charge once immediately before dispatch, including failures and silence.
    pub fn charge(&mut self, now: u64) -> bool {
        if !self.available(now) {
            return false;
        }
        while self
            .calls
            .front()
            .is_some_and(|t| now.saturating_sub(*t) >= HOUR_MS)
        {
            self.calls.pop_front();
        }
        self.calls.push_back(now);
        self.total += 1;
        true
    }
}

#[derive(Debug, Default)]
pub struct Scenes {
    candidate: Vec<u8>,
    dispatched: Vec<u8>,
    since: u64,
    luna: u64,
    sent_luna: u64,
    pub sequence: u64,
    baseline: bool,
}
impl Scenes {
    /// Fresh foreground baseline suppresses a backlog after switching apps.
    pub fn suspend(&mut self) {
        self.candidate.clear();
        self.baseline = true;
        self.sequence += 1;
    }
    pub fn observe(&mut self, signature: &[u8], luna: u64, now: u64) -> bool {
        let luna = if luna == 0 { self.luna } else { luna };
        let changed = self.candidate.is_empty()
            || meaningful(&self.candidate, signature)
            || luna != self.luna;
        if changed {
            self.candidate = signature.to_vec();
            self.luna = luna;
            self.since = now;
            self.sequence += 1;
            if self.baseline {
                self.dispatched = self.candidate.clone();
                self.sent_luna = luna;
                self.baseline = false;
            }
        }
        changed
    }
    pub fn ready(&self, now: u64) -> bool {
        !self.candidate.is_empty()
            && now.saturating_sub(self.since) >= STABLE_MS
            && (meaningful(&self.dispatched, &self.candidate) || self.luna != self.sent_luna)
    }
    pub fn dispatched(&mut self) {
        self.dispatched = self.candidate.clone();
        self.sent_luna = self.luna;
    }
}
pub fn meaningful(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() || a.is_empty() {
        return true;
    }
    let delta: u64 = a
        .iter()
        .zip(b)
        .map(|(x, y)| u64::from(x.abs_diff(*y)))
        .sum();
    let changed = a
        .iter()
        .zip(b)
        .filter(|(x, y)| x.abs_diff(**y) > 20)
        .count();
    delta > a.len() as u64 * 6 || changed * 1000 > a.len() * 15
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Reaction {
    react: bool,
    text: String,
}
pub fn reaction(raw: &str) -> Result<Option<String>, &'static str> {
    let answer: Reaction = serde_json::from_str(raw).map_err(|_| "vision_invalid_reaction")?;
    let text = answer.text.trim();
    if text.len() > 1600 || (answer.react && text.is_empty()) || (!answer.react && !text.is_empty())
    {
        return Err("vision_invalid_reaction");
    }
    Ok(answer.react.then(|| text.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stability_duplicates_and_typewriter() {
        let mut s = Scenes::default();
        s.observe(&[0; 100], 0, 0);
        assert!(!s.ready(1999));
        assert!(s.ready(2000));
        s.dispatched();
        assert!(!s.ready(60000));
        s.observe(&[255; 100], 0, 60001);
        s.observe(&[128; 100], 0, 61500);
        assert!(!s.ready(62001));
        assert!(s.ready(63500));
        s.dispatched();
        s.observe(&[128; 100], 1, 64000);
        assert!(s.ready(66000));
    }
    #[test]
    fn focus_return_establishes_baseline_and_revokes_sequence() {
        let mut s = Scenes::default();
        s.observe(&[0; 100], 0, 0);
        let seq = s.sequence;
        s.suspend();
        assert!(s.sequence > seq);
        s.observe(&[255; 100], 8, 10000);
        assert!(!s.ready(20000));
        s.observe(&[0; 100], 9, 30000);
        assert!(s.ready(32000));
    }
    #[test]
    fn failures_silence_and_cancelled_dispatches_count_with_no_reset() {
        let mut b = Budget::default();
        for i in 0..12 {
            assert!(b.charge(i * COOLDOWN_MS));
            assert!(!b.charge(i * COOLDOWN_MS + 1));
        }
        assert!(!b.charge(12 * COOLDOWN_MS));
        assert!(b.charge(HOUR_MS));
        assert_eq!(b.total, 13);
        for i in 1..12 {
            assert!(b.charge(HOUR_MS + i * COOLDOWN_MS));
        }
        for i in 0..6 {
            assert!(b.charge(2 * HOUR_MS + i * COOLDOWN_MS));
        }
        assert_eq!(b.total, 30);
        assert!(!b.charge(10 * HOUR_MS));
    }
    #[test]
    fn validated_silence_and_bounded_reactions() {
        assert_eq!(reaction(r#"{"react":false,"text":""}"#).unwrap(), None);
        assert_eq!(
            reaction(r#"{"react":true,"text":"Interesting scene."}"#).unwrap(),
            Some("Interesting scene.".into())
        );
        for bad in [
            r#"{"react":false,"text":"hidden"}"#,
            r#"{"react":true,"text":"","tool":"x"}"#,
            "hello",
        ] {
            assert!(reaction(bad).is_err());
        }
    }
    #[test]
    fn crop_rejects_overflow_and_empty() {
        assert!(Crop::default().valid());
        assert!(
            !Crop {
                x: u32::MAX,
                ..Crop::default()
            }
            .valid()
        );
        assert!(
            !Crop {
                width: 0,
                ..Crop::default()
            }
            .valid()
        );
    }
}
