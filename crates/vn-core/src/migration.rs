//! Migration bookkeeping shared by the app and by tests.
//!
//! The SQL itself lives in `src-tauri/src/db/migrations`. This module holds the
//! rules that must not drift: how migration names are ordered, and what schema
//! version a backup file declares.

use serde::{Deserialize, Serialize};

/// Current schema version. Bump with a matching migration and test (`AGENTS.md` §5).
pub const SCHEMA_VERSION: u32 = 37;

/// Highest backup schema version this build reads; newer backups are refused.
pub const MAX_READABLE_SCHEMA: u32 = SCHEMA_VERSION;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MigrationError {
    #[error("migration name must look like 0001_description.sql, got {0:?}")]
    BadName(String),
    #[error("backup schema {found} is newer than supported {max}")]
    BackupTooNew { found: u32, max: u32 },
    #[error("backup schema version must be at least 1")]
    ZeroVersion,
}

/// Parse the numeric prefix of a `<number>_<name>.sql` migration filename.
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

/// Validate that migration versions are strictly increasing (positive, no duplicates).
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

/// Header of a backup file, checked before reading any data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupHeader {
    /// Backup payload format, distinct from `schema_version`.
    pub format: u32,
    pub schema_version: u32,
    /// UTC ms.
    pub created_at_ms: i64,
    pub app_version: String,
}

pub const BACKUP_FORMAT: u32 = 1;

impl BackupHeader {
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
