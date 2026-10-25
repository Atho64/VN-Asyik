//! Cache for VNDB payloads and cover images.
//!
//! Two layers, both local-first:
//!
//! * `vndb_cache` holds raw JSON responses with a TTL, so a repeated search is
//!   served from disk and the app works offline for anything already seen.
//! * `vndb_images` holds downloaded covers plus a pre-blurred version, so the
//!   backdrop can be generated without touching the network.
//!
//! Cache failures are never fatal: a miss simply means "go fetch it".

use std::path::{Path, PathBuf};

use sqlx::SqlitePool;
use vn_core::clock::Timestamp;

use crate::db::DbError;

/// How long a cached search response stays fresh.
pub const SEARCH_TTL_MS: i64 = 24 * 60 * 60 * 1000;
/// How long a cached detail response stays fresh.
pub const DETAIL_TTL_MS: i64 = 7 * 24 * 60 * 60 * 1000;

/// Read a cached payload when it is still fresh.
pub async fn get_fresh(
    pool: &SqlitePool,
    key: &str,
    now: Timestamp,
) -> Result<Option<String>, DbError> {
    let row: Option<(String, i64)> =
        sqlx::query_as("SELECT payload, expires_at FROM vndb_cache WHERE cache_key = ?")
            .bind(key)
            .fetch_optional(pool)
            .await?;
    Ok(row
        .filter(|(_, expires_at)| *expires_at > now.as_millis())
        .map(|(payload, _)| payload))
}

/// Read a cached payload regardless of age. Used as an offline fallback.
pub async fn get_any(pool: &SqlitePool, key: &str) -> Result<Option<String>, DbError> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT payload FROM vndb_cache WHERE cache_key = ?")
            .bind(key)
            .fetch_optional(pool)
            .await?;
    Ok(row.map(|(payload,)| payload))
}

/// Store a payload with a TTL.
pub async fn put(
    pool: &SqlitePool,
    key: &str,
    payload: &str,
    ttl_ms: i64,
    now: Timestamp,
) -> Result<(), DbError> {
    sqlx::query(
        "INSERT INTO vndb_cache (cache_key, payload, fetched_at, expires_at) VALUES (?, ?, ?, ?) \
         ON CONFLICT(cache_key) DO UPDATE SET payload = excluded.payload, \
            fetched_at = excluded.fetched_at, expires_at = excluded.expires_at",
    )
    .bind(key)
    .bind(payload)
    .bind(now.as_millis())
    .bind(now.as_millis() + ttl_ms)
    .execute(pool)
    .await?;
    Ok(())
}

/// A cached cover for one VN.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedImage {
    /// Remote URL the file came from.
    pub url: String,
    /// Absolute path of the full-size file, when downloaded.
    pub local_path: Option<PathBuf>,
    /// Absolute path of the pre-blurred backdrop, when generated.
    pub blur_path: Option<PathBuf>,
    /// Pixel width.
    pub width: Option<u32>,
    /// Pixel height.
    pub height: Option<u32>,
    /// Sexual-content rating in hundredths.
    pub sexual: Option<i64>,
    /// Violence rating in hundredths.
    pub violence: Option<i64>,
}

/// Database row for a cached cover.
#[derive(Debug, sqlx::FromRow)]
struct ImageRow {
    url: String,
    local_path: Option<String>,
    blur_path: Option<String>,
    width: Option<i64>,
    height: Option<i64>,
    sexual: Option<i64>,
    violence: Option<i64>,
}

impl From<ImageRow> for CachedImage {
    fn from(row: ImageRow) -> Self {
        Self {
            url: row.url,
            local_path: row.local_path.map(PathBuf::from),
            blur_path: row.blur_path.map(PathBuf::from),
            width: row.width.and_then(|value| u32::try_from(value).ok()),
            height: row.height.and_then(|value| u32::try_from(value).ok()),
            sexual: row.sexual,
            violence: row.violence,
        }
    }
}

/// Read the cached cover metadata for a VN.
pub async fn get_image(pool: &SqlitePool, vn_id: &str) -> Result<Option<CachedImage>, DbError> {
    let row: Option<ImageRow> = sqlx::query_as(
        "SELECT url, local_path, blur_path, width, height, sexual, violence FROM vndb_images WHERE vn_id = ?",
    )
    .bind(vn_id)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(CachedImage::from))
}

/// Record a downloaded cover.
#[allow(clippy::too_many_arguments)]
pub async fn put_image(
    pool: &SqlitePool,
    vn_id: &str,
    url: &str,
    local_path: Option<&Path>,
    blur_path: Option<&Path>,
    size: (Option<u32>, Option<u32>),
    ratings: (Option<i64>, Option<i64>),
    now: Timestamp,
) -> Result<(), DbError> {
    sqlx::query(
        "INSERT INTO vndb_images (vn_id, url, local_path, blur_path, width, height, sexual, violence, fetched_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT(vn_id) DO UPDATE SET url = excluded.url, local_path = excluded.local_path, \
            blur_path = excluded.blur_path, width = excluded.width, height = excluded.height, \
            sexual = excluded.sexual, violence = excluded.violence, fetched_at = excluded.fetched_at",
    )
    .bind(vn_id)
    .bind(url)
    .bind(local_path.map(|path| path.to_string_lossy().to_string()))
    .bind(blur_path.map(|path| path.to_string_lossy().to_string()))
    .bind(size.0.map(i64::from))
    .bind(size.1.map(i64::from))
    .bind(ratings.0)
    .bind(ratings.1)
    .bind(now.as_millis())
    .execute(pool)
    .await?;
    Ok(())
}

/// A cached screenshot for one VN.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedScreenshot {
    /// Owning VN id.
    pub vn_id: String,
    /// VNDB image id, e.g. `sf190`. The cache key.
    pub image_id: String,
    /// Remote URL the file came from.
    pub url: String,
    /// Absolute path of the sharp thumbnail, when one was written.
    pub thumb_path: Option<PathBuf>,
    /// Absolute path of the lightbox copy, when one was written.
    pub full_path: Option<PathBuf>,
    /// Absolute path of the pre-blurred tile copy, when one was written.
    pub blur_path: Option<PathBuf>,
    /// Absolute path of the pre-blurred lightbox copy, when one was written.
    pub blur_full_path: Option<PathBuf>,
    /// Pixel width of the cached files, as generated.
    ///
    /// Stored so the pipeline can tell a file made by an older, smaller width from a
    /// current one. Without it, changing `SCREENSHOT_WIDTH` left every existing file
    /// in place and the fix never reached a user who already had a gallery — which is
    /// exactly what happened when the blur radius was wrong.
    pub width: Option<u32>,
    /// Pixel height of the cached files, as generated.
    pub height: Option<u32>,
    /// Sexual-content rating in hundredths.
    pub sexual: Option<i64>,
    /// Violence rating in hundredths.
    pub violence: Option<i64>,
}

impl CachedScreenshot {
    /// Whether the community's flagging votes make this explicit.
    ///
    /// The level is rounded by `dto::flag_level` and compared by
    /// `vn_core::model::is_explicit_at_level`, which is the same pair a cover's stored
    /// rating goes through. A screenshot and a cover with identical votes are therefore
    /// concealed by the same setting — before, this path used a `>= 1` test while the
    /// cover used `>= 2`, so the same scene could be blurred in the gallery and shown
    /// in the library.
    #[must_use]
    pub fn is_explicit(&self) -> bool {
        flag_level_is_explicit(self.sexual) || flag_level_is_explicit(self.violence)
    }
}

/// Whether a stored vote counts as explicit.
///
/// Delegates to [`crate::vndb::dto::flag_level`] for the rounding and to the shared
/// `vn_core` comparison for the threshold, so the screenshot gallery, the importer and
/// the frontend cannot drift apart again.
fn flag_level_is_explicit(hundredths: Option<i64>) -> bool {
    hundredths.is_some_and(|value| {
        vn_core::model::is_explicit_at_level(Some(crate::vndb::dto::flag_level(value)))
    })
}

/// Database row for a cached screenshot.
#[derive(Debug, sqlx::FromRow)]
struct ScreenshotRow {
    image_id: String,
    url: String,
    thumb_path: Option<String>,
    full_path: Option<String>,
    blur_path: Option<String>,
    blur_full_path: Option<String>,
    width: Option<i64>,
    height: Option<i64>,
    sexual: Option<i64>,
    violence: Option<i64>,
}

impl ScreenshotRow {
    fn into_cached(self, vn_id: &str) -> CachedScreenshot {
        CachedScreenshot {
            vn_id: vn_id.to_owned(),
            image_id: self.image_id,
            url: self.url,
            thumb_path: self.thumb_path.map(PathBuf::from),
            full_path: self.full_path.map(PathBuf::from),
            blur_path: self.blur_path.map(PathBuf::from),
            blur_full_path: self.blur_full_path.map(PathBuf::from),
            width: self.width.and_then(|value| u32::try_from(value).ok()),
            height: self.height.and_then(|value| u32::try_from(value).ok()),
            sexual: self.sexual,
            violence: self.violence,
        }
    }
}

/// Every cached screenshot for a VN, in VNDB's order.
///
/// `position` is stored rather than relying on rowid: the gallery must keep the
/// order VNDB returned, which the UI reads as "the first screenshots the site shows".
pub async fn get_screenshots(
    pool: &SqlitePool,
    vn_id: &str,
) -> Result<Vec<CachedScreenshot>, DbError> {
    let rows: Vec<ScreenshotRow> = sqlx::query_as(
        "SELECT image_id, url, thumb_path, full_path, blur_path, blur_full_path, \
                width, height, sexual, violence \
         FROM vndb_screenshots WHERE vn_id = ? ORDER BY position ASC",
    )
    .bind(vn_id)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|row| row.into_cached(vn_id)).collect())
}

/// Paths recorded for a single screenshot, ignoring the VN.
///
/// Separate from [`get_screenshots`] because the lightbox asks for one image by id
/// and must not depend on which VN opened it.
pub async fn get_screenshot_paths(
    pool: &SqlitePool,
    image_id: &str,
) -> Result<Option<(Option<PathBuf>, Option<PathBuf>)>, DbError> {
    let row: Option<(Option<String>, Option<String>)> =
        sqlx::query_as("SELECT thumb_path, blur_path FROM vndb_screenshots WHERE image_id = ?")
            .bind(image_id)
            .fetch_optional(pool)
            .await?;
    Ok(row.map(|(thumb, blur)| (thumb.map(PathBuf::from), blur.map(PathBuf::from))))
}

/// Record a cached screenshot.
///
/// The position is not passed in: it is the row's place in the list as stored, and
/// a re-cache of an existing image keeps whatever position it already had. Order is
/// written by [`put_screenshot_at`], which the caller uses when it knows the index.
#[allow(clippy::too_many_arguments)]
pub async fn put_screenshot(
    pool: &SqlitePool,
    vn_id: &str,
    image_id: &str,
    url: &str,
    tile_path: Option<&Path>,
    full_path: Option<&Path>,
    blur_path: Option<&Path>,
    blur_full_path: Option<&Path>,
    width: Option<u32>,
    height: Option<u32>,
    sexual: Option<i64>,
    violence: Option<i64>,
    now: Timestamp,
) -> Result<(), DbError> {
    sqlx::query(
        "INSERT INTO vndb_screenshots (image_id, vn_id, url, thumb_path, full_path, \
            blur_path, blur_full_path, width, height, sexual, violence, position, fetched_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, \
            (SELECT COALESCE(MAX(position), -1) + 1 FROM vndb_screenshots WHERE vn_id = ?), ?) \
         ON CONFLICT(image_id) DO UPDATE SET url = excluded.url, vn_id = excluded.vn_id, \
            thumb_path = COALESCE(excluded.thumb_path, vndb_screenshots.thumb_path), \
            full_path = COALESCE(excluded.full_path, vndb_screenshots.full_path), \
            blur_path = COALESCE(excluded.blur_path, vndb_screenshots.blur_path), \
            blur_full_path = COALESCE(excluded.blur_full_path, vndb_screenshots.blur_full_path), \
            width = COALESCE(excluded.width, vndb_screenshots.width), \
            height = COALESCE(excluded.height, vndb_screenshots.height), \
            sexual = excluded.sexual, violence = excluded.violence, \
            fetched_at = excluded.fetched_at",
    )
    .bind(image_id)
    .bind(vn_id)
    .bind(url)
    .bind(tile_path.map(|path| path.to_string_lossy().to_string()))
    .bind(full_path.map(|path| path.to_string_lossy().to_string()))
    .bind(blur_path.map(|path| path.to_string_lossy().to_string()))
    .bind(blur_full_path.map(|path| path.to_string_lossy().to_string()))
    .bind(width.map(i64::from))
    .bind(height.map(i64::from))
    .bind(sexual)
    .bind(violence)
    .bind(vn_id)
    .bind(now.as_millis())
    .execute(pool)
    .await?;
    Ok(())
}

/// Rewrite the gallery order for a VN in one transaction.
///
/// VNDB's own order is what the gallery shows; a set that arrives in a different
/// order (screenshots added, removed or re-ordered on the site) must not leave the
/// tiles scrambled against the list the API just returned.
pub async fn set_order(pool: &SqlitePool, image_ids: &[String]) -> Result<(), DbError> {
    let mut tx = pool.begin().await?;
    for (position, image_id) in image_ids.iter().enumerate() {
        // A gallery is capped at `MAX_SCREENSHOTS` (40), so the narrowing is bounded
        // by construction, not by trusting the caller.
        let position = i64::try_from(position).unwrap_or(i64::MAX);
        sqlx::query("UPDATE vndb_screenshots SET position = ? WHERE image_id = ?")
            .bind(position)
            .bind(image_id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;

    fn now() -> Timestamp {
        Timestamp::from_millis(1_700_000_000_000)
    }

    #[tokio::test]
    async fn payloads_expire_but_remain_readable_as_a_fallback() {
        let db = Db::open_memory().await.expect("open");
        put(
            db.pool(),
            "search:umineko",
            "{\"results\":[]}",
            1_000,
            now(),
        )
        .await
        .expect("put");

        let fresh = get_fresh(db.pool(), "search:umineko", now())
            .await
            .expect("get");
        assert_eq!(fresh.as_deref(), Some("{\"results\":[]}"));

        let later = Timestamp::from_millis(now().as_millis() + 5_000);
        assert!(
            get_fresh(db.pool(), "search:umineko", later)
                .await
                .expect("get")
                .is_none()
        );
        // Offline fallback still has it.
        assert!(
            get_any(db.pool(), "search:umineko")
                .await
                .expect("any")
                .is_some()
        );
    }

    #[tokio::test]
    async fn put_replaces_an_existing_entry() {
        let db = Db::open_memory().await.expect("open");
        put(db.pool(), "k", "first", 1_000, now())
            .await
            .expect("put");
        put(db.pool(), "k", "second", 1_000, now())
            .await
            .expect("put");
        assert_eq!(
            get_any(db.pool(), "k").await.expect("any").as_deref(),
            Some("second")
        );
    }

    #[tokio::test]
    async fn missing_keys_are_none() {
        let db = Db::open_memory().await.expect("open");
        assert!(
            get_fresh(db.pool(), "nope", now())
                .await
                .expect("get")
                .is_none()
        );
        assert!(get_any(db.pool(), "nope").await.expect("any").is_none());
    }

    #[tokio::test]
    async fn images_round_trip_with_ratings_and_paths() {
        let db = Db::open_memory().await.expect("open");
        put_image(
            db.pool(),
            "v24",
            "https://t.vndb.org/cv/24/1.jpg",
            Some(Path::new("/tmp/v24.jpg")),
            Some(Path::new("/tmp/v24.blur.jpg")),
            (Some(256), Some(360)),
            (Some(150), Some(40)),
            now(),
        )
        .await
        .expect("put");

        let image = get_image(db.pool(), "v24")
            .await
            .expect("get")
            .expect("present");
        assert_eq!(image.width, Some(256));
        assert_eq!(image.height, Some(360));
        assert_eq!(image.sexual, Some(150));
        assert_eq!(image.violence, Some(40));
        assert_eq!(image.local_path.as_deref(), Some(Path::new("/tmp/v24.jpg")));
    }

    async fn put_shot(db: &Db, vn_id: &str, image_id: &str, sexual: Option<i64>) {
        put_screenshot(
            db.pool(),
            vn_id,
            image_id,
            &format!("https://t.vndb.org/sf/{image_id}.jpg"),
            Some(Path::new("/tmp/tile.jpg")),
            Some(Path::new("/tmp/full.jpg")),
            Some(Path::new("/tmp/blur.jpg")),
            Some(Path::new("/tmp/blur.full.jpg")),
            Some(640),
            Some(480),
            sexual,
            Some(0),
            now(),
        )
        .await
        .expect("put screenshot");
    }

    #[tokio::test]
    async fn screenshots_round_trip_in_vndb_order() {
        let db = Db::open_memory().await.expect("open");
        put_shot(&db, "v17", "sf3", Some(0)).await;
        put_shot(&db, "v17", "sf1", Some(0)).await;
        put_shot(&db, "v17", "sf2", Some(150)).await;

        let shots = get_screenshots(db.pool(), "v17").await.expect("get");
        // Insertion order, not sorted by id: the gallery must match the API's list.
        assert_eq!(
            shots
                .iter()
                .map(|s| s.image_id.as_str())
                .collect::<Vec<_>>(),
            vec!["sf3", "sf1", "sf2"]
        );
        assert_eq!(shots[2].sexual, Some(150));
        assert!(shots[2].is_explicit());
        assert!(!shots[0].is_explicit());
    }

    #[tokio::test]
    async fn set_order_rewrites_the_gallery_sequence() {
        let db = Db::open_memory().await.expect("open");
        put_shot(&db, "v17", "sf1", Some(0)).await;
        put_shot(&db, "v17", "sf2", Some(0)).await;
        set_order(db.pool(), &["sf2".to_owned(), "sf1".to_owned()])
            .await
            .expect("order");
        let shots = get_screenshots(db.pool(), "v17").await.expect("get");
        assert_eq!(
            shots
                .iter()
                .map(|s| s.image_id.as_str())
                .collect::<Vec<_>>(),
            vec!["sf2", "sf1"]
        );
    }

    #[tokio::test]
    async fn re_caching_keeps_a_position_and_fills_missing_paths() {
        // A metadata-only refresh has no new file to write, so it must not clear the
        // path a previous download recorded and leave the gallery unable to paint.
        let db = Db::open_memory().await.expect("open");
        put_shot(&db, "v17", "sf1", Some(0)).await;
        put_screenshot(
            db.pool(),
            "v17",
            "sf1",
            "https://t.vndb.org/sf/new.jpg",
            None,
            None,
            None,
            None,
            None,
            None,
            Some(200),
            Some(0),
            now(),
        )
        .await
        .expect("re-put");

        let shots = get_screenshots(db.pool(), "v17").await.expect("get");
        assert_eq!(shots.len(), 1);
        assert_eq!(shots[0].url, "https://t.vndb.org/sf/new.jpg");
        // A metadata-only pass must not clear the paths a download recorded, at
        // either size.
        assert_eq!(
            shots[0].thumb_path.as_deref(),
            Some(Path::new("/tmp/tile.jpg"))
        );
        assert_eq!(
            shots[0].full_path.as_deref(),
            Some(Path::new("/tmp/full.jpg"))
        );
        assert_eq!(shots[0].sexual, Some(200));
    }

    #[tokio::test]
    async fn screenshots_are_scoped_to_their_vn() {
        let db = Db::open_memory().await.expect("open");
        put_shot(&db, "v17", "sf1", Some(0)).await;
        put_shot(&db, "v24", "sf9", Some(0)).await;
        assert_eq!(
            get_screenshots(db.pool(), "v17").await.expect("get").len(),
            1
        );
        assert!(
            get_screenshots(db.pool(), "v999")
                .await
                .expect("get")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn screenshot_paths_are_readable_by_image_id_alone() {
        let db = Db::open_memory().await.expect("open");
        put_shot(&db, "v17", "sf1", Some(0)).await;
        let paths = get_screenshot_paths(db.pool(), "sf1")
            .await
            .expect("get")
            .expect("present");
        assert_eq!(paths.0.as_deref(), Some(Path::new("/tmp/tile.jpg")));
        assert!(
            get_screenshot_paths(db.pool(), "sf404")
                .await
                .expect("get")
                .is_none()
        );
    }

    #[tokio::test]
    async fn both_sizes_round_trip() {
        // The lightbox reads a different file from the grid. If only one path were
        // stored, whichever view is not it would be an upscale.
        let db = Db::open_memory().await.expect("open");
        put_shot(&db, "v17", "sf1", Some(0)).await;
        let rows = get_screenshots(db.pool(), "v17").await.expect("get");
        assert_eq!(rows[0].width, Some(640));
        assert_eq!(
            rows[0].blur_full_path.as_deref(),
            Some(Path::new("/tmp/blur.full.jpg"))
        );
    }

    #[test]
    fn an_unrated_screenshot_is_not_explicit() {
        let shot = CachedScreenshot {
            vn_id: "v1".to_owned(),
            image_id: "sf1".to_owned(),
            url: String::new(),
            thumb_path: None,
            full_path: None,
            blur_path: None,
            blur_full_path: None,
            width: None,
            height: None,
            sexual: None,
            violence: None,
        };
        // Missing ratings mean "VNDB does not know", which must not be read as either
        // consent or a reason to hide.
        assert!(!shot.is_explicit());
    }
}
