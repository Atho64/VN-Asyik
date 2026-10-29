//! Walkthrough IPC: fetch, scrape, store, translate, and manage visual novel walkthroughs.

use tauri::State;
use vn_core::walkthrough::{
    WalkthroughIndexEntry, WalkthroughRoute, find_best_match, parse_seiya_walkthrough,
};

use super::{CommandError, CommandResult};
use crate::state::AppState;
use crate::walkthrough::{
    USER_AGENT, WalkthroughView, build_seiya_client, clear_checked_items, fetch_master_index,
    fetch_seiya_html, get_walkthrough_view, save_translation, save_walkthrough,
    toggle_checked_item, translate_batch_ai, translate_batch_google,
};

fn error(err: impl std::fmt::Display) -> CommandError {
    CommandError::new("walkthrough", err.to_string())
}

type VndbDetailsRow = (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

/// Retrieve candidate title variants for matching against Seiya-Saiga's index.
async fn collect_candidate_titles(
    pool: &sqlx::SqlitePool,
    game_id: &str,
    vn_id: Option<&str>,
) -> Vec<String> {
    let mut candidates = Vec::new();
    let mut seen = std::collections::HashSet::new();

    let mut add_candidate = |s: String| {
        let trimmed = s.trim().to_string();
        if !trimmed.is_empty() && seen.insert(trimmed.clone()) {
            candidates.push(trimmed);
        }
    };

    let add_with_subtitles = |s: &str, adder: &mut dyn FnMut(String)| {
        adder(s.to_string());
        for delim in ['～', '〜', '-', '－', ':', '：', '　'] {
            if let Some((base, _)) = s.split_once(delim) {
                let b = base.trim();
                if !b.is_empty() {
                    adder(b.to_string());
                }
            }
        }
    };

    // 1. Fetch title, title_original, and vndb_id from `games` table
    let mut game_vn_id = None;
    if let Ok(Some((title, title_orig, db_vn_id))) =
        sqlx::query_as::<_, (String, Option<String>, Option<String>)>(
            "SELECT title, title_original, vndb_id FROM games WHERE id = ?",
        )
        .bind(game_id)
        .fetch_optional(pool)
        .await
    {
        game_vn_id = db_vn_id;

        // PRIORITIZE JAPANESE TITLE: add title_original FIRST so Japanese matching ranks at the top!
        if let Some(orig) = title_orig {
            add_with_subtitles(&orig, &mut add_candidate);
        }
        add_with_subtitles(&title, &mut add_candidate);
    }

    // 2. VNDB titles and aliases
    let target_vn_id = vn_id.map(ToString::to_string).or(game_vn_id);
    if let Some(ref vid) = target_vn_id {
        let row: Option<VndbDetailsRow> = sqlx::query_as(
            "SELECT title, alttitle, titles_json, aliases_json FROM vndb_details WHERE vn_id = ?",
        )
        .bind(vid)
        .fetch_optional(pool)
        .await
        .unwrap_or(None);

        if let Some((v_title, alttitle, titles_json, aliases_json)) = row {
            // Priority to Japanese alttitle
            if let Some(alt) = alttitle {
                add_with_subtitles(&alt, &mut add_candidate);
            }
            if let Some(tj) = titles_json {
                #[derive(serde::Deserialize)]
                struct TV {
                    title: Option<String>,
                }
                if let Ok(variants) = serde_json::from_str::<Vec<TV>>(&tj) {
                    for v in variants {
                        if let Some(vt) = v.title {
                            add_with_subtitles(&vt, &mut add_candidate);
                        }
                    }
                }
            }
            if let Some(aliases) = aliases_json
                .as_deref()
                .and_then(|aj| serde_json::from_str::<Vec<String>>(aj).ok())
            {
                for a in aliases {
                    add_with_subtitles(&a, &mut add_candidate);
                }
            }
            if let Some(t) = v_title {
                add_with_subtitles(&t, &mut add_candidate);
            }
        }
    }

    candidates
}

/// Get the stored walkthrough and translations for a game.
#[tauri::command]
pub async fn walkthrough_get(
    state: State<'_, AppState>,
    game_id: String,
    vn_id: Option<String>,
) -> CommandResult<Option<WalkthroughView>> {
    get_walkthrough_view(state.db().pool(), &game_id, vn_id.as_deref())
        .await
        .map_err(error)
}

/// Search for matching walkthrough candidates from Seiya-Saiga's master index.
#[tauri::command]
pub async fn walkthrough_search_candidate(
    state: State<'_, AppState>,
    game_id: String,
    vn_id: Option<String>,
) -> CommandResult<Vec<WalkthroughIndexEntry>> {
    let client = build_seiya_client().map_err(error)?;
    let index = fetch_master_index(&client).await.map_err(error)?;
    let candidates = collect_candidate_titles(state.db().pool(), &game_id, vn_id.as_deref()).await;

    let matches = vn_core::walkthrough::find_candidate_matches(&index, &candidates, 8);
    Ok(matches)
}

/// Free-text search across Seiya-Saiga's entire master walkthrough catalog.
#[tauri::command]
pub async fn walkthrough_search(query: String) -> CommandResult<Vec<WalkthroughIndexEntry>> {
    let client = build_seiya_client().map_err(error)?;
    let index = fetch_master_index(&client).await.map_err(error)?;
    let results = vn_core::walkthrough::search_index(&index, &query, 16);
    Ok(results)
}

/// Fetch and parse a walkthrough from a specific URL or automatically via the master index.
#[tauri::command]
pub async fn walkthrough_fetch(
    state: State<'_, AppState>,
    game_id: String,
    vn_id: Option<String>,
    url: Option<String>,
) -> CommandResult<WalkthroughView> {
    let client = build_seiya_client().map_err(error)?;

    // Determine target URL
    let target_url = if let Some(custom_url) = url {
        let trimmed = custom_url.trim().to_string();
        if trimmed.is_empty() {
            return Err(error("URL cannot be empty"));
        }
        trimmed
    } else {
        let index = fetch_master_index(&client).await.map_err(error)?;
        let candidates =
            collect_candidate_titles(state.db().pool(), &game_id, vn_id.as_deref()).await;
        find_best_match(&index, &candidates)
            .map(|e| e.url.clone())
            .or_else(|| {
                vn_core::walkthrough::find_candidate_matches(&index, &candidates, 1)
                    .into_iter()
                    .next()
                    .map(|e| e.url)
            })
            .ok_or_else(|| error("Could not find a matching walkthrough on Seiya-Saiga. Please search or provide the URL manually."))?
    };

    let html = fetch_seiya_html(&client, &target_url)
        .await
        .map_err(error)?;
    let parsed = parse_seiya_walkthrough(&html, &target_url).map_err(|e| error(e.to_string()))?;

    save_walkthrough(state.db().pool(), &game_id, &parsed)
        .await
        .map_err(error)?;

    let view = get_walkthrough_view(state.db().pool(), &game_id, vn_id.as_deref())
        .await
        .map_err(error)?
        .ok_or_else(|| error("Failed to load saved walkthrough"))?;

    Ok(view)
}

/// Translate the walkthrough into English or Indonesian using AI (with VNDB glossary) or Google Translate.
#[tauri::command]
pub async fn walkthrough_translate(
    state: State<'_, AppState>,
    game_id: String,
    vn_id: Option<String>,
    provider: String,
    target_lang: String,
) -> CommandResult<WalkthroughView> {
    let existing = get_walkthrough_view(state.db().pool(), &game_id, vn_id.as_deref())
        .await
        .map_err(error)?
        .ok_or_else(|| error("No walkthrough to translate. Please fetch one first."))?;

    let glossary =
        crate::walkthrough::load_character_glossary(state.db().pool(), vn_id.as_deref()).await;

    // Collect all strings that need translation:
    // Route titles + item texts
    let mut texts_to_translate = Vec::new();
    for route in &existing.routes {
        texts_to_translate.push(route.title.clone());
        for item in &route.items {
            texts_to_translate.push(item.text.clone());
        }
    }

    let http = crate::vndb::http_client_builder()
        .user_agent(USER_AGENT)
        .build()
        .map_err(|e| error(e.to_string()))?;

    let translated_strings = if provider == "google" {
        translate_batch_google(&http, &texts_to_translate, &target_lang, &glossary)
            .await
            .map_err(error)?
    } else {
        // AI translation, through the active saved profile.
        let chat = crate::ai::profiles::resolve(state.db().pool()).await;
        translate_batch_ai(
            &http,
            &chat.config,
            chat.key.as_ref(),
            &texts_to_translate,
            &target_lang,
            &glossary,
        )
        .await
        .map_err(error)?
    };

    // Reconstruct translated routes
    let mut str_ptr = 0;
    let mut mapped_routes = Vec::new();
    for route in &existing.routes {
        let r_title = if str_ptr < translated_strings.len() {
            translated_strings[str_ptr].clone()
        } else {
            route.title.clone()
        };
        str_ptr += 1;

        let mut r_items = Vec::new();
        for item in &route.items {
            let i_text = if str_ptr < translated_strings.len() {
                translated_strings[str_ptr].clone()
            } else {
                item.text.clone()
            };
            str_ptr += 1;

            let mut m_item = item.clone();
            m_item.text = i_text;
            r_items.push(m_item);
        }

        mapped_routes.push(WalkthroughRoute {
            id: route.id.clone(),
            title: r_title,
            items: r_items,
        });
    }

    save_translation(
        state.db().pool(),
        &game_id,
        &provider,
        &target_lang,
        &mapped_routes,
    )
    .await
    .map_err(error)?;

    let updated_view = get_walkthrough_view(state.db().pool(), &game_id, vn_id.as_deref())
        .await
        .map_err(error)?
        .ok_or_else(|| error("Failed to load translated walkthrough"))?;

    Ok(updated_view)
}

/// Toggle checked status of an item in a walkthrough.
#[tauri::command]
pub async fn walkthrough_toggle_item(
    state: State<'_, AppState>,
    game_id: String,
    item_id: String,
) -> CommandResult<Vec<String>> {
    toggle_checked_item(state.db().pool(), &game_id, &item_id)
        .await
        .map_err(error)
}

/// Reset/clear all checked progress in a walkthrough.
#[tauri::command]
pub async fn walkthrough_clear_progress(
    state: State<'_, AppState>,
    game_id: String,
) -> CommandResult<()> {
    clear_checked_items(state.db().pool(), &game_id)
        .await
        .map_err(error)
}
