//! Playtime tracking modes: `Auto` (focused window + OS idle timer) vs `Always` (wall clock while running).

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

/// Setting key holding the mode.
const MODE_KEY: &str = "tracking.mode";

/// How detected playtime is measured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackingMode {
    /// Focus-aware. Time counts while the game window is in front; idle stretches
    /// past the grace threshold are filed as away (or folded into play, when the
    /// user turned AFK exclusion off).
    Auto,
    /// Wall clock. Every second the game process is running counts as play, and
    /// the run ends when the process exits.
    Always,
}

impl TrackingMode {
    /// The stored string form.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Always => "always",
        }
    }

    /// Parse the stored form, falling back to `Auto`.
    ///
    /// Falling back to `Auto` is deliberate: it is the measurement that cannot
    /// overstate playtime, and an unknown value means this build does not know what
    /// the user chose.
    #[must_use]
    pub fn parse(value: &str) -> Self {
        match value {
            "always" => Self::Always,
            _ => Self::Auto,
        }
    }
}

/// Read the stored mode, defaulting to automatic.
pub async fn load(pool: &SqlitePool) -> TrackingMode {
    let found: Result<Option<(String,)>, _> =
        sqlx::query_as("SELECT value FROM sync_state WHERE key = ?")
            .bind(MODE_KEY)
            .fetch_optional(pool)
            .await;
    found
        .ok()
        .flatten()
        .map_or(TrackingMode::Auto, |(value,)| TrackingMode::parse(&value))
}

/// Store the mode. Configuration, not data: this device's choice, like the title
/// format, so it lives in `sync_state` rather than a synced table.
pub async fn save(pool: &SqlitePool, mode: TrackingMode) {
    let _ = sqlx::query(
        "INSERT INTO sync_state (key, value) VALUES (?, ?)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(MODE_KEY)
    .bind(mode.as_str())
    .execute(pool)
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strings_round_trip() {
        for mode in [TrackingMode::Auto, TrackingMode::Always] {
            assert_eq!(TrackingMode::parse(mode.as_str()), mode);
        }
    }

    #[test]
    fn an_unknown_value_falls_back_to_the_honest_mode() {
        // `Always` can only overstate playtime; guessing it would be a silent
        // regression, so anything unrecognised means focus-aware.
        assert_eq!(TrackingMode::parse("wat"), TrackingMode::Auto);
        assert_eq!(TrackingMode::parse(""), TrackingMode::Auto);
    }
}
