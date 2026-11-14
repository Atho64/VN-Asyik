//! Appearance commands: accent color, background image (`data:` URL), and theme state stored in `sync_state`.

use serde::{Deserialize, Serialize};
use tauri::State;

use super::{CommandError, CommandResult};
use crate::state::AppState;

/// Key the record lives under in `sync_state`.
const KEY: &str = "ui.appearance";
/// Largest background accepted, in bytes. A wallpaper is not a library.
const MAX_BYTES: u64 = 8 * 1024 * 1024;
/// Marker for a theme file, so a library export is not mistaken for one.
const THEME_FORMAT: &str = "vn-asyik-theme";

/// The tokens a theme owns.
///
/// Mirrors `ThemeTokens` in `src/lib/theme.ts`. Rust owns the *shape* and the
/// ranges; the contrast maths stays in TypeScript, where the palette helpers
/// already live and where the theme is actually resolved.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThemeTokens {
    pub bg: String,
    pub bg_elevated: String,
    pub surface_solid: String,
    pub text: String,
    pub text_muted: String,
    pub text_faint: String,
    /// `sharp` | `soft` | `round`.
    pub radius: String,
    /// `compact` | `comfortable`.
    pub density: String,
    /// 0..=1.
    pub glass: f64,
}

/// A named set of token overrides.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThemeView {
    pub id: String,
    pub name: String,
    /// `dark` | `light`. A theme carries its own mode.
    pub base: String,
    pub tokens: ThemeTokens,
}

/// A theme file on disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThemeFile {
    format: String,
    version: u32,
    theme: ThemeView,
}

fn default_true() -> bool {
    true
}

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
    /// The chosen theme, or absent for the built-in dark/light/system ramp.
    pub theme: Option<ThemeView>,
    /// Whether a focused game may tint the interface at all.
    pub game_tint: bool,
}

impl Default for AppearanceView {
    fn default() -> Self {
        Self {
            accent: None,
            background_name: None,
            background: None,
            background_dim: 0.45,
            theme: None,
            game_tint: true,
        }
    }
}

/// The stored part: the path, not the bytes.
///
/// Every field defaults, so a row written before themes existed still reads and
/// an older build ignores the new ones instead of failing to parse.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Stored {
    accent: Option<String>,
    background_path: Option<String>,
    background_dim: f64,
    theme: Option<ThemeView>,
    #[serde(default = "default_true")]
    game_tint: bool,
}

impl Default for Stored {
    fn default() -> Self {
        Self {
            accent: None,
            background_path: None,
            background_dim: 0.45,
            theme: None,
            game_tint: true,
        }
    }
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
        theme: stored.theme.clone(),
        game_tint: stored.game_tint,
    })
}

/// Everything one save can change.
///
/// One struct rather than eight parameters: the command grew a theme, a clear
/// flag and a tint switch, and a positional call with eight arguments is a
/// mis-send waiting to happen.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppearanceInput {
    pub accent: Option<String>,
    pub background_path: Option<String>,
    pub clear_background: bool,
    pub background_dim: f64,
    pub theme: Option<ThemeView>,
    pub clear_theme: bool,
    pub game_tint: bool,
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
    input: AppearanceInput,
) -> CommandResult<AppearanceView> {
    let pool = state.db().pool();
    let mut stored = load(pool).await;
    stored.accent = match input.accent.as_deref().map(str::trim) {
        Some("") | None => None,
        Some(value) => Some(validate_hex(value)?),
    };
    if input.clear_background {
        stored.background_path = None;
    } else if let Some(path) = input
        .background_path
        .as_deref()
        .filter(|path| !path.is_empty())
    {
        // Read it now: a path that cannot be read is not worth saving, and the
        // user should hear that while the picker is still open in their mind.
        read_image(path)?;
        stored.background_path = Some(path.to_owned());
    }
    if input.clear_theme {
        stored.theme = None;
    } else if let Some(chosen) = input.theme.as_ref() {
        stored.theme = Some(validate_theme(chosen)?);
    }
    stored.game_tint = input.game_tint;
    stored.background_dim = input.background_dim.clamp(0.0, 0.9);
    store(pool, &stored).await?;
    appearance_get(state).await
}

/// Save the given theme as a shareable file.
///
/// # Errors
/// `invalid_theme` for a theme that fails validation, or a document failure.
#[tauri::command]
pub async fn theme_export_file(app: tauri::AppHandle, theme: ThemeView) -> CommandResult<bool> {
    let theme = validate_theme(&theme)?;
    let contents = serde_json::to_string_pretty(&ThemeFile {
        format: THEME_FORMAT.to_owned(),
        version: 1,
        theme,
    })
    .map_err(|err| CommandError::new("document", err.to_string()))?;
    crate::platform::storage::save_document(app, "vn-asyik-theme.json".to_owned(), contents).await
}

/// Read a theme file, validating it exactly as a hand-edited one would be.
///
/// Returns `None` when the picker was dismissed.
///
/// # Errors
/// `invalid_theme` when the file is not a theme, or a document failure.
#[tauri::command]
pub async fn theme_import_file(app: tauri::AppHandle) -> CommandResult<Option<ThemeView>> {
    let Some(contents) = crate::platform::storage::open_document(app).await? else {
        return Ok(None);
    };
    let file: ThemeFile = serde_json::from_str(&contents)
        .map_err(|_| CommandError::new("invalid_theme", "that file is not a VN Asyik theme"))?;
    if file.format != THEME_FORMAT {
        return Err(CommandError::new(
            "invalid_theme",
            "that file is not a VN Asyik theme",
        ));
    }
    validate_theme(&file.theme).map(Some)
}

/// Accept only a theme whose every field is usable.
///
/// The frontend clamps text for contrast; this rejects what cannot be clamped:
/// a malformed colour, an unknown enum, a strength outside its range. A theme
/// file arrives from another machine, so it is data, not code.
fn validate_theme(theme: &ThemeView) -> Result<ThemeView, CommandError> {
    let id = theme.id.trim();
    if id.is_empty() || id.chars().count() > 120 {
        return Err(CommandError::new(
            "invalid_theme",
            "a theme needs an id of at most 120 characters",
        ));
    }
    let name = theme.name.trim();
    if name.is_empty() || name.chars().count() > 60 {
        return Err(CommandError::new(
            "invalid_theme",
            "a theme needs a name of at most 60 characters",
        ));
    }
    if theme.base != "dark" && theme.base != "light" {
        return Err(CommandError::new(
            "invalid_theme",
            "a theme's base must be dark or light",
        ));
    }
    let tokens = &theme.tokens;
    let colours = [
        ("background", &tokens.bg),
        ("elevated surface", &tokens.bg_elevated),
        ("solid surface", &tokens.surface_solid),
        ("text", &tokens.text),
        ("muted text", &tokens.text_muted),
        ("faint text", &tokens.text_faint),
    ];
    for (label, value) in colours {
        validate_hex(value).map_err(|_| {
            CommandError::new("invalid_theme", format!("{label} must be a #rrggbb colour"))
        })?;
    }
    if !matches!(tokens.radius.as_str(), "sharp" | "soft" | "round") {
        return Err(CommandError::new("invalid_theme", "unknown corner style"));
    }
    if !matches!(tokens.density.as_str(), "compact" | "comfortable") {
        return Err(CommandError::new("invalid_theme", "unknown density"));
    }
    if tokens.glass.is_nan() || !(0.0..=1.0).contains(&tokens.glass) {
        return Err(CommandError::new(
            "invalid_theme",
            "glass strength must be between 0 and 1",
        ));
    }
    // Normalise what is storable: lower-case hex, trimmed text.
    let hex = |value: &str| validate_hex(value).expect("checked above");
    Ok(ThemeView {
        id: id.to_owned(),
        name: name.to_owned(),
        base: theme.base.clone(),
        tokens: ThemeTokens {
            bg: hex(&tokens.bg),
            bg_elevated: hex(&tokens.bg_elevated),
            surface_solid: hex(&tokens.surface_solid),
            text: hex(&tokens.text),
            text_muted: hex(&tokens.text_muted),
            text_faint: hex(&tokens.text_faint),
            radius: tokens.radius.clone(),
            density: tokens.density.clone(),
            glass: tokens.glass,
        },
    })
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

    /// A row written by 0.2.3 has no theme and no tint switch. Reading it must
    /// keep the old behaviour, which means tinting on and no theme, not a parse
    /// failure that would drop the reader's accent along with it.
    #[test]
    fn a_record_written_before_themes_existed_still_reads() {
        let stored: Stored = serde_json::from_str(
            "{\"accent\":\"#3bd1a4\",\"backgroundPath\":null,\"backgroundDim\":0.6}",
        )
        .expect("json");
        assert_eq!(stored.accent.as_deref(), Some("#3bd1a4"));
        assert!(stored.theme.is_none());
        assert!(
            stored.game_tint,
            "tinting defaults on, as it behaved before"
        );
    }

    fn theme(tokens: ThemeTokens) -> ThemeView {
        ThemeView {
            id: "custom:test".to_owned(),
            name: "Test".to_owned(),
            base: "dark".to_owned(),
            tokens,
        }
    }

    fn tokens() -> ThemeTokens {
        ThemeTokens {
            bg: "#101719".to_owned(),
            bg_elevated: "#182225".to_owned(),
            surface_solid: "#202D30".to_owned(),
            text: "#EDF7F4".to_owned(),
            text_muted: "#b6c9c4".to_owned(),
            text_faint: "#91aaa4".to_owned(),
            radius: "soft".to_owned(),
            density: "comfortable".to_owned(),
            glass: 1.0,
        }
    }

    #[test]
    fn a_valid_theme_is_normalised_to_lower_case_hex() {
        let checked = validate_theme(&theme(tokens())).expect("valid");
        assert_eq!(checked.tokens.surface_solid, "#202d30");
        assert_eq!(checked.tokens.text, "#edf7f4");
    }

    #[test]
    fn a_theme_with_a_colour_that_is_not_a_colour_is_refused() {
        let mut bad = tokens();
        bad.text = "red".to_owned();
        assert_eq!(
            validate_theme(&theme(bad)).unwrap_err().code,
            "invalid_theme"
        );
    }

    #[test]
    fn a_theme_with_an_unknown_enum_or_out_of_range_strength_is_refused() {
        let mut radius = tokens();
        radius.radius = "squircle".to_owned();
        assert!(validate_theme(&theme(radius)).is_err());

        let mut density = tokens();
        density.density = "airy".to_owned();
        assert!(validate_theme(&theme(density)).is_err());

        let mut glass = tokens();
        glass.glass = 1.5;
        assert!(validate_theme(&theme(glass)).is_err());

        let mut base = theme(tokens());
        base.base = "sepia".to_owned();
        assert!(validate_theme(&base).is_err());
    }

    #[test]
    fn a_theme_without_a_name_or_id_is_refused() {
        let mut unnamed = theme(tokens());
        unnamed.name = "   ".to_owned();
        assert!(validate_theme(&unnamed).is_err());

        let mut anonymous = theme(tokens());
        anonymous.id = String::new();
        assert!(validate_theme(&anonymous).is_err());
    }

    #[test]
    fn a_theme_file_round_trips_through_json() {
        let file = ThemeFile {
            format: THEME_FORMAT.to_owned(),
            version: 1,
            theme: theme(tokens()),
        };
        let json = serde_json::to_string(&file).expect("serialise");
        let parsed: ThemeFile = serde_json::from_str(&json).expect("parse");
        assert_eq!(parsed.theme.tokens.bg_elevated, "#182225");
    }
}
