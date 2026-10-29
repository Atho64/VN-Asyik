//! Appearance: the accent colour and the background image.
//!
//! Two preferences that are neither theme mode nor content policy, so they live in
//! their own small record instead of widening the prefs snapshot every backup
//! carries. The image is returned as a data URL, the same way a custom cover is:
//! the webview has no filesystem access, and opening one would grant it more than
//! this feature needs.

use serde::{Deserialize, Serialize};
use tauri::State;

use super::{CommandError, CommandResult};
use crate::state::AppState;

/// Key the record lives under in `sync_state`.
const KEY: &str = "ui.appearance";
/// Largest background accepted, in bytes. A wallpaper is not a library.
const MAX_BYTES: u64 = 8 * 1024 * 1024;

/// Encoder for the data URL the webview loads. Imported through the trait below
/// because `encode` is a trait method, not an inherent one.
use base64::Engine as _;

/// Encoder for the data URL the webview loads.
const BASE64: base64::engine::general_purpose::GeneralPurpose =
    base64::engine::general_purpose::STANDARD;

/// What the UI applies.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppearanceView {
    /// `#rrggbb`, or absent for the theme's own accent.
    pub accent: Option<String>,
    /// File name of the chosen image, for the settings row.
    pub background_name: Option<String>,
    /// The image as a data URL, ready for `background-image`.
    pub background: Option<String>,
    /// How much the scrim darkens it, 0.0 to 0.9.
    pub background_dim: f64,
}

impl Default for AppearanceView {
    fn default() -> Self {
        Self {
            accent: None,
            background_name: None,
            background: None,
            background_dim: 0.45,
        }
    }
}

/// The stored part: the path, not the bytes.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Stored {
    accent: Option<String>,
    background_path: Option<String>,
    background_dim: f64,
}

impl Stored {
    fn dim(&self) -> f64 {
        if self.background_dim == 0.0 {
            0.45
        } else {
            self.background_dim
        }
    }
}

/// One row of the settings screen, before the image is read.
async fn load(pool: &sqlx::SqlitePool) -> Stored {
    let row: Result<Option<(String,)>, _> =
        sqlx::query_as("SELECT value FROM sync_state WHERE key = ?1")
            .bind(KEY)
            .fetch_optional(pool)
            .await;
    match row {
        Ok(Some((json,))) => serde_json::from_str::<Stored>(&json).unwrap_or_default(),
        _ => Stored::default(),
    }
}

async fn store(pool: &sqlx::SqlitePool, stored: &Stored) -> Result<(), crate::db::DbError> {
    let json = serde_json::to_string(stored)
        .map_err(|err| crate::db::DbError::Corrupt(err.to_string()))?;
    sqlx::query(
        "INSERT INTO sync_state (key, value) VALUES (?1, ?2) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(KEY)
    .bind(json)
    .execute(pool)
    .await?;
    Ok(())
}

/// Read the background file as a data URL.
///
/// # Errors
/// `unsupported_image` for anything that is not a readable image under the size
/// cap. The message carries the reason, because "it did not work" is useless here.
fn read_image(path: &str) -> Result<(String, String), CommandError> {
    let file = std::path::Path::new(path);
    let name = file
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| CommandError::new("unsupported_image", "that is not a file"))?;
    let extension = file
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    let mime = match extension.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "gif" => "image/gif",
        _ => {
            return Err(CommandError::new(
                "unsupported_image",
                "use a JPG, PNG, WebP, AVIF or GIF image",
            ));
        }
    };
    let meta = std::fs::metadata(file)
        .map_err(|_| CommandError::new("unsupported_image", "the file could not be read"))?;
    if !meta.is_file() {
        return Err(CommandError::new("unsupported_image", "that is not a file"));
    }
    if meta.len() > MAX_BYTES {
        return Err(CommandError::new(
            "unsupported_image",
            "the image is larger than 8 MB",
        ));
    }
    let bytes = std::fs::read(file)
        .map_err(|_| CommandError::new("unsupported_image", "the file could not be read"))?;
    let encoded = BASE64.encode(bytes);
    Ok((format!("data:{mime};base64,{encoded}"), name.to_owned()))
}

/// The current appearance.
///
/// # Errors
/// Database failures.
#[tauri::command]
pub async fn appearance_get(state: State<'_, AppState>) -> CommandResult<AppearanceView> {
    let stored = load(state.db().pool()).await;
    let image = stored
        .background_path
        .as_deref()
        .and_then(|path| read_image(path).ok());
    Ok(AppearanceView {
        accent: stored.accent.clone(),
        background_name: image.as_ref().map(|(_, name)| name.clone()),
        background: image.map(|(url, _)| url),
        background_dim: stored.dim(),
    })
}

/// Save the accent colour, the background image and the scrim strength.
///
/// `clear_background` removes the image; `background_path` replaces it. Both
/// absent leaves the current one alone, so changing the accent does not re-read a
/// wallpaper.
///
/// # Errors
/// `invalid_accent` for anything but `#rrggbb`, `unsupported_image` for an image
/// that cannot be used, and database failures.
#[tauri::command]
pub async fn appearance_set(
    state: State<'_, AppState>,
    accent: Option<String>,
    background_path: Option<String>,
    clear_background: bool,
    background_dim: f64,
) -> CommandResult<AppearanceView> {
    let pool = state.db().pool();
    let mut stored = load(pool).await;
    stored.accent = match accent.as_deref().map(str::trim) {
        Some("") | None => None,
        Some(value) => Some(validate_hex(value)?),
    };
    if clear_background {
        stored.background_path = None;
    } else if let Some(path) = background_path.as_deref().filter(|path| !path.is_empty()) {
        // Read it now: a path that cannot be read is not worth saving, and the
        // user should hear that while the picker is still open in their mind.
        read_image(path)?;
        stored.background_path = Some(path.to_owned());
    }
    stored.background_dim = background_dim.clamp(0.0, 0.9);
    store(pool, &stored).await?;
    appearance_get(state).await
}

/// Accept only `#rrggbb`.
fn validate_hex(value: &str) -> Result<String, CommandError> {
    let hex = value.trim();
    let digits = hex.strip_prefix('#').unwrap_or_default();
    if digits.len() != 6 || !digits.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err(CommandError::new(
            "invalid_accent",
            "the accent must be a #rrggbb colour",
        ));
    }
    Ok(format!("#{}", digits.to_ascii_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_accent_is_a_six_digit_hex_colour() {
        assert_eq!(validate_hex("#3BD1A4").ok().as_deref(), Some("#3bd1a4"));
        assert!(validate_hex("#3bd1a4").is_ok());
        for bad in ["#fff", "3bd1a4", "#12345g", "", "#1234567"] {
            assert!(validate_hex(bad).is_err(), "{bad} should be refused");
        }
    }

    #[test]
    fn an_image_that_is_not_an_image_is_refused_before_it_is_stored() {
        let dir = std::env::temp_dir().join("vn-asyik-appearance-test");
        std::fs::create_dir_all(&dir).expect("temp dir");
        let text = dir.join("notes.txt");
        std::fs::write(&text, b"not an image").expect("write");
        assert_eq!(
            read_image(text.to_str().expect("path")).unwrap_err().code,
            "unsupported_image"
        );
        let image = dir.join("wallpaper.png");
        std::fs::write(&image, [0x89, b'P', b'N', b'G']).expect("write");
        let (url, name) = read_image(image.to_str().expect("path")).expect("image");
        assert_eq!(name, "wallpaper.png");
        assert!(url.starts_with("data:image/png;base64,"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_scrim_defaults_when_a_stored_record_has_no_dimension() {
        let stored: Stored = serde_json::from_str("{\"accent\":\"#3bd1a4\"}").expect("json");
        assert!((stored.dim() - 0.45).abs() < f64::EPSILON);
    }
}
