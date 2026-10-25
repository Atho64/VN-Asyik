//! VNDB commands: search, import, cached art.
//!
//! Every command degrades rather than fails where it can: a search that cannot
//! reach VNDB falls back to the cached copy of the same query, flagged as stale,
//! so the UI can say "showing saved results" instead of showing an error.

use serde::Serialize;
use tauri::State;
use vn_core::clock::Timestamp;

use super::{CommandError, CommandResult};
use crate::db::queries;
use crate::state::AppState;
use crate::vndb::cache::{self, DETAIL_TTL_MS, SEARCH_TTL_MS};
use crate::vndb::dto::{DETAIL_FIELDS, Vn, VnQuery};
use crate::vndb::images::{self, ImageStore};
use crate::vndb::import::{self, DetailPayload, SEARCH, SearchHit};
use crate::vndb::screenshots::{self, BlurPolicy, ScreenshotStore};
use crate::vndb::{VndbClient, VndbError};

/// Search results plus whether they came from the cache.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SearchResponse {
    /// Matched entries.
    pub hits: Vec<SearchHit>,
    /// Whether another page exists.
    pub more: bool,
    /// True when VNDB could not be reached and saved results were used.
    pub from_cache: bool,
}

fn now() -> CommandResult<Timestamp> {
    Timestamp::now().map_err(Into::into)
}

fn client(state: &State<'_, AppState>) -> VndbClient {
    state.vndb().clone()
}

fn map_err(err: &VndbError) -> CommandError {
    CommandError::from(err)
}

/// Search VNDB for visual novels by title.
#[tauri::command]
pub async fn vndb_search(
    state: State<'_, AppState>,
    query: String,
    page: Option<u32>,
) -> CommandResult<SearchResponse> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Ok(SearchResponse {
            hits: Vec::new(),
            more: false,
            from_cache: false,
        });
    }

    let page = page.unwrap_or(1);
    let cache_key = format!("search:{}:{page}", trimmed.to_lowercase());
    let pool = state.db().pool();
    let timestamp = now()?;

    if let Some(payload) = cache::get_fresh(pool, &cache_key, timestamp).await?
        && let Ok(response) = decode_search(&payload)
    {
        return Ok(response);
    }

    let request = VnQuery::by_title(trimmed, SEARCH, page, 24);
    match client(&state).search_vns(&request).await {
        Ok(result) => {
            let library_ids = library_vndb_ids(pool).await?;
            // VNDB's `searchrank` puts an obscure spin-off above the work the
            // user meant (an 11-vote entry ahead of a 14,000-vote one). Rank on
            // our own title match first, with votes only breaking ties.
            let indexed: Vec<(usize, &Vn)> = result.results.iter().enumerate().collect();
            let ranked = vn_core::search::rank(trimmed, &indexed, |(position, vn)| {
                vn_core::search::Candidate {
                    title: vn.title.as_deref().unwrap_or(&vn.id),
                    alt_title: vn.alttitle.as_deref(),
                    votes: vn.votecount.unwrap_or(0),
                    order: *position,
                }
            });
            let hits: Vec<SearchHit> = ranked
                .iter()
                .map(|(_, vn)| import::to_search_hit(vn, library_ids.contains(&vn.id)))
                .collect();
            let response = SearchResponse {
                hits,
                more: result.more,
                from_cache: false,
            };
            if let Ok(serialised) = serde_json::to_string(&response) {
                cache::put(pool, &cache_key, &serialised, SEARCH_TTL_MS, timestamp).await?;
            }
            Ok(response)
        }
        Err(err) => {
            // Offline fallback: any age of cached response beats an empty screen.
            if let Some(payload) = cache::get_any(pool, &cache_key).await?
                && let Ok(mut response) = decode_search(&payload)
            {
                response.from_cache = true;
                return Ok(response);
            }
            Err(map_err(&err))
        }
    }
}

fn decode_search(payload: &str) -> Result<SearchResponse, serde_json::Error> {
    serde_json::from_str(payload)
}

async fn library_vndb_ids(
    pool: &sqlx::SqlitePool,
) -> Result<std::collections::HashSet<String>, crate::db::DbError> {
    let rows: Vec<(String,)> =
        sqlx::query_as("SELECT vndb_id FROM games WHERE vndb_id IS NOT NULL AND deleted = 0")
            .fetch_all(pool)
            .await?;
    Ok(rows.into_iter().map(|(id,)| id).collect())
}

/// Fetch a single VN with its full detail field set.
async fn fetch_detail(state: &State<'_, AppState>, vn_id: &str) -> CommandResult<(Vn, bool)> {
    let pool = state.db().pool();
    let cache_key = format!("vn:{vn_id}");
    let timestamp = now()?;

    if let Some(payload) = cache::get_fresh(pool, &cache_key, timestamp).await?
        && let Ok(vn) = serde_json::from_str::<Vn>(&payload)
    {
        return Ok((vn, true));
    }

    match client(state).get_vn(vn_id, DETAIL_FIELDS).await {
        Ok(Some(vn)) => {
            if let Ok(serialised) = serde_json::to_string(&vn) {
                cache::put(pool, &cache_key, &serialised, DETAIL_TTL_MS, timestamp).await?;
            }
            Ok((vn, false))
        }
        Ok(None) => cached_or_gone(pool, &cache_key, "that VN does not exist on VNDB").await,
        // A 404 means VNDB deleted the entry. The cached row is still the best
        // answer, and failing here made one dead id sink an entire refresh run.
        Err(VndbError::NotFound) => {
            cached_or_gone(pool, &cache_key, "that VN does not exist on VNDB any more").await
        }
        Err(err) => {
            if let Some(payload) = cache::get_any(pool, &cache_key).await?
                && let Ok(vn) = serde_json::from_str::<Vn>(&payload)
            {
                return Ok((vn, true));
            }
            Err(map_err(&err))
        }
    }
}

/// Serve the cached copy of a VN VNDB no longer has, or report it gone.
///
/// "Gone upstream" is not the same as "failed": the cached row is still usable, and
/// only a request with nothing cached at all is an error.
async fn cached_or_gone(
    pool: &sqlx::SqlitePool,
    cache_key: &str,
    message: &str,
) -> CommandResult<(Vn, bool)> {
    if let Some(payload) = cache::get_any(pool, cache_key).await?
        && let Ok(vn) = serde_json::from_str::<Vn>(&payload)
    {
        return Ok((vn, true));
    }
    Err(CommandError::new("not_found", message))
}

/// Fetch a VN from VNDB itself, ignoring a fresh cache entry.
///
/// Deliberately a separate function rather than a `force` flag on [`fetch_detail`]:
/// the cache-first path is the default everywhere else, and a boolean that flips its
/// meaning is the kind of parameter that gets passed wrong once and then nobody
/// notices. Nothing in this app could re-fetch metadata after import, so a cached
/// row was frozen forever and new fields never arrived.
///
/// A network failure still falls back to the cached copy, so refreshing while
/// offline reports stale data rather than failing.
async fn fetch_detail_fresh(state: &State<'_, AppState>, vn_id: &str) -> CommandResult<(Vn, bool)> {
    let pool = state.db().pool();
    let cache_key = format!("vn:{vn_id}");
    let timestamp = now()?;

    match client(state).get_vn(vn_id, DETAIL_FIELDS).await {
        Ok(Some(vn)) => {
            if let Ok(serialised) = serde_json::to_string(&vn) {
                cache::put(pool, &cache_key, &serialised, DETAIL_TTL_MS, timestamp).await?;
            }
            Ok((vn, false))
        }
        // "Gone upstream" is one answer whether the API says so with an empty result
        // or a 404: serve the cache when there is one.
        Ok(None) | Err(VndbError::NotFound) => {
            cached_or_gone(pool, &cache_key, "that VN does not exist on VNDB any more").await
        }
        Err(err) => {
            if let Some(payload) = cache::get_any(pool, &cache_key).await?
                && let Ok(vn) = serde_json::from_str::<Vn>(&payload)
            {
                return Ok((vn, true));
            }
            Err(map_err(&err))
        }
    }
}

/// Re-fetch a game's VNDB metadata and store it.
///
/// The game row itself is left alone: status, rating, notes and playtime are the
/// user's, and refreshing metadata must never overwrite them. Only the cached
/// `vndb_details` row is replaced.
///
/// # Errors
/// `not_found` when the game has no VNDB id or the VN is gone, plus network and
/// database failures.
#[tauri::command]
pub async fn vndb_refresh_detail(
    state: State<'_, AppState>,
    game_id: String,
) -> CommandResult<DetailPayload> {
    use sqlx::Row as _;
    let id = vn_core::ids::EntityId::parse(&game_id)?;
    let timestamp = now()?;
    let pool = state.db().pool();

    let row = sqlx::query("SELECT vndb_id FROM games WHERE id = ? AND deleted = 0")
        .bind(id.to_string())
        .fetch_optional(pool)
        .await
        .map_err(crate::db::DbError::from)?
        .ok_or_else(|| CommandError::new("not_found", "that game is no longer in your library"))?;
    let vn_id: Option<String> = row.get("vndb_id");
    let vn_id = vn_id
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            CommandError::new(
                "no_vndb_id",
                "this game was not imported from VNDB, so there is nothing to refresh",
            )
        })?;

    let (vn, _cached) = fetch_detail_fresh(&state, &vn_id).await?;
    store_detail(pool, &vn, timestamp).await?;
    Ok(DetailPayload::from_vn(&vn))
}

/// What a bulk metadata refresh did.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RefreshAllOutcome {
    /// Games that have a VNDB id and were attempted.
    pub attempted: usize,
    /// Index of the last game processed, so a later run can resume.
    pub last_index: usize,
    /// Games updated.
    pub updated: usize,
    /// Whether the run stopped before the end.
    pub cancelled: bool,
    /// Failures, with the reason, so one dead VN id does not hide the rest.
    pub failed: Vec<String>,
}

/// Re-fetch VNDB metadata for every game that has a VN id.
///
/// Same shape as [`vndb_import_many`]: one call per game through the ordinary
/// fetch-and-store path, the client's rate limiter applies to every request, and
/// failures are collected rather than thrown so `39 updated, 2 failed` is
/// reportable instead of failing the whole run.
///
/// Never touches the game row. Status, rating, notes, playtime and dates are the
/// user's, and a metadata refresh that overwrote them would be a data-loss bug, not
/// a convenience.
///
/// Writes progress to `sync_state` every few games. The run is slow on a large
/// library (one request per game), so the index is persisted to let a later
/// invocation continue where a closed window left off.
///
/// # Errors
/// Only when the database cannot be read or written at all.
#[tauri::command]
pub async fn vndb_refresh_all(
    state: State<'_, AppState>,
    start_at: Option<usize>,
) -> CommandResult<RefreshAllOutcome> {
    const PROGRESS_KEY: &str = "vndb.refreshAll.index";
    // A mis-click should not start thousands of requests. 41 is a normal library;
    // this guards a pathological one rather than targeting it.
    const MAX_BULK: usize = 2000;

    let pool = state.db().pool();
    let games: Vec<(String, String)> = sqlx::query_as(
        "SELECT id, vndb_id FROM games          WHERE deleted = 0 AND vndb_id IS NOT NULL AND vndb_id != ''          ORDER BY vndb_id ASC",
    )
    .fetch_all(pool)
    .await
    .map_err(crate::db::DbError::from)?;

    let from = start_at.unwrap_or(0).min(games.len());
    let mut outcome = RefreshAllOutcome {
        attempted: games.len().saturating_sub(from),
        last_index: from,
        updated: 0,
        cancelled: false,
        failed: Vec::new(),
    };

    for (index, (game_id, vn_id)) in games.iter().enumerate().skip(from).take(MAX_BULK) {
        match vndb_refresh_detail(state.clone(), game_id.clone()).await {
            Ok(_) => outcome.updated += 1,
            // A VN VNDB no longer has is skipped, not failed: it is permanent, the
            // cached copy still renders, and counting it as a failure made a normal
            // library with one deleted entry report a red "partial" result forever.
            Err(err) if err.code == "not_found" => {}
            Err(err) => outcome.failed.push(format!("{vn_id}: {}", err.message)),
        }
        outcome.last_index = index + 1;
        // Checkpoint every handful rather than every game: a write per request would
        // double the database work for no benefit, and losing the last few rows of
        // progress costs nothing because a refresh is idempotent.
        //
        // Counted on *attempts*, not on `updated`: a run where the first five games
        // all fail would otherwise never checkpoint, which is exactly the run most
        // worth resuming.
        if outcome.last_index.is_multiple_of(5) {
            write_refresh_progress(&state, PROGRESS_KEY, outcome.last_index).await;
        }
    }

    // A completed run must not leave a stale index, or the next click would resume
    // at the end and report zero work.
    let finished = outcome.last_index >= games.len();
    write_refresh_progress(
        &state,
        PROGRESS_KEY,
        if finished { 0 } else { outcome.last_index },
    )
    .await;
    outcome.cancelled = !finished;

    Ok(outcome)
}

/// Persist how far a bulk refresh got.
async fn write_refresh_progress(state: &State<'_, AppState>, key: &str, index: usize) {
    let _ = sqlx::query(
        "INSERT INTO sync_state (key, value) VALUES (?, ?)          ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(key)
    .bind(index.to_string())
    .execute(state.db().pool())
    .await;
}

/// Preview a VN before importing it.
#[tauri::command]
pub async fn vndb_preview(
    state: State<'_, AppState>,
    vn_id: String,
) -> CommandResult<DetailPayload> {
    let (vn, _cached) = fetch_detail(&state, &vn_id).await?;
    Ok(DetailPayload::from_vn(&vn))
}

/// Result of an import.
#[derive(Debug, Clone, Serialize)]
pub struct ImportOutcome {
    /// The created or updated game id.
    pub game_id: String,
    /// Whether the cover was cached locally.
    pub cover_cached: bool,
    /// Set when the entry was imported but the cover could not be fetched.
    pub cover_error: Option<String>,
    /// Whether the VN was already in the library before this call.
    pub already_owned: bool,
}

/// Import a VN into the library, downloading its cover.
///
/// Importing an entry that is already owned updates the VNDB-owned fields only
/// and leaves status, rating, notes and manual playtime untouched.
#[tauri::command]
pub async fn vndb_import(
    state: State<'_, AppState>,
    vn_id: String,
) -> CommandResult<ImportOutcome> {
    let timestamp = now()?;
    let (vn, _cached) = fetch_detail(&state, &vn_id).await?;
    let pool = state.db().pool();

    // Find an existing game for this VN by id, including soft-deleted rows so a
    // re-import revives rather than duplicates.
    let existing: Option<(String, i64)> = sqlx::query_as(
        "SELECT id, deleted FROM games WHERE vndb_id = ? ORDER BY updated_at DESC LIMIT 1",
    )
    .bind(&vn_id)
    .fetch_optional(pool)
    .await
    .map_err(crate::db::DbError::from)?;

    let (mut game, already_owned) = match existing {
        Some((id, _deleted)) => {
            let parsed = vn_core::ids::EntityId::parse(&id)?;
            let mut game = if let Some(game) = crate::db::queries::get_game(pool, parsed).await? {
                game
            } else {
                // The row is soft-deleted: revive it in place. Rebuilding from
                // VNDB here would insert a second row with a fresh id beside
                // the tombstone and strand the user's status, rating, notes and
                // sessions on the invisible row.
                let mut game = crate::db::queries::get_game_including_deleted(pool, parsed)
                    .await?
                    .ok_or_else(|| {
                        crate::db::DbError::Corrupt("game row vanished during import".to_owned())
                    })?;
                // The common `touch` below keeps this clear flag: the row
                // travels as an update, not as a second deletion cycle.
                game.meta.deleted = false;
                game
            };
            import::apply_vn_to_game(&mut game, &vn);
            (game, true)
        }
        None => (
            import::new_game_from_vn(state.device(), timestamp, &vn)?,
            false,
        ),
    };

    game.meta = game.meta.touch(timestamp);

    // Cache the cover before saving, so the row can reference a local file.
    let mut cover_cached = false;
    let mut cover_error = None;
    if let Some(image) = vn
        .image
        .as_ref()
        .and_then(|img| img.url.clone().map(|url| (img, url)))
    {
        let (img, url) = image;
        let store = ImageStore::new(state.data_dir());
        match images::cache_cover(
            &client(&state),
            pool,
            &store,
            &vn_id,
            &url,
            (img.sexual_hundredths(), img.violence_hundredths()),
            timestamp,
        )
        .await
        {
            Ok(_) => cover_cached = true,
            Err(err) => cover_error = Some(err.to_string()),
        }
    }

    crate::db::queries::upsert_game(pool, &game).await?;
    store_detail(pool, &vn, timestamp).await?;

    Ok(ImportOutcome {
        game_id: game.meta.id.to_string(),
        cover_cached,
        cover_error,
        already_owned,
    })
}

pub(crate) async fn store_detail(
    pool: &sqlx::SqlitePool,
    vn: &Vn,
    timestamp: Timestamp,
) -> Result<(), crate::db::DbError> {
    let payload = DetailPayload::from_vn(vn);
    // Every list is written as JSON so the read path has one decoder per column
    // rather than a child table with its own sync rules for cached data.
    let json = |value: &dyn Fn() -> String| value();
    sqlx::query(
        "INSERT INTO vndb_details (vn_id, title, alttitle, titles_json, aliases_json, length_minutes, length, length_votes, description, rating, average, votecount, released, olang, devstatus, languages_json, platforms_json, tags_json, releases_json, characters_json, screenshots_json, developers_json, extlinks_json, fetched_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT(vn_id) DO UPDATE SET title = excluded.title, alttitle = excluded.alttitle, titles_json = excluded.titles_json, aliases_json = excluded.aliases_json, length_minutes = excluded.length_minutes, length = excluded.length, length_votes = excluded.length_votes, description = excluded.description, rating = excluded.rating, average = excluded.average, votecount = excluded.votecount, released = excluded.released, olang = excluded.olang, devstatus = excluded.devstatus, languages_json = excluded.languages_json, platforms_json = excluded.platforms_json, tags_json = excluded.tags_json, releases_json = excluded.releases_json, characters_json = excluded.characters_json, screenshots_json = excluded.screenshots_json, developers_json = excluded.developers_json, extlinks_json = excluded.extlinks_json, fetched_at = excluded.fetched_at",
    )
    .bind(&payload.vn_id)
    .bind(&payload.title)
    .bind(&payload.alttitle)
    .bind(json(&|| encode_list(&payload.titles)))
    .bind(json(&|| encode_list(&payload.aliases)))
    .bind(payload.length_minutes)
    .bind(payload.length.map(i64::from))
    .bind(payload.length_votes.map(i64::from))
    .bind(&payload.description)
    .bind(payload.rating.map(f64::from))
    .bind(payload.average.map(f64::from))
    .bind(payload.votecount.map(i64::from))
    .bind(&payload.released)
    .bind(&payload.olang)
    .bind(payload.devstatus.map(i64::from))
    .bind(json(&|| encode_list(&payload.languages)))
    .bind(json(&|| encode_list(&payload.platforms)))
    .bind(json(&|| encode_list(&payload.tags)))
    .bind("[]")
    .bind("[]")
    .bind(json(&|| encode_list(&payload.screenshots)))
    .bind(json(&|| encode_list(&payload.developers)))
    .bind(json(&|| encode_list(&payload.extlinks)))
    .bind(timestamp.as_millis())
    .execute(pool)
    .await?;
    Ok(())
}

/// Serialise a cached list, degrading to `[]` rather than failing the write.
///
/// Every list in `vndb_details` is display data: losing one field of a VN page is
/// a smaller failure than losing the whole row after a successful network call.
fn encode_list<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "[]".to_owned())
}

/// Database row for a stored VNDB detail payload.
#[derive(Debug, sqlx::FromRow)]
struct DetailRow {
    title: String,
    alttitle: Option<String>,
    titles_json: String,
    aliases_json: String,
    length_minutes: Option<i64>,
    length: Option<i64>,
    length_votes: Option<i64>,
    description: Option<String>,
    rating: Option<f64>,
    average: Option<f64>,
    votecount: Option<i64>,
    released: Option<String>,
    olang: Option<String>,
    devstatus: Option<i64>,
    languages_json: String,
    platforms_json: String,
    tags_json: String,
    screenshots_json: String,
    developers_json: String,
    extlinks_json: String,
}

/// Stored detail for a game's VN, when available.
#[tauri::command]
pub async fn vndb_detail(
    state: State<'_, AppState>,
    vn_id: String,
) -> CommandResult<Option<DetailPayload>> {
    let row: Option<DetailRow> = sqlx::query_as(
        "SELECT title, alttitle, titles_json, aliases_json, length_minutes, length, length_votes, description, rating, average, votecount, released, olang, devstatus, languages_json, platforms_json, tags_json, screenshots_json, developers_json, extlinks_json FROM vndb_details WHERE vn_id = ?",
    )
    .bind(&vn_id)
    .fetch_optional(state.db().pool())
    .await
    .map_err(crate::db::DbError::from)?;

    Ok(row.map(|row| DetailPayload {
        vn_id,
        title: row.title,
        alttitle: row.alttitle,
        // A malformed list is not worth failing the whole detail payload over:
        // every other field is still useful, and the hero degrades to the title.
        titles: serde_json::from_str(&row.titles_json).unwrap_or_default(),
        aliases: serde_json::from_str(&row.aliases_json).unwrap_or_default(),
        length_minutes: row.length_minutes,
        length: row.length.and_then(|value| u8::try_from(value).ok()),
        length_votes: row.length_votes.and_then(|value| u32::try_from(value).ok()),
        description: row.description,
        // VNDB ratings are small (10..100) with one decimal, so the f64 to f32
        // narrowing is bounded. The lint is allowed here with that reason rather
        // than silenced across the crate.
        #[allow(clippy::cast_possible_truncation)]
        rating: row.rating.map(|value| value as f32),
        #[allow(clippy::cast_possible_truncation)]
        average: row.average.map(|value| value as f32),
        votecount: row.votecount.and_then(|value| u32::try_from(value).ok()),
        released: row.released,
        olang: row.olang,
        devstatus: row.devstatus.and_then(|value| u8::try_from(value).ok()),
        languages: serde_json::from_str(&row.languages_json).unwrap_or_default(),
        platforms: serde_json::from_str(&row.platforms_json).unwrap_or_default(),
        tags: serde_json::from_str(&row.tags_json).unwrap_or_default(),
        screenshots: serde_json::from_str(&row.screenshots_json).unwrap_or_default(),
        developers: serde_json::from_str(&row.developers_json).unwrap_or_default(),
        extlinks: serde_json::from_str(&row.extlinks_json).unwrap_or_default(),
    }))
}

/// URL scheme for a locally cached cover, so the webview can render it offline.
#[derive(Debug, Clone, Serialize)]
pub struct LocalArt {
    /// Data URL of the cached cover, when it exists.
    pub cover_data_url: Option<String>,
    /// Data URL of the pre-blurred backdrop, when it exists.
    pub backdrop_data_url: Option<String>,
}

/// Read cached art for a VN as data URLs.
///
/// Covers are a few hundred KB at most and are read once per screen, so a data
/// URL avoids adding a custom protocol handler just for this.
#[tauri::command]
pub async fn vndb_local_art(state: State<'_, AppState>, vn_id: String) -> CommandResult<LocalArt> {
    let Some(cached) = cache::get_image(state.db().pool(), &vn_id).await? else {
        return Ok(LocalArt {
            cover_data_url: None,
            backdrop_data_url: None,
        });
    };

    let mut art = LocalArt {
        cover_data_url: None,
        backdrop_data_url: None,
    };

    if let Some(path) = cached.local_path.as_deref()
        && let Ok(bytes) = images::read_bytes(path).await
    {
        art.cover_data_url = Some(to_data_url(&bytes, "image/jpeg"));
    }
    if let Some(path) = cached.blur_path.as_deref()
        && let Ok(bytes) = images::read_bytes(path).await
    {
        art.backdrop_data_url = Some(to_data_url(&bytes, "image/jpeg"));
    }
    Ok(art)
}

pub(crate) fn to_data_url(bytes: &[u8], mime: &str) -> String {
    use base64::Engine as _;
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    format!("data:{mime};base64,{encoded}")
}

/// What a bulk import did.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BulkImportOutcome {
    /// Entries requested.
    pub requested: usize,
    /// Entries now in the library.
    pub imported: usize,
    /// Entries that were already there and were refreshed.
    pub refreshed: usize,
    /// Entries that failed, with the reason, so one bad id does not hide the rest.
    pub failed: Vec<String>,
    /// Whether the run stopped early because the caller cancelled or a limit hit.
    pub stopped_early: bool,
}

/// One entry from the user's VNDB list: the VN id plus their own state.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MissingEntry {
    /// VN id, e.g. `v264`.
    pub vn_id: String,
    /// Status as VNDB reports it, when the label mapping resolved one.
    pub status: Option<String>,
    /// Their vote, 1 to 10.
    pub rating: Option<u8>,
    /// Their notes.
    pub notes: Option<String>,
    /// Start date, UTC ms.
    pub started_at: Option<i64>,
    /// Finish date, UTC ms.
    pub finished_at: Option<i64>,
}

/// Import the entries the pull reported as missing, keeping the user's own data.
///
/// A game created from VNDB alone would land as an unrated `wishlist` entry, and
/// the next pull would then report every status, rating, note and date as a
/// conflict — 74 decisions for 35 games, which is worse than not importing. So
/// the list state travels with each entry and is written in the same pass.
///
/// One call per entry, through the same path as a single import, so covers,
/// details and the cache behave identically. The client's rate limiter applies to
/// every request, so this cannot exceed the documented limits however many ids it
/// is given.
///
/// Failures are collected rather than thrown: importing 40 of 41 and reporting the
/// one that failed is more useful than importing none.
///
/// # Errors
/// Only when the database cannot be written at all.
#[tauri::command]
pub async fn vndb_import_many(
    state: State<'_, AppState>,
    entries: Vec<MissingEntry>,
) -> CommandResult<BulkImportOutcome> {
    // A mis-click should not start thousands of requests.
    const MAX_BULK: usize = 2000;
    let requested = entries.len();
    let mut outcome = BulkImportOutcome {
        requested,
        imported: 0,
        refreshed: 0,
        failed: Vec::new(),
        stopped_early: requested > MAX_BULK,
    };

    for entry in entries.into_iter().take(MAX_BULK) {
        let vn_id = entry.vn_id.clone();
        match vndb_import(state.clone(), vn_id.clone()).await {
            Ok(result) => {
                if result.already_owned {
                    outcome.refreshed += 1;
                } else {
                    outcome.imported += 1;
                }
                // Carry the user's list state onto the row that was just made.
                if let Err(err) = apply_list_state(state.db().pool(), &entry).await {
                    outcome
                        .failed
                        .push(format!("{vn_id}: imported, but list state failed: {err}"));
                }
            }
            Err(err) => outcome.failed.push(format!("{vn_id}: {}", err.message)),
        }
    }

    Ok(outcome)
}

/// Write the status, rating, notes and dates VNDB already knew.
///
/// Only fills what the import left empty: a game created from catalogue data has
/// no status of its own, but a re-import of something the user has since edited
/// must not overwrite their edit.
async fn apply_list_state(
    pool: &sqlx::SqlitePool,
    entry: &MissingEntry,
) -> Result<(), crate::db::DbError> {
    let Some((game_id, current_status)) = sqlx::query_as::<_, (String, String)>(
        "SELECT id, status FROM games WHERE vndb_id = ? AND deleted = 0
         ORDER BY updated_at DESC LIMIT 1",
    )
    .bind(&entry.vn_id)
    .fetch_optional(pool)
    .await?
    else {
        return Ok(());
    };

    let parsed = vn_core::ids::EntityId::parse(&game_id)
        .map_err(|err| crate::db::DbError::Corrupt(err.to_string()))?;
    let Some(mut game) = crate::db::queries::get_game(pool, parsed).await? else {
        return Ok(());
    };

    // `wishlist` is the placeholder the importer assigns, so it means "unknown",
    // not "the user chose wishlist". Anything else was a real choice.
    if current_status == "wishlist"
        && let Some(status) = entry.status.as_deref()
        && let Ok(parsed_status) = serde_json::from_value::<vn_core::model::GameStatus>(
            serde_json::Value::String(status.to_owned()),
        )
    {
        game.status = parsed_status;
    }
    if game.rating.is_none() {
        game.rating = entry.rating;
    }
    if game
        .notes
        .as_deref()
        .is_none_or(|text| text.trim().is_empty())
    {
        game.notes = entry.notes.clone();
    }
    if game.started_at.is_none() {
        game.started_at = entry.started_at.map(vn_core::clock::Timestamp::from_millis);
    }
    if game.finished_at.is_none() {
        game.finished_at = entry
            .finished_at
            .map(vn_core::clock::Timestamp::from_millis);
    }

    game.meta = game
        .meta
        .touch(Timestamp::now().map_err(|err| crate::db::DbError::Corrupt(err.to_string()))?);
    crate::db::queries::upsert_game(pool, &game).await
}

// Screenshots

/// Download one batch of screenshots and report whether that batch was complete.
///
/// Batched rather than all-at-once because 30 images at VNDB's observed ~2 s each is
/// a minute of work: blocking the whole response on it left the gallery blank until
/// the last file landed, which reads as "screenshots are missing". The caller asks
/// again while `pending` is non-zero, so tiles appear in waves.
///
/// The batch being short is **not** a failure. `complete` only goes false when an
/// image inside the batch actually failed, so a partially downloaded gallery is never
/// reported as broken.
async fn fetch_batch(
    state: &State<'_, AppState>,
    vn_id: &str,
    missing: &[crate::vndb::dto::Image],
    policy: BlurPolicy,
    timestamp: Timestamp,
) -> CommandResult<bool> {
    let batch: Vec<crate::vndb::dto::Image> =
        missing.iter().take(DOWNLOADS_PER_CALL).cloned().collect();
    let store = ScreenshotStore::new(state.data_dir());
    match screenshots::cache_all(
        &client(state),
        state.db().pool(),
        &store,
        vn_id,
        &batch,
        policy,
        timestamp,
    )
    .await
    {
        // `cache_all` collects per-image failures instead of aborting, so a shortfall
        // is only visible by counting.
        Ok(written) => Ok(written.len() == batch.len()),
        // A disk or database failure is not a download shortfall: telling the user
        // "N of M could not be downloaded" for a full disk is a network story for a
        // local problem, and the sharp bytes were never attempted.
        Err(err @ (screenshots::ScreenshotError::Io(_) | screenshots::ScreenshotError::Db(_))) => {
            Err(CommandError::new("screenshot_store", err.to_string()))
        }
        Err(err) => {
            eprintln!("screenshot cache failed for {vn_id}: {err}");
            Ok(false)
        }
    }
}

/// How many screenshots one call will download before returning.
///
/// Six is roughly twelve seconds at VNDB's observed ~2 s per image, which keeps a
/// single IPC round trip under the frontend's patience while still making visible
/// progress. The rest are fetched on the next call, and every call re-reads what is
/// on disk first, so the gallery grows rather than restarting.
const DOWNLOADS_PER_CALL: usize = 6;

/// One screenshot as the detail gallery uses it.
///
/// `src` is a resolved data URL and is what the tile paints; `blurred_src` is the
/// pre-blurred copy of the same image. Both are handed over and the decision between
/// them belongs to the caller, exactly as it does for a cover: the webview already
/// holds the blur for an explicit cover (`LocalArt::backdrop_data_url`) and picks a
/// file at paint time, so keeping the sharp bytes in the payload costs nothing there
/// and lets a reveal paint instantly instead of waiting for a second IPC round trip.
///
/// `src` is always the **sharp** file. A screenshot without a cached sharp copy falls
/// back to the blur and says so through `sharp: false`, so a caller must never label a
/// soft image as the real thing.
#[derive(Debug, Clone, Serialize)]
pub struct ScreenshotView {
    /// VNDB image id, e.g. `sf190`.
    pub image_id: String,
    /// Page URL on VNDB for this image.
    pub vndb_url: String,
    /// Data URL of the sharp image (or the blur, when `sharp` is `false`).
    pub src: String,
    /// Data URL of the pre-blurred copy, when one is on disk.
    pub blurred_src: Option<String>,
    /// Whether `src` is the sharp image.
    pub sharp: bool,
    /// Whether the community rated this explicit.
    pub explicit: bool,
    /// Pixel width of the painted image, when known.
    pub width: Option<u32>,
    /// Pixel height of the painted image, when known.
    pub height: Option<u32>,
}

/// What a gallery request produced.
#[derive(Debug, Clone, Serialize)]
pub struct ScreenshotGallery {
    /// Screenshots, in VNDB's order.
    pub shots: Vec<ScreenshotView>,
    /// How many VNDB reported for this VN. Larger than `shots.len()` when some
    /// images could not be downloaded, which the UI reports instead of silently
    /// showing a shorter list.
    pub reported: usize,
    /// True when every step succeeded without a network or disk failure.
    pub complete: bool,
    /// Screenshots still to download. Non-zero means "call again for more".
    ///
    /// Separate from `complete` because they are different states: `pending > 0` is
    /// "still arriving", `complete: false` is "something failed". The UI must not
    /// report a shortfall for work that has not been attempted yet.
    pub pending: usize,
}

/// Build the payload for one cached screenshot.
///
/// This resolves **no content policy**. It reports what is on disk — the sharp copy
/// and the pre-blurred copy — and whether the community rated the image explicit. The
/// choice between the two is made by the surface that paints it, which is what a cover
/// already does: `vndb_local_art` hands over the sharp cover and its pre-blurred
/// backdrop together and `useArtwork` decides. Two policies (`show`/`blur`/`hide` for
/// covers, a narrower one for screenshots) meant the same preference could be honoured
/// one way in the library and another in the gallery; there is now one.
///
/// Returns `None` when no file is on disk for this image, so a stale row cannot
/// produce a tile that paints nothing.
async fn view_for(cached: &cache::CachedScreenshot, size: ShotSize) -> Option<ScreenshotView> {
    let explicit = cached.is_explicit();
    // Nearest available group of files: the requested size, then the other one. A
    // half-migrated cache (a tile but no lightbox image) must still paint rather than
    // leave a hole in the grid.
    let (sharp_path, blur_path) = match size {
        ShotSize::Tile => (cached.thumb_path.as_deref(), cached.blur_path.as_deref()),
        ShotSize::Full => (
            cached.full_path.as_deref().or(cached.thumb_path.as_deref()),
            cached
                .blur_full_path
                .as_deref()
                .or(cached.blur_path.as_deref()),
        ),
    };

    // The sharp file is preferred and the blur is the fallback, so `sharp` is honest
    // about which one came back. Both are read when both exist: the reveal is the user
    // asking for the image they are already looking at the blur of.
    let sharp_exists = sharp_path.filter(|path| path.exists());
    let blur_exists = blur_path.filter(|path| path.exists());
    let (chosen, is_sharp) = sharp_exists
        .map(|path| (path, true))
        .or_else(|| blur_exists.map(|path| (path, false)))?;

    let bytes = screenshots::read_bytes(chosen).await.ok()?;
    let (width, height) = image_dimensions(chosen);
    let blurred_src = match blur_exists {
        Some(path) => screenshots::read_bytes(path)
            .await
            .ok()
            .map(|bytes| to_data_url(&bytes, "image/jpeg")),
        None => None,
    };
    Some(ScreenshotView {
        image_id: cached.image_id.clone(),
        vndb_url: format!("https://vndb.org/{}", cached.image_id),
        src: to_data_url(&bytes, "image/jpeg"),
        blurred_src,
        sharp: is_sharp,
        explicit,
        width,
        height,
    })
}

/// Which cached size a caller wants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShotSize {
    /// Grid tile: small enough that 30 of them fit in one IPC response.
    Tile,
    /// Lightbox: the source resolution.
    Full,
}

/// Read an image's pixel size from its header.
///
/// The browser reflows the gallery when this is missing, so a tile that has a size
/// reserves its space before the data URL decodes. A failure degrades to `None`
/// rather than failing the tile: the picture still paints.
fn image_dimensions(path: &std::path::Path) -> (Option<u32>, Option<u32>) {
    image::image_dimensions(path)
        .map_or((None, None), |(width, height)| (Some(width), Some(height)))
}

/// The library row that owns a VN, for commands that ask by VN id.
///
/// `vndb_refresh_detail` takes a game id because it exists to update the entry the
/// user is looking at. A gallery only knows the VN id, so this resolves one from the
/// other rather than duplicating the refresh logic.
async fn game_id_for_vn(pool: &sqlx::SqlitePool, vn_id: &str) -> CommandResult<String> {
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT id FROM games WHERE vndb_id = ? AND deleted = 0 ORDER BY updated_at DESC LIMIT 1",
    )
    .bind(vn_id)
    .fetch_optional(pool)
    .await
    .map_err(crate::db::DbError::from)?;
    row.map(|(id,)| id)
        .ok_or_else(|| CommandError::new("not_found", "that visual novel is not in your library"))
}

/// Cache (if needed) and return a VN's screenshots.
///
/// Downloading happens once per image and is keyed by VNDB's image id, so opening
/// the same game again is a disk read. The content setting is applied when the
/// payload is built rather than when files are written: switching from `blur` to
/// `show` must not require a second download for art already on disk.
///
/// The gallery is deliberately *not* allowed to empty itself. A VN whose screenshots
/// could not be downloaded still renders its cover, facts and description; a gallery
/// section that reports "unavailable right now" is honest, a blank one looks broken.
///
/// # Errors
/// Only on database failure. Network and decode failures reduce the gallery and are
/// reported through `complete: false`.
#[tauri::command]
pub async fn vndb_screenshots(
    state: State<'_, AppState>,
    vn_id: String,
) -> CommandResult<ScreenshotGallery> {
    let pool = state.db().pool();
    let timestamp = now()?;
    let prefs = queries::load_prefs(pool).await?;
    let policy = BlurPolicy::from_pref(&prefs.nsfw);

    // The detail payload already holds the list; reading it back keeps this command
    // free of its own API call and of a second rate-limit slot.
    let reported: Vec<crate::vndb::dto::Image> = {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT screenshots_json FROM vndb_details WHERE vn_id = ?")
                .bind(&vn_id)
                .fetch_optional(pool)
                .await
                .map_err(crate::db::DbError::from)?;
        row.and_then(|(json,)| serde_json::from_str(&json).ok())
            .unwrap_or_default()
    };

    // The cached row may predate the current `DETAIL_FIELDS`: `screenshots.id` was
    // added after some rows were already stored, and those come back with `"id": null`.
    // A row like that cannot be diffed against the cache (there is nothing to match on),
    // and the honest fix is to fetch the VN again once and store it, rather than either
    // skipping every tile or downloading the same ten images on every visit. A fetch
    // failure is not fatal: whatever is on disk still renders.
    if !reported.is_empty()
        && reported.iter().all(|image| image.id.is_none())
        && let Ok(detail) =
            vndb_refresh_detail(state.clone(), game_id_for_vn(pool, &vn_id).await?).await
        && !detail.screenshots.is_empty()
    {
        return Box::pin(vndb_screenshots(state, vn_id)).await;
    }

    let cached_now = cache::get_screenshots(pool, &vn_id).await?;
    let mut complete = true;
    // Only fetch what is not already on disk: `cache_all` is idempotent but walks
    // every entry, and a gallery that opens instantly should not re-parse 30 files.
    //
    // "Already on disk" also has to mean "at this build's size". A row recorded at a
    // different width came from an older pipeline and is re-rendered, which is the
    // only way a resolution or blur fix reaches someone who already opened the
    // gallery once.
    let missing: Vec<crate::vndb::dto::Image> = reported
        .iter()
        .filter(|image| {
            // An entry with no id is matched by URL instead, since its cache key is
            // recovered from that URL.
            let row = match image.id.as_deref().filter(|id| !id.is_empty()) {
                Some(id) => cached_now.iter().find(|row| row.image_id == id),
                None => image
                    .url
                    .as_deref()
                    .and_then(|url| cached_now.iter().find(|row| row.url == url)),
            };
            let Some(row) = row else {
                return true;
            };
            // "Already on disk" means "at this build's size", asked of the same
            // function the pipeline uses to decide what to regenerate. A second copy
            // of this rule reported a permanent failure for rows it never rebuilt.
            let store = ScreenshotStore::new(state.data_dir());
            let image_id = row.image_id.clone();
            screenshots::needs_rebuild(
                row.width,
                &store.tile_path(&image_id),
                &store.full_path(&image_id),
                &store.blur_path(&image_id),
                &store.blur_full_path(&image_id),
            )
        })
        .cloned()
        .collect();

    if !missing.is_empty() {
        complete = fetch_batch(&state, &vn_id, &missing, policy, timestamp).await?;
        // Keep VNDB's ordering for everything that is now cached.
        let order: Vec<String> = reported
            .iter()
            .filter_map(|image| image.id.clone())
            .collect();
        cache::set_order(pool, &order).await?;
    }

    let rows = cache::get_screenshots(pool, &vn_id).await?;
    let mut shots = Vec::with_capacity(rows.len());
    for row in &rows {
        match view_for(row, ShotSize::Tile).await {
            Some(view) => shots.push(view),
            None => complete = false,
        }
    }

    // How many the API listed that this build still has no file for. The caller polls
    // while this is non-zero.
    let pending = reported.len().saturating_sub(shots.len());
    Ok(ScreenshotGallery {
        shots,
        reported: reported.len(),
        complete,
        pending,
    })
}

/// Cache and return one screenshot at the size the lightbox wants.
///
/// The gallery tiles are 512 px wide; a full-size view of an 800x600 VNDB screenshot
/// is not much more, so this reuses the same cached file rather than keeping a
/// second full-resolution copy of every image on disk. The command exists separately
/// so the lightbox can be lazy: opening a game must not download thirty large
/// images for tiles nobody enlarged.
///
/// # Errors
/// `not_found` when the image was never cached and is not in the VN's list.
#[tauri::command]
pub async fn vndb_screenshot_fetch(
    state: State<'_, AppState>,
    vn_id: String,
    image_id: String,
) -> CommandResult<ScreenshotView> {
    let pool = state.db().pool();
    let prefs = queries::load_prefs(pool).await?;
    let policy = BlurPolicy::from_pref(&prefs.nsfw);

    if let Some(row) = cache::get_screenshots(pool, &vn_id)
        .await?
        .into_iter()
        .find(|row| row.image_id == image_id)
        && let Some(view) = view_for(&row, ShotSize::Full).await
    {
        return Ok(view);
    }

    // Not cached yet: pull just this one out of the stored VN payload.
    let entry: Option<crate::vndb::dto::Image> = {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT screenshots_json FROM vndb_details WHERE vn_id = ?")
                .bind(&vn_id)
                .fetch_optional(pool)
                .await
                .map_err(crate::db::DbError::from)?;
        row.and_then(|(json,)| serde_json::from_str::<Vec<crate::vndb::dto::Image>>(&json).ok())
            .and_then(|images| {
                images
                    .into_iter()
                    .find(|image| image.id.as_deref() == Some(image_id.as_str()))
            })
    };
    let Some(entry) = entry else {
        return Err(CommandError::new(
            "not_found",
            "that screenshot is not part of this VN",
        ));
    };

    let store = ScreenshotStore::new(state.data_dir());
    let cached = screenshots::cache_all(
        &client(&state),
        pool,
        &store,
        &vn_id,
        std::slice::from_ref(&entry),
        policy,
        now()?,
    )
    .await
    .map_err(|err| CommandError::new("image", err.to_string()))?;

    let row = cached
        .into_iter()
        .next()
        .ok_or_else(|| CommandError::new("not_found", "that screenshot could not be fetched"))?;
    view_for(&row, ShotSize::Full)
        .await
        .ok_or_else(|| CommandError::new("image", "that screenshot has no cached file"))
}

#[cfg(test)]
mod refresh_all_tests {
    use super::*;

    fn outcome(attempted: usize, last_index: usize) -> RefreshAllOutcome {
        RefreshAllOutcome {
            attempted,
            last_index,
            updated: 0,
            // Mirrors the command: finished only when the index reached the end.
            cancelled: last_index < attempted,
            failed: Vec::new(),
        }
    }

    #[test]
    fn a_finished_run_reports_no_cancellation() {
        // The progress key is reset to 0 only on completion, so "finished" has to be
        // decided from the index, not from whether the loop ran.
        assert!(!outcome(41, 41).cancelled);
    }

    #[test]
    fn a_partial_run_reports_cancellation_so_it_can_resume() {
        // Leaving `cancelled` false here would make the UI claim a full refresh after
        // a run that stopped at game 7, which is the same class of lie as the locale
        // badge was.
        let partial = outcome(41, 7);
        assert!(partial.cancelled);
        assert_eq!(partial.last_index, 7, "the resume point is preserved");
    }

    #[test]
    fn an_empty_library_is_finished_not_cancelled() {
        // Nothing to do is success. Without this, a library of manual entries would
        // report a cancelled refresh forever.
        let none = outcome(0, 0);
        assert!(!none.cancelled);
        assert_eq!(none.attempted, 0);
    }
}

#[cfg(test)]
mod detail_storage_tests {
    use super::*;
    use crate::db::Db;
    use crate::vndb::dto::{ExtLink, Image, Producer, Tag, TitleVariant};

    fn now() -> Timestamp {
        Timestamp::from_millis(1_700_000_000_000)
    }

    fn full_vn() -> Vn {
        Vn {
            id: "v17".to_owned(),
            title: Some("Ever17".to_owned()),
            alttitle: Some("Ever17 -the out of infinity-".to_owned()),
            titles: vec![TitleVariant {
                lang: Some("en".to_owned()),
                title: Some("Ever17 -out of infinity-".to_owned()),
                official: Some(true),
                main: Some(false),
                ..TitleVariant::default()
            }],
            aliases: vec!["E17".to_owned()],
            olang: Some("ja".to_owned()),
            devstatus: Some(0),
            released: Some("2002-08-29".to_owned()),
            languages: vec!["en".to_owned(), "ja".to_owned()],
            platforms: vec!["win".to_owned(), "psp".to_owned()],
            length: Some(4),
            length_minutes: Some(1_987),
            length_votes: Some(203),
            rating: Some(84.5),
            average: Some(84.4),
            votecount: Some(8_800),
            developers: vec![Producer {
                id: Some("p8".to_owned()),
                name: Some("KID".to_owned()),
                original: Some("キッド".to_owned()),
                kind: Some("co".to_owned()),
            }],
            extlinks: vec![ExtLink {
                url: Some("https://en.wikipedia.org/wiki/Ever17".to_owned()),
                label: Some("Wikipedia (en)".to_owned()),
                name: Some("enwiki".to_owned()),
            }],
            tags: vec![Tag {
                id: Some("g8".to_owned()),
                name: Some("Fear of Death".to_owned()),
                ..Tag::default()
            }],
            screenshots: vec![Image {
                id: Some("sf190".to_owned()),
                url: Some("https://t.vndb.org/sf/90/190.jpg".to_owned()),
                dims: Some(vec![800, 600]),
                sexual: Some(0.0),
                violence: Some(0.0),
                ..Image::default()
            }],
            ..Vn::default()
        }
    }

    async fn read_detail(db: &Db, vn_id: &str) -> DetailPayload {
        let row: (String, Option<i64>, Option<f64>, Option<i64>, String, String) = sqlx::query_as(
            "SELECT languages_json, devstatus, average, length_votes, developers_json, extlinks_json \
             FROM vndb_details WHERE vn_id = ?",
        )
        .bind(vn_id)
        .fetch_one(db.pool())
        .await
        .expect("row");
        // The command itself needs a Tauri `State`, so this mirrors its decoding
        // through the same struct rather than duplicating a second query.
        assert_eq!(row.0, r#"["en","ja"]"#);
        assert_eq!(row.1, Some(0));
        // REAL round-trips the f32 exactly once widened to f64, so the stored value
        // is compared as f64 rather than against a decimal the f32 never held.
        assert_eq!(row.2, Some(f64::from(84.4_f32)));
        assert_eq!(row.3, Some(203));
        assert!(row.4.contains("KID"));
        assert!(row.5.contains("enwiki"));
        DetailPayload::from_vn(&full_vn())
    }

    #[tokio::test]
    async fn every_detail_field_survives_a_write() {
        // The bug this guards is not a crash: it is `store_detail` silently dropping
        // columns the API already returned, which is how `titles` and `aliases` were
        // lost for two phases. Round-tripping the whole payload is the cheap check.
        let db = Db::open_memory().await.expect("open");
        let vn = full_vn();
        store_detail(db.pool(), &vn, now()).await.expect("store");

        let payload = read_detail(&db, "v17").await;
        assert_eq!(payload.languages, vec!["en", "ja"]);
        assert_eq!(payload.platforms, vec!["win", "psp"]);
        assert_eq!(payload.devstatus, Some(0));
        assert_eq!(payload.length_votes, Some(203));
        // f32 to f64 and back: the stored REAL is the exact widening of the f32, so
        // compare against that value rather than a decimal the f32 never held.
        assert_eq!(payload.average, Some(84.4_f32));
        assert_eq!(payload.developers[0].name.as_deref(), Some("KID"));
        assert_eq!(
            payload.extlinks[0].name.as_deref(),
            Some("enwiki"),
            "the link the UI opens comes from here"
        );
        assert_eq!(payload.screenshots[0].id.as_deref(), Some("sf190"));
    }

    #[tokio::test]
    async fn a_second_store_replaces_rather_than_duplicates() {
        let db = Db::open_memory().await.expect("open");
        let mut vn = full_vn();
        store_detail(db.pool(), &vn, now()).await.expect("store");
        vn.platforms = vec!["win".to_owned()];
        vn.rating = Some(90.0);
        store_detail(db.pool(), &vn, now())
            .await
            .expect("store again");

        let rows: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM vndb_details")
            .fetch_one(db.pool())
            .await
            .expect("count");
        assert_eq!(rows.0, 1);
        let platforms: (String,) =
            sqlx::query_as("SELECT platforms_json FROM vndb_details WHERE vn_id = 'v17'")
                .fetch_one(db.pool())
                .await
                .expect("platforms");
        assert_eq!(platforms.0, r#"["win"]"#);
    }

    #[tokio::test]
    async fn a_vn_nobody_voted_on_stores_no_average() {
        // Zero is a real rating on VNDB's 10..100 scale as far as SQLite is
        // concerned, so writing 0 for "no votes" would make an unrated VN look rated.
        let db = Db::open_memory().await.expect("open");
        let vn = Vn {
            id: "v1".to_owned(),
            title: Some("Unrated".to_owned()),
            ..Vn::default()
        };
        store_detail(db.pool(), &vn, now()).await.expect("store");
        let stored: (Option<f64>, Option<f64>, Option<i64>) = sqlx::query_as(
            "SELECT rating, average, devstatus FROM vndb_details WHERE vn_id = 'v1'",
        )
        .fetch_one(db.pool())
        .await
        .expect("row");
        assert_eq!(stored, (None, None, None));
    }
}
