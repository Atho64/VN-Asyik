//! Cover download, caching and pre-blurred backdrop generation.
//!
//! Why the blur happens here and not in CSS: DESIGN.md §2 requires a pre-blurred,
//! downscaled backdrop because a large CSS blur on a scrolling `WebView`
//! surface is what breaks frame rate on low-end Android. Generating a ~96 px
//! blurred JPEG once per cover and letting the browser upscale it looks identical
//! and costs almost nothing at render time.
//!
//! Files are written into the app data directory, never into the game's folder
//! and never into the source tree.

use std::path::{Path, PathBuf};

use image::imageops::FilterType;
use vn_core::clock::Timestamp;

use crate::vndb::cache::{self, CachedImage};
use crate::vndb::{VndbClient, VndbError};

/// Width of the generated backdrop, in pixels.
///
/// 96 px is the value DESIGN.md suggests (64-128.) It is wide enough that the
/// upscaled gradient does not band, narrow enough that the file is a few KB.
pub const BACKDROP_WIDTH: u32 = 96;

/// Errors from the image pipeline.
#[derive(Debug, thiserror::Error)]
pub enum ImageError {
    /// The download failed.
    #[error(transparent)]
    Network(#[from] VndbError),
    /// The bytes were not a decodable image.
    #[error("cover image could not be decoded: {0}")]
    Decode(String),
    /// Writing to disk failed.
    #[error("cover image could not be saved: {0}")]
    Io(String),
    /// The database write failed.
    #[error(transparent)]
    Db(#[from] crate::db::DbError),
}

/// Directory layout for cached art.
#[derive(Debug, Clone)]
pub struct ImageStore {
    root: PathBuf,
}

impl ImageStore {
    /// Use `root/covers` as the store.
    pub fn new(data_dir: impl AsRef<Path>) -> Self {
        Self {
            root: data_dir.as_ref().join("covers"),
        }
    }

    /// Ensure the directory exists.
    ///
    /// # Errors
    /// Fails when the directory cannot be created.
    pub fn ensure(&self) -> Result<(), ImageError> {
        std::fs::create_dir_all(&self.root).map_err(|err| ImageError::Io(err.to_string()))
    }

    /// Full-size cover path for a VN id.
    #[must_use]
    pub fn cover_path(&self, vn_id: &str) -> PathBuf {
        self.root.join(format!("{}.jpg", sanitise(vn_id)))
    }

    /// Backdrop path for a VN id.
    #[must_use]
    pub fn backdrop_path(&self, vn_id: &str) -> PathBuf {
        self.root.join(format!("{}.backdrop.jpg", sanitise(vn_id)))
    }

    /// Directory holding user-chosen covers.
    ///
    /// A separate directory from the VNDB cache on purpose: the cache is
    /// disposable and re-downloadable, while a custom cover is the only copy of
    /// an image the user picked off their own disk. Keeping them apart means
    /// clearing the VNDB cache can never delete a chosen cover by accident.
    #[must_use]
    pub fn custom_dir(&self) -> PathBuf {
        self.root.join("custom")
    }

    /// Full-size path for a game's custom cover.
    ///
    /// Keyed by the game's UUID, not its VNDB id: a non-VNDB game has no VN id,
    /// and that is the case this whole feature exists for. The UUID is sanitised
    /// like any other external string.
    #[must_use]
    pub fn custom_cover_path(&self, game_id: &str) -> PathBuf {
        self.custom_dir().join(format!("{}.jpg", sanitise(game_id)))
    }

    /// Blurred backdrop path for a game's custom cover.
    #[must_use]
    pub fn custom_backdrop_path(&self, game_id: &str) -> PathBuf {
        self.custom_dir()
            .join(format!("{}.backdrop.jpg", sanitise(game_id)))
    }

    /// The `covers/…`-relative form stored in the database.
    ///
    /// A relative path is the only portable form: the library file moves between
    /// devices and machines, and an absolute path from this machine would resolve
    /// to nothing anywhere else.
    #[must_use]
    pub fn custom_relative(game_id: &str) -> String {
        format!("covers/custom/{}.jpg", sanitise(game_id))
    }

    /// Resolve a stored relative custom-cover path against the data directory.
    ///
    /// Returns `None` for anything that is not a plain relative path inside the
    /// custom directory. A stored value is data, not code, and a tampered or
    /// hand-edited row must not be able to point at `..\..\Windows\System32`.
    #[must_use]
    pub fn resolve_custom(data_dir: &Path, stored: &str) -> Option<PathBuf> {
        let name = stored.strip_prefix("covers/custom/")?;
        // One path segment only: no separators, no traversal, no drive letters.
        if name.is_empty()
            || name.contains('/')
            || name.contains('\\')
            || name.contains("..")
            || name.contains(':')
        {
            return None;
        }
        let path = data_dir.join("covers").join("custom").join(name);
        path.is_file().then_some(path)
    }
}

/// VN ids are `v` + digits, but never trust an external string with a path.
fn sanitise(id: &str) -> String {
    id.chars()
        .filter(char::is_ascii_alphanumeric)
        .take(32)
        .collect()
}

/// Download a cover, write it to disk, and generate its blurred backdrop.
///
/// Returns the cached image row so the caller can persist it. Re-running for an
/// already cached VN is a cheap no-op when the files still exist.
///
/// # Errors
/// Propagates network, decode, IO and database failures.
pub async fn cache_cover(
    client: &VndbClient,
    pool: &sqlx::SqlitePool,
    store: &ImageStore,
    vn_id: &str,
    url: &str,
    ratings: (Option<i64>, Option<i64>),
    now: Timestamp,
) -> Result<CachedImage, ImageError> {
    store.ensure()?;
    let cover_path = store.cover_path(vn_id);
    let backdrop_path = store.backdrop_path(vn_id);

    let needs_download = !cover_path.exists();
    if needs_download {
        let bytes = client.fetch_bytes(url).await?;
        std::fs::write(&cover_path, &bytes).map_err(|err| ImageError::Io(err.to_string()))?;
    }

    let decoded = image::open(&cover_path).map_err(|err| ImageError::Decode(err.to_string()))?;
    let (width, height) = (decoded.width(), decoded.height());

    if !backdrop_path.exists() {
        write_backdrop(&decoded, &backdrop_path)?;
    }

    cache::put_image(
        pool,
        vn_id,
        url,
        Some(&cover_path),
        Some(&backdrop_path),
        (Some(width), Some(height)),
        ratings,
        now,
    )
    .await?;

    Ok(CachedImage {
        url: url.to_owned(),
        local_path: Some(cover_path),
        blur_path: Some(backdrop_path),
        width: Some(width),
        height: Some(height),
        sexual: ratings.0,
        violence: ratings.1,
    })
}

/// Downscale, gently saturate and save the backdrop.
fn write_backdrop(source: &image::DynamicImage, path: &Path) -> Result<(), ImageError> {
    let width = BACKDROP_WIDTH;
    // Integer arithmetic: height = width * source_height / source_width, clamped
    // to at least one pixel. No float cast, so nothing can truncate.
    let source_width = u64::from(source.width().max(1));
    let source_height = u64::from(source.height());
    let height = (u64::from(width) * source_height / source_width).max(1);
    #[allow(clippy::cast_possible_truncation)]
    let height = height.min(u64::from(u32::MAX)) as u32;

    let resized = source.resize_exact(width, height, FilterType::Triangle);
    // Slight saturation lift, matching DESIGN.md §2 layer 1.
    let mut rgb = resized.to_rgb8();
    for pixel in rgb.pixels_mut() {
        let [r, g, b] = pixel.0;
        let luma = 0.299 * f32::from(r) + 0.587 * f32::from(g) + 0.114 * f32::from(b);
        // Clamped into 0..=255 before conversion, so the narrowing is explicit
        // and cannot wrap or lose sign.
        let lift = |channel: u8| -> u8 {
            let value = f32::from(channel);
            let lifted = (luma + (value - luma) * 1.25).clamp(0.0, 255.0);
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let byte = lifted.round() as u8;
            byte
        };
        *pixel = image::Rgb([lift(r), lift(g), lift(b)]);
    }

    // Blur by downscaling further and scaling back up: the same trick the design
    // document describes, done with the resampler we already have.
    let tiny = image::DynamicImage::ImageRgb8(rgb).resize_exact(
        (width / 4).max(4),
        (height / 4).max(4),
        FilterType::Gaussian,
    );
    let blurred = tiny.resize_exact(width, height, FilterType::Triangle);

    blurred
        .save_with_format(path, image::ImageFormat::Jpeg)
        .map_err(|err| ImageError::Io(err.to_string()))
}

/// Read a cached cover file as bytes, for serving to the webview.
///
/// # Errors
/// Fails when the file is missing or unreadable.
pub async fn read_bytes(path: &Path) -> Result<Vec<u8>, ImageError> {
    tokio::fs::read(path)
        .await
        .map_err(|err| ImageError::Io(err.to_string()))
}

/// Longest edge written for a custom cover.
///
/// A phone photo or a 4K wallpaper is routinely 5000 px wide and several MB.
/// Covers render at a few hundred px in the grid and about 600 px in the detail
/// hero, so re-encoding to this bound keeps the file small without visible loss
/// at either size. `resize` preserves aspect ratio and never upscales.
pub const CUSTOM_COVER_MAX_EDGE: u32 = 1600;

/// Width of the blurred image uploaded for the presence card.
///
/// Discord renders the large image around 160 px, so 512 px of blur is already
/// generous headroom; the point of the blur is that the original is never sent,
/// not that it stays readable.
pub const RPC_BLUR_WIDTH: u32 = 512;

/// The `EwE` blur: downscale hard, upscale back.
///
/// A 1/20 downscale followed by a bilinear upscale is what `EwE` does to its
/// covers before upload (`presence.py::_blur_image_file`); the result reads as
/// a colour smear that keeps the composition but no detail. Re-encoded as a
/// bounded-width JPEG so the upload stays small.
///
/// # Errors
/// Fails when `source` is not a decodable image.
pub fn blurred_cover_bytes(source: &[u8]) -> Result<Vec<u8>, ImageError> {
    let decoded =
        image::load_from_memory(source).map_err(|err| ImageError::Decode(err.to_string()))?;
    let width = RPC_BLUR_WIDTH.min(decoded.width().max(1));
    let source_width = u64::from(decoded.width().max(1));
    let source_height = u64::from(decoded.height());
    let height = (u64::from(width) * source_height / source_width).max(1);
    #[allow(clippy::cast_possible_truncation)]
    let height = height.min(u64::from(u32::MAX)) as u32;

    let small_w = (width / 20).max(4);
    let small_h = (height / 20).max(4);
    let blurred = decoded
        .resize_exact(small_w, small_h, FilterType::Triangle)
        .resize_exact(width, height, FilterType::Triangle);

    let mut out = Vec::new();
    let cursor = std::io::Cursor::new(&mut out);
    blurred
        .write_to(cursor, image::ImageFormat::Jpeg)
        .map_err(|err| ImageError::Io(err.to_string()))?;
    Ok(out)
}

/// Import a user-chosen image file as a game's cover.
///
/// # Errors
/// Fails when the source cannot be read or decoded, or the destination cannot
/// be written.
pub fn import_custom_cover(
    store: &ImageStore,
    game_id: &str,
    source: &Path,
) -> Result<String, ImageError> {
    let bytes = std::fs::read(source).map_err(|err| ImageError::Io(err.to_string()))?;
    import_custom_cover_bytes(store, game_id, &bytes)
}

/// Import a user-chosen image, decoded from raw bytes, as a game's cover.
///
/// The source is decoded, not merely copied: bytes that are not a real image
/// are rejected here instead of failing later in the webview, and re-encoding
/// bounds the stored size and normalises the format to JPEG. Decoding sniffs
/// the format from the bytes, never from a file extension: Android's picker
/// hands back `content://` documents whose names carry no extension, and a
/// renamed file is the same bytes either way. Returns the app-data-relative
/// path to store on the game row.
///
/// # Errors
/// Fails when the source cannot be decoded or the destination cannot be written.
pub fn import_custom_cover_bytes(
    store: &ImageStore,
    game_id: &str,
    bytes: &[u8],
) -> Result<String, ImageError> {
    store.ensure()?;
    let dir = store.custom_dir();
    std::fs::create_dir_all(&dir).map_err(|err| ImageError::Io(err.to_string()))?;

    let decoded =
        image::load_from_memory(bytes).map_err(|err| ImageError::Decode(err.to_string()))?;
    // `resize` fits inside the box and keeps the aspect ratio, upscaling never.
    let bounded =
        if decoded.width() > CUSTOM_COVER_MAX_EDGE || decoded.height() > CUSTOM_COVER_MAX_EDGE {
            decoded.resize(
                CUSTOM_COVER_MAX_EDGE,
                CUSTOM_COVER_MAX_EDGE,
                FilterType::Lanczos3,
            )
        } else {
            decoded
        };

    let cover_path = store.custom_cover_path(game_id);
    bounded
        .to_rgb8()
        .save_with_format(&cover_path, image::ImageFormat::Jpeg)
        .map_err(|err| ImageError::Io(err.to_string()))?;

    // Regenerate the backdrop every time: a stale blurred image behind a new
    // sharp cover would show the previous game's artwork while concealed.
    write_backdrop(&bounded, &store.custom_backdrop_path(game_id))?;

    Ok(ImageStore::custom_relative(game_id))
}

/// Delete a game's custom cover files, if present.
///
/// Missing files are not an error: the row is the source of truth, and a user
/// clearing a cover whose files were removed by hand should still succeed.
///
/// # Errors
/// Fails only when a file exists and cannot be removed.
pub fn remove_custom_cover(store: &ImageStore, game_id: &str) -> Result<(), ImageError> {
    for path in [
        store.custom_cover_path(game_id),
        store.custom_backdrop_path(game_id),
    ] {
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(ImageError::Io(err.to_string())),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_derived_from_the_id_and_cannot_escape_the_directory() {
        let store = ImageStore::new("C:/data");
        assert!(store.cover_path("v24").ends_with("v24.jpg"));
        assert!(
            store
                .backdrop_path("v24")
                .to_string_lossy()
                .contains("v24.backdrop.jpg")
        );
        // Path traversal attempts are stripped to safe characters.
        let escaped = store.cover_path("../../etc/passwd");
        assert!(!escaped.to_string_lossy().contains(".."));
        assert!(escaped.starts_with(&store.root));
    }

    #[test]
    fn sanitise_drops_separators_and_limits_length() {
        assert_eq!(sanitise("v123"), "v123");
        assert_eq!(sanitise("a/b\\c"), "abc");
        assert_eq!(sanitise("x".repeat(100).as_str()).len(), 32);
    }

    #[test]
    fn backdrop_generation_writes_a_small_decodable_file() {
        let dir = std::env::temp_dir().join("vnt-backdrop-test");
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("test.backdrop.jpg");
        let _ = std::fs::remove_file(&path);

        // A 400x600 synthetic "cover".
        let cover = image::RgbImage::from_fn(400, 600, |x, y| {
            // x and y are bounded by the image size (400x600), so both fit in u8.
            image::Rgb([(x % 256).to_le_bytes()[0], (y % 256).to_le_bytes()[0], 128])
        });
        write_backdrop(&image::DynamicImage::ImageRgb8(cover), &path).expect("write");

        let written = image::open(&path).expect("decode");
        assert_eq!(written.width(), BACKDROP_WIDTH);
        // Aspect ratio preserved.
        assert_eq!(written.height(), BACKDROP_WIDTH * 3 / 2);
        let size = std::fs::metadata(&path).expect("meta").len();
        assert!(size < 40_000, "backdrop should be small, got {size} bytes");

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn backdrop_handles_a_tiny_source_image() {
        let dir = std::env::temp_dir().join("vnt-backdrop-tiny");
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("tiny.backdrop.jpg");
        let _ = std::fs::remove_file(&path);

        let cover = image::RgbImage::from_pixel(4, 4, image::Rgb([10, 20, 30]));
        write_backdrop(&image::DynamicImage::ImageRgb8(cover), &path).expect("write");
        assert!(path.exists());

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn blurred_cover_bytes_keep_the_aspect_ratio_and_stay_small() {
        let cover = image::RgbImage::from_fn(800, 1200, |x, y| {
            image::Rgb([(x % 256).to_le_bytes()[0], (y % 256).to_le_bytes()[0], 128])
        });
        let mut png = Vec::new();
        let cursor = std::io::Cursor::new(&mut png);
        image::DynamicImage::ImageRgb8(cover)
            .write_to(cursor, image::ImageFormat::Png)
            .expect("encode source");

        let blurred = blurred_cover_bytes(&png).expect("blur");
        let decoded = image::load_from_memory(&blurred).expect("decode blurred");
        assert_eq!(decoded.width(), RPC_BLUR_WIDTH);
        assert_eq!(decoded.height(), RPC_BLUR_WIDTH * 3 / 2);
        assert!(blurred.len() < 120_000, "got {} bytes", blurred.len());
        // The blur must actually destroy detail: it cannot round-trip to the
        // source, or we would be uploading the original art under another name.
        assert_ne!(blurred.len(), png.len());
    }

    #[test]
    fn blurred_cover_bytes_reject_non_images() {
        assert!(blurred_cover_bytes(b"not an image").is_err());
    }

    #[tokio::test]
    async fn reading_a_missing_file_reports_io() {
        let result = read_bytes(Path::new("definitely/not/here.jpg")).await;
        assert!(result.is_err());
    }

    /// A fresh store rooted in a unique temp directory.
    fn temp_store(tag: &str) -> (ImageStore, PathBuf) {
        let root = std::env::temp_dir().join(format!("vnt-custom-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("temp root");
        (ImageStore::new(&root), root)
    }

    /// Write a real PNG so the decoder has something valid to read.
    fn write_source(dir: &Path, name: &str, width: u32, height: u32) -> PathBuf {
        let path = dir.join(name);
        let image = image::RgbImage::from_pixel(width, height, image::Rgb([200, 40, 90]));
        image.save(&path).expect("source image");
        path
    }

    #[test]
    fn importing_a_cover_writes_both_files_and_returns_a_relative_path() {
        let (store, root) = temp_store("import");
        let source = write_source(&root, "pick.png", 300, 400);
        let id = "0192f0aa-0000-7000-8000-000000000001";

        let relative = import_custom_cover(&store, id, &source).expect("import");

        // The stored value must be relative: an absolute path would be meaningless
        // on another device, and this column travels in the sync payload.
        // `sanitise` keeps ASCII alphanumerics only, so the UUID's hyphens go; the
        // result is still unique per game.
        assert_eq!(
            relative,
            "covers/custom/0192f0aa000070008000000000000001.jpg"
        );
        assert!(!relative.contains(&root.to_string_lossy().to_string()));
        assert!(store.custom_cover_path(id).exists());
        // The backdrop is what the content policy blurs, so both must exist.
        assert!(store.custom_backdrop_path(id).exists());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_cover_imports_from_extensionless_bytes() {
        // Android's picker answers with a `content://` document copied to a
        // cache file that carries no image extension, so decoding must sniff
        // the format from the bytes. If this regresses to an extension guess,
        // every Android cover pick fails as undecodable.
        let (store, root) = temp_store("bytes");
        let source = write_source(&root, "pick.png", 120, 90);
        let bytes = std::fs::read(&source).expect("read png bytes");
        let id = "0192f0aa-0000-7000-8000-000000000002";

        let relative = import_custom_cover_bytes(&store, id, &bytes).expect("import from bytes");
        assert_eq!(
            relative,
            "covers/custom/0192f0aa000070008000000000000002.jpg"
        );
        assert!(store.custom_cover_path(id).exists());

        // Bytes that are not an image are rejected, not stored broken.
        assert!(import_custom_cover_bytes(&store, id, b"not an image").is_err());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_oversized_source_is_scaled_down_but_never_up() {
        let (store, root) = temp_store("bounds");
        let big = write_source(&root, "big.png", 4000, 2000);
        let small = write_source(&root, "small.png", 50, 80);

        import_custom_cover(&store, "big", &big).expect("big import");
        import_custom_cover(&store, "small", &small).expect("small import");

        let big_stored = image::open(store.custom_cover_path("big")).expect("big cover");
        assert_eq!(big_stored.width(), CUSTOM_COVER_MAX_EDGE);
        // 4000x2000 is 2:1, and the aspect ratio survives the resize.
        assert_eq!(big_stored.height(), CUSTOM_COVER_MAX_EDGE / 2);

        let small_stored = image::open(store.custom_cover_path("small")).expect("small cover");
        assert_eq!(small_stored.width(), 50);
        assert_eq!(small_stored.height(), 80);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_file_that_is_not_an_image_is_rejected() {
        let (store, root) = temp_store("notimage");
        let fake = root.join("fake.jpg");
        std::fs::write(&fake, b"this is not a JPEG").expect("write fake");

        let result = import_custom_cover(&store, "game", &fake);
        assert!(result.is_err(), "a non-image must not be accepted");
        // Nothing half-written is left behind for the read path to trip over.
        assert!(!store.custom_cover_path("game").exists());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn removing_a_cover_deletes_both_files_and_tolerates_absence() {
        let (store, root) = temp_store("remove");
        let source = write_source(&root, "pick.png", 100, 100);
        import_custom_cover(&store, "game", &source).expect("import");
        assert!(store.custom_cover_path("game").exists());

        remove_custom_cover(&store, "game").expect("remove");
        assert!(!store.custom_cover_path("game").exists());
        assert!(!store.custom_backdrop_path("game").exists());

        // Clearing twice is not an error: the row is the source of truth.
        remove_custom_cover(&store, "game").expect("second remove");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_stored_path_cannot_escape_the_custom_directory() {
        let (store, root) = temp_store("traversal");
        import_custom_cover(&store, "game", &write_source(&root, "p.png", 10, 10)).expect("import");
        let outside = root.join("secret.jpg");
        std::fs::write(&outside, b"x").expect("write outside");

        // A hand-edited or tampered row must not be able to read arbitrary files.
        for hostile in [
            "covers/custom/../../../secret.jpg",
            "covers/custom/sub/secret.jpg",
            "covers/secret.jpg",
            "/etc/passwd",
            "C:/Windows/System32/config",
            "covers/custom/",
        ] {
            assert!(
                ImageStore::resolve_custom(&root, hostile).is_none(),
                "{hostile} must not resolve"
            );
        }

        // The legitimate stored form still resolves.
        assert!(ImageStore::resolve_custom(&root, &ImageStore::custom_relative("game")).is_some());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_missing_file_resolves_to_none_rather_than_a_broken_path() {
        let (_store, root) = temp_store("missing");
        // The row can arrive from a peer without its image, because sync ships
        // rows only. That must read as "no art", not as an error.
        assert!(ImageStore::resolve_custom(&root, &ImageStore::custom_relative("a")).is_none());
        let _ = std::fs::remove_dir_all(&root);
    }
}
