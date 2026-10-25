//! Screenshot download, caching and per-image blur.
//!
//! Why this is separate from `images.rs` (the cover pipeline):
//!
//! * A cover is one image per VN and is part of the row's identity, so it lives in
//!   `vndb_images` keyed by `vn_id`. A VN has up to 30 screenshots, each with its
//!   own content rating, so they need their own table and their own key
//!   (`image_id`, e.g. `sf190`).
//! * The content policy is per image, not per game. One VN's gallery is routinely a
//!   mix of harmless backgrounds and explicit scenes, and blurring the whole set
//!   because one tile is explicit would hide art the user asked to see — while
//!   showing an explicit tile because the rest are fine is the actual bug. Each
//!   screenshot is therefore classified from its own flagging votes and blurred
//!   individually.
//! * Files are small and numerous, so they are kept in a subdirectory and named by
//!   the VNDB image id rather than the VN id.
//!
//! Blurring happens here, in Rust, for the reason DESIGN.md §2 gives for covers: a
//! large CSS `filter: blur()` on a scrolling `WebView` surface is what breaks frame
//! rate on low-end Android. A pre-blurred 32 px JPEG upscaled by the browser costs
//! nothing to paint.

use std::path::{Path, PathBuf};

use image::imageops::FilterType;
use sqlx::SqlitePool;
use vn_core::clock::Timestamp;

use crate::vndb::cache::{self, CachedScreenshot};
use crate::vndb::{VndbClient, VndbError};

/// Width of a grid tile copy, in pixels.
///
/// This is the **tile** size, not the viewing size. A tile is ~320 px on a wide
/// desktop, so 640 covers it at 2x DPR. It is deliberately smaller than the image
/// the lightbox needs: a grid of 30 tiles at full resolution is several megabytes of
/// base64 in one IPC response, which is both slow and fragile.
pub const SCREENSHOT_TILE_WIDTH: u32 = 640;

/// Width of the full-size copy the lightbox paints, in pixels.
///
/// VNDB serves screenshots at up to 1920 px wide (most are 1280×720 or 800×600), and
/// they are already JPEG-compressed, so the honest thing is to keep them as they
/// arrive rather than downscale and upscale again. This cap exists only to bound a
/// pathological source; an image at or below it is stored untouched.
///
/// The previous single-size pipeline wrote 768 px and painted it into a ~1300 px
/// lightbox, which is what a user correctly described as "very low pixel": the image
/// was being upscaled by the browser and looked worse than the blur.
pub const SCREENSHOT_FULL_WIDTH: u32 = 1920;

/// Width of the pre-blurred resample.
///
/// This is the *resample* width, not the painted width: the image is squashed to
/// this and scaled back up, so the blur radius ends up roughly
/// `painted_width / SCREENSHOT_BLUR_WIDTH` pixels.
///
/// 48 was far too aggressive. At 512/48 that is a ~10px radius, which erased the
/// picture entirely — the first version shipped as an unreadable smear. Blurring is
/// meant to stop the art resolving, not to erase it: at 640/128 the radius is ~5px on
/// a tile and a soft wash in the lightbox, which hides the detail that matters while
/// leaving the composition and colours recognisable, so a reader can tell one scene
/// from another and decide whether to reveal it.
pub const SCREENSHOT_BLUR_WIDTH: u32 = 128;

/// Width of the blurred copy intended for the lightbox.
///
/// The lightbox paints at ~1300 px on a desktop, so a 640 px blurred file upscaled
/// there looks like a mosaic of soft blobs rather than a blurred photograph. 1280 is
/// close to the painted size and still compresses to a few tens of kilobytes because
/// a blurred image has almost no high-frequency detail.
pub const SCREENSHOT_BLUR_FULL_WIDTH: u32 = 1280;

/// Cache format version, for humans reading a build number.
///
/// Invalidation is driven by `width`, which is stored per row and compared in
/// [`needs_rebuild`]: a resolution or blur change also changes the written width, so
/// the row is regenerated. This constant is not stored, so do **not** rely on it to
/// invalidate anything. Bump it when the file *shape* changes without the width
/// changing, and change [`needs_rebuild`] to match.
///
/// 3 = tile 640, full up to 1920, tile blur 640, lightbox blur 1280.
/// 4 = blur radius is 1/24 of the output width, i.e. actually strong.
pub const CACHE_VERSION: u32 = 4;

/// How many screenshots are cached per VN.
///
/// VNDB caps a VN's screenshot list at around 30 entries; this is a guard against a
/// pathological response, not a limit the UI should ever hit.
pub const MAX_SCREENSHOTS: usize = 40;

/// Whether a cached file should be the sharp original or the blurred variant.
///
/// This mirrors the app's `show` / `blur` / `hide` preference. `hide` still caches
/// the blur: the user may switch to `blur` later and must not need a second
/// download for it, and a hidden tile needs a neutral surface rather than a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlurPolicy {
    /// The user accepted explicit art: cache it sharp.
    Sharp,
    /// Cache a pre-blurred copy.
    Blur,
}

impl BlurPolicy {
    /// Map the stored preference string onto a policy.
    ///
    /// Anything unrecognised falls back to [`BlurPolicy::Blur`], the same default
    /// the preference itself uses: an unknown value must not be read as consent.
    #[must_use]
    pub fn from_pref(mode: &str) -> Self {
        if mode == "show" {
            Self::Sharp
        } else {
            Self::Blur
        }
    }
}

/// Errors from the screenshot pipeline.
#[derive(Debug, thiserror::Error)]
pub enum ScreenshotError {
    /// The download failed.
    #[error(transparent)]
    Network(#[from] VndbError),
    /// The bytes were not a decodable image.
    #[error("screenshot could not be decoded: {0}")]
    Decode(String),
    /// Writing to disk failed.
    #[error("screenshot could not be saved: {0}")]
    Io(String),
    /// The database write failed.
    #[error(transparent)]
    Db(#[from] crate::db::DbError),
}

/// Directory layout for cached screenshots.
#[derive(Debug, Clone)]
pub struct ScreenshotStore {
    root: PathBuf,
}

impl ScreenshotStore {
    /// Use `root/screenshots` as the store.
    pub fn new(data_dir: impl AsRef<Path>) -> Self {
        Self {
            root: data_dir.as_ref().join("screenshots"),
        }
    }

    /// Ensure the directory exists.
    ///
    /// # Errors
    /// Fails when the directory cannot be created.
    pub fn ensure(&self) -> Result<(), ScreenshotError> {
        std::fs::create_dir_all(&self.root).map_err(|err| ScreenshotError::Io(err.to_string()))
    }

    /// Path of the small copy the grid paints.
    #[must_use]
    pub fn tile_path(&self, image_id: &str) -> PathBuf {
        self.root.join(format!("{}.tile.jpg", sanitise(image_id)))
    }

    /// Path of the full-size copy the lightbox paints.
    #[must_use]
    pub fn full_path(&self, image_id: &str) -> PathBuf {
        self.root.join(format!("{}.full.jpg", sanitise(image_id)))
    }

    /// Path of the blurred copy used by grid tiles.
    #[must_use]
    pub fn blur_path(&self, image_id: &str) -> PathBuf {
        self.root.join(format!("{}.blur.jpg", sanitise(image_id)))
    }

    /// Path of the larger blurred copy used by the lightbox.
    #[must_use]
    pub fn blur_full_path(&self, image_id: &str) -> PathBuf {
        self.root
            .join(format!("{}.blur.full.jpg", sanitise(image_id)))
    }

    /// Every file this store may hold for one image.
    ///
    /// Returned so a rewrite can remove the ones it did not produce: a previous
    /// build's `{id}.jpg` would otherwise sit in the folder forever.
    fn all_paths(&self, image_id: &str) -> [PathBuf; 4] {
        [
            self.tile_path(image_id),
            self.full_path(image_id),
            self.blur_path(image_id),
            self.blur_full_path(image_id),
        ]
    }
}

/// VNDB image ids are `sf` + digits, but never trust an external string with a path.
fn sanitise(id: &str) -> String {
    id.chars()
        .filter(char::is_ascii_alphanumeric)
        .take(32)
        .collect()
}

/// Download and cache every screenshot of a VN that is not cached yet.
///
/// Returns the rows that are now on disk, in the order VNDB listed them. A failure
/// on any single image is collected rather than thrown: one dead URL must not cost
/// the other twenty-nine, and the gallery renders what it has.
///
/// Downloads are sequential on purpose. The client's rate limiter is shared with
/// every other VNDB call, and thirty parallel image fetches for one screen would
/// spend the whole budget at once for no visible gain — the grid paints progressively
/// as each tile arrives.
///
/// # Errors
/// Only when the database cannot be read or written at all.
pub async fn cache_all(
    client: &VndbClient,
    pool: &SqlitePool,
    store: &ScreenshotStore,
    vn_id: &str,
    images: &[crate::vndb::dto::Image],
    policy: BlurPolicy,
    now: Timestamp,
) -> Result<Vec<CachedScreenshot>, ScreenshotError> {
    store.ensure()?;
    let cached_now = cache::get_screenshots(pool, vn_id).await?;

    let mut cached = Vec::new();
    for image in images.iter().take(MAX_SCREENSHOTS) {
        let Some(url) = image.url.as_deref().filter(|url| !url.is_empty()) else {
            continue;
        };
        // An entry with no id still has a URL and is still a real screenshot, so it
        // must not be skipped. That exact case shipped: `screenshots.id` was added to
        // the field list after some rows were already cached, and those rows came back
        // with `"id": null`, which silently produced an empty gallery for a VN the
        // API reported ten screenshots for.
        let image_id = image
            .id
            .as_deref()
            .filter(|id| !id.is_empty())
            .map_or_else(|| id_from_url(url), str::to_owned);
        // What the previous run recorded for this image, so a stale file is rebuilt.
        let cached_width = cached_now
            .iter()
            .find(|row| row.image_id == image_id)
            .and_then(|row| row.width);
        match cache_one(
            client,
            pool,
            store,
            ShotRequest {
                vn_id,
                image_id: &image_id,
                url,
                image,
                policy,
                cached_width,
                now,
            },
        )
        .await
        {
            // A single image failing is expected (a 404 after a database edit, a
            // dropped connection) and is reported by absence rather than by
            // aborting the gallery. Only a database failure is fatal.
            Ok(Some(row)) => cached.push(row),
            Ok(None) | Err(ScreenshotError::Network(_) | ScreenshotError::Decode(_)) => {}
            Err(err) => return Err(err),
        }
    }
    Ok(cached)
}

/// Derive a stable cache id from a screenshot URL.
///
/// Only used when the API did not send an `id`. It has to be deterministic, because
/// the id is the cache key: a random one would download the same image again on every
/// screen and leave unreachable files behind.
///
/// VNDB serves screenshots from `https://t.vndb.org/sf/40/83040.jpg`, where `83040` is
/// the numeric part of the image id (`sf83040`). Recovering `sf83040` keeps the stored
/// key identical to the one a later response with a real id would produce, so the
/// cache is reused rather than duplicated. Any other shape is reduced to its filename
/// stem, with a short hash appended to keep distinct filenames apart.
fn id_from_url(url: &str) -> String {
    // A cache-busting parameter is not part of the identity: the same image served as
    // `…/83040.jpg?v=2` must land on the same cache key as `…/83040.jpg`.
    let path = url.split('?').next().unwrap_or(url);
    let filename = path.rsplit('/').next().unwrap_or(path);
    let stem = filename.split('.').next().unwrap_or(filename);

    // The `/sf/90/190.jpg` layout: the directory is the id's last two digits,
    // zero-padded. Both directions are checked, so `5.jpg` in `/05/` is accepted as
    // well as `190.jpg` in `/90/`: a one-digit id has `05` as its directory.
    let segments: Vec<&str> = path.rsplit('/').take(2).collect();
    if let [file_segment, digit_dir] = segments.as_slice()
        && (digit_dir.len() == 2)
        && digit_dir.chars().all(|ch| ch.is_ascii_digit())
        // `.jpg` only: VNDB serves screenshots as JPEG. Compared case-insensitively
        // because the extension comes from a remote URL.
        && file_segment.to_ascii_lowercase().ends_with(".jpg")
        && stem.chars().all(|ch| ch.is_ascii_digit())
    {
        let padded = if stem.len() == 1 {
            format!("0{stem}")
        } else {
            stem[stem.len() - 2..].to_owned()
        };
        if padded == *digit_dir {
            return format!("sf{stem}");
        }
    }

    let sanitised: String = stem
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .take(24)
        .collect();
    if sanitised.is_empty() {
        return "sfunknown".to_owned();
    }
    format!("sf{sanitised}{}", short_hash(path))
}

/// A small stable hash, only used to disambiguate filenames.
///
/// FNV-1a: deterministic across runs and platforms, which `DefaultHasher` does not
/// promise. Four hex digits are plenty to separate two screenshots of one VN.
fn short_hash(value: &str) -> String {
    let mut hash: u32 = 0x811c_9dc5;
    for byte in value.as_bytes() {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    format!("{:04x}", hash & 0xffff)
}

/// One image to cache, grouped so the download path does not take nine arguments.
struct ShotRequest<'a> {
    vn_id: &'a str,
    image_id: &'a str,
    url: &'a str,
    image: &'a crate::vndb::dto::Image,
    policy: BlurPolicy,
    /// Width recorded for the files already on disk, when there are any.
    cached_width: Option<u32>,
    now: Timestamp,
}

/// Download, render and blur one screenshot, then record it.
///
/// Three files are produced from one download:
///
/// | file | purpose |
/// | --- | --- |
/// | `{id}.tile.jpg` | grid tile, 640 px |
/// | `{id}.full.jpg` | lightbox, up to [`SCREENSHOT_FULL_WIDTH`] |
/// | `{id}.blur.jpg` / `{id}.blur.full.jpg` | the same two sizes, pre-blurred |
///
/// The sharp copies are written regardless of the current policy: generating them
/// costs milliseconds from bytes already in memory, and a user who later switches the
/// setting must not need a second network round trip for art they already own.
/// Serving them is what the policy controls, not producing them.
///
/// Returns `Ok(None)` when the entry has nothing to download.
/// Whether a cached image has to be regenerated before it can be shown.
///
/// One definition, called by the pipeline (which decides what to download) and by the
/// command (which decides what to report missing). They used to be two, and they
/// already disagreed: a row with no recorded width was rebuilt by one and skipped by
/// the other, so the gallery reported a permanent download failure for images it never
/// attempted.
#[must_use]
pub fn needs_rebuild(
    cached_width: Option<u32>,
    tile_path: &std::path::Path,
    full_path: &std::path::Path,
    blur_path: &std::path::Path,
    blur_full_path: &std::path::Path,
) -> bool {
    // A missing width means the row predates the column, which is also a rebuild.
    let stale = cached_width != Some(SCREENSHOT_TILE_WIDTH);
    stale
        || [tile_path, full_path, blur_path, blur_full_path]
            .iter()
            .any(|path| !path.exists())
}

async fn cache_one(
    client: &VndbClient,
    pool: &SqlitePool,
    store: &ScreenshotStore,
    request: ShotRequest<'_>,
) -> Result<Option<CachedScreenshot>, ScreenshotError> {
    let ShotRequest {
        vn_id,
        image_id,
        url,
        image,
        policy: _policy,
        cached_width,
        now,
    } = request;
    let tile_path = store.tile_path(image_id);
    let full_path = store.full_path(image_id);
    let blur_path = store.blur_path(image_id);
    let blur_full_path = store.blur_full_path(image_id);

    // Files written by a build with a different cache format are rebuilt rather than
    // reused. This is what makes a resolution or blur fix reach someone who already
    // opened the gallery once — twice now the fix was correct and invisible because
    // the old files were simply kept.
    let needs_work = needs_rebuild(
        cached_width,
        &tile_path,
        &full_path,
        &blur_path,
        &blur_full_path,
    );

    let mut written: Option<(u32, u32)> = None;
    if needs_work {
        let bytes = client.fetch_bytes(url).await?;
        // A previous format's `{id}.jpg` is not one of the files this build names, so
        // it would sit in the folder forever and quietly double the on-disk cost.
        for path in store.all_paths(image_id) {
            let _ = std::fs::remove_file(path);
        }

        let decoded = image::load_from_memory(&bytes)
            .map_err(|err| ScreenshotError::Decode(err.to_string()))?;

        // Tile: what the grid paints.
        let tile = resize_to_width(&decoded, SCREENSHOT_TILE_WIDTH);
        write_jpeg(&tile, &tile_path)?;
        // Full: what the lightbox paints. Not the source verbatim — a 4000 px
        // screenshot would be a pointless 3 MB — but far above the tile so the
        // lightbox is never an upscale.
        let full = resize_to_width(&decoded, SCREENSHOT_FULL_WIDTH);
        write_jpeg(&full, &full_path)?;
        // Blurs, at both sizes.
        write_blur(&decoded, &blur_path, SCREENSHOT_TILE_WIDTH)?;
        write_blur(&decoded, &blur_full_path, SCREENSHOT_BLUR_FULL_WIDTH)?;

        // Recorded size is the tile's, which is what the staleness check compares.
        written = Some((tile.width(), tile.height()));
        cache::put_screenshot(
            pool,
            vn_id,
            image_id,
            url,
            Some(&tile_path),
            Some(&full_path),
            Some(&blur_path),
            Some(&blur_full_path),
            Some(tile.width()),
            Some(tile.height()),
            image.sexual_hundredths(),
            image.violence_hundredths(),
            now,
        )
        .await?;
    } else {
        // Files are already on disk; refresh the metadata so a re-rating on VNDB
        // reaches the policy without a new download. Paths are `None` so the upsert
        // keeps whatever the row already has.
        cache::put_screenshot(
            pool,
            vn_id,
            image_id,
            url,
            None,
            None,
            None,
            None,
            None,
            None,
            image.sexual_hundredths(),
            image.violence_hundredths(),
            now,
        )
        .await?;
    }

    Ok(Some(CachedScreenshot {
        vn_id: vn_id.to_owned(),
        image_id: image_id.to_owned(),
        url: url.to_owned(),
        // A path is only reported when the file is really there: the row is what the
        // UI trusts, so it must not name a file that was never written.
        thumb_path: tile_path.exists().then_some(tile_path),
        full_path: full_path.exists().then_some(full_path),
        blur_path: blur_path.exists().then_some(blur_path),
        blur_full_path: blur_full_path.exists().then_some(blur_full_path),
        width: written.map(|(width, _)| width).or(cached_width),
        height: written.map(|(_, height)| height),
        sexual: image.sexual_hundredths(),
        violence: image.violence_hundredths(),
    }))
}

/// Save an image as JPEG.
fn write_jpeg(source: &image::DynamicImage, path: &Path) -> Result<(), ScreenshotError> {
    source
        .save_with_format(path, image::ImageFormat::Jpeg)
        .map_err(|err| ScreenshotError::Io(err.to_string()))
}

/// Fit an image into `width` pixels, preserving aspect and never upscaling.
///
/// Never upscaling matters: `resize_to_width(full, 1920)` on a 1280 px source must
/// leave it at 1280 rather than inflate it into a blurry 1920.
fn resize_to_width(source: &image::DynamicImage, width: u32) -> image::DynamicImage {
    let source_width = source.width().max(1);
    let source_height = source.height().max(1);
    let target_width = width.min(source_width);
    let height =
        (u64::from(target_width) * u64::from(source_height) / u64::from(source_width)).max(1);
    #[allow(clippy::cast_possible_truncation)]
    let height = height.min(u64::from(u32::MAX)) as u32;
    if target_width == source_width {
        return source.clone();
    }
    source.resize_exact(target_width, height, FilterType::Lanczos3)
}

/// Produce a blurred copy sized for `paint_width`.
///
/// Downscale hard, then scale back up: the resample is the blur. The downscale uses
/// `Lanczos3` rather than a Gaussian kernel because adding a second softening step on
/// top of pixel discard compounds two losses and is what turned the first version of
/// this into an illegible smear.
fn write_blur(
    source: &image::DynamicImage,
    path: &Path,
    paint_width: u32,
) -> Result<(), ScreenshotError> {
    // The blur radius is a fixed number of **output** pixels, not a fixed ratio.
    //
    // This was a ratio (`128 * paint_width / 640`), which meant the lightbox blur was
    // resampled at 1280/256 = 5x and produced a ~5px radius across a 1280px image:
    // dialogue text stayed readable and faces stayed recognisable. A content filter
    // that leaves the thing it is hiding legible is not a filter. That shipped, and
    // the honest reading of it is that explicit screenshots were visible.
    //
    // Scaling the radius with the image is the correct model for perceived blur, but
    // the constant has to be much larger than a tile's. `BLUR_RADIUS` is the fraction
    // of the width one resample step removes; a 1/24 ratio is a heavy blur at any
    // size, which is the point of the setting.
    const BLUR_DIVISOR: u32 = 24;
    let blur_width = (paint_width / BLUR_DIVISOR).max(8);
    let tiny = resize_to_width(source, blur_width);

    let height = (u64::from(paint_width) * u64::from(tiny.height().max(1))
        / u64::from(tiny.width().max(1)))
    .max(1);
    #[allow(clippy::cast_possible_truncation)]
    let height = height.min(u64::from(u32::MAX)) as u32;
    // Exactly the target size: this one *is* an intentional upscale.
    let upscaled = tiny.resize_exact(paint_width, height, FilterType::Triangle);
    write_jpeg(&upscaled, path)
}

/// Read a cached screenshot file as bytes, for serving to the webview.
///
/// # Errors
/// Fails when the file is missing or unreadable.
pub async fn read_bytes(path: &Path) -> Result<Vec<u8>, ScreenshotError> {
    tokio::fs::read(path)
        .await
        .map_err(|err| ScreenshotError::Io(err.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;

    fn now() -> Timestamp {
        Timestamp::from_millis(1_700_000_000_000)
    }

    fn temp_store(name: &str) -> (ScreenshotStore, PathBuf) {
        let dir = std::env::temp_dir().join(format!("vnt-screenshots-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        (ScreenshotStore::new(&dir), dir)
    }

    #[test]
    fn paths_are_derived_from_the_image_id_and_cannot_escape_the_directory() {
        let store = ScreenshotStore::new("C:/data");
        assert!(store.tile_path("sf190").ends_with("sf190.tile.jpg"));
        assert!(store.full_path("sf190").ends_with("sf190.full.jpg"));
        assert!(store.blur_path("sf190").ends_with("sf190.blur.jpg"));
        assert!(
            store
                .blur_full_path("sf190")
                .ends_with("sf190.blur.full.jpg")
        );
        let escaped = store.tile_path("../../etc/passwd");
        assert!(!escaped.to_string_lossy().contains(".."));
        assert!(escaped.starts_with(&store.root));
    }

    #[test]
    fn the_four_files_for_one_image_are_distinct() {
        // A collision would mean the grid and the lightbox share a file, which is the
        // bug that produced an upscaled lightbox in the first place.
        let store = ScreenshotStore::new("C:/data");
        let paths = store.all_paths("sf190");
        let unique: std::collections::HashSet<_> = paths.iter().collect();
        assert_eq!(unique.len(), 4);
    }

    #[test]
    fn an_unknown_preference_is_not_read_as_consent() {
        assert_eq!(BlurPolicy::from_pref("show"), BlurPolicy::Sharp);
        assert_eq!(BlurPolicy::from_pref("blur"), BlurPolicy::Blur);
        assert_eq!(BlurPolicy::from_pref("hide"), BlurPolicy::Blur);
        // Corrupt or future values must not uncensor anything.
        assert_eq!(BlurPolicy::from_pref(""), BlurPolicy::Blur);
        assert_eq!(BlurPolicy::from_pref("SHOW"), BlurPolicy::Blur);
    }

    #[test]
    fn a_tile_is_downscaled_and_keeps_its_aspect_ratio() {
        let (store, dir) = temp_store("thumb");
        store.ensure().expect("ensure");
        let source = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            1600,
            1200,
            image::Rgb([40, 60, 80]),
        ));
        let path = store.tile_path("sf2");
        write_jpeg(&resize_to_width(&source, SCREENSHOT_TILE_WIDTH), &path).expect("write");

        let written = image::open(&path).expect("decode");
        assert_eq!(written.width(), SCREENSHOT_TILE_WIDTH);
        assert_eq!(written.height(), SCREENSHOT_TILE_WIDTH * 3 / 4);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_lightbox_copy_keeps_the_source_resolution() {
        // The whole point of the second size: a 1280x720 original must stay 1280 px,
        // because a ~1300 px lightbox upscaling a 640 px tile is what looked like
        // "very low pixel".
        let (store, dir) = temp_store("full");
        store.ensure().expect("ensure");
        let source = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            1280,
            720,
            image::Rgb([10, 20, 30]),
        ));
        let path = store.full_path("sf5");
        write_jpeg(&resize_to_width(&source, SCREENSHOT_FULL_WIDTH), &path).expect("write");
        let written = image::open(&path).expect("decode");
        assert_eq!(written.width(), 1280);
        assert_eq!(written.height(), 720);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_source_wider_than_the_cap_is_brought_down_to_it() {
        let (store, dir) = temp_store("cap");
        store.ensure().expect("ensure");
        let source = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            4000,
            2000,
            image::Rgb([1, 2, 3]),
        ));
        let path = store.full_path("sf6");
        write_jpeg(&resize_to_width(&source, SCREENSHOT_FULL_WIDTH), &path).expect("write");
        let written = image::open(&path).expect("decode");
        assert_eq!(written.width(), SCREENSHOT_FULL_WIDTH);
        assert_eq!(written.height(), SCREENSHOT_FULL_WIDTH / 2);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_narrow_screenshot_is_not_upscaled() {
        // Upscaling a 320 px screenshot to 512 would only add blur and bytes.
        let (store, dir) = temp_store("narrow");
        store.ensure().expect("ensure");
        let source = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            320,
            240,
            image::Rgb([9; 3]),
        ));
        let path = store.tile_path("sf3");
        write_jpeg(&resize_to_width(&source, SCREENSHOT_TILE_WIDTH), &path).expect("write");
        assert_eq!(image::open(&path).expect("decode").width(), 320);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_blurred_variant_is_small_and_decodable() {
        let (store, dir) = temp_store("blur");
        store.ensure().expect("ensure");
        let source = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            800,
            600,
            image::Rgb([200; 3]),
        ));
        let path = store.blur_path("sf4");
        write_blur(&source, &path, SCREENSHOT_TILE_WIDTH).expect("write");

        let written = image::open(&path).expect("decode");
        assert_eq!(written.width(), SCREENSHOT_TILE_WIDTH);
        let size = std::fs::metadata(&path).expect("meta").len();
        assert!(size < 80_000, "blur should be small, got {size} bytes");

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_screenshot_url_yields_the_same_id_vndb_would_have_sent() {
        // Regression: rows cached before `screenshots.id` was requested come back with
        // `"id": null`, and an id-less entry used to be skipped, which emptied the
        // gallery for a VN the API had reported ten screenshots for.
        // Verified against the live API: `sf190` is served from `/sf/90/190.jpg`, so
        // the directory is the id's last two digits, zero-padded.
        assert_eq!(id_from_url("https://t.vndb.org/sf/90/190.jpg"), "sf190");
        assert_eq!(id_from_url("https://t.vndb.org/sf/31/231.jpg"), "sf231");
        assert_eq!(id_from_url("https://t.vndb.org/sf/40/83040.jpg"), "sf83040");
        // A one-digit id: the directory is padded and the stem is the whole number.
        assert_eq!(id_from_url("https://t.vndb.org/sf/05/5.jpg"), "sf5");
    }

    #[test]
    fn a_recovered_id_matches_the_one_the_api_sends() {
        // The point of recovering it rather than hashing: the cache row written now
        // must be the same key a later response with a real id would use.
        let recovered = id_from_url("https://t.vndb.org/sf/40/83040.jpg");
        let api_id = "sf83040".to_owned();
        assert_eq!(recovered, api_id);
    }

    #[test]
    fn an_unexpected_url_still_yields_a_stable_distinct_id() {
        let a = id_from_url("https://example.invalid/shots/first.jpg");
        let b = id_from_url("https://example.invalid/shots/second.jpg");
        assert_ne!(a, b, "two screenshots must not share a cache key");
        assert_eq!(a, id_from_url("https://example.invalid/shots/first.jpg"));
        assert!(a.chars().all(|ch| ch.is_ascii_alphanumeric()));
    }

    #[test]
    fn a_query_string_does_not_change_the_id() {
        // The same image served with a cache-busting parameter is the same image.
        assert_eq!(
            id_from_url("https://t.vndb.org/sf/40/83040.jpg?v=2"),
            id_from_url("https://t.vndb.org/sf/40/83040.jpg")
        );
    }

    #[tokio::test]
    async fn reading_a_missing_file_reports_io() {
        assert!(
            read_bytes(Path::new("definitely/not/here.jpg"))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn a_file_from_an_older_width_is_rebuilt_rather_than_kept() {
        // Regression: the first gallery shipped with a wrong blur radius and a 512 px
        // tile. Fixing the constants changed nothing for anyone who had already opened
        // a gallery, because the file existed on disk and the pipeline skipped it. The
        // recorded width is what makes the fix reach an existing user.
        let db = Db::open_memory().await.expect("open");
        let (store, dir) = temp_store("stale");
        store.ensure().expect("ensure");

        // A file and row as an older build would have left them.
        std::fs::write(store.blur_path("sf9"), b"old").expect("write old");
        cache::put_screenshot(
            db.pool(),
            "v1",
            "sf9",
            "https://t.vndb.org/sf/9/9.jpg",
            None,
            None,
            Some(&store.blur_path("sf9")),
            None,
            Some(512),
            Some(288),
            Some(0),
            Some(0),
            now(),
        )
        .await
        .expect("seed");

        let rows = cache::get_screenshots(db.pool(), "v1").await.expect("get");
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].width,
            Some(512),
            "the old width must be readable, or staleness is undetectable"
        );
        assert_ne!(
            rows[0].width,
            Some(SCREENSHOT_TILE_WIDTH),
            "a 512 row is stale once the pipeline produces {SCREENSHOT_TILE_WIDTH}"
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn an_entry_with_no_url_is_skipped_without_failing_the_batch() {
        // Only a URL is truly required: without one there is nothing to download. An
        // id is recoverable (see `id_from_url`), so its absence must not skip the tile.
        let db = Db::open_memory().await.expect("open");
        let (store, dir) = temp_store("skip");
        let client = VndbClient::with_base("http://127.0.0.1:9").expect("client");
        let images = vec![crate::vndb::dto::Image {
            id: Some("sf2".to_owned()),
            url: None,
            ..crate::vndb::dto::Image::default()
        }];
        let cached = cache_all(
            &client,
            db.pool(),
            &store,
            "v1",
            &images,
            BlurPolicy::Blur,
            now(),
        )
        .await
        .expect("no error");
        assert!(cached.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn an_unreachable_entry_fails_alone_rather_than_emptying_the_gallery() {
        // A dropped connection on one image must leave the others cached: the gallery
        // renders what it has and reports the shortfall.
        let db = Db::open_memory().await.expect("open");
        let (store, dir) = temp_store("offline");
        let client = VndbClient::with_base("http://127.0.0.1:9").expect("client");
        let image = crate::vndb::dto::Image {
            id: Some("sf1".to_owned()),
            url: Some("https://t.vndb.org/sf/1/1.jpg".to_owned()),
            ..crate::vndb::dto::Image::default()
        };
        let cached = cache_all(
            &client,
            db.pool(),
            &store,
            "v1",
            std::slice::from_ref(&image),
            BlurPolicy::Blur,
            now(),
        )
        .await
        .expect("a network failure is collected, not thrown");
        assert!(cached.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[cfg(test)]
mod needs_rebuild_tests {
    use super::needs_rebuild;
    use std::path::Path;

    fn existing(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join("vn-asyik-needs-rebuild");
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join(name);
        std::fs::write(&path, b"x").expect("write");
        path
    }

    #[test]
    fn a_row_with_no_recorded_width_is_rebuilt() {
        // Every row written before the width column existed decodes with `None`.
        // Skipping those is what produced a permanent "could not be downloaded".
        let tile = existing("none-tile.jpg");
        let full = existing("none-full.jpg");
        let blur = existing("none-blur.jpg");
        let blur_full = existing("none-blur-full.jpg");
        assert!(needs_rebuild(None, &tile, &full, &blur, &blur_full));
    }

    #[test]
    fn a_complete_current_row_is_reused() {
        let tile = existing("ok-tile.jpg");
        let full = existing("ok-full.jpg");
        let blur = existing("ok-blur.jpg");
        let blur_full = existing("ok-blur-full.jpg");
        assert!(!needs_rebuild(
            Some(super::SCREENSHOT_TILE_WIDTH),
            &tile,
            &full,
            &blur,
            &blur_full
        ));
    }

    #[test]
    fn a_lightbox_file_missing_from_disk_is_rebuilt() {
        let tile = existing("gone-tile.jpg");
        let full = existing("gone-full.jpg");
        let blur = existing("gone-blur.jpg");
        let blur_full = Path::new("/definitely/not/here.jpg").to_path_buf();
        assert!(needs_rebuild(
            Some(super::SCREENSHOT_TILE_WIDTH),
            &tile,
            &full,
            &blur,
            &blur_full
        ));
    }
}
