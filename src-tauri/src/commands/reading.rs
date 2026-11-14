//! Reading checkpoint commands. Journal writes never alter playtime or save files.

use std::collections::HashMap;

use tauri::State;
use vn_core::journal::{JournalDocument, JournalEntryView, JournalFormat, JournalPhoto, PhotoGap};
use vn_core::model::{Game, NsfwMode};
use vn_core::{EntityId, reading::ReadingEntry};

use super::journey::TimelineCursor;
use super::{CommandError, CommandResult};
use crate::db::{DbError, queries, reading};
use crate::state::AppState;
use crate::vndb::cache::{self, CachedScreenshot};
use crate::vndb::screenshots::{self, ScreenshotStore};

#[tauri::command]
pub async fn reading_context(
    state: State<'_, AppState>,
    game_id: String,
    offset: Option<i64>,
) -> CommandResult<reading::ReadingContext> {
    reading::context(
        state.db().pool(),
        EntityId::parse(&game_id)?,
        offset.unwrap_or(0),
    )
    .await
}

#[tauri::command]
pub async fn reading_session_context(
    state: State<'_, AppState>,
    game_id: String,
    session_id: String,
    offset: Option<i64>,
) -> CommandResult<reading::SessionReadingContext> {
    reading::session_context(
        state.db().pool(),
        EntityId::parse(&game_id)?,
        EntityId::parse(&session_id)?,
        offset.unwrap_or(0),
    )
    .await
}

#[tauri::command]
pub async fn reading_save(
    state: State<'_, AppState>,
    input: reading::SaveReadingInput,
) -> CommandResult<ReadingEntry> {
    reading::save(state.db().pool(), state.device(), input).await
}

#[tauri::command]
pub async fn reading_delete(
    state: State<'_, AppState>,
    game_id: String,
    id: String,
    expected_updated_at: i64,
) -> CommandResult<()> {
    reading::delete(
        state.db().pool(),
        state.device(),
        EntityId::parse(&game_id)?,
        EntityId::parse(&id)?,
        expected_updated_at,
    )
    .await
}

/// What a journal export wrote.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JournalExportOutcome {
    /// False when the user cancelled the document picker.
    pub saved: bool,
    /// Suggested file name, also used by the picker.
    pub filename: String,
    /// Checkpoints in the document.
    pub entries: usize,
    /// Pinned photos embedded in the document.
    pub photos: usize,
    /// Pinned photos left out because the content setting conceals them.
    pub concealed: usize,
}

/// Export one game's journal as Markdown or HTML, resolving pinned photos from the local cache only (`AGENTS.md` §2.6 offline rule).
///
/// # Errors
/// `not_found` for an unknown game, `invalid` for an unknown format, or document write errors.
#[tauri::command]
pub async fn journal_export_file(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    game_id: String,
    format: String,
) -> CommandResult<JournalExportOutcome> {
    let format = JournalFormat::parse(&format).map_err(|err| CommandError::new("invalid", err))?;
    let pool = state.db().pool();
    let game_id = EntityId::parse(&game_id)?;
    let game = queries::get_game(pool, game_id)
        .await?
        .ok_or_else(|| CommandError::new("not_found", "game not found"))?;

    let rows = reading::list_for_export(pool, game_id).await?;
    let prefs = queries::load_prefs(pool).await?;
    let mode = prefs.nsfw_mode();

    // One read of the VN's cached screenshots, not one per checkpoint: a journal
    // with thirty photo entries would otherwise ask the database thirty times.
    let cached: HashMap<String, CachedScreenshot> = match game.vndb_id.as_deref() {
        Some(vn_id) => cache::get_screenshots(pool, vn_id)
            .await?
            .into_iter()
            .map(|row| (row.image_id.clone(), row))
            .collect(),
        None => HashMap::new(),
    };
    let store = ScreenshotStore::new(state.data_dir());

    let mut entries = Vec::with_capacity(rows.len());
    let mut photos = 0_usize;
    let mut concealed = 0_usize;
    for row in rows {
        let photo = if row.entry.photo_image_id.is_empty() {
            None
        } else {
            let resolved = resolve_photo(&cached, &store, &row.entry.photo_image_id, mode).await;
            match &resolved {
                JournalPhoto::Embedded { .. } => photos += 1,
                JournalPhoto::Missing(PhotoGap::Concealed) => concealed += 1,
                JournalPhoto::Missing(PhotoGap::NotCached) => {}
            }
            Some(resolved)
        };
        entries.push(JournalEntryView {
            date: row.date,
            time: row.time,
            played_date: row.played_date,
            note: row.entry.note,
            save_slot: row.entry.save_slot,
            route_name: row.entry.route_name,
            chapter_name: row.entry.chapter_name,
            photo,
        });
    }

    let document = JournalDocument {
        title: game.title,
        title_original: game.title_original,
        vndb_id: game.vndb_id,
        exported_on: local_today(pool).await?,
        entries,
    };
    let filename = document.file_name(format);
    let contents = document.render(format);
    let saved = crate::platform::storage::save_document_as(
        app,
        filename.clone(),
        contents,
        format.mime(),
        format.extension(),
    )
    .await?;
    Ok(JournalExportOutcome {
        saved,
        filename,
        entries: document.entries.len(),
        photos,
        concealed,
    })
}

/// One journal search hit: the checkpoint and its live parent game.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JournalSearchResult {
    pub entry: ReadingEntry,
    pub game: Game,
}

/// A bounded page of journal search results, newest first.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JournalSearchPage {
    pub results: Vec<JournalSearchResult>,
    pub has_more: bool,
}

/// Search journal checkpoints across the library, or within one game.
///
/// An empty query is an empty page, not an error: the search box clears back to
/// the feed and must not flash a failure on the way. The query is capped so a
/// paste of the whole novel cannot become a table scan with a 4 KB pattern.
///
/// # Errors
/// `invalid` for an over-long query or a bad game id, `db_error` on failure.
#[tauri::command]
pub async fn journal_search(
    state: State<'_, AppState>,
    query: String,
    game_id: Option<String>,
    before: Option<TimelineCursor>,
) -> CommandResult<JournalSearchPage> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(JournalSearchPage {
            results: Vec::new(),
            has_more: false,
        });
    }
    if query.chars().count() > 200 {
        return Err(CommandError::new("invalid", "search query is too long"));
    }
    let game = game_id.map(|id| EntityId::parse(&id)).transpose()?;
    let pool = state.db().pool();
    let before = before.map(|cursor| (cursor.ts, cursor.id.to_string()));
    let (entries, has_more) = reading::search(
        pool,
        query,
        game,
        before.as_ref().map(|(ts, id)| (*ts, id.as_str())),
    )
    .await?;
    let ids: Vec<String> = entries
        .iter()
        .map(|entry| entry.game_id.to_string())
        .collect();
    let games = queries::journey::games_by_ids(pool, &ids).await?;
    let mut results = Vec::with_capacity(entries.len());
    for entry in entries {
        // The search joins live games, so a miss is only the delete-between-queries
        // race; dropping the row is the honest read of it, as on the timeline.
        let Some(game) = games.get(&entry.game_id.to_string()) else {
            continue;
        };
        results.push(JournalSearchResult {
            entry,
            game: game.clone(),
        });
    }
    Ok(JournalSearchPage { results, has_more })
}

/// The reader's local date, from the same calendar authority the queries use.
async fn local_today(pool: &sqlx::SqlitePool) -> CommandResult<String> {
    let (today,): (String,) = sqlx::query_as("SELECT strftime('%Y-%m-%d', 'now', 'localtime')")
        .fetch_one(pool)
        .await
        .map_err(DbError::from)?;
    Ok(today)
}

/// Resolve a pinned image ID under `NsfwMode` (`show` embeds sharp, `blur` embeds pre-blurred, `hide` embeds none).
/// Missing sharp files are reported rather than silently substituted with a blurred image.
async fn resolve_photo(
    cached: &HashMap<String, CachedScreenshot>,
    store: &ScreenshotStore,
    image_id: &str,
    mode: NsfwMode,
) -> JournalPhoto {
    let Some(row) = cached.get(image_id) else {
        return JournalPhoto::Missing(PhotoGap::NotCached);
    };
    let explicit = row.is_explicit();
    if explicit && mode == NsfwMode::Hide {
        return JournalPhoto::Missing(PhotoGap::Concealed);
    }
    let blurred = explicit && mode == NsfwMode::Blur;
    // The cache row's own paths are absolute and were written by the pipeline; ask
    // the store for the current layout so a moved data directory still resolves.
    let path = if blurred {
        store.blur_path(image_id)
    } else {
        store.tile_path(image_id)
    };
    let path = if path.exists() {
        Some(path)
    } else if blurred {
        // A blur-mode row written before the blurred copy existed still has its
        // sharp tile; embedding that would publish what the setting conceals.
        None
    } else {
        row.thumb_path.clone().filter(|path| path.exists())
    };
    let Some(path) = path else {
        return JournalPhoto::Missing(if blurred {
            PhotoGap::Concealed
        } else {
            PhotoGap::NotCached
        });
    };
    match screenshots::read_bytes(&path).await {
        Ok(bytes) => JournalPhoto::Embedded {
            data_url: super::vndb::to_data_url(&bytes, "image/jpeg"),
            concealed: blurred,
        },
        Err(_) => JournalPhoto::Missing(PhotoGap::NotCached),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vn_core::model::NsfwMode;

    /// A temp store with one cached screenshot on disk.
    ///
    /// The bytes are arbitrary: the export only reads the file and base64s it, so
    /// what matters here is *which* file was chosen, which is the whole policy.
    struct Fixture {
        store: ScreenshotStore,
        root: std::path::PathBuf,
    }

    impl Fixture {
        fn new(tag: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "vn-asyik-journal-photo-{tag}-{}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&root);
            let store = ScreenshotStore::new(&root);
            store.ensure().expect("screenshot dir");
            std::fs::write(store.tile_path("sf1"), b"sharp").expect("tile");
            std::fs::write(store.blur_path("sf1"), b"blur").expect("blur");
            Self { store, root }
        }

        fn row(&self, sexual: Option<i64>) -> CachedScreenshot {
            CachedScreenshot {
                vn_id: "v17".to_owned(),
                image_id: "sf1".to_owned(),
                url: "https://t.vndb.org/sf/1/1.jpg".to_owned(),
                thumb_path: Some(self.store.tile_path("sf1")),
                full_path: None,
                blur_path: Some(self.store.blur_path("sf1")),
                blur_full_path: None,
                width: Some(640),
                height: Some(480),
                sexual,
                violence: None,
            }
        }

        fn map(&self, sexual: Option<i64>) -> HashMap<String, CachedScreenshot> {
            HashMap::from([("sf1".to_owned(), self.row(sexual))])
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn embedded(photo: &JournalPhoto) -> (&str, bool) {
        match photo {
            JournalPhoto::Embedded {
                data_url,
                concealed,
            } => (data_url.as_str(), *concealed),
            other @ JournalPhoto::Missing(_) => {
                panic!("expected an embedded photo, got {other:?}")
            }
        }
    }

    #[tokio::test]
    async fn show_embeds_the_sharp_copy() {
        let fixture = Fixture::new("show");
        let photo = resolve_photo(
            &fixture.map(Some(100)),
            &fixture.store,
            "sf1",
            NsfwMode::Show,
        )
        .await;
        let (url, concealed) = embedded(&photo);
        // base64("sharp")
        assert!(url.ends_with("c2hhcnA="), "sharp bytes: {url}");
        assert!(!concealed);
    }

    #[tokio::test]
    async fn blur_embeds_the_pre_blurred_copy_and_says_so() {
        let fixture = Fixture::new("blur");
        let photo = resolve_photo(
            &fixture.map(Some(100)),
            &fixture.store,
            "sf1",
            NsfwMode::Blur,
        )
        .await;
        let (url, concealed) = embedded(&photo);
        // base64("blur")
        assert!(url.ends_with("Ymx1cg=="), "blurred bytes: {url}");
        assert!(concealed, "a soft image must be labelled as such");
    }

    #[tokio::test]
    async fn hide_embeds_nothing() {
        let fixture = Fixture::new("hide");
        let photo = resolve_photo(
            &fixture.map(Some(100)),
            &fixture.store,
            "sf1",
            NsfwMode::Hide,
        )
        .await;
        assert_eq!(photo, JournalPhoto::Missing(PhotoGap::Concealed));
    }

    #[tokio::test]
    async fn an_unrated_image_is_not_concealed_by_the_blur_setting() {
        // The in-app policy treats an unrated row as not explicit (AGENTS.md §6).
        let fixture = Fixture::new("plain");
        let photo =
            resolve_photo(&fixture.map(Some(0)), &fixture.store, "sf1", NsfwMode::Blur).await;
        let (url, concealed) = embedded(&photo);
        assert!(url.ends_with("c2hhcnA="));
        assert!(!concealed);
    }

    #[tokio::test]
    async fn a_missing_blurred_file_never_falls_back_to_the_sharp_one() {
        // The failure this guards: a cache written before the blurred copy existed
        // still has its sharp tile, and embedding that would publish exactly what
        // the setting conceals.
        let fixture = Fixture::new("no-blur");
        std::fs::remove_file(fixture.store.blur_path("sf1")).expect("remove blur");
        let photo = resolve_photo(
            &fixture.map(Some(100)),
            &fixture.store,
            "sf1",
            NsfwMode::Blur,
        )
        .await;
        assert_eq!(photo, JournalPhoto::Missing(PhotoGap::Concealed));
    }

    #[tokio::test]
    async fn an_unknown_or_uncached_image_is_reported_not_invented() {
        let fixture = Fixture::new("unknown");
        let photo = resolve_photo(
            &fixture.map(Some(0)),
            &fixture.store,
            "sf404",
            NsfwMode::Show,
        )
        .await;
        assert_eq!(photo, JournalPhoto::Missing(PhotoGap::NotCached));

        std::fs::remove_file(fixture.store.tile_path("sf1")).expect("remove tile");
        let photo =
            resolve_photo(&fixture.map(Some(0)), &fixture.store, "sf1", NsfwMode::Show).await;
        assert_eq!(photo, JournalPhoto::Missing(PhotoGap::NotCached));
    }
}
