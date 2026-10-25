//! Typed queries.
//!
//! Queries return domain types. Import/merge queries accept an executor so their
//! reads and writes can share one transaction instead of borrowing another pool connection.

use sqlx::{Executor, Sqlite, SqlitePool};
use vn_core::clock::Timestamp;
use vn_core::ids::{DeviceId, EntityId};
use vn_core::library::{StatusBreakdown, StatusRowTotal};
use vn_core::model::{Chapter, Ending, Game, GameStatus, PlaySession, Route, SessionSource};

use crate::db::models::{ChapterRow, EndingRow, GameRow, PrefsSnapshot, RouteRow, SessionRow};
use crate::db::pool::DbError;

fn corrupt(err: &vn_core::model::ModelError) -> DbError {
    DbError::Corrupt(err.to_string())
}

fn games_from(rows: Vec<GameRow>) -> Result<Vec<Game>, DbError> {
    rows.into_iter()
        .map(Game::try_from)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| corrupt(&err))
}

/// Load every live game, newest first.
pub async fn list_games(pool: &SqlitePool) -> Result<Vec<Game>, DbError> {
    let rows: Vec<GameRow> = sqlx::query_as(
        "SELECT id, title, title_original, vndb_id, status, rating, notes, length_minutes, manual_playtime_ms, accent_color, cover_url, cover_sexual, cover_violence, custom_cover_path, custom_cover_source_vndb, started_at, finished_at, updated_at, device_id, deleted
         FROM games WHERE deleted = 0 ORDER BY updated_at DESC",
    )
    .fetch_all(pool)
    .await?;
    games_from(rows)
}

/// Every game, including tombstoned ones, for sync.
///
/// The UI queries filter `deleted = 0`, which is right for the UI and wrong for
/// sync: a delete has to travel, or the row returns on the next pull.
pub async fn list_games_including_deleted<'e, E>(executor: E) -> Result<Vec<Game>, DbError>
where
    E: Executor<'e, Database = Sqlite>,
{
    let rows: Vec<GameRow> = sqlx::query_as(
        "SELECT id, title, title_original, vndb_id, status, rating, notes, length_minutes, manual_playtime_ms, accent_color, cover_url, cover_sexual, cover_violence, custom_cover_path, custom_cover_source_vndb, started_at, finished_at, updated_at, device_id, deleted          FROM games ORDER BY updated_at DESC",
    )
    .fetch_all(executor)
    .await?;
    games_from(rows)
}

/// Fetch one live game by id.
pub async fn get_game(pool: &SqlitePool, id: EntityId) -> Result<Option<Game>, DbError> {
    let row: Option<GameRow> = sqlx::query_as(
        "SELECT id, title, title_original, vndb_id, status, rating, notes, length_minutes, manual_playtime_ms, accent_color, cover_url, cover_sexual, cover_violence, custom_cover_path, custom_cover_source_vndb, started_at, finished_at, updated_at, device_id, deleted
         FROM games WHERE id = ? AND deleted = 0",
    )
    .bind(id.to_string())
    .fetch_optional(pool)
    .await?;
    row.map(Game::try_from)
        .transpose()
        .map_err(|err| corrupt(&err))
}

/// Fetch one game by id even when soft-deleted.
///
/// The tombstone carries the user's status, rating, notes and sessions; a
/// re-import that wants to revive the game must read this row and clear the
/// flag rather than insert a duplicate beside it.
pub async fn get_game_including_deleted(
    pool: &SqlitePool,
    id: EntityId,
) -> Result<Option<Game>, DbError> {
    let row: Option<GameRow> = sqlx::query_as(
        "SELECT id, title, title_original, vndb_id, status, rating, notes, length_minutes, manual_playtime_ms, accent_color, cover_url, cover_sexual, cover_violence, custom_cover_path, custom_cover_source_vndb, started_at, finished_at, updated_at, device_id, deleted
         FROM games WHERE id = ?",
    )
    .bind(id.to_string())
    .fetch_optional(pool)
    .await?;
    row.map(Game::try_from)
        .transpose()
        .map_err(|err| corrupt(&err))
}

/// Fetch the live game attached to a VNDB id, if any.
pub async fn get_game_by_vndb_id(
    pool: &SqlitePool,
    vndb_id: &str,
) -> Result<Option<Game>, DbError> {
    let row: Option<GameRow> = sqlx::query_as(
        "SELECT id, title, title_original, vndb_id, status, rating, notes, length_minutes, manual_playtime_ms, accent_color, cover_url, cover_sexual, cover_violence, custom_cover_path, custom_cover_source_vndb, started_at, finished_at, updated_at, device_id, deleted FROM games WHERE vndb_id = ? AND deleted = 0 ORDER BY updated_at DESC LIMIT 1",
    )
    .bind(vndb_id)
    .fetch_optional(pool)
    .await?;
    row.map(Game::try_from)
        .transpose()
        .map_err(|err| corrupt(&err))
}

/// Insert or replace a game row, preserving its identity.
pub async fn upsert_game<'e, E>(executor: E, game: &Game) -> Result<(), DbError>
where
    E: Executor<'e, Database = Sqlite>,
{
    sqlx::query(
        "INSERT INTO games (id, title, title_original, vndb_id, status, rating, notes,                             length_minutes, manual_playtime_ms, accent_color, cover_url,                             cover_sexual, cover_violence, custom_cover_path,                             custom_cover_source_vndb, started_at, finished_at,                             updated_at, device_id, deleted)          VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)          ON CONFLICT(id) DO UPDATE SET             title = excluded.title, title_original = excluded.title_original,             vndb_id = excluded.vndb_id, status = excluded.status, rating = excluded.rating,             notes = excluded.notes, length_minutes = excluded.length_minutes,             manual_playtime_ms = excluded.manual_playtime_ms,             accent_color = excluded.accent_color, cover_url = excluded.cover_url,             cover_sexual = excluded.cover_sexual, cover_violence = excluded.cover_violence,             custom_cover_path = excluded.custom_cover_path,             custom_cover_source_vndb = excluded.custom_cover_source_vndb,             started_at = excluded.started_at, finished_at = excluded.finished_at,             updated_at = excluded.updated_at, device_id = excluded.device_id,             deleted = excluded.deleted",
    )
    .bind(game.meta.id.to_string())
    .bind(&game.title)
    .bind(&game.title_original)
    .bind(&game.vndb_id)
    .bind(game.status.as_str())
    .bind(game.rating.map(i64::from))
    .bind(&game.notes)
    .bind(game.length_minutes)
    .bind(game.manual_playtime_ms)
    .bind(&game.accent_color)
    .bind(&game.cover_url)
    .bind(game.cover_sexual.map(i64::from))
    .bind(game.cover_violence.map(i64::from))
    .bind(&game.custom_cover_path)
    .bind(i64::from(game.custom_cover_source_vndb))
    .bind(game.started_at.map(Timestamp::as_millis))
    .bind(game.finished_at.map(Timestamp::as_millis))
    .bind(game.meta.updated_at.as_millis())
    .bind(game.meta.device_id_device.device().as_str())
    .bind(i64::from(game.meta.deleted))
    .execute(executor)
    .await?;
    Ok(())
}

/// Soft-delete a game and everything that belongs to it.
pub async fn soft_delete_game(
    pool: &SqlitePool,
    id: EntityId,
    now: Timestamp,
    device: &DeviceId,
) -> Result<(), DbError> {
    // One statement per table, written out rather than built with format!, so
    // sqlx can audit them and the set of touched tables stays greppable.
    let mut tx = pool.begin().await?;
    let ms = now.as_millis();
    let key = id.to_string();
    let device = device.as_str();

    sqlx::query(
        "UPDATE games SET deleted = 1, updated_at = ?, device_id = ? WHERE id = ? AND deleted = 0",
    )
    .bind(ms)
    .bind(device)
    .bind(&key)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "UPDATE chapters SET deleted = 1, updated_at = ?, device_id = ? WHERE game_id = ? AND deleted = 0",
    )
    .bind(ms)
    .bind(device)
    .bind(&key)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "UPDATE play_sessions SET deleted = 1, updated_at = ?, device_id = ? WHERE game_id = ? AND deleted = 0",
    )
    .bind(ms)
    .bind(device)
    .bind(&key)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "UPDATE endings SET deleted = 1, updated_at = ?, device_id = ? WHERE game_id = ? AND deleted = 0",
    )
    .bind(ms)
    .bind(device)
    .bind(&key)
    .execute(&mut *tx)
    .await?;

    // Routes have a `deleted` column and `list_routes` filters on it, so leaving them
    // live kept a deleted game's routes travelling in every sync payload.
    sqlx::query(
        "UPDATE routes SET deleted = 1, updated_at = ?, device_id = ? WHERE game_id = ? AND deleted = 0",
    )
    .bind(ms)
    .bind(device)
    .bind(&key)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(())
}

/// Move every child of `source` onto `target`.
///
/// Takes a transaction so the three statements and the `merged_into` write that
/// explains them commit together: a merge that moved the children but failed to record
/// the pointer would split the story in half.
pub async fn reassign_route_children(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    source: EntityId,
    target: EntityId,
) -> Result<(), DbError> {
    let source = source.to_string();
    let target = target.to_string();
    // Three literal statements rather than a table-name loop: sqlx 0.9 rejects
    // interpolated SQL, and a table name is exactly the kind of string that rule
    // exists to keep out of a query.
    sqlx::query("UPDATE chapters SET route_id = ? WHERE route_id = ?")
        .bind(&target)
        .bind(&source)
        .execute(&mut **tx)
        .await?;
    sqlx::query("UPDATE endings SET route_id = ? WHERE route_id = ?")
        .bind(&target)
        .bind(&source)
        .execute(&mut **tx)
        .await?;
    sqlx::query("UPDATE play_sessions SET route_id = ? WHERE route_id = ?")
        .bind(&target)
        .bind(&source)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// Routes for a game, in display order, tombstones included.
///
/// Sync needs the tombstones: a delete has to travel, or the row returns on the next
/// pull.
pub async fn list_routes_including_deleted<'e, E>(
    executor: E,
    game_id: EntityId,
) -> Result<Vec<Route>, DbError>
where
    E: Executor<'e, Database = Sqlite>,
{
    let rows: Vec<RouteRow> = sqlx::query_as(
        "SELECT id, game_id, name, sort_order, origin, merged_into, updated_at, device_id,                 deleted          FROM routes WHERE game_id = ? ORDER BY name ASC",
    )
    .bind(game_id.to_string())
    .fetch_all(executor)
    .await?;
    rows.into_iter()
        .map(Route::try_from)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| corrupt(&err))
}

/// Every ending for a game, including tombstoned ones, for sync.
pub async fn list_endings_including_deleted<'e, E>(
    executor: E,
    game_id: EntityId,
) -> Result<Vec<Ending>, DbError>
where
    E: Executor<'e, Database = Sqlite>,
{
    let rows: Vec<EndingRow> = sqlx::query_as(
        "SELECT id, game_id, name, route, achieved, achieved_at, updated_at, device_id,                 deleted, route_id          FROM endings WHERE game_id = ? ORDER BY name ASC",
    )
    .bind(game_id.to_string())
    .fetch_all(executor)
    .await?;
    rows.into_iter()
        .map(Ending::try_from)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| corrupt(&err))
}

/// One ending by its own id, live or tombstoned.
///
/// Separate from [`list_endings_including_deleted`], which is keyed by game: an
/// edit knows the ending's id but not which game it belongs to, and looking it up
/// by game would mean loading every ending to find one.
///
/// # Errors
/// Any database error. `Ok(None)` means no such row.
pub async fn ending_by_id(pool: &SqlitePool, id: EntityId) -> Result<Option<Ending>, DbError> {
    let row: Option<EndingRow> = sqlx::query_as(
        "SELECT id, game_id, name, route, achieved, achieved_at, updated_at, device_id, deleted, route_id \
         FROM endings WHERE id = ?",
    )
    .bind(id.to_string())
    .fetch_optional(pool)
    .await?;
    row.map(Ending::try_from)
        .transpose()
        .map_err(|err| corrupt(&err))
}

/// Endings for a game.
pub async fn list_endings(pool: &SqlitePool, game_id: EntityId) -> Result<Vec<Ending>, DbError> {
    // The `games` join is not decoration: `PRAGMA foreign_keys` is per connection, so
    // an ending whose game was removed can outlive it. A tombstoned game hides its
    // children instead of leaving them on screen.
    let rows: Vec<EndingRow> = sqlx::query_as(
        "SELECT e.id, e.game_id, e.name, e.route, e.achieved, e.achieved_at,                 e.updated_at, e.device_id, e.deleted, e.route_id          FROM endings e JOIN games g ON g.id = e.game_id AND g.deleted = 0          WHERE e.game_id = ? AND e.deleted = 0 ORDER BY e.name ASC",
    )
    .bind(game_id.to_string())
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(Ending::try_from)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| corrupt(&err))
}

/// Insert or replace an ending.
pub async fn upsert_ending<'e, E>(executor: E, ending: &Ending) -> Result<(), DbError>
where
    E: Executor<'e, Database = Sqlite>,
{
    sqlx::query(
        "INSERT INTO endings (id, game_id, name, route, achieved, achieved_at, route_id,                               updated_at, device_id, deleted)          VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)          ON CONFLICT(id) DO UPDATE SET name = excluded.name, route = excluded.route,             achieved = excluded.achieved, achieved_at = excluded.achieved_at,             route_id = excluded.route_id, updated_at = excluded.updated_at,             device_id = excluded.device_id, deleted = excluded.deleted",
    )
    .bind(ending.meta.id.to_string())
    .bind(ending.game_id.to_string())
    .bind(&ending.name)
    .bind(&ending.route)
    .bind(i64::from(ending.achieved))
    .bind(ending.achieved_at.map(Timestamp::as_millis))
    .bind(ending.route_id.map(|id| id.to_string()))
    .bind(ending.meta.updated_at.as_millis())
    .bind(ending.meta.device_id_device.device().as_str())
    .bind(i64::from(ending.meta.deleted))
    .execute(executor)
    .await?;
    Ok(())
}

/// Remove an ending, as a tombstone.
///
/// A soft delete rather than a row removal: sync merges by id, so a tombstone is
/// the only way the other device learns the ending is gone instead of
/// resurrecting it on the next pull.
///
/// # Errors
/// Any database error.
pub async fn soft_delete_ending(
    pool: &SqlitePool,
    id: EntityId,
    now: Timestamp,
    device: &DeviceId,
) -> Result<(), DbError> {
    sqlx::query(
        "UPDATE endings SET deleted = 1, updated_at = ?, device_id = ? \n         WHERE id = ? AND deleted = 0",
    )
    .bind(now.as_millis())
    .bind(device.as_str())
    .bind(id.to_string())
    .execute(pool)
    .await?;
    Ok(())
}

/// Note that sessions were recovered after an unclean shutdown.
///
/// Written to the diagnostics log rather than shown as a toast: recovering is
/// normal, but the user should be able to find out why a session exists.
pub async fn record_recovery(
    pool: &SqlitePool,
    recovered: usize,
    now: Timestamp,
) -> Result<(), DbError> {
    sqlx::query(
        "INSERT INTO vndb_sync_log (at, kind, vn_id, message) VALUES (?, 'error', NULL, ?)",
    )
    .bind(now.as_millis())
    .bind(format!(
        "recovered {recovered} play session(s) after an unclean shutdown"
    ))
    .execute(pool)
    .await?;
    Ok(())
}

/// Current AFK configuration.
pub async fn load_afk_config(pool: &SqlitePool) -> Result<vn_core::afk::AfkConfig, DbError> {
    let row: (i64, i64) =
        sqlx::query_as("SELECT threshold_ms, exclude_afk FROM afk_settings WHERE id = 1")
            .fetch_one(pool)
            .await?;
    Ok(vn_core::afk::AfkConfig::new(row.0, row.1 != 0))
}

/// Persist the AFK configuration, clamping the threshold into its valid range.
pub async fn save_afk_config(
    pool: &SqlitePool,
    config: vn_core::afk::AfkConfig,
    now: Timestamp,
) -> Result<vn_core::afk::AfkConfig, DbError> {
    let clamped = vn_core::afk::AfkConfig::new(config.threshold_ms, config.exclude_afk);
    sqlx::query(
        "UPDATE afk_settings SET threshold_ms = ?, exclude_afk = ?, updated_at = ? WHERE id = 1",
    )
    .bind(clamped.threshold_ms)
    .bind(i64::from(clamped.exclude_afk))
    .bind(now.as_millis())
    .execute(pool)
    .await?;
    Ok(clamped)
}

/// Read all local preferences, with defaults for anything missing.
pub async fn load_prefs(pool: &SqlitePool) -> Result<PrefsSnapshot, DbError> {
    let rows: Vec<(String, String)> = sqlx::query_as("SELECT key, value FROM local_prefs")
        .fetch_all(pool)
        .await?;
    let mut prefs = PrefsSnapshot::default();
    for (key, value) in rows {
        match key.as_str() {
            "theme" => prefs.theme = value,
            "language" => prefs.language = value,
            "nsfw" => prefs.nsfw = value,
            "reduce_effects" => prefs.reduce_effects = value == "true",
            _ => {}
        }
    }
    Ok(prefs)
}

/// Read Discord Rich Presence settings from local preferences.
pub async fn load_presence_settings(
    pool: &SqlitePool,
) -> Result<vn_core::presence::PresenceSettings, DbError> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT value FROM local_prefs WHERE key = 'presence_settings'")
            .fetch_optional(pool)
            .await?;
    if let Some((json,)) = row
        && let Ok(settings) = serde_json::from_str::<vn_core::presence::PresenceSettings>(&json)
    {
        return Ok(settings);
    }
    Ok(vn_core::presence::PresenceSettings::default())
}

/// Persist Discord Rich Presence settings in local preferences.
pub async fn save_presence_settings(
    pool: &SqlitePool,
    settings: &vn_core::presence::PresenceSettings,
) -> Result<(), DbError> {
    let json = serde_json::to_string(settings).map_err(|e| DbError::Corrupt(e.to_string()))?;
    set_pref(pool, "presence_settings", &json).await
}

/// Persist a single preference.
pub async fn set_pref<'e, E>(executor: E, key: &str, value: &str) -> Result<(), DbError>
where
    E: Executor<'e, Database = Sqlite>,
{
    sqlx::query(
        "INSERT INTO local_prefs (key, value) VALUES (?, ?) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(key)
    .bind(value)
    .execute(executor)
    .await?;
    Ok(())
}

/// Generate and remember this installation's device id if it has none.
pub async fn ensure_device_id(pool: &SqlitePool) -> Result<DeviceId, DbError> {
    let existing: Option<(String,)> =
        sqlx::query_as("SELECT value FROM sync_state WHERE key = 'device_id'")
            .fetch_optional(pool)
            .await?;
    if let Some((value,)) = existing {
        return DeviceId::parse(&value).map_err(|err| corrupt(&err));
    }
    let id = uuid::Uuid::now_v7().to_string();
    sqlx::query("INSERT INTO sync_state (key, value) VALUES ('device_id', ?)")
        .bind(&id)
        .execute(pool)
        .await?;
    DeviceId::parse(&id).map_err(|err| corrupt(&err))
}

/// Sources used by a game, for the "how was this tracked" hint.
pub async fn session_sources(pool: &SqlitePool) -> Result<Vec<SessionSource>, DbError> {
    let rows: Vec<(String,)> = sqlx::query_as("SELECT DISTINCT source FROM play_sessions")
        .fetch_all(pool)
        .await?;
    Ok(rows
        .into_iter()
        .filter_map(|(source,)| SessionSource::parse(&source).ok())
        .collect())
}

#[cfg(test)]
mod tests;

/// Chapters for a game, in display order.
pub async fn list_chapters(pool: &SqlitePool, game_id: EntityId) -> Result<Vec<Chapter>, DbError> {
    let rows: Vec<ChapterRow> = sqlx::query_as(
        "SELECT id, game_id, name, sort_index, reached, first_seen_at, updated_at, device_id, \
                deleted, display_name, route_key, route_name, position, renamed, route_id \
         FROM chapters WHERE game_id = ? AND deleted = 0 \
         ORDER BY sort_index ASC, name ASC",
    )
    .bind(game_id.to_string())
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(Chapter::try_from)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| corrupt(&err))
}

/// Chapters for a game, tombstones included, for sync.
pub async fn list_chapters_including_deleted<'e, E>(
    executor: E,
    game_id: EntityId,
) -> Result<Vec<Chapter>, DbError>
where
    E: Executor<'e, Database = Sqlite>,
{
    let rows: Vec<ChapterRow> = sqlx::query_as(
        "SELECT id, game_id, name, sort_index, reached, first_seen_at, updated_at, device_id, \
                deleted, display_name, route_key, route_name, position, renamed, route_id \
         FROM chapters WHERE game_id = ? ORDER BY sort_index ASC, name ASC",
    )
    .bind(game_id.to_string())
    .fetch_all(executor)
    .await?;
    rows.into_iter()
        .map(Chapter::try_from)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| corrupt(&err))
}

/// One chapter by its own id, live or tombstoned.
///
/// Separate from [`list_chapters`], which is keyed by game: a route reassignment knows
/// the chapter's id but not which game it belongs to, and loading a whole game's
/// chapters to change one would be waste.
pub async fn chapter_by_id(pool: &SqlitePool, id: EntityId) -> Result<Option<Chapter>, DbError> {
    let row: Option<ChapterRow> = sqlx::query_as(
        "SELECT id, game_id, name, sort_index, reached, first_seen_at, updated_at, device_id, \
                deleted, display_name, route_key, route_name, position, renamed, route_id \
         FROM chapters WHERE id = ?",
    )
    .bind(id.to_string())
    .fetch_optional(pool)
    .await?;
    row.map(Chapter::try_from)
        .transpose()
        .map_err(|err| corrupt(&err))
}

/// Insert or update a chapter.
pub async fn upsert_chapter<'e, E>(executor: E, chapter: &Chapter) -> Result<(), DbError>
where
    E: Executor<'e, Database = Sqlite>,
{
    sqlx::query(
        "INSERT INTO chapters (id, game_id, name, sort_index, reached, first_seen_at, \
                               updated_at, device_id, deleted, display_name, route_key, \
                               route_name, route_id, position, renamed) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT(id) DO UPDATE SET name = excluded.name, sort_index = excluded.sort_index, \
            reached = excluded.reached, first_seen_at = excluded.first_seen_at, \
            updated_at = excluded.updated_at, device_id = excluded.device_id, \
            deleted = excluded.deleted, display_name = excluded.display_name, \
            route_key = excluded.route_key, route_name = excluded.route_name, \
            route_id = excluded.route_id, position = excluded.position, \
            renamed = excluded.renamed",
    )
    .bind(chapter.meta.id.to_string())
    .bind(chapter.game_id.to_string())
    .bind(&chapter.name)
    .bind(i64::from(chapter.sort_index))
    .bind(i64::from(chapter.reached))
    .bind(chapter.first_seen_at.map(Timestamp::as_millis))
    .bind(chapter.meta.updated_at.as_millis())
    .bind(chapter.meta.device_id_device.device().as_str())
    .bind(i64::from(chapter.meta.deleted))
    .bind(chapter.display_name.as_deref())
    .bind(chapter.route_key.as_deref())
    .bind(chapter.route_name.as_deref())
    .bind(chapter.route_id.map(|id| id.to_string()))
    .bind(chapter.position.as_deref())
    .bind(i64::from(chapter.renamed))
    .execute(executor)
    .await?;
    Ok(())
}

/// Sessions for a game, oldest first.
pub async fn list_sessions(
    pool: &SqlitePool,
    game_id: EntityId,
) -> Result<Vec<PlaySession>, DbError> {
    let rows: Vec<SessionRow> = sqlx::query_as(
        "SELECT id, game_id, chapter_id, route_id, started_at, ended_at, counted_ms, afk_ms, \
                source, updated_at, device_id, deleted \
         FROM play_sessions WHERE game_id = ? AND deleted = 0 ORDER BY started_at ASC",
    )
    .bind(game_id.to_string())
    .fetch_all(pool)
    .await?;
    sessions_from(rows)
}

/// Sessions for a game, tombstones included, for sync.
pub async fn list_sessions_including_deleted<'e, E>(
    executor: E,
    game_id: EntityId,
) -> Result<Vec<PlaySession>, DbError>
where
    E: Executor<'e, Database = Sqlite>,
{
    let rows: Vec<SessionRow> = sqlx::query_as(
        "SELECT id, game_id, chapter_id, route_id, started_at, ended_at, counted_ms, afk_ms, \
                source, updated_at, device_id, deleted \
         FROM play_sessions WHERE game_id = ? ORDER BY started_at ASC",
    )
    .bind(game_id.to_string())
    .fetch_all(executor)
    .await?;
    sessions_from(rows)
}

fn sessions_from(rows: Vec<SessionRow>) -> Result<Vec<PlaySession>, DbError> {
    rows.into_iter()
        .map(PlaySession::try_from)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| corrupt(&err))
}

/// Append a session.
///
/// Returns `false` when the id already exists rather than an error: sessions are
/// append-only and merge by uuid, so a duplicate is the ordinary case on a second
/// import, not a failure.
pub async fn insert_session<'e, E>(executor: E, session: &PlaySession) -> Result<bool, DbError>
where
    E: Executor<'e, Database = Sqlite>,
{
    let result = sqlx::query(
        "INSERT INTO play_sessions (id, game_id, chapter_id, route_id, started_at, ended_at, \
                                    counted_ms, afk_ms, source, updated_at, device_id, deleted) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT(id) DO NOTHING",
    )
    .bind(session.meta.id.to_string())
    .bind(session.game_id.to_string())
    .bind(session.chapter_id.map(|id| id.to_string()))
    .bind(session.route_id.map(|id| id.to_string()))
    .bind(session.started_at.as_millis())
    .bind(session.ended_at.as_millis())
    .bind(session.counted_ms)
    .bind(session.afk_ms)
    .bind(session.source.as_str())
    .bind(session.meta.updated_at.as_millis())
    .bind(session.meta.device_id_device.device().as_str())
    .bind(i64::from(session.meta.deleted))
    .execute(executor)
    .await?;
    Ok(result.rows_affected() > 0)
}

/// Tracked playtime for one game, ms.
///
/// Joins `games` so a session whose game was removed cannot inflate a total: `PRAGMA
/// foreign_keys` is per connection, so orphans are always possible.
pub async fn tracked_playtime_ms(pool: &SqlitePool, game_id: EntityId) -> Result<i64, DbError> {
    let row: (i64,) = sqlx::query_as(
        "SELECT COALESCE(SUM(ps.counted_ms), 0) FROM play_sessions ps \
         JOIN games g ON g.id = ps.game_id AND g.deleted = 0 \
         WHERE ps.game_id = ? AND ps.deleted = 0",
    )
    .bind(game_id.to_string())
    .fetch_one(pool)
    .await?;
    Ok(row.0)
}

/// Away time for one game, ms.
pub async fn afk_playtime_ms(pool: &SqlitePool, game_id: EntityId) -> Result<i64, DbError> {
    let row: (i64,) = sqlx::query_as(
        "SELECT COALESCE(SUM(ps.afk_ms), 0) FROM play_sessions ps \
         JOIN games g ON g.id = ps.game_id AND g.deleted = 0 \
         WHERE ps.game_id = ? AND ps.deleted = 0",
    )
    .bind(game_id.to_string())
    .fetch_one(pool)
    .await?;
    Ok(row.0)
}

/// Tracked and away totals for every game, as `(game_id, counted_ms, afk_ms)`.
pub async fn playtime_totals(pool: &SqlitePool) -> Result<Vec<(EntityId, i64, i64)>, DbError> {
    let rows: Vec<(String, i64, i64)> = sqlx::query_as(
        "SELECT ps.game_id, COALESCE(SUM(ps.counted_ms), 0), COALESCE(SUM(ps.afk_ms), 0)          FROM play_sessions ps JOIN games g ON g.id = ps.game_id AND g.deleted = 0          WHERE ps.deleted = 0 GROUP BY ps.game_id",
    )
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|(id, counted, afk)| {
            Ok((
                EntityId::parse(&id).map_err(|err| corrupt(&err))?,
                counted,
                afk,
            ))
        })
        .collect::<Result<Vec<_>, DbError>>()
}

/// Tracked time per game, for the playtime sort.
pub async fn playtime_by_game(pool: &SqlitePool) -> Result<Vec<(EntityId, i64)>, DbError> {
    let rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT ps.game_id, COALESCE(SUM(ps.counted_ms), 0) FROM play_sessions ps          JOIN games g ON g.id = ps.game_id AND g.deleted = 0          WHERE ps.deleted = 0 GROUP BY ps.game_id",
    )
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|(id, counted)| Ok((EntityId::parse(&id).map_err(|err| corrupt(&err))?, counted)))
        .collect::<Result<Vec<_>, DbError>>()
}

/// Tracked time per local calendar day, as `(day_index, counted_ms)`.
///
/// A day index is days since 1970-01-01 of the *local* date — the same unit the
/// heatmap grid on the stats page walks when it labels a cell, so a bucket and
/// a cell can never disagree about units. The `'localtime'` modifier buckets by
/// the machine's calendar day: a session read between midnight and dawn belongs
/// to the night the reader experienced, not to the UTC date it happened to
/// carry. (DST transitions can shift a boundary by an hour; acceptable for a
/// heatmap, and `SQLite` resolves historical offsets for the timestamps it knows.)
pub async fn playtime_by_day(pool: &SqlitePool) -> Result<Vec<(i64, i64)>, DbError> {
    sqlx::query_as(
        "SELECT CAST(strftime('%s', ps.started_at / 1000, 'unixepoch', 'localtime', 'start of day') AS INTEGER) / 86400 AS day, \
                COALESCE(SUM(ps.counted_ms), 0) \
         FROM play_sessions ps JOIN games g ON g.id = ps.game_id AND g.deleted = 0 \
         WHERE ps.deleted = 0 \
         GROUP BY day ORDER BY day ASC",
    )
    .fetch_all(pool)
    .await
    .map_err(DbError::from)
}

/// How many games sit in each status.
pub async fn status_breakdown(pool: &SqlitePool) -> Result<StatusBreakdown, DbError> {
    let rows: Vec<(String, i64)> =
        sqlx::query_as("SELECT status, COUNT(*) FROM games WHERE deleted = 0 GROUP BY status")
            .fetch_all(pool)
            .await?;
    let totals: Vec<StatusRowTotal> = rows
        .into_iter()
        .filter_map(|(status, count)| {
            GameStatus::parse(&status)
                .ok()
                .map(|s| StatusRowTotal::new(s, usize::try_from(count).unwrap_or(0)))
        })
        .collect();
    Ok(StatusBreakdown::from_rows(&totals))
}

/// One route by id, live or tombstoned.
pub async fn route_by_id(pool: &SqlitePool, id: EntityId) -> Result<Option<Route>, DbError> {
    let row: Option<RouteRow> = sqlx::query_as(
        "SELECT id, game_id, name, sort_order, origin, merged_into, updated_at, device_id, \
                deleted \
         FROM routes WHERE id = ?",
    )
    .bind(id.to_string())
    .fetch_optional(pool)
    .await?;
    row.map(Route::try_from)
        .transpose()
        .map_err(|err| corrupt(&err))
}

/// Live routes for a game, in display order.
pub async fn list_routes(pool: &SqlitePool, game_id: EntityId) -> Result<Vec<Route>, DbError> {
    // Same orphan guard as every other aggregate over a child table: a route whose
    // game was removed must not be listed.
    let rows: Vec<RouteRow> = sqlx::query_as(
        "SELECT r.id, r.game_id, r.name, r.sort_order, r.origin, r.merged_into,                 r.updated_at, r.device_id, r.deleted          FROM routes r JOIN games g ON g.id = r.game_id AND g.deleted = 0          WHERE r.game_id = ? AND r.deleted = 0          ORDER BY r.sort_order IS NULL, r.sort_order, r.name ASC",
    )
    .bind(game_id.to_string())
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(Route::try_from)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| corrupt(&err))
}

/// Insert or update a route.
pub async fn upsert_route<'e, E>(executor: E, route: &Route) -> Result<(), DbError>
where
    E: Executor<'e, Database = Sqlite>,
{
    sqlx::query(
        "INSERT INTO routes (id, game_id, name, sort_order, origin, merged_into, updated_at, \
                             device_id, deleted) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT(id) DO UPDATE SET name = excluded.name, \
            sort_order = excluded.sort_order, origin = excluded.origin, \
            merged_into = excluded.merged_into, updated_at = excluded.updated_at, \
            device_id = excluded.device_id, deleted = excluded.deleted",
    )
    .bind(route.meta.id.to_string())
    .bind(route.game_id.to_string())
    .bind(&route.name)
    .bind(route.sort_order.map(i64::from))
    .bind(route.origin.as_str())
    .bind(route.merged_into.map(|id| id.to_string()))
    .bind(route.meta.updated_at.as_millis())
    .bind(route.meta.device_id_device.device().as_str())
    .bind(i64::from(route.meta.deleted))
    .execute(executor)
    .await?;
    Ok(())
}
