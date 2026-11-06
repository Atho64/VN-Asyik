//! Reading-session lifecycle and source eligibility. No OS, database or model calls.
pub mod memory;
pub mod profile;
#[cfg(test)]
mod tests;
pub mod vision;

use serde::{Deserialize, Serialize};

use crate::chat::{ChatOptions, Persona, Spoilers};

pub const MAX_SOURCE_AGE_MS: i64 = 120_000;
pub const MAX_SOURCE_BYTES: usize = 12_000;
pub const MAX_SOURCE_LINES: usize = 30;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Knowledge {
    Metadata,
    #[default]
    Observed,
    All,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DialogueSource {
    #[default]
    Manual,
    Luna,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SessionOptions {
    pub game_id: String,
    /// A label is a reminder, never proof of story chronology.
    pub playthrough_label: String,
    pub knowledge: Knowledge,
    pub boundary: String,
    pub source: DialogueSource,
    pub language: String,
    pub persona: Persona,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<profile::ProfileSnapshot>,
}

impl SessionOptions {
    pub fn validate(&self) -> Result<(), String> {
        if let Some(profile) = &self.profile {
            profile.validate()?;
        }
        uuid::Uuid::parse_str(&self.game_id).map_err(|_| "invalid_game")?;
        if self.playthrough_label.trim().is_empty() || self.playthrough_label.chars().count() > 80 {
            return Err("invalid_playthrough".into());
        }
        self.chat_options().validate()
    }

    pub fn prompt(&self, mode: profile::PromptMode) -> String {
        match &self.profile {
            Some(profile) => profile.settings.prompt(&self.chat_options(), mode),
            None => match mode {
                profile::PromptMode::Answer => self.chat_options().prompt(),
                _ => profile::Preset::legacy(self.persona)
                    .snapshot()
                    .settings
                    .prompt(&self.chat_options(), mode),
            },
        }
    }

    /// Keep legacy None/Boundary/All wire semantics intact. Observed material
    /// uses Boundary with an explicit reading-context instruction, even when
    /// the reader supplies no additional restriction.
    pub fn chat_options(&self) -> ChatOptions {
        ChatOptions {
            game_id: Some(self.game_id.clone()),
            spoilers: match self.knowledge {
                Knowledge::Metadata => Spoilers::None,
                Knowledge::Observed => Spoilers::Boundary,
                Knowledge::All => Spoilers::All,
            },
            boundary: if self.knowledge == Knowledge::Observed && self.boundary.trim().is_empty() {
                "Only passages supplied or observed in this reading context. No future-story knowledge.".into()
            } else {
                self.boundary.clone()
            },
            allow_online: false,
            language: self.language.clone(),
            persona: self.persona,
        }
    }
}

/// Sequence never resets during a process lifetime. Generation changes whenever
/// the connection is replaced, disabled, disconnected or cleared.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceCursor {
    pub generation: u64,
    pub sequence: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Observation {
    pub cursor: SourceCursor,
    pub text: String,
    pub at: i64,
}

/// Select a bounded, immutable window for one turn. Timestamps alone cannot
/// distinguish a reconnect or two lines arriving within the same millisecond.
pub fn eligible_lines(
    lines: &[Observation],
    after: SourceCursor,
    through: SourceCursor,
    now: i64,
) -> Vec<Observation> {
    if after.generation != through.generation {
        return Vec::new();
    }
    let mut bytes = 0;
    let mut selected = Vec::new();
    for line in lines.iter().rev().filter(|line| {
        line.cursor.generation == after.generation
            && line.cursor.sequence > after.sequence
            && line.cursor.sequence <= through.sequence
            && line.at <= now
            && now.saturating_sub(line.at) <= MAX_SOURCE_AGE_MS
    }) {
        if selected.len() == MAX_SOURCE_LINES || bytes + line.text.len() > MAX_SOURCE_BYTES {
            break;
        }
        bytes += line.text.len();
        selected.push(line.clone());
    }
    selected.reverse();
    selected
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Lifecycle {
    #[default]
    Off,
    Starting,
    Active,
    Paused,
    Ending,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnToken {
    pub epoch: u64,
    pub turn: u64,
}

/// UI subscriptions are deliberately absent: navigation and minimization are
/// not lifecycle transitions. Each invalidation revokes every old callback.
#[derive(Debug, Default)]
pub struct SessionMachine {
    pub lifecycle: Lifecycle,
    pub epoch: u64,
    pub source_cursor: Option<SourceCursor>,
    pub turn: Option<TurnToken>,
    next_turn: u64,
}

impl SessionMachine {
    pub fn starting(&mut self) -> Result<(), String> {
        if self.lifecycle != Lifecycle::Off {
            return Err("companion_active".into());
        }
        self.invalidate();
        self.lifecycle = Lifecycle::Starting;
        Ok(())
    }

    pub fn activate(&mut self, cursor: Option<SourceCursor>) -> Result<(), String> {
        if !matches!(self.lifecycle, Lifecycle::Starting | Lifecycle::Paused) {
            return Err("invalid_session_state".into());
        }
        self.invalidate();
        self.source_cursor = cursor;
        self.lifecycle = Lifecycle::Active;
        Ok(())
    }

    pub fn pause(&mut self) -> Result<(), String> {
        if !matches!(self.lifecycle, Lifecycle::Active | Lifecycle::Paused) {
            return Err("invalid_session_state".into());
        }
        self.invalidate();
        self.lifecycle = Lifecycle::Paused;
        Ok(())
    }

    pub fn ending(&mut self) {
        self.invalidate();
        self.lifecycle = Lifecycle::Ending;
    }

    pub fn off(&mut self) {
        self.invalidate();
        self.source_cursor = None;
        self.lifecycle = Lifecycle::Off;
    }

    pub fn begin_turn(&mut self) -> Result<TurnToken, String> {
        if self.lifecycle != Lifecycle::Active {
            return Err("companion_paused".into());
        }
        if self.turn.is_some() {
            return Err("chat_busy".into());
        }
        self.next_turn += 1;
        let token = TurnToken {
            epoch: self.epoch,
            turn: self.next_turn,
        };
        self.turn = Some(token);
        Ok(token)
    }

    pub fn accepts(&self, token: TurnToken) -> bool {
        self.lifecycle == Lifecycle::Active && self.turn == Some(token) && self.epoch == token.epoch
    }

    pub fn finish_turn(&mut self, token: TurnToken) {
        if self.accepts(token) {
            self.turn = None;
        }
    }

    pub fn cancel_turn(&mut self) {
        self.invalidate();
    }

    fn invalidate(&mut self) {
        self.epoch += 1;
        self.turn = None;
    }
}
