//! Storing the window-title parsing choice.
//!
//! Two strings in `sync_state`: this is configuration, not data, so it does not
//! need a table, a device id, or a migration.

use sqlx::SqlitePool;

use vn_core::title_settings::{TitleFormat, TitleSettings};

/// Setting key for the format.
const FORMAT_KEY: &str = "title.format";
/// Setting key for the custom pattern.
const PATTERN_KEY: &str = "title.pattern";

/// Read the stored settings, falling back to the verified default.
pub async fn load(pool: &SqlitePool) -> TitleSettings {
    let format = read(pool, FORMAT_KEY).await;
    let pattern = read(pool, PATTERN_KEY).await;
    match format {
        Some(value) => TitleSettings {
            format: TitleFormat::parse(&value),
            pattern,
        },
        // Nothing stored: the app has never been configured, so use what it
        // shipped with rather than reporting no format.
        None => TitleSettings {
            pattern,
            ..TitleSettings::default()
        },
    }
}

/// Save settings, clearing the pattern when it is absent.
///
/// An empty pattern is deleted rather than stored as "": keeping an empty string
/// would make `Some("")` mean "custom with no pattern" instead of "none".
pub async fn save(pool: &SqlitePool, settings: &TitleSettings) {
    let _ = sqlx::query(
        "INSERT INTO sync_state (key, value) VALUES (?, ?)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(FORMAT_KEY)
    .bind(settings.format.as_str())
    .execute(pool)
    .await;

    match settings.pattern.as_deref().map(str::trim) {
        Some(pattern) if !pattern.is_empty() => {
            let _ = sqlx::query(
                "INSERT INTO sync_state (key, value) VALUES (?, ?)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            )
            .bind(PATTERN_KEY)
            .bind(pattern)
            .execute(pool)
            .await;
        }
        _ => {
            let _ = sqlx::query("DELETE FROM sync_state WHERE key = ?")
                .bind(PATTERN_KEY)
                .execute(pool)
                .await;
        }
    }
}

async fn read(pool: &SqlitePool, key: &str) -> Option<String> {
    let found: Result<Option<(String,)>, _> =
        sqlx::query_as("SELECT value FROM sync_state WHERE key = ?")
            .bind(key)
            .fetch_optional(pool)
            .await;
    found.ok().flatten().map(|(value,)| value)
}
