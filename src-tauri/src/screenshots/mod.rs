//! Reader-captured screenshots stored beside the database (separate from disposable `vndb::screenshots`).
//! Excluded from folder sync (paths are device-local); inlined in library backup exports.

use std::path::{Path, PathBuf};

#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(windows)]
pub mod windows;

/// Longest edge written for a capture.
///
/// A 4K game window is 3840 px wide and would cost several MB per shot as JPEG.
/// The library shows these at a few hundred px and the viewer at screen size, so
/// this bounds the file without visible loss at either.
pub const MAX_EDGE: u32 = 1920;

/// JPEG quality for stored captures. High enough that text in a visual novel
/// stays crisp, which is the whole reason someone screenshots a VN.
const JPEG_QUALITY: u8 = 88;

/// Only the Windows capture path constructs these today; Linux has no capture
/// yet, so the variants are declared and unused there rather than absent.
#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Debug, thiserror::Error)]
pub enum ScreenshotError {
    /// This platform has no capture path, or it is not available right now.
    #[error("capture is not available on this platform")]
    Unsupported,
    /// The game's window is gone, minimised, or replaced.
    #[error("the game window could not be captured")]
    WindowUnavailable,
    /// The window asked the OS to be excluded from capture.
    #[error("this window is protected from capture")]
    Protected,
    /// The frame pool produced nothing in time.
    #[error("the capture timed out")]
    Timeout,
    /// The frame could not be turned into a JPEG.
    #[error("the image could not be encoded: {0}")]
    Encode(String),
    /// The file could not be written or read.
    #[error("the screenshot file could not be used: {0}")]
    Io(String),
}

/// Where captures live, and how a stored path maps back to a file.
#[derive(Debug, Clone)]
pub struct ScreenshotStore {
    root: PathBuf,
}

impl ScreenshotStore {
    #[must_use]
    pub fn new(data_dir: &Path) -> Self {
        Self {
            root: data_dir.join("screenshots"),
        }
    }

    /// Directory for one game's captures.
    #[must_use]
    pub fn dir(&self, game_id: &str) -> PathBuf {
        self.root.join(sanitise(game_id))
    }

    /// The `screenshots/…`-relative form stored in the database.
    ///
    /// Relative on purpose: the library file moves between devices and machines,
    /// and an absolute path recorded here would resolve to nothing anywhere else.
    #[must_use]
    pub fn relative(game_id: &str, id: &str) -> String {
        format!("screenshots/{}/{}.jpg", sanitise(game_id), sanitise(id))
    }

    /// Resolves a relative capture path inside the capture directory; returns `None` on traversal.
    #[must_use]
    pub fn resolve(&self, relative: &str) -> Option<PathBuf> {
        let rest = relative.strip_prefix("screenshots/")?;
        let mut parts = rest.split('/');
        let game = parts.next()?;
        let file = parts.next()?;
        if parts.next().is_some() {
            return None;
        }
        for part in [game, file] {
            if part.is_empty()
                || part.contains('\\')
                || part.contains("..")
                || part.contains(':')
                || part == "."
            {
                return None;
            }
        }
        Some(self.root.join(game).join(file))
    }

    /// Write one encoded capture and return its relative path.
    ///
    /// # Errors
    /// Fails when the directory or the file cannot be written.
    pub async fn write(
        &self,
        game_id: &str,
        id: &str,
        bytes: &[u8],
    ) -> Result<String, ScreenshotError> {
        let dir = self.dir(game_id);
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(|err| ScreenshotError::Io(err.to_string()))?;
        let relative = Self::relative(game_id, id);
        let path = self
            .resolve(&relative)
            .ok_or_else(|| ScreenshotError::Io("the path could not be resolved".to_owned()))?;
        tokio::fs::write(&path, bytes)
            .await
            .map_err(|err| ScreenshotError::Io(err.to_string()))?;
        Ok(relative)
    }

    /// Read a stored capture.
    ///
    /// # Errors
    /// Fails when the path is not a capture path or the file cannot be read.
    pub async fn read(&self, relative: &str) -> Result<Vec<u8>, ScreenshotError> {
        let path = self
            .resolve(relative)
            .ok_or_else(|| ScreenshotError::Io("the path is not a capture".to_owned()))?;
        tokio::fs::read(&path)
            .await
            .map_err(|err| ScreenshotError::Io(err.to_string()))
    }

    /// Remove one capture's file. A missing file is not an error: the row is the
    /// source of truth, and clearing a shot whose file was removed by hand must
    /// still succeed.
    pub async fn remove(&self, relative: &str) {
        if let Some(path) = self.resolve(relative) {
            let _ = tokio::fs::remove_file(path).await;
        }
    }
}

/// Keep one path segment to something a filesystem accepts on every platform.
fn sanitise(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

/// One captured frame, decoded and ready to encode.
pub struct Captured {
    pub width: u32,
    pub height: u32,
    /// RGBA, row-major, `width * height * 4` bytes.
    pub rgba: Vec<u8>,
    /// `windows_window` or `android_screen`.
    pub source: &'static str,
}

impl std::fmt::Debug for Captured {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Dimensions only: the buffer is a whole frame, and printing it would
        // put several megabytes into a log line.
        f.debug_struct("Captured")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("bytes", &self.rgba.len())
            .field("source", &self.source)
            .finish()
    }
}

/// Encode a captured frame as a bounded JPEG.
///
/// # Errors
/// Fails when the buffer size disagrees with the dimensions, or the encoder
/// rejects the image.
pub fn encode_jpeg(frame: &Captured) -> Result<Vec<u8>, ScreenshotError> {
    let expected = (frame.width as usize)
        .checked_mul(frame.height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| ScreenshotError::Encode("the frame size overflowed".to_owned()))?;
    if frame.rgba.len() != expected {
        return Err(ScreenshotError::Encode(
            "the frame buffer does not match its dimensions".to_owned(),
        ));
    }
    let image = image::RgbaImage::from_raw(frame.width, frame.height, frame.rgba.clone())
        .ok_or_else(|| ScreenshotError::Encode("the frame buffer is not an image".to_owned()))?;
    let decoded = image::DynamicImage::ImageRgba8(image);
    let bounded = if decoded.width() > MAX_EDGE || decoded.height() > MAX_EDGE {
        decoded.resize(MAX_EDGE, MAX_EDGE, image::imageops::FilterType::Lanczos3)
    } else {
        decoded
    };
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, JPEG_QUALITY)
        .encode_image(&bounded.to_rgb8())
        .map_err(|err| ScreenshotError::Encode(err.to_string()))?;
    Ok(out)
}

/// A stored capture, as the UI needs it.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct ScreenshotRow {
    pub id: String,
    pub game_id: String,
    pub relative_path: String,
    pub width: i64,
    pub height: i64,
    pub captured_at: i64,
    pub source: String,
    pub caption: String,
}

/// Insert one capture.
///
/// # Errors
/// Database failures.
pub async fn insert(pool: &sqlx::SqlitePool, row: &ScreenshotRow) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO game_screenshots \
         (id, game_id, relative_path, width, height, captured_at, source, caption, deleted) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0)",
    )
    .bind(&row.id)
    .bind(&row.game_id)
    .bind(&row.relative_path)
    .bind(row.width)
    .bind(row.height)
    .bind(row.captured_at)
    .bind(&row.source)
    .bind(&row.caption)
    .execute(pool)
    .await?;
    Ok(())
}

/// Captures for one game, newest first.
///
/// Joins `games` and excludes deleted rows: a screenshot whose game is gone must
/// not appear in any list (AGENTS.md §5).
///
/// # Errors
/// Database failures.
pub async fn list(
    pool: &sqlx::SqlitePool,
    game_id: &str,
) -> Result<Vec<ScreenshotRow>, sqlx::Error> {
    sqlx::query_as::<_, ScreenshotRow>(
        "SELECT s.id, s.game_id, s.relative_path, s.width, s.height, s.captured_at, \
                s.source, s.caption \
         FROM game_screenshots s \
         JOIN games g ON g.id = s.game_id AND g.deleted = 0 \
         WHERE s.game_id = ?1 AND s.deleted = 0 \
         ORDER BY s.captured_at DESC, s.id DESC",
    )
    .bind(game_id)
    .fetch_all(pool)
    .await
}

/// A capture with the game it belongs to, for the cross-game library.
#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct ScreenshotFeedRow {
    pub id: String,
    pub game_id: String,
    pub game_title: String,
    pub width: i64,
    pub height: i64,
    pub captured_at: i64,
    pub source: String,
    pub caption: String,
}

/// Every live capture across the library, newest first.
///
/// Joins `games` and excludes deleted rows on both sides, for the same reason
/// [`list`] does: a shot whose game is gone must not appear anywhere.
///
/// # Errors
/// Database failures.
pub async fn list_all(
    pool: &sqlx::SqlitePool,
    limit: i64,
) -> Result<Vec<ScreenshotFeedRow>, sqlx::Error> {
    sqlx::query_as::<_, ScreenshotFeedRow>(
        "SELECT s.id, s.game_id, g.title AS game_title, s.width, s.height,                 s.captured_at, s.source, s.caption          FROM game_screenshots s          JOIN games g ON g.id = s.game_id AND g.deleted = 0          WHERE s.deleted = 0          ORDER BY s.captured_at DESC, s.id DESC          LIMIT ?1",
    )
    .bind(limit)
    .fetch_all(pool)
    .await
}

/// One capture by id, live only.
///
/// # Errors
/// Database failures.
pub async fn get(pool: &sqlx::SqlitePool, id: &str) -> Result<Option<ScreenshotRow>, sqlx::Error> {
    sqlx::query_as::<_, ScreenshotRow>(
        "SELECT s.id, s.game_id, s.relative_path, s.width, s.height, s.captured_at, \
                s.source, s.caption \
         FROM game_screenshots s \
         JOIN games g ON g.id = s.game_id AND g.deleted = 0 \
         WHERE s.id = ?1 AND s.deleted = 0",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
}

/// Tombstone one capture.
///
/// # Errors
/// Database failures.
pub async fn soft_delete(pool: &sqlx::SqlitePool, id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE game_screenshots SET deleted = 1 WHERE id = ?1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests;
