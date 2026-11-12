//! One-shot capture of a running game's window over X11.
//!
//! `GetImage` on the window itself, not the root: the root would include
//! whatever else is on screen, and the point of the Windows path is that a
//! screenshot is the game.
//!
//! **Wayland.** A native Wayland window cannot be captured by another client at
//! all. A visual novel running through `XWayland` is an X11 window and is
//! captured normally. When the window is not an X11 window this returns
//! `WindowUnavailable`, which is the honest answer rather than a black frame.
//!
//! The capture is a fresh connection per call on purpose: a screenshot is a rare
//! user action, and holding an X connection open for it would mean owning its
//! lifetime, its error handling and its teardown for no benefit.

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{Atom, AtomEnum, ConnectionExt as _, ImageFormat, Window};
use x11rb::rust_connection::RustConnection;

use super::{Captured, ScreenshotError};

/// Capture the top-level window owned by `pid`.
///
/// # Errors
/// `WindowUnavailable` when the process has no X11 window, `Unsupported` when
/// the visual is not the 32-bit BGRX this decoder understands, and `Io` when the
/// X server refuses the request.
pub fn capture_game_window(pid: u32) -> Result<Captured, ScreenshotError> {
    let (connection, screen) = RustConnection::connect(None)
        .map_err(|err| ScreenshotError::Io(format!("no X11 display: {err}")))?;
    let root = connection
        .setup()
        .roots
        .get(screen)
        .map(|screen| screen.root)
        .ok_or(ScreenshotError::WindowUnavailable)?;

    let window = window_for_pid(&connection, root, pid)?;
    let geometry = connection
        .get_geometry(window)
        .map_err(|err| ScreenshotError::Io(err.to_string()))?
        .reply()
        .map_err(|err| ScreenshotError::Io(err.to_string()))?;
    if geometry.width == 0 || geometry.height == 0 {
        return Err(ScreenshotError::WindowUnavailable);
    }

    let image = connection
        .get_image(
            ImageFormat::Z_PIXMAP,
            window,
            0,
            0,
            geometry.width,
            geometry.height,
            !0,
        )
        .map_err(|err| ScreenshotError::Io(err.to_string()))?
        .reply()
        .map_err(|err| ScreenshotError::Io(err.to_string()))?;

    let rgba = bgrx_to_rgba(
        &image.data,
        u32::from(geometry.width),
        u32::from(geometry.height),
    )?;
    Ok(Captured {
        width: u32::from(geometry.width),
        height: u32::from(geometry.height),
        rgba,
        source: "linux_window",
    })
}

/// The first window in `_NET_CLIENT_LIST` owned by `pid`.
fn window_for_pid(
    connection: &RustConnection,
    root: Window,
    pid: u32,
) -> Result<Window, ScreenshotError> {
    let atom = |name: &str| -> Result<Atom, ScreenshotError> {
        connection
            .intern_atom(false, name.as_bytes())
            .map_err(|err| ScreenshotError::Io(err.to_string()))?
            .reply()
            .map(|reply| reply.atom)
            .map_err(|err| ScreenshotError::Io(err.to_string()))
    };
    let client_list = atom("_NET_CLIENT_LIST")?;
    let wm_pid = atom("_NET_WM_PID")?;

    let windows = connection
        .get_property(false, root, client_list, AtomEnum::WINDOW, 0, u32::MAX)
        .map_err(|err| ScreenshotError::Io(err.to_string()))?
        .reply()
        .map_err(|err| ScreenshotError::Io(err.to_string()))?;

    for window in windows.value32().into_iter().flatten() {
        let owner = connection
            .get_property(false, window, wm_pid, AtomEnum::CARDINAL, 0, 1)
            .map_err(|err| ScreenshotError::Io(err.to_string()))?
            .reply()
            .map_err(|err| ScreenshotError::Io(err.to_string()))?;
        if owner.value32().and_then(|mut values| values.next()) == Some(pid) {
            return Ok(window);
        }
    }
    Err(ScreenshotError::WindowUnavailable)
}

/// BGRX to RGBA.
///
/// `GetImage` returns the server's own pixel layout. On every 24- or 32-bit
/// `TrueColor` visual that is four bytes per pixel with the alpha byte unused, so
/// the byte order is B, G, R, pad. Anything else is refused rather than decoded
/// with a guess: a screenshot with swapped channels is worse than an error.
fn bgrx_to_rgba(data: &[u8], width: u32, height: u32) -> Result<Vec<u8>, ScreenshotError> {
    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| ScreenshotError::Encode("the frame size overflowed".to_owned()))?;
    if data.len() != expected {
        return Err(ScreenshotError::Unsupported);
    }
    let mut rgba = Vec::with_capacity(expected);
    for pixel in data.as_chunks::<4>().0 {
        rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], 255]);
    }
    Ok(rgba)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bgrx_is_swapped_to_rgba_with_an_opaque_alpha() {
        // One pixel: B=10, G=20, R=30, pad=0.
        let rgba = bgrx_to_rgba(&[10, 20, 30, 0], 1, 1).expect("decode");
        assert_eq!(rgba, [30, 20, 10, 255]);
    }

    #[test]
    fn a_buffer_that_is_not_four_bytes_per_pixel_is_refused() {
        // 24bpp packed RGB is a different layout; guessing would swap channels.
        assert!(matches!(
            bgrx_to_rgba(&[1, 2, 3], 1, 1),
            Err(ScreenshotError::Unsupported)
        ));
    }
}
