//! Migration bookkeeping shared by the app and by tests.
//!
//! The SQL itself lives in `src-tauri/src/db/migrations`. This module holds the
//! rules that must not drift: how migration names are ordered, and what schema
//! version a backup file declares.

use serde::{Deserialize, Serialize};

/// Current schema version. Bump whenever a migration is added.
///
/// Rules (AGENTS.md §5): migrations are numbered, applied once, and never edited
/// after they ship. A version bump must come with a migration and a test.
pub const SCHEMA_VERSION: u32 = 28;

/// Maximum schema version this build can read in a backup file.
///
/// Reading a newer backup is refused rather than guessed at.
pub const MAX_READABLE_SCHEMA: u32 = SCHEMA_VERSION;

/// Errors raised while validating migration identifiers or versions.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MigrationError {
    /// The migration file name was not `<number>_<name>.sql`.
    #[error("migration name must look like 0001_description.sql, got {0:?}")]
    BadName(String),
    /// A backup declares a schema newer than this build understands.
    #[error("backup schema {found} is newer than supported {max}")]
    BackupTooNew {
        /// Version found in the file.
        found: u32,
        /// Highest version this build reads.
        max: u32,
    },
    /// A backup declares version 0.
    #[error("backup schema version must be at least 1")]
    ZeroVersion,
}

/// Parse the numeric prefix of a migration file name.
///
/// # Errors
/// Fails when the prefix is missing, non-numeric, or the name does not end in `.sql`.
pub fn migration_version(file_name: &str) -> Result<u32, MigrationError> {
    let Some((prefix, rest)) = file_name.split_once('_') else {
        return Err(MigrationError::BadName(file_name.to_owned()));
    };
    if !rest.to_ascii_lowercase().ends_with(".sql") {
        return Err(MigrationError::BadName(file_name.to_owned()));
    }
    let prefix = prefix.trim();
    if prefix.is_empty() || !prefix.chars().all(|c| c.is_ascii_digit()) {
        return Err(MigrationError::BadName(file_name.to_owned()));
    }
    prefix
        .parse::<u32>()
        .map_err(|_| MigrationError::BadName(file_name.to_owned()))
}

/// Validate that `versions` are non-empty-safe, sorted ascending and gap-free
/// starting at 1. Gaps are allowed only if intentional, so we require strictly
/// increasing and reject duplicates.
///
/// # Errors
/// Returns the offending sequence when duplicates or disorder are found.
pub fn check_ordering(versions: &[u32]) -> Result<(), String> {
    let mut previous = 0;
    for &version in versions {
        if version <= previous {
            return Err(format!(
                "migration versions must be strictly increasing; found {version} after {previous}"
            ));
        }
        previous = version;
    }
    Ok(())
}

/// Header of a backup file, checked before any data is read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupHeader {
    /// Backup payload format. Distinct from the schema version.
    pub format: u32,
    /// Schema version the data was written with.
    pub schema_version: u32,
    /// When the backup was taken, UTC ms.
    pub created_at_ms: i64,
    /// Which build produced it.
    pub app_version: String,
}

/// Current backup format version.
pub const BACKUP_FORMAT: u32 = 1;

impl BackupHeader {
    /// Validate that this build can read the backup.
    ///
    /// # Errors
    /// Fails on version 0 or a schema newer than [`MAX_READABLE_SCHEMA`].
    pub fn validate(&self) -> Result<(), MigrationError> {
        if self.schema_version == 0 {
            return Err(MigrationError::ZeroVersion);
        }
        if self.schema_version > MAX_READABLE_SCHEMA {
            return Err(MigrationError::BackupTooNew {
                found: self.schema_version,
                max: MAX_READABLE_SCHEMA,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_numbered_migrations() {
        assert_eq!(migration_version("0001_init.sql").unwrap(), 1);
        assert_eq!(migration_version("0042_add_endings.sql").unwrap(), 42);
    }

    #[test]
    fn rejects_malformed_migration_names() {
        assert!(migration_version("init.sql").is_err());
        assert!(migration_version("0001_init.txt").is_err());
        assert!(migration_version("abc_init.sql").is_err());
        assert!(migration_version("_init.sql").is_err());
    }

    #[test]
    fn ordering_rejects_duplicates_and_disorder() {
        assert!(check_ordering(&[1, 2, 3]).is_ok());
        assert!(check_ordering(&[]).is_ok());
        assert!(check_ordering(&[1, 1]).is_err());
        assert!(check_ordering(&[2, 1]).is_err());
    }

    #[test]
    fn backup_validation_blocks_future_schemas() {
        let ok = BackupHeader {
            format: BACKUP_FORMAT,
            schema_version: SCHEMA_VERSION,
            created_at_ms: 0,
            app_version: "0.1.0".to_owned(),
        };
        assert!(ok.validate().is_ok());

        let future = BackupHeader {
            schema_version: MAX_READABLE_SCHEMA + 1,
            ..ok.clone()
        };
        assert!(matches!(
            future.validate(),
            Err(MigrationError::BackupTooNew { .. })
        ));

        let zero = BackupHeader {
            schema_version: 0,
            ..ok
        };
        assert_eq!(zero.validate(), Err(MigrationError::ZeroVersion));
    }
}
