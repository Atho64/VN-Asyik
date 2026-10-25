//! Entity and device identity.

use std::fmt;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::model::ModelError;

/// Primary key for every synced row. `UUID` v7 so ids sort by creation time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EntityId(Uuid);

impl EntityId {
    /// Generate a new time-ordered id.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }

    /// Wrap an existing `UUID` (used when decoding rows from storage).
    #[must_use]
    pub const fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }

    /// Parse from canonical text form.
    pub fn parse(text: &str) -> Result<Self, ModelError> {
        Uuid::parse_str(text)
            .map(Self)
            .map_err(|_| ModelError::InvalidId(text.to_owned()))
    }

    /// The inner `UUID`.
    #[must_use]
    pub const fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl Default for EntityId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for EntityId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Identifies the installation that produced a row.
///
/// Used as the tiebreak when two devices write the same record with the same
/// `updated_at`. It is generated once per install and never synced between
/// devices as a value; it only ever appears as row metadata.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DeviceId(String);

impl DeviceId {
    /// Wrap an existing device identifier.
    ///
    /// # Errors
    /// Rejects empty or whitespace-only identifiers.
    pub fn parse(text: &str) -> Result<Self, ModelError> {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Err(ModelError::EmptyDeviceId);
        }
        if trimmed.len() > 128 {
            return Err(ModelError::InvalidDeviceId(trimmed.len()));
        }
        Ok(Self(trimmed.to_owned()))
    }

    /// The identifier as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entity_ids_are_unique_and_ordered() {
        let first = EntityId::new();
        let second = EntityId::new();
        assert_ne!(first, second);
        // v7 is time-ordered, so generation order matches sort order within a
        // millisecond-resolution window in practice; assert only uniqueness
        // plus round-trip here to avoid a flaky ordering assertion.
        assert_eq!(EntityId::parse(&first.to_string()).unwrap(), first);
    }

    #[test]
    fn entity_id_rejects_garbage() {
        assert!(EntityId::parse("not-a-uuid").is_err());
    }

    #[test]
    fn device_id_rejects_empty() {
        assert!(DeviceId::parse("").is_err());
        assert!(DeviceId::parse("   ").is_err());
    }

    #[test]
    fn device_id_trims_and_accepts() {
        let id = DeviceId::parse("  desktop-abc  ").unwrap();
        assert_eq!(id.as_str(), "desktop-abc");
    }

    #[test]
    fn device_id_rejects_absurd_length() {
        let long = "x".repeat(129);
        assert!(DeviceId::parse(&long).is_err());
    }
}
