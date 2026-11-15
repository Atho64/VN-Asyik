//! Reads embedded icon resources through Win32 and renders only into a memory bitmap.
#![allow(unsafe_code)]

use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS,
    DeleteDC, DeleteObject, GdiFlush, HBITMAP, HDC, HGDIOBJ, SelectObject,
};
use windows::Win32::UI::Shell::ExtractIconExW;
use windows::Win32::UI::WindowsAndMessaging::{DI_NORMAL, DestroyIcon, DrawIconEx, HICON};
use windows::core::PCWSTR;

const EDGE: i32 = 32;
const BYTE_LEN: usize = 32 * 32 * 4;

struct OwnedIcon(HICON);
impl Drop for OwnedIcon {
    fn drop(&mut self) {
        // SAFETY: this handle is owned exclusively after ExtractIconExW.
        unsafe {
            let _ = DestroyIcon(self.0);
        }
    }
}

struct Canvas {
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
}
impl Drop for Canvas {
    fn drop(&mut self) {
        // SAFETY: restore the selected object before releasing our bitmap and DC.
        unsafe {
            if !self.previous.0.is_null() && self.previous.0.addr() != usize::MAX {
                SelectObject(self.dc, self.previous);
            }
            if !self.bitmap.0.is_null() {
                let _ = DeleteObject(HGDIOBJ(self.bitmap.0));
            }
            let _ = DeleteDC(self.dc);
        }
    }
}

pub(super) fn extract_png(path: &Path) -> Option<Vec<u8>> {
    let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    if wide.contains(&0) {
        return None;
    }
    wide.push(0);
    let mut handle = HICON::default();
    // SAFETY: the filename is NUL-terminated and the output holds one icon. This reads resources, never runs the file.
    let count = unsafe { ExtractIconExW(PCWSTR(wide.as_ptr()), 0, Some(&raw mut handle), None, 1) };
    if handle.0.is_null() {
        return None;
    }
    let icon = OwnedIcon(handle);
    if count != 1 {
        return None;
    }
    let black = render(icon.0, 0)?;
    let white = render(icon.0, 255)?;
    // Two backgrounds recover transparency for both alpha and older mask icons.
    let pixels = rgba_from_backdrops(&black, &white);
    let image = image::RgbaImage::from_raw(32, 32, pixels)?;
    let mut png = std::io::Cursor::new(Vec::new());
    image.write_to(&mut png, image::ImageFormat::Png).ok()?;
    Some(png.into_inner())
}

fn render(icon: HICON, background: u8) -> Option<Vec<u8>> {
    // SAFETY: this creates an independent memory DC, with no screen/window target.
    let dc = unsafe { CreateCompatibleDC(None) };
    if dc.0.is_null() {
        return None;
    }
    let mut canvas = Canvas {
        dc,
        bitmap: HBITMAP::default(),
        previous: HGDIOBJ::default(),
    };
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: u32::try_from(std::mem::size_of::<BITMAPINFOHEADER>()).ok()?,
            biWidth: EDGE,
            biHeight: -EDGE,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits = std::ptr::null_mut();
    // SAFETY: a top-down 32-bit 32x32 DIB has exactly BYTE_LEN bytes; GDI owns its memory until Drop.
    canvas.bitmap = unsafe {
        CreateDIBSection(
            Some(dc),
            &raw const info,
            DIB_RGB_COLORS,
            &raw mut bits,
            None,
            0,
        )
    }
    .ok()?;
    if bits.is_null() {
        return None;
    }
    // SAFETY: both handles are live and owned here; Drop restores the previous object.
    canvas.previous = unsafe { SelectObject(dc, HGDIOBJ(canvas.bitmap.0)) };
    if canvas.previous.0.is_null() || canvas.previous.0.addr() == usize::MAX {
        return None;
    }
    // SAFETY: the fixed DIB allocation is valid, writable, and not yet being drawn to.
    unsafe {
        std::ptr::write_bytes(bits.cast::<u8>(), background, BYTE_LEN);
    }
    // SAFETY: icon and DC stay alive through drawing; flushing completes GDI writes before CPU access.
    unsafe {
        DrawIconEx(dc, 0, 0, icon, EDGE, EDGE, 0, None, DI_NORMAL).ok()?;
        if !GdiFlush().as_bool() {
            return None;
        }
    }
    // SAFETY: GDI is flushed, the allocation has BYTE_LEN bytes and Canvas remains alive until the copy finishes.
    Some(unsafe { std::slice::from_raw_parts(bits.cast::<u8>(), BYTE_LEN) }.to_vec())
}

fn rgba_from_backdrops(black: &[u8], white: &[u8]) -> Vec<u8> {
    let mut rgba = Vec::with_capacity(black.len());
    for (black, white) in black.chunks_exact(4).zip(white.chunks_exact(4)) {
        let transparent = (0..3)
            .map(|i| white[i].saturating_sub(black[i]))
            .max()
            .unwrap_or(255);
        let alpha = 255 - transparent;
        for channel in [2, 1, 0] {
            let color = if alpha == 0 {
                0
            } else {
                u8::try_from(
                    (u16::from(black[channel]) * 255 + u16::from(alpha) / 2) / u16::from(alpha),
                )
                .unwrap_or(255)
            };
            rgba.push(color);
        }
        rgba.push(alpha);
    }
    rgba
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transparency_and_bgra_order_survive_conversion() {
        let black = [0, 0, 0, 0, 20, 40, 80, 0, 0, 0, 64, 0];
        let white = [255, 255, 255, 0, 20, 40, 80, 0, 127, 127, 191, 0];
        assert_eq!(
            rgba_from_backdrops(&black, &white),
            [0, 0, 0, 0, 80, 40, 20, 255, 128, 0, 0, 128]
        );
    }

    #[test]
    fn missing_or_non_resource_files_have_no_icon() {
        assert!(extract_png(Path::new("Z:/vn-asyik-missing-executable.exe")).is_none());
        assert!(extract_png(&Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")).is_none());
    }
}
