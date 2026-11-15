//! Export only the already-painted screenshot bytes, preserving concealment.
use std::io::Cursor;

use base64::Engine as _;
use image::{DynamicImage, ImageFormat, ImageReader, Limits};

use crate::commands::{CommandError, CommandResult};

const MAX_BYTES: usize = 16 * 1024 * 1024;
const MAX_PIXELS: u64 = 16 * 1024 * 1024;

pub(crate) fn decode(data_url: &str) -> CommandResult<DynamicImage> {
    let (header, encoded) = data_url
        .split_once(',')
        .ok_or_else(|| invalid("Expected an image data URL"))?;
    let format = match header {
        "data:image/png;base64" => ImageFormat::Png,
        "data:image/jpeg;base64" => ImageFormat::Jpeg,
        "data:image/webp;base64" => ImageFormat::WebP,
        _ => return Err(invalid("Unsupported image format")),
    };
    // Reject oversized input before allocating its decoded base64 buffer.
    if encoded.len() > MAX_BYTES.div_ceil(3) * 4 {
        return Err(too_large());
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| invalid("Invalid image data"))?;
    if bytes.len() > MAX_BYTES {
        return Err(too_large());
    }
    let (width, height) = ImageReader::with_format(Cursor::new(&bytes), format)
        .into_dimensions()
        .map_err(|_| invalid("Could not read the image"))?;
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > MAX_PIXELS {
        return Err(too_large());
    }
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = Limits::default();
    limits.max_alloc = Some(MAX_PIXELS * 4);
    reader.limits(limits);
    reader
        .decode()
        .map_err(|_| invalid("Could not decode the image"))
}

pub(crate) fn png(data_url: &str) -> CommandResult<Vec<u8>> {
    let image = decode(data_url)?;
    let mut output = Cursor::new(Vec::new());
    image
        .write_to(&mut output, ImageFormat::Png)
        .map_err(|_| invalid("Could not encode the image"))?;
    let bytes = output.into_inner();
    if bytes.len() > MAX_BYTES {
        return Err(too_large());
    }
    Ok(bytes)
}

pub(crate) fn filename(requested: &str) -> String {
    let stem: String = requested
        .strip_suffix(".png")
        .unwrap_or(requested)
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
        .take(80)
        .collect();
    format!(
        "vn-asyik-{}.png",
        if stem.is_empty() { "screenshot" } else { &stem }
    )
}

fn invalid(message: &str) -> CommandError {
    CommandError::new("invalid_image", message)
}

fn too_large() -> CommandError {
    CommandError::new("file_too_large", "The image exceeds the export size limit")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exports_jpeg_and_png_without_changing_dimensions_or_png_pixels() {
        let source = DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            3,
            2,
            image::Rgba([65, 110, 170, 255]),
        ));
        for (format, mime) in [(ImageFormat::Png, "png"), (ImageFormat::Jpeg, "jpeg")] {
            let mut encoded = Cursor::new(Vec::new());
            source.to_rgb8().write_to(&mut encoded, format).unwrap();
            let url = format!(
                "data:image/{mime};base64,{}",
                base64::engine::general_purpose::STANDARD.encode(encoded.into_inner())
            );
            let exported = image::load_from_memory(&png(&url).unwrap()).unwrap();
            assert_eq!((exported.width(), exported.height()), (3, 2));
            if format == ImageFormat::Png {
                assert_eq!(exported.to_rgba8(), source.to_rgba8());
            }
        }
    }

    #[test]
    fn rejects_urls_non_images_and_oversized_input() {
        for value in [
            "https://example.com/image.png",
            "data:text/plain;base64,AA==",
            "data:image/png;base64,AA==",
        ] {
            assert!(decode(value).is_err());
        }
        let huge = format!(
            "data:image/png;base64,{}",
            "A".repeat(MAX_BYTES.div_ceil(3) * 4 + 1)
        );
        assert_eq!(decode(&huge).unwrap_err().code, "file_too_large");
        assert_eq!(filename("../CON.png"), "vn-asyik-CON.png");
        assert_eq!(filename(""), "vn-asyik-screenshot.png");
    }
}
