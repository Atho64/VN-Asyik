//! Cross-game journey commands: the Timeline feed and the Year in Review.

use serde::{Deserialize, Serialize};
use tauri::State;
use vn_core::ids::EntityId;
use vn_core::model::{Game, SessionSource};

use crate::commands::CommandResult;
use crate::state::AppState;

/// Events per timeline page, matching the session history page size.
pub const TIMELINE_PAGE: i64 = 50;

/// Keyset cursor of the last event on the loaded page, mirroring the session
/// history cursor: `(ts, id)` of the last row, so paging is stable while new
/// events arrive.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineCursor {
    pub ts: i64,
    pub id: EntityId,
}

/// One cross-game event on the Timeline.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineEvent {
    /// `"session" | "journal" | "ending" | "finished"`.
    pub kind: String,
    /// Event timestamp, UTC ms — a session's start, a journal entry's creation,
    /// an ending's achieved date, a game's finish date.
    pub ts: i64,
    /// Row id of the event (unique per source table).
    pub id: String,
    /// The live game the event belongs to; the row renders its cover and links
    /// to the game page.
    pub game: Game,
    /// Session fields: counted/away time, ms.
    pub counted_ms: Option<i64>,
    pub afk_ms: Option<i64>,
    /// Session field: what wrote the session.
    pub source: Option<SessionSource>,
    /// Journal field: the note (session events carry their latest note preview),
    /// capped at 240 chars + ellipsis.
    pub note: Option<String>,
    /// Journal/ending field: route label, historical snapshot text.
    pub route_name: Option<String>,
    /// Journal field: chapter label snapshot.
    pub chapter_name: Option<String>,
    /// Ending field: the ending's name.
    pub ending_name: Option<String>,
}

/// One bounded page of the Timeline, newest first.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelinePage {
    pub events: Vec<TimelineEvent>,
    /// True when older events exist and `events`' last row is the next cursor.
    pub has_more: bool,
}

/// The newest cross-game events for the Timeline page.
#[tauri::command]
pub async fn timeline_page(
    state: State<'_, AppState>,
    before: Option<TimelineCursor>,
) -> CommandResult<TimelinePage> {
    Ok(crate::db::queries::journey::timeline_page(state.db().pool(), before).await?)
}

/// One past moment that shares today's month and day.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryEvent {
    /// `"journal" | "ending"`.
    pub kind: String,
    /// Event timestamp, UTC ms.
    pub ts: i64,
    /// Row id of the event.
    pub id: String,
    /// The live game the moment belongs to.
    pub game: Game,
    /// Journal field: the note, capped at 240 chars + ellipsis.
    pub note: Option<String>,
    /// Journal/ending field: route label, historical snapshot text.
    pub route_name: Option<String>,
    /// Journal field: chapter label snapshot.
    pub chapter_name: Option<String>,
    /// Ending field: the ending's name.
    pub ending_name: Option<String>,
    /// Whole calendar years between the moment and today, at least 1.
    pub years_ago: i64,
}

/// Moments from earlier years that happened on today's date.
///
/// Empty is a normal answer, not an error: most days have no anniversary, and the
/// card that shows this is hidden rather than rendered as "nothing happened".
#[tauri::command]
pub async fn on_this_day(state: State<'_, AppState>) -> CommandResult<Vec<MemoryEvent>> {
    Ok(crate::db::queries::journey::on_this_day(state.db().pool()).await?)
}

/// One game inside a Year in Review's top list.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct YearReviewGame {
    pub game_id: EntityId,
    pub title: String,
    /// Tracked time in the year, ms.
    pub counted_ms: i64,
}

/// The longest single session of the year.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LongestSessionMoment {
    pub game_id: EntityId,
    pub title: String,
    pub counted_ms: i64,
}

/// The first game read in the year.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FirstGameMoment {
    pub game_id: EntityId,
    pub title: String,
    /// When that first session started, UTC ms.
    pub started_at: i64,
}

/// The month with the most reading time in the year.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MonthMoment {
    /// Calendar month, 1 to 12.
    pub month: i32,
    pub counted_ms: i64,
}

/// The local day with the most reading time in the year.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DayMoment {
    /// Local day index, encoded as-if-UTC like the heatmap's day cells.
    pub day: i64,
    pub counted_ms: i64,
}

/// The route with the most reading time in the year.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteMoment {
    pub name: String,
    pub counted_ms: i64,
}

/// The year with the most reading time across the whole library.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct YearMoment {
    pub year: i32,
    pub counted_ms: i64,
}

/// Library-wide aggregates for the all-time wrapped card.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryWrapped {
    pub total_ms: i64,
    pub afk_ms: i64,
    pub session_count: i64,
    /// Distinct local days with recorded time, across every year.
    pub active_days: i64,
    /// Games with at least one session.
    pub games_tracked: i64,
    /// Games whose status is `completed`.
    pub games_finished: i64,
    /// Every live game in the library, played or not.
    pub games_in_library: i64,
    pub endings_reached: i64,
    pub journal_entries: i64,
    /// Mean rating of every rated live game, in decivotes.
    pub average_rating_decivotes: Option<i64>,
    pub average_session_ms: Option<i64>,
    pub longest_session: Option<LongestSessionMoment>,
    pub longest_streak_days: i64,
    pub first_game: Option<FirstGameMoment>,
    /// Local year of the first session, for the "since" line.
    pub first_year: Option<i32>,
    /// Top 5 games by tracked time across the whole library.
    pub top_games: Vec<YearReviewGame>,
    pub top_route: Option<RouteMoment>,
    pub busiest_month: Option<MonthMoment>,
    pub busiest_day: Option<DayMoment>,
    pub most_read_year: Option<YearMoment>,
    /// Reading time per local weekday, Monday first (index 0..6).
    pub weekday_ms: Vec<i64>,
    /// Reading time per local hour of day (index 0..23).
    pub hour_ms: Vec<i64>,
}

/// Yearly aggregates for the Stats page's Year in Review card.
///
/// Days and years are the reader's local calendar (see `db::queries::journey`),
/// so the card agrees with the heatmap beside it.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct YearReview {
    pub year: i32,
    /// Active reading time in the year, ms.
    pub total_ms: i64,
    pub afk_ms: i64,
    pub session_count: i64,
    /// Distinct local days with recorded time.
    pub active_days: i64,
    pub endings_reached: i64,
    pub journal_entries: i64,
    /// Games whose first-ever session falls in the year.
    pub games_started: i64,
    /// Games whose finish date (`finished_at`, else first achieved ending)
    /// falls in the year.
    pub games_finished: i64,
    /// Mean rating (decivotes) of the games finished in the year, when any of
    /// them carries a rating.
    pub avg_rating_decivotes: Option<i64>,
    pub longest_session: Option<LongestSessionMoment>,
    pub first_game: Option<FirstGameMoment>,
    /// Top 5 games by tracked time in the year.
    pub top_games: Vec<YearReviewGame>,
    /// The busiest calendar month, when the year has counted time.
    pub busiest_month: Option<MonthMoment>,
    /// The busiest local day, using the heatmap's day-index encoding.
    pub busiest_day: Option<DayMoment>,
    /// Mean counted time per non-empty session in the year.
    pub average_session_ms: Option<i64>,
    /// Longest run of consecutive local reading days inside the year.
    pub longest_streak_days: i64,
    /// The route with the most time in the year, when sessions name one.
    pub top_route: Option<RouteMoment>,
    /// Games read in the year whose first-ever session was earlier.
    pub games_continued: i64,
    /// Reading time per local weekday, Monday first (index 0..6).
    pub weekday_ms: Vec<i64>,
    /// Reading time per local hour of day (index 0..23).
    pub hour_ms: Vec<i64>,
}

/// Yearly aggregates for the review card.
///
/// # Errors
/// `invalid` when the year is outside the plausible range; the backend buckets
/// by calendar year, and a nonsense year would render an empty card that looks
/// like an empty year.
#[tauri::command]
pub async fn year_review(state: State<'_, AppState>, year: i32) -> CommandResult<YearReview> {
    if !(1970..=2200).contains(&year) {
        return Err(crate::commands::CommandError::new(
            "invalid",
            format!("year out of range: {year}"),
        ));
    }
    Ok(crate::db::queries::journey::year_review(state.db().pool(), year).await?)
}

/// Library-wide aggregates for the all-time wrapped card.
///
/// # Errors
/// Database failures.
#[tauri::command]
pub async fn library_wrapped(state: State<'_, AppState>) -> CommandResult<LibraryWrapped> {
    Ok(crate::db::queries::wrapped::library_wrapped(state.db().pool()).await?)
}

/// Save a Year in Review share card as a PNG.
///
/// The canvas lives in the webview; this command owns the file boundary, so the
/// data URL is decoded and checked before the platform picker sees it.
///
/// # Errors
/// `invalid_image` for a malformed data URL, `file_too_large`, or a picker/write
/// failure.
#[tauri::command]
pub async fn save_share_card(
    app: tauri::AppHandle,
    filename: String,
    data_url: String,
) -> CommandResult<bool> {
    let bytes = crate::platform::storage::decode_png_data_url(&data_url)?;
    crate::platform::storage::save_image(app, filename, bytes).await
}

/// Share a Year in Review share card through the Android share sheet.
///
/// # Errors
/// `invalid_image` for a malformed data URL, `file_too_large`, or `unsupported_platform`
/// on desktop, where Save image is the supported path.
#[tauri::command]
pub async fn share_share_card(filename: String, data_url: String) -> CommandResult<()> {
    let bytes = crate::platform::storage::decode_png_data_url(&data_url)?;
    crate::platform::storage::share_image(filename, bytes).await
}
