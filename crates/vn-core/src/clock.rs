//! UTC millisecond timestamps.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::model::ModelError;

/// Milliseconds since the Unix epoch, UTC.
///
/// Stored as `i64` so `SQLite` round-trips it exactly and JS can read it without
/// losing precision. Local time is a display concern only; see `time_fmt`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Timestamp(i64);

impl Timestamp {
    /// The Unix epoch.
    pub const EPOCH: Self = Self(0);

    /// Wrap a raw millisecond value.
    #[must_use]
    pub const fn from_millis(ms: i64) -> Self {
        Self(ms)
    }

    /// Raw milliseconds since the epoch.
    #[must_use]
    pub const fn as_millis(self) -> i64 {
        self.0
    }

    /// Current wall-clock time, or an error if the system clock is out of range.
    pub fn now() -> Result<Self, ModelError> {
        let now = OffsetDateTime::now_utc();
        let ms = i64::try_from(now.unix_timestamp_nanos() / 1_000_000)
            .map_err(|_| ModelError::ClockOutOfRange)?;
        Ok(Self(ms))
    }

    /// True when this timestamp is later than `other`.
    #[must_use]
    pub const fn is_after(self, other: Self) -> bool {
        self.0 > other.0
    }

    /// Signed difference in milliseconds (`self - other`).
    #[must_use]
    pub const fn saturating_diff_ms(self, other: Self) -> i64 {
        self.0.saturating_sub(other.0)
    }
}

impl Default for Timestamp {
    fn default() -> Self {
        Self::EPOCH
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_is_zero() {
        assert_eq!(Timestamp::EPOCH.as_millis(), 0);
        assert_eq!(Timestamp::default(), Timestamp::EPOCH);
    }

    #[test]
    fn ordering_is_numeric() {
        let a = Timestamp::from_millis(1_000);
        let b = Timestamp::from_millis(2_000);
        assert!(b.is_after(a));
        assert!(!a.is_after(b));
        assert_eq!(b.saturating_diff_ms(a), 1_000);
        assert_eq!(a.saturating_diff_ms(b), -1_000);
    }

    #[test]
    fn now_is_after_epoch() {
        let now = Timestamp::now().expect("clock in range");
        assert!(now.is_after(Timestamp::EPOCH));
    }
}
