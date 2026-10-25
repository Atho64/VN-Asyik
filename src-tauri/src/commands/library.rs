//! Library commands: games, chapters, sessions, endings, aggregates.

use serde::{Deserialize, Serialize};
use tauri::State;
use vn_core::clock::Timestamp;
use vn_core::ids::EntityId;
use vn_core::library::StatusBreakdown;
use vn_core::model::{Chapter, Ending, Game, GameStatus, PlaySession, Route, SessionSource};

use super::{CommandError, CommandResult};
use crate::db::{models::PrefsSnapshot, queries};
use crate::state::AppState;

/// A game together with its derived totals, as the UI wants them.
#[derive(Debug, Clone, Serialize)]
pub struct GameCard {
    /// The stored game.
    pub game: Game,
    /// Active tracked time from sessions, ms. This is the headline number.
    pub tracked_ms: i64,
    /// Time the user was away during tracked sessions, ms. Shown separately and
    /// never folded into `tracked_ms`.
    pub afk_ms: i64,
    /// Active tracked time plus the manual adjustment, ms.
    pub total_ms: i64,
}

fn now() -> CommandResult<Timestamp> {
    Timestamp::now().map_err(Into::into)
}

/// List every live game with its playtime totals.
#[tauri::command]
pub async fn list_games(state: State<'_, AppState>) -> CommandResult<Vec<GameCard>> {
    let pool = state.db().pool();
    let games = queries::list_games(pool).await?;
    let totals = queries::playtime_totals(pool).await?;
    Ok(games
        .into_iter()
        .map(|game| {
            let (tracked_ms, afk_ms) = totals
                .iter()
                .find(|(id, _, _)| *id == game.meta.id)
                .map_or((0, 0), |(_, active, afk)| (*active, *afk));
            let total_ms = game.total_playtime_ms(tracked_ms);
            GameCard {
                game,
                tracked_ms,
                afk_ms,
                total_ms,
            }
        })
        .collect())
}

/// Fetch a single game with totals.
#[tauri::command]
pub async fn get_game(state: State<'_, AppState>, id: String) -> CommandResult<Option<GameCard>> {
    let pool = state.db().pool();
    let parsed = EntityId::parse(&id)?;
    let Some(game) = queries::get_game(pool, parsed).await? else {
        return Ok(None);
    };
    let tracked_ms = queries::tracked_playtime_ms(pool, parsed).await?;
    let afk_ms = queries::afk_playtime_ms(pool, parsed).await?;
    let total_ms = game.total_playtime_ms(tracked_ms);
    Ok(Some(GameCard {
        game,
        tracked_ms,
        afk_ms,
        total_ms,
    }))
}

/// Payload for creating a game.
#[derive(Debug, Deserialize)]
pub struct CreateGameInput {
    /// Display title.
    pub title: String,
    /// Initial status.
    pub status: Option<String>,
}

/// Create a game and return it.
#[tauri::command]
pub async fn create_game(
    state: State<'_, AppState>,
    input: CreateGameInput,
) -> CommandResult<Game> {
    let status = match input.status.as_deref() {
        None => GameStatus::Wishlist,
        Some(raw) => GameStatus::parse(raw)
            .map_err(|_| CommandError::new("invalid_status", format!("unknown status: {raw}")))?,
    };
    let game = Game::new(state.device(), now()?, input.title, status)?;
    queries::upsert_game(state.db().pool(), &game).await?;
    Ok(game)
}

/// Payload for updating the mutable parts of a game.
#[derive(Debug, Deserialize)]
pub struct UpdateGameInput {
    /// Game to update.
    pub id: String,
    /// New title, when changed.
    pub title: Option<String>,
    /// New status, when changed.
    pub status: Option<String>,
    /// New rating, when changed. `null` clears it.
    #[serde(default, deserialize_with = "deserialize_some")]
    pub rating: Option<Option<u8>>,
    /// New notes, when changed. `null` clears them.
    #[serde(default, deserialize_with = "deserialize_some")]
    pub notes: Option<Option<String>>,
    /// New manual playtime in ms, when changed.
    pub manual_playtime_ms: Option<i64>,
    /// New VNDB id, when changed.
    pub vndb_id: Option<String>,
}

/// Distinguishes "field absent" from "field present and null".
fn deserialize_some<'de, T, D>(deserializer: D) -> Result<Option<T>, D::Error>
where
    T: Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    T::deserialize(deserializer).map(Some)
}

/// Apply a partial update to a game.
#[tauri::command]
pub async fn update_game(
    state: State<'_, AppState>,
    input: UpdateGameInput,
) -> CommandResult<Game> {
    let pool = state.db().pool();
    let id = EntityId::parse(&input.id)?;
    let Some(mut game) = queries::get_game(pool, id).await? else {
        return Err(CommandError::new("not_found", "game not found"));
    };

    if let Some(title) = input.title {
        if title.trim().is_empty() {
            return Err(CommandError::new("invalid", "title must not be empty"));
        }
        title.trim().clone_into(&mut game.title);
    }
    if let Some(status) = input.status {
        game.status = GameStatus::parse(&status).map_err(|_| {
            CommandError::new("invalid_status", format!("unknown status: {status}"))
        })?;
    }
    if let Some(rating) = input.rating {
        game.set_rating(rating)?;
    }
    if let Some(notes) = input.notes {
        game.set_notes(notes)?;
    }
    if let Some(ms) = input.manual_playtime_ms {
        if ms < 0 {
            return Err(CommandError::new(
                "invalid",
                "manual playtime must not be negative",
            ));
        }
        game.manual_playtime_ms = ms;
    }
    if let Some(vndb) = input.vndb_id {
        game.vndb_id = Some(vndb);
    }

    game.meta = game.meta.touch(now()?);
    queries::upsert_game(pool, &game).await?;
    Ok(game)
}

/// Soft-delete a game and its children.
#[tauri::command]
pub async fn delete_game(state: State<'_, AppState>, id: String) -> CommandResult<()> {
    let parsed = EntityId::parse(&id)?;
    queries::soft_delete_game(state.db().pool(), parsed, now()?, state.device()).await?;
    Ok(())
}

/// Chapters belonging to a game.
#[tauri::command]
pub async fn list_chapters(
    state: State<'_, AppState>,
    game_id: String,
) -> CommandResult<Vec<Chapter>> {
    let id = EntityId::parse(&game_id)?;
    Ok(queries::list_chapters(state.db().pool(), id).await?)
}

/// Sessions belonging to a game.
#[tauri::command]
pub async fn list_sessions(
    state: State<'_, AppState>,
    game_id: String,
) -> CommandResult<Vec<PlaySession>> {
    let id = EntityId::parse(&game_id)?;
    Ok(queries::list_sessions(state.db().pool(), id).await?)
}

/// Endings belonging to a game.
#[tauri::command]
pub async fn list_endings(
    state: State<'_, AppState>,
    game_id: String,
) -> CommandResult<Vec<Ending>> {
    let id = EntityId::parse(&game_id)?;
    Ok(queries::list_endings(state.db().pool(), id).await?)
}

/// Add an ending to a game.
///
/// Names are not enforced unique: a novel may legitimately have "Bad End" in two
/// different routes, and the route label is what tells them apart.
///
/// # Errors
/// `invalid` when the name is blank, or a database failure.
#[tauri::command]
pub async fn create_ending(
    state: State<'_, AppState>,
    game_id: String,
    name: String,
    route: Option<String>,
    achieved: bool,
) -> CommandResult<Ending> {
    let game = EntityId::parse(&game_id)?;
    let at = now()?;
    let mut ending = Ending::new(state.device(), at, game, name)?;
    ending.route = route
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    // "Achieved" is a date, not a flag: recording when a route was finished is the
    // point, and a flag without a date loses that. Clearing it removes the date too,
    // so the two can never disagree.
    ending.achieved = achieved;
    ending.achieved_at = if achieved { Some(at) } else { None };
    queries::upsert_ending(state.db().pool(), &ending).await?;
    Ok(ending)
}

/// Change an ending's name, route, or whether it was achieved.
///
/// # Errors
/// `invalid` for a blank name, `not_found` for an unknown id, or a database
/// failure.
#[tauri::command]
pub async fn update_ending(
    state: State<'_, AppState>,
    ending_id: String,
    name: String,
    route: Option<String>,
    achieved: bool,
) -> CommandResult<Ending> {
    let id = EntityId::parse(&ending_id)?;
    let pool = state.db().pool();
    let existing = queries::ending_by_id(pool, id)
        .await?
        .filter(|ending| !ending.meta.deleted)
        .ok_or_else(|| CommandError::new("not_found", "that ending no longer exists"))?;

    let at = now()?;
    // Rebuild through the domain constructor so the length and blank-name rules live
    // in one place rather than being re-checked by hand here.
    let mut updated = Ending::new(state.device(), at, existing.game_id, name)?;
    // Keep the identity: this edits a row, it does not create one.
    updated.meta = vn_core::model::SyncMeta {
        id,
        updated_at: at,
        device_id_device: existing.meta.device_id_device.clone(),
        deleted: false,
    };
    updated.route = route
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    updated.achieved = achieved;
    updated.achieved_at = if achieved {
        // Preserve the original date: re-saving an ending must not silently rewrite
        // when the user says they reached it.
        existing.achieved_at.or(Some(at))
    } else {
        None
    };
    queries::upsert_ending(pool, &updated).await?;
    Ok(updated)
}

/// Remove an ending, as a tombstone so the deletion syncs.
///
/// # Errors
/// `invalid` for a malformed id, or a database failure.
#[tauri::command]
pub async fn delete_ending(state: State<'_, AppState>, ending_id: String) -> CommandResult<()> {
    let id = EntityId::parse(&ending_id)?;
    queries::soft_delete_ending(state.db().pool(), id, now()?, state.device()).await?;
    Ok(())
}

/// Payload for a hand-entered session.
#[derive(Debug, Deserialize)]
pub struct ManualSessionInput {
    /// Game the session belongs to.
    pub game_id: String,
    /// Start, UTC ms.
    pub started_at_ms: i64,
    /// End, UTC ms.
    pub ended_at_ms: i64,
    /// Optional away time to subtract from the entered span, ms.
    #[serde(default)]
    pub afk_ms: Option<i64>,
}

/// Record a manual session.
///
/// Without `afk_ms` the whole span is active time. With it, the span minus the
/// away time is active, which lets a user correct "I left it running for an hour".
#[tauri::command]
pub async fn add_manual_session(
    state: State<'_, AppState>,
    input: ManualSessionInput,
) -> CommandResult<PlaySession> {
    let game_id = EntityId::parse(&input.game_id)?;
    if queries::get_game(state.db().pool(), game_id)
        .await?
        .is_none()
    {
        return Err(CommandError::new("not_found", "game not found"));
    }
    let started = Timestamp::from_millis(input.started_at_ms);
    let ended = Timestamp::from_millis(input.ended_at_ms);
    let span = ended.saturating_diff_ms(started);
    let afk = input.afk_ms.unwrap_or(0).clamp(0, span);
    let counted = span - afk;
    let session = PlaySession::new(
        state.device(),
        now()?,
        game_id,
        started,
        ended,
        counted,
        SessionSource::Manual,
    )?
    .with_afk(afk)?;
    queries::insert_session(state.db().pool(), &session).await?;
    Ok(session)
}

/// Counts per status.
#[tauri::command]
pub async fn status_breakdown(state: State<'_, AppState>) -> CommandResult<StatusBreakdown> {
    Ok(queries::status_breakdown(state.db().pool()).await?)
}

/// Tracked time per local calendar day, as `(day_index, counted_ms)` — day
/// indexes are days since 1970-01-01 of the local date, the unit the heatmap
/// grid walks. Newest last.
#[tauri::command]
pub async fn playtime_by_day(state: State<'_, AppState>) -> CommandResult<Vec<(i64, i64)>> {
    Ok(queries::playtime_by_day(state.db().pool()).await?)
}

/// Library-wide `(active_ms, afk_ms)`.
#[tauri::command]
pub async fn playtime_totals(state: State<'_, AppState>) -> CommandResult<(i64, i64)> {
    let totals = queries::playtime_totals(state.db().pool()).await?;
    let active = totals.iter().map(|(_, active, _)| *active).sum();
    let afk = totals.iter().map(|(_, _, afk)| *afk).sum();
    Ok((active, afk))
}

/// Current AFK configuration.
#[tauri::command]
pub async fn get_afk_config(state: State<'_, AppState>) -> CommandResult<vn_core::afk::AfkConfig> {
    Ok(queries::load_afk_config(state.db().pool()).await?)
}

/// Update the AFK threshold (clamped) and whether idle time is excluded.
///
/// The setting is both persisted and applied to the running tracker: the
/// tracker reads its config on every poll, so a change made mid-session takes
/// effect from the next 2-second tick instead of waiting for a restart.
#[tauri::command]
pub async fn set_afk_config(
    state: State<'_, AppState>,
    threshold_ms: i64,
    exclude_afk: bool,
) -> CommandResult<vn_core::afk::AfkConfig> {
    let config = vn_core::afk::AfkConfig::new(threshold_ms, exclude_afk);
    let saved = queries::save_afk_config(state.db().pool(), config, now()?).await?;
    state.tracker().set_afk_config(saved).await;
    Ok(saved)
}

/// Persist the dominant colour extracted from a game's cover.
///
/// The frontend extracts the palette once (it already has the decoded bitmap) and
/// writes it back, so the database carries the colour on the next start without
/// re-analysing the image. Invalid values are rejected rather than stored.
#[tauri::command]
pub async fn set_game_accent(
    state: State<'_, AppState>,
    id: String,
    accent: String,
) -> CommandResult<Game> {
    if !is_hex_colour(&accent) {
        return Err(CommandError::new(
            "invalid",
            "accent must be a #rrggbb colour",
        ));
    }
    let pool = state.db().pool();
    let parsed = EntityId::parse(&id)?;
    let Some(mut game) = queries::get_game(pool, parsed).await? else {
        return Err(CommandError::new("not_found", "game not found"));
    };
    if game.accent_color.as_deref() == Some(accent.as_str()) {
        return Ok(game);
    }
    game.accent_color = Some(accent);
    game.meta = game.meta.touch(now()?);
    queries::upsert_game(pool, &game).await?;
    Ok(game)
}

/// One route with its chapters and endings, ready for the routes card.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteView {
    /// Route id. `None` for the bucket holding rows that belong to no route.
    pub id: Option<String>,
    /// What to show.
    pub name: String,
    /// Where the row came from: `engine`, `ending` or `manual`.
    pub origin: String,
    /// Chapters on this route.
    pub chapters: Vec<super::tracking::ChapterView>,
    /// Endings on this route.
    pub endings: Vec<Ending>,
    /// Chapters reached.
    pub chapters_reached: usize,
    /// Chapters known.
    pub chapters_total: usize,
    /// Endings reached.
    pub endings_reached: usize,
    /// Endings known.
    pub endings_total: usize,
    /// Tracked time on this route, ms.
    pub playtime_ms: i64,
    /// Other routes on the same game this one could merge into.
    pub merge_candidates: Vec<String>,
}

/// Time recorded against each route itself, with no chapter named.
///
/// Kept separate from the per-chapter sums so a route total is the whole truth: a user
/// who timed "the Shiroha route" for an hour and then two of its chapters for thirty
/// minutes each has two and a half hours on that route, not one.
async fn route_only_playtime(
    pool: &sqlx::SqlitePool,
    game_id: EntityId,
) -> CommandResult<Vec<(String, i64)>> {
    // The `games` join is not decoration: `PRAGMA foreign_keys` is per connection, so
    // a session whose game was removed can outlive it and would otherwise inflate a
    // total the UI has no game to show it under.
    sqlx::query_as(
        "SELECT ps.route_id, COALESCE(SUM(ps.counted_ms), 0) \
         FROM play_sessions ps \
         JOIN games g ON g.id = ps.game_id AND g.deleted = 0 \
         WHERE ps.game_id = ? AND ps.route_id IS NOT NULL AND ps.deleted = 0 \
         GROUP BY ps.route_id",
    )
    .bind(game_id.to_string())
    .fetch_all(pool)
    .await
    .map_err(|err| CommandError::new("db", err.to_string()))
}

/// Chapters' time plus the route's own time.
///
/// `survivor` resolves a merged-away id to the route that absorbed it, so time timed
/// against a route a peer merged away still lands on the survivor's card.
fn route_total(
    chapters: &[super::tracking::ChapterView],
    route_only: &[(String, i64)],
    key: &str,
    survivor: &dyn Fn(&str) -> Option<String>,
) -> i64 {
    chapters.iter().map(|c| c.playtime_ms).sum::<i64>()
        + route_only
            .iter()
            .filter(|(rid, _)| survivor(rid).as_deref().unwrap_or(rid.as_str()) == key)
            .map(|(_, ms)| *ms)
            .sum::<i64>()
}

/// Every route for a game with its chapters and endings.
///
/// Children whose `route_id` names a route that was merged away are folded into the
/// survivor, so a merge is one write on the route row rather than a rewrite of every
/// child.
///
/// # Errors
/// `invalid` for a malformed id, or a database failure.
#[tauri::command]
pub async fn game_routes(
    state: State<'_, AppState>,
    game_id: String,
) -> CommandResult<Vec<RouteView>> {
    let id = EntityId::parse(&game_id)?;
    let pool = state.db().pool();

    // The same chapter views the chapter list uses, so playtime and the rename and
    // manual flags have one definition.
    let chapters = super::tracking::chapter_views(state.db(), &game_id).await?;
    let routes = queries::list_routes(pool, id).await?;
    let endings = queries::list_endings(pool, id).await?;
    let route_only_ms = route_only_playtime(pool, id).await?;

    // Follow a merge chain with a bound: a corrupt peer file could contain a cycle,
    // and looping forever on it would hang the screen.
    let survivor = |start: &str| -> Option<String> {
        let mut current = start.to_owned();
        for _ in 0..8 {
            let Some(route) = routes.iter().find(|r| r.meta.id.to_string() == current) else {
                return Some(current);
            };
            match route.merged_into {
                None => return Some(current),
                Some(next) => current = next.to_string(),
            }
        }
        None
    };

    let live: Vec<_> = routes
        .iter()
        .filter(|route| route.merged_into.is_none())
        .collect();

    let mut views: Vec<RouteView> = live
        .iter()
        .map(|route| {
            let key = route.meta.id.to_string();
            let route_chapters: Vec<_> = chapters
                .iter()
                .filter(|chapter| {
                    chapter
                        .route_id
                        .as_deref()
                        .and_then(&survivor)
                        .is_some_and(|target| target == key)
                })
                .cloned()
                .collect();
            let route_endings: Vec<_> = endings
                .iter()
                .filter(|ending| {
                    ending
                        .route_id
                        .map(|rid| rid.to_string())
                        .as_deref()
                        .and_then(&survivor)
                        .is_some_and(|target| target == key)
                })
                .cloned()
                .collect();
            RouteView {
                id: Some(key.clone()),
                name: route.name.clone(),
                origin: route.origin.as_str().to_owned(),
                chapters_reached: route_chapters.iter().filter(|c| c.reached).count(),
                chapters_total: route_chapters.len(),
                endings_reached: route_endings.iter().filter(|e| e.achieved).count(),
                endings_total: route_endings.len(),
                playtime_ms: route_total(&route_chapters, &route_only_ms, &key, &survivor),
                chapters: route_chapters,
                endings: route_endings,
                merge_candidates: live
                    .iter()
                    .filter(|other| other.meta.id != route.meta.id)
                    .map(|other| other.meta.id.to_string())
                    .collect(),
            }
        })
        .collect();

    // Anything with no route at all still has to be reachable, or creating a route
    // would hide the chapters that made the user create it. One bucket, shown last.
    let unlinked_chapters: Vec<_> = chapters
        .iter()
        .filter(|chapter| chapter.route_id.is_none())
        .cloned()
        .collect();
    let unlinked_endings: Vec<_> = endings
        .iter()
        .filter(|ending| ending.route_id.is_none())
        .cloned()
        .collect();
    if !unlinked_chapters.is_empty() || !unlinked_endings.is_empty() {
        views.push(RouteView {
            id: None,
            name: String::new(),
            origin: "manual".to_owned(),
            chapters_reached: unlinked_chapters.iter().filter(|c| c.reached).count(),
            chapters_total: unlinked_chapters.len(),
            endings_reached: unlinked_endings.iter().filter(|e| e.achieved).count(),
            endings_total: unlinked_endings.len(),
            playtime_ms: 0,
            chapters: unlinked_chapters,
            endings: unlinked_endings,
            merge_candidates: Vec::new(),
        });
    }

    Ok(views)
}

/// Create a user-named route and return its id.
///
/// # Errors
/// `invalid` for a blank or over-long name, a malformed id, or a database failure.
#[tauri::command]
pub async fn create_route(
    state: State<'_, AppState>,
    game_id: String,
    name: String,
) -> CommandResult<String> {
    let game = EntityId::parse(&game_id)?;
    let route = Route::new(state.device(), now()?, game, name)?;
    let id = route.meta.id.to_string();
    queries::upsert_route(state.db().pool(), &route).await?;
    Ok(id)
}

/// Rename a route.
///
/// # Errors
/// `not_found` for an unknown or deleted route, `invalid` for a blank name, or a
/// database failure.
#[tauri::command]
pub async fn rename_route(
    state: State<'_, AppState>,
    route_id: String,
    name: String,
) -> CommandResult<()> {
    let id = EntityId::parse(&route_id)?;
    let pool = state.db().pool();
    let Some(mut route) = queries::route_by_id(pool, id).await? else {
        return Err(CommandError::new("not_found", "No such route"));
    };
    if route.meta.deleted {
        return Err(CommandError::new("not_found", "No such route"));
    }
    let renamed = Route::new(state.device(), now()?, route.game_id, name)?;
    route.name = renamed.name;
    route.meta.updated_at = now()?;
    route.meta.device_id_device = vn_core::model::DeviceTag::new(state.device());
    queries::upsert_route(pool, &route).await?;
    Ok(())
}

/// Merge one route into another.
///
/// The source row is kept with a `merged_into` pointer rather than deleted: folder
/// sync merges by id, so a delete could be resurrected by a device that had not seen
/// it, while a pointer travels with the row and both devices converge.
///
/// # Errors
/// `invalid` when the ids match, either route is deleted or already merged, or they
/// belong to different games; `not_found` for an unknown id; or a database failure.
#[tauri::command]
pub async fn merge_routes(
    state: State<'_, AppState>,
    source_id: String,
    target_id: String,
) -> CommandResult<()> {
    let source = EntityId::parse(&source_id)?;
    let target = EntityId::parse(&target_id)?;
    if source == target {
        return Err(CommandError::new(
            "invalid",
            "A route cannot merge into itself",
        ));
    }
    let pool = state.db().pool();
    let Some(mut row) = queries::route_by_id(pool, source).await? else {
        return Err(CommandError::new("not_found", "No such route"));
    };
    let Some(target_row) = queries::route_by_id(pool, target).await? else {
        return Err(CommandError::new(
            "not_found",
            "Target route does not exist",
        ));
    };
    // A chain would have to be papered over by every reader, and a cycle drops the
    // chapters entirely, so a dead or already-merged target is refused outright.
    if row.meta.deleted || target_row.meta.deleted {
        return Err(CommandError::new(
            "invalid",
            "A deleted route cannot be merged",
        ));
    }
    if row.merged_into.is_some() || target_row.merged_into.is_some() {
        return Err(CommandError::new(
            "invalid",
            "This route was already merged into another",
        ));
    }
    if row.game_id != target_row.game_id {
        return Err(CommandError::new(
            "invalid",
            "Routes from different games cannot be merged",
        ));
    }

    // The children move and the pointer that explains the move commit together: a
    // half-applied merge would point children at a route nothing says was merged.
    let mut tx = pool.begin().await.map_err(crate::db::DbError::from)?;
    queries::reassign_route_children(&mut tx, source, target).await?;
    row.merged_into = Some(target);
    row.meta.updated_at = now()?;
    row.meta.device_id_device = vn_core::model::DeviceTag::new(state.device());
    queries::upsert_route(&mut *tx, &row).await?;
    tx.commit().await.map_err(crate::db::DbError::from)?;
    Ok(())
}

/// Point a chapter at a route, or at none.
///
/// # Errors
/// `not_found` for an unknown chapter, or a database failure.
#[tauri::command]
pub async fn set_chapter_route(
    state: State<'_, AppState>,
    chapter_id: String,
    route_id: Option<String>,
) -> CommandResult<()> {
    let id = EntityId::parse(&chapter_id)?;
    let route = route_id.map(|value| EntityId::parse(&value)).transpose()?;
    let pool = state.db().pool();
    let Some(mut chapter) = queries::chapter_by_id(pool, id).await? else {
        return Err(CommandError::new(
            "not_found",
            "that chapter no longer exists",
        ));
    };
    chapter.route_id = route;
    chapter.meta = chapter.meta.touch(now()?);
    chapter.meta.device_id_device = vn_core::model::DeviceTag::new(state.device());
    queries::upsert_chapter(pool, &chapter).await?;
    Ok(())
}

/// Point an ending at a route, or at none.
///
/// # Errors
/// `not_found` for an unknown ending, or a database failure.
#[tauri::command]
pub async fn set_ending_route(
    state: State<'_, AppState>,
    ending_id: String,
    route_id: Option<String>,
) -> CommandResult<()> {
    let id = EntityId::parse(&ending_id)?;
    let route = route_id.map(|value| EntityId::parse(&value)).transpose()?;
    let pool = state.db().pool();
    let Some(mut ending) = queries::ending_by_id(pool, id).await? else {
        return Err(CommandError::new(
            "not_found",
            "that ending no longer exists",
        ));
    };
    ending.route_id = route;
    ending.meta = ending.meta.touch(now()?);
    ending.meta.device_id_device = vn_core::model::DeviceTag::new(state.device());
    queries::upsert_ending(pool, &ending).await?;
    Ok(())
}

/// Cached art for one game, as data URLs.
#[derive(Debug, Clone, Serialize)]
pub struct LocalArt {
    /// Data URL of the custom cover, when the user set one.
    pub cover_data_url: Option<String>,
    /// Data URL of the pre-blurred backdrop, when it exists.
    pub backdrop_data_url: Option<String>,
}

/// Read a game's own artwork as data URLs.
///
/// Returns empty art rather than an error when the files are missing: folder sync and
/// backup carry sync rows only, so a peer resolves no custom art and has to fall back
/// instead of failing the screen.
///
/// # Errors
/// `invalid` for a malformed id, or a database failure.
#[tauri::command]
pub async fn game_custom_art(state: State<'_, AppState>, id: String) -> CommandResult<LocalArt> {
    let game_id = EntityId::parse(&id)?;
    let mut art = LocalArt {
        cover_data_url: None,
        backdrop_data_url: None,
    };
    // The row is the source of truth for whether art exists; a file without a row is
    // left over from a cleared cover and is not shown.
    let Some(game) = queries::get_game(state.db().pool(), game_id).await? else {
        return Ok(art);
    };
    if game.custom_cover_path.is_none() {
        return Ok(art);
    }

    let store = crate::vndb::images::ImageStore::new(state.data_dir());
    let key = game_id.to_string();
    if let Ok(bytes) = crate::vndb::images::read_bytes(&store.custom_cover_path(&key)).await {
        art.cover_data_url = Some(super::vndb::to_data_url(&bytes, "image/jpeg"));
    }
    if let Ok(bytes) = crate::vndb::images::read_bytes(&store.custom_backdrop_path(&key)).await {
        art.backdrop_data_url = Some(super::vndb::to_data_url(&bytes, "image/jpeg"));
    }
    Ok(art)
}

/// Import an image file as this game's cover.
///
/// The file is re-encoded rather than copied, so a non-image is rejected here instead
/// of becoming a broken cover the webview cannot paint. On Android the picker
/// returns a `content://` URI; it is copied through the content resolver into
/// a cache file, read, and deleted (AGENTS.md §9).
///
/// # Errors
/// `invalid` for a malformed id or an undecodable file, `document` for a file
/// that cannot be read (including an Android document that cannot be copied),
/// `not_found` for an unknown game, or a database failure.
#[tauri::command]
pub async fn set_game_custom_cover(
    state: State<'_, AppState>,
    id: String,
    path: String,
) -> CommandResult<Game> {
    let game_id = EntityId::parse(&id)?;
    let pool = state.db().pool();
    let Some(mut game) = queries::get_game(pool, game_id).await? else {
        return Err(CommandError::new("not_found", "game not found"));
    };

    // The resolver bridges Android's content URIs; the guard deletes the
    // cache copy once the bytes are read. The bridge call blocks, so it runs
    // off the UI thread.
    let resolved = tauri::async_runtime::spawn_blocking(move || {
        crate::platform::storage::resolve_document_uri(&path)
    })
    .await
    .map_err(|err| CommandError::new("document", err.to_string()))?
    .map_err(|err| CommandError::new("document", err))?;
    let bytes = tokio::fs::read(resolved.path())
        .await
        .map_err(|err| CommandError::new("document", err.to_string()))?;
    drop(resolved);

    let store = crate::vndb::images::ImageStore::new(state.data_dir());
    let key = game_id.to_string();
    // Re-encoding is the validation step: a renamed text file fails here.
    let relative = crate::vndb::images::import_custom_cover_bytes(&store, &key, &bytes)
        .map_err(|err| CommandError::new("invalid", err.to_string()))?;

    // The path is stored relative to the app data dir: the column travels in the sync
    // payload, and an absolute path is meaningless on another device.
    game.custom_cover_path = Some(relative);
    game.meta = game.meta.touch(now()?);
    game.meta.device_id_device = vn_core::model::DeviceTag::new(state.device());
    queries::upsert_game(pool, &game).await?;
    Ok(game)
}

/// Remove this game's custom cover and fall back to the previous artwork.
///
/// # Errors
/// `invalid` for a malformed id, or a database failure.
#[tauri::command]
pub async fn clear_game_custom_cover(
    state: State<'_, AppState>,
    id: String,
) -> CommandResult<Game> {
    let game_id = EntityId::parse(&id)?;
    let pool = state.db().pool();
    let Some(mut game) = queries::get_game(pool, game_id).await? else {
        return Err(CommandError::new("not_found", "game not found"));
    };

    let store = crate::vndb::images::ImageStore::new(state.data_dir());
    // Missing files are not an error: the row is the source of truth, and clearing a
    // cover whose files were already removed must still succeed.
    crate::vndb::images::remove_custom_cover(&store, &game_id.to_string())
        .map_err(|err| CommandError::new("io", err.to_string()))?;

    game.custom_cover_path = None;
    game.meta = game.meta.touch(now()?);
    game.meta.device_id_device = vn_core::model::DeviceTag::new(state.device());
    queries::upsert_game(pool, &game).await?;
    Ok(game)
}

/// `#rrggbb`, case-insensitive.
fn is_hex_colour(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 7 && bytes[0] == b'#' && bytes[1..].iter().all(u8::is_ascii_hexdigit)
}

/// Preferences are read by the library screen too (NSFW mode affects covers).
#[tauri::command]
pub async fn prefs_for_library(state: State<'_, AppState>) -> CommandResult<PrefsSnapshot> {
    Ok(queries::load_prefs(state.db().pool()).await?)
}

#[cfg(test)]
mod route_tests {
    use vn_core::clock::Timestamp;
    use vn_core::model::{Game, GameStatus, PlaySession, Route, SessionSource};

    use crate::db::{Db, queries};

    fn at(ms: i64) -> Timestamp {
        Timestamp::from_millis(1_700_000_000_000 + ms)
    }

    /// A chapter view standing in for the one `chapter_views` builds.
    fn chapter_view(playtime_ms: i64) -> super::super::tracking::ChapterView {
        super::super::tracking::ChapterView {
            id: String::new(),
            raw_name: String::new(),
            display_name: String::new(),
            renamed: false,
            manual: false,
            order: None,
            route_key: None,
            route_name: None,
            route_label: String::new(),
            reached: false,
            playtime_ms,
            route_id: None,
        }
    }

    #[tokio::test]
    async fn a_route_total_is_its_chapters_plus_its_own_time() {
        let db = Db::open_memory().await.expect("open");
        let device = queries::ensure_device_id(db.pool()).await.expect("device");
        let game = Game::new(&device, at(0), "Sanoba Witch", GameStatus::Playing).expect("game");
        queries::upsert_game(db.pool(), &game).await.expect("game");
        let route = Route::new(&device, at(0), game.meta.id, "Shiroha").expect("route");
        queries::upsert_route(db.pool(), &route)
            .await
            .expect("route");

        let mut timed = PlaySession::new(
            &device,
            at(0),
            game.meta.id,
            at(0),
            at(3_600_000),
            3_600_000,
            SessionSource::Manual,
        )
        .expect("session");
        timed.route_id = Some(route.meta.id);
        queries::insert_session(db.pool(), &timed)
            .await
            .expect("session");

        let route_only = super::route_only_playtime(db.pool(), game.meta.id)
            .await
            .expect("rollup");
        let survivor = |id: &str| Some(id.to_owned());
        let key = route.meta.id.to_string();

        // One hour on the route itself, plus half an hour of chapters under it.
        assert_eq!(
            super::route_total(&[chapter_view(1_800_000)], &route_only, &key, &survivor),
            3_600_000 + 1_800_000
        );
    }

    #[tokio::test]
    async fn time_timed_against_a_merged_away_route_follows_the_survivor() {
        // A merge performed on a peer leaves sessions pointing at the route that was
        // merged away. Those hours belong to the survivor, not to nobody.
        let db = Db::open_memory().await.expect("open");
        let device = queries::ensure_device_id(db.pool()).await.expect("device");
        let game = Game::new(&device, at(0), "Sanoba Witch", GameStatus::Playing).expect("game");
        queries::upsert_game(db.pool(), &game).await.expect("game");
        let survivor_route = Route::new(&device, at(0), game.meta.id, "Shiroha").expect("route");
        queries::upsert_route(db.pool(), &survivor_route)
            .await
            .expect("survivor");
        let merged_away =
            Route::new(&device, at(0), game.meta.id, "Shiroha (engine)").expect("route");
        queries::upsert_route(db.pool(), &merged_away)
            .await
            .expect("merged away");

        let mut timed = PlaySession::new(
            &device,
            at(0),
            game.meta.id,
            at(0),
            at(3_600_000),
            3_600_000,
            SessionSource::Manual,
        )
        .expect("session");
        timed.route_id = Some(merged_away.meta.id);
        queries::insert_session(db.pool(), &timed)
            .await
            .expect("session");

        let route_only = super::route_only_playtime(db.pool(), game.meta.id)
            .await
            .expect("rollup");
        let merged_key = merged_away.meta.id.to_string();
        let survivor_key = survivor_route.meta.id.to_string();
        let fold = |id: &str| {
            if id == merged_key {
                Some(survivor_key.clone())
            } else {
                Some(id.to_owned())
            }
        };

        assert_eq!(
            super::route_total(&[], &route_only, &survivor_key, &fold),
            3_600_000,
            "the merged-away route's own time must follow the fold"
        );
    }

    #[tokio::test]
    async fn a_route_of_a_deleted_game_is_not_listed() {
        // `PRAGMA foreign_keys` is per connection, so an orphan is always possible.
        let db = Db::open_memory().await.expect("open");
        let device = queries::ensure_device_id(db.pool()).await.expect("device");
        let game = Game::new(&device, at(0), "Gone", GameStatus::Playing).expect("game");
        queries::upsert_game(db.pool(), &game).await.expect("game");
        let route = Route::new(&device, at(0), game.meta.id, "Shiroha").expect("route");
        queries::upsert_route(db.pool(), &route)
            .await
            .expect("route");

        queries::soft_delete_game(db.pool(), game.meta.id, at(1), &device)
            .await
            .expect("delete");

        let live = queries::list_routes(db.pool(), game.meta.id)
            .await
            .expect("list");
        assert_eq!(
            live.len(),
            0,
            "a route whose game is gone must not be listed"
        );

        // Soft delete must reach the route row, or it keeps travelling in sync payloads.
        let all = queries::list_routes_including_deleted(db.pool(), game.meta.id)
            .await
            .expect("all");
        assert_eq!(all.len(), 1);
        assert!(all[0].meta.deleted, "deleting a game tombstones its routes");
    }

    #[test]
    fn a_chapter_with_no_route_still_contributes_its_time() {
        let survivor = |id: &str| Some(id.to_owned());
        assert_eq!(
            super::route_total(&[chapter_view(5_000)], &[], "any", &survivor),
            5_000
        );
    }
}
