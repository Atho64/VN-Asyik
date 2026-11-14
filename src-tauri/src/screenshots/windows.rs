//! One-shot window capture delegating D3D11/COM plumbing to `companion::vision::windows`.

use super::{Captured, ScreenshotError};

/// Capture the top-level window owned by `pid`.
///
/// # Errors
/// `WindowUnavailable` when the process has no capturable window (closed,
/// minimised, or replaced), `Protected` when the window excludes capture, and
/// `Timeout` when no frame arrives.
pub fn capture_game_window(pid: u32) -> Result<Captured, ScreenshotError> {
    let (width, height, rgba) =
        crate::companion::vision::capture_window_once(pid).map_err(classify)?;
    Ok(Captured {
        width,
        height,
        rgba,
        source: "windows_window",
    })
}

/// Turn the vision module's stable error codes into something the UI can say.
fn classify(code: &'static str) -> ScreenshotError {
    match code {
        "vision_protected" => ScreenshotError::Protected,
        "vision_window_changed" | "vision_window_unavailable" => ScreenshotError::WindowUnavailable,
        "vision_timeout" => ScreenshotError::Timeout,
        // HDR windows are refused rather than captured with a guessed tone map,
        // which is the same call the companion made.
        _ => ScreenshotError::Unsupported,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Live capture smoke test (ignored by default; needs a desktop GUI session).
    /// Run with `cargo test --lib capture -- --ignored --nocapture`.
    #[test]
    #[ignore = "opens a window and needs a live desktop session"]
    fn a_real_window_produces_a_non_blank_frame() {
        let mut child = std::process::Command::new("notepad.exe")
            .spawn()
            .expect("notepad should start");
        // Give the window time to be created and painted.
        std::thread::sleep(std::time::Duration::from_millis(1200));

        let captured = capture_game_window(child.id());
        let _ = child.kill();
        let _ = child.wait();

        let frame = captured.expect("a visible window must produce a frame");
        assert!(frame.width > 0 && frame.height > 0);
        assert_eq!(
            frame.rgba.len(),
            (frame.width as usize) * (frame.height as usize) * 4
        );
        // A black frame is what a protected or failed capture looks like, so it
        // must not pass as a successful one.
        let lit = frame
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .any(|px| px[0] > 8 || px[1] > 8 || px[2] > 8);
        assert!(lit, "the captured frame is blank");
    }
}
