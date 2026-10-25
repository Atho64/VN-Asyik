//! Tauri commands. Thin wrappers: validation and rules live in `vn-core`.

pub mod ai;
pub mod backup;
pub mod chat;
pub mod data;
pub mod hook;
pub mod launch;
pub mod library;
pub mod luna;
pub mod prefs;
pub mod presence;
pub mod script;
pub mod sync;
pub mod system;
pub mod tracking;
pub mod vndb;
pub mod vndb_sync;

use serde::Serialize;

/// Error shape returned to the frontend.
///
/// Commands return this instead of raw errors so the UI can show a friendly
/// message while the detail stays in the log.
#[derive(Debug, Serialize)]
pub struct CommandError {
    /// Stable machine-readable code.
    pub code: &'static str,
    /// Human-readable message, already safe to display.
    pub message: String,
}

impl CommandError {
    /// Build an error from a stable code and a display message.
    #[must_use]
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl From<crate::db::DbError> for CommandError {
    fn from(err: crate::db::DbError) -> Self {
        Self::new("db", err.to_string())
    }
}

impl From<vn_core::model::ModelError> for CommandError {
    fn from(err: vn_core::model::ModelError) -> Self {
        Self::new("invalid", err.to_string())
    }
}

/// The one place a VNDB failure becomes a code the UI can branch on.
///
/// Both command modules used to carry their own identical copy of this table, so a
/// new variant changed behaviour in whichever file was edited.
impl From<&crate::vndb::VndbError> for CommandError {
    fn from(err: &crate::vndb::VndbError) -> Self {
        use crate::vndb::VndbError;
        let code = match err {
            VndbError::Offline(_) => "offline",
            // Permanent, not retryable: the UI offers "open on VNDB" instead of a
            // retry button for an entry that no longer exists.
            VndbError::NotFound => "not_found",
            VndbError::Throttled => "throttled",
            VndbError::Unauthorized => "unauthorized",
            VndbError::BadRequest(_) | VndbError::TooMuchData => "bad_query",
            VndbError::Server { .. } => "server",
            VndbError::Decode(_) => "decode",
        };
        Self::new(code, err.to_string())
    }
}

/// Alias used by command signatures.
pub type CommandResult<T> = Result<T, CommandError>;
