//! Reader-captured screenshots: capture, list, view, delete.
//!
//! The capture itself is platform-specific and lives in `crate::screenshots`.
//! This layer only decides *what* to capture (the tracked game), turns the frame
//! into a stored file, and answers the UI.

use serde::Serialize;
use tauri::State;

use super::{CommandError, CommandResult};
use crate::screenshots::{self, ScreenshotRow, ScreenshotStore};
use crate::state::AppState;

/// One capture, as the library lists it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenshotCard {
    pub id: String,
    pub game_id: String,
    pub width: i64,
    pub height: i64,
    pub captured_at: i64,
    /// `windows_window` or `android_screen`.
    pub source: String,
    pub caption: String,
}

impl From<ScreenshotRow> for ScreenshotCard {
    fn from(row: ScreenshotRow) -> Self {
        Self {
            id: row.id,
            game_id: row.game_id,
            width: row.width,
            height: row.height,
            captured_at: row.captured_at,
            source: row.source,
            caption: row.caption,
        }
    }
}

/// Whether the capture button can do anything right now, and if not, why.
///
/// Answered by the backend rather than guessed in the UI: the reason differs per
/// platform (Android needs a projection the reader granted, Windows needs a
/// capturable window) and the button must say which one is missing instead of
/// looking broken.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureStatus {
    /// This build can capture on this platform.
    pub supported: bool,
    /// The game a capture would be attributed to, when one is tracked.
    pub game_id: Option<String>,
    /// Why a capture would fail right now, as a stable code the UI translates.
    pub reason: Option<String>,
}

const UNSUPPORTED: &str = "screenshot_unsupported";
const NO_GAME: &str = "screenshot_no_game";

/// Report what the capture button would do.
///
/// # Errors
/// Never fails today; the signature is a command result so the UI handles one
/// shape for every capture call.
#[tauri::command]
pub async fn screenshot_status(state: State<'_, AppState>) -> CommandResult<CaptureStatus> {
    let game = state.tracker().companion_target().await;
    let supported = cfg!(windows) || cfg!(target_os = "android");
    Ok(CaptureStatus {
        supported,
        game_id: game.as_ref().map(|(game_id, _)| game_id.clone()),
        reason: if !supported {
            Some(UNSUPPORTED.to_owned())
        } else if game.is_none() {
            Some(NO_GAME.to_owned())
        } else {
            None
        },
    })
}

/// Capture one screenshot of the tracked game.
///
/// # Errors
/// `screenshot_no_game` when nothing is tracked, and the capture error code when
/// the frame could not be taken (see `crate::screenshots::ScreenshotError`).
#[tauri::command]
pub async fn screenshot_capture(state: State<'_, AppState>) -> CommandResult<ScreenshotCard> {
    let Some((game_id, pid)) = state.tracker().companion_target().await else {
        return Err(CommandError::new(NO_GAME, "no game is being tracked"));
    };

    let frame = capture_frame(pid).await?;
    let jpeg = screenshots::encode_jpeg(&frame).map_err(|err| capture_error(&err))?;
    let id = vn_core::ids::EntityId::new().to_string();
    let store = ScreenshotStore::new(state.data_dir());
    let relative = store
        .write(&game_id, &id, &jpeg)
        .await
        .map_err(|err| capture_error(&err))?;

    let row = ScreenshotRow {
        id,
        game_id,
        relative_path: relative,
        width: i64::from(frame.width),
        height: i64::from(frame.height),
        captured_at: now_ms()?,
        source: frame.source.to_owned(),
        caption: String::new(),
    };
    screenshots::insert(state.db().pool(), &row)
        .await
        .map_err(|err| CommandError::new("db", err.to_string()))?;
    Ok(row.into())
}

/// Every capture for one game, newest first.
///
/// # Errors
/// Database failures.
#[tauri::command]
pub async fn screenshot_list(
    state: State<'_, AppState>,
    game_id: String,
) -> CommandResult<Vec<ScreenshotCard>> {
    let rows = screenshots::list(state.db().pool(), &game_id)
        .await
        .map_err(|err| CommandError::new("db", err.to_string()))?;
    Ok(rows.into_iter().map(ScreenshotCard::from).collect())
}

/// One capture with its game, for the cross-game library.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenshotFeedCard {
    pub id: String,
    pub game_id: String,
    pub game_title: String,
    pub width: i64,
    pub height: i64,
    pub captured_at: i64,
    pub source: String,
    pub caption: String,
}

/// Every capture across the library, newest first.
///
/// Bounded rather than unlimited: a reader with thousands of shots scrolls, and
/// an unbounded list would build a payload of every row before the first tile
/// paints.
///
/// # Errors
/// Database failures.
#[tauri::command]
pub async fn screenshot_list_all(
    state: State<'_, AppState>,
) -> CommandResult<Vec<ScreenshotFeedCard>> {
    const LIMIT: i64 = 500;
    let rows = screenshots::list_all(state.db().pool(), LIMIT)
        .await
        .map_err(|err| CommandError::new("db", err.to_string()))?;
    Ok(rows
        .into_iter()
        .map(|row| ScreenshotFeedCard {
            id: row.id,
            game_id: row.game_id,
            game_title: row.game_title,
            width: row.width,
            height: row.height,
            captured_at: row.captured_at,
            source: row.source,
            caption: row.caption,
        })
        .collect())
}

/// One capture as a data URL, ready for `background-image` / `src`.
///
/// The webview has no filesystem access, and granting it one for this would give
/// it far more than the feature needs.
///
/// # Errors
/// `not_found` for an unknown or deleted shot, `io` when the file is gone.
#[tauri::command]
pub async fn screenshot_image(state: State<'_, AppState>, id: String) -> CommandResult<String> {
    let row = screenshots::get(state.db().pool(), &id)
        .await
        .map_err(|err| CommandError::new("db", err.to_string()))?
        .ok_or_else(|| CommandError::new("not_found", "that screenshot is gone"))?;
    let store = ScreenshotStore::new(state.data_dir());
    let bytes = store
        .read(&row.relative_path)
        .await
        .map_err(|err| capture_error(&err))?;
    Ok(data_url(&bytes))
}

/// Delete one capture.
///
/// The row is tombstoned before the file is removed: a missing file is harmless,
/// a row pointing at nothing is an error the UI would have to explain.
///
/// # Errors
/// Database failures.
#[tauri::command]
pub async fn screenshot_delete(state: State<'_, AppState>, id: String) -> CommandResult<()> {
    let row = screenshots::get(state.db().pool(), &id)
        .await
        .map_err(|err| CommandError::new("db", err.to_string()))?;
    screenshots::soft_delete(state.db().pool(), &id)
        .await
        .map_err(|err| CommandError::new("db", err.to_string()))?;
    if let Some(row) = row {
        ScreenshotStore::new(state.data_dir())
            .remove(&row.relative_path)
            .await;
    }
    Ok(())
}

/// Take one frame on this platform.
async fn capture_frame(pid: u32) -> CommandResult<screenshots::Captured> {
    #[cfg(windows)]
    {
        // The capture owns COM objects and must create and drop them on one
        // thread, so it runs on a blocking worker rather than the async pool.
        tauri::async_runtime::spawn_blocking(move || {
            crate::screenshots::windows::capture_game_window(pid)
        })
        .await
        .map_err(|err| CommandError::new("screenshot_failed", err.to_string()))?
        .map_err(|err| capture_error(&err))
    }
    #[cfg(target_os = "linux")]
    {
        // A fresh X connection per capture; it is a rare user action, so there is
        // nothing worth keeping open between shots.
        return tauri::async_runtime::spawn_blocking(move || {
            crate::screenshots::linux::capture_game_window(pid)
        })
        .await
        .map_err(|err| CommandError::new("screenshot_failed", err.to_string()))?
        .map_err(|err| capture_error(&err));
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = pid;
        Err(CommandError::new(
            "screenshot_unsupported",
            "capture is not available on this platform",
        ))
    }
}

fn capture_error(error: &screenshots::ScreenshotError) -> CommandError {
    let code = match error {
        screenshots::ScreenshotError::Unsupported => "screenshot_unsupported",
        screenshots::ScreenshotError::WindowUnavailable => "screenshot_window_unavailable",
        screenshots::ScreenshotError::Protected => "screenshot_protected",
        screenshots::ScreenshotError::Timeout => "screenshot_timeout",
        screenshots::ScreenshotError::Encode(_) => "screenshot_encode",
        screenshots::ScreenshotError::Io(_) => "screenshot_io",
    };
    CommandError::new(code, error.to_string())
}

fn data_url(bytes: &[u8]) -> String {
    use base64::Engine as _;
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    format!("data:image/jpeg;base64,{encoded}")
}

fn now_ms() -> CommandResult<i64> {
    vn_core::clock::Timestamp::now()
        .map(vn_core::clock::Timestamp::as_millis)
        .map_err(|err| CommandError::new("clock", err.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_capture_path_cannot_escape_its_directory() {
        let store = ScreenshotStore::new(std::path::Path::new("C:/data"));
        assert!(store.resolve("screenshots/game/shot.jpg").is_some());
        for bad in [
            "screenshots/../shot.jpg",
            "screenshots/game/../../x.jpg",
            "screenshots/game/sub/shot.jpg",
            "covers/game/shot.jpg",
            "screenshots/game\\shot.jpg",
            "screenshots/game/C:/shot.jpg",
            "",
        ] {
            assert!(store.resolve(bad).is_none(), "{bad} should be refused");
        }
    }

    #[test]
    fn a_capture_path_is_relative_and_sanitised() {
        assert_eq!(
            ScreenshotStore::relative("01a1/../x", "id:1"),
            "screenshots/01a1____x/id_1.jpg"
        );
    }
}
