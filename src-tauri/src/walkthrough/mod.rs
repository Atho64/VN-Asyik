//! Walkthrough service: fetches, parses, stores, and translates visual novel walkthroughs.

use std::collections::HashMap;
use std::fmt::Write;
use std::net::SocketAddr;
use std::sync::LazyLock;
use std::time::Duration;
use tokio::sync::RwLock;

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use vn_core::walkthrough::{
    Walkthrough, WalkthroughIndexEntry, WalkthroughRoute, decode_shift_jis, parse_index_links,
};

use crate::ai::chat::{ChatConfig, complete};
use crate::secrets::ChatApiKey;

const SEIYA_IP_PORT_443: &str = "157.7.107.46:443";
const SEIYA_PC_KOURYAKU_URL: &str = "https://seiya-saiga.com/game/kouryaku.html";
const SEIYA_GALGE_URL: &str = "https://galge.seiya-saiga.com/game/galge.html";
const SEIYA_MAIN_URL: &str = "https://seiya-saiga.com/";
const SEIYA_BEKKAN_KOURYAKU_URL: &str = "https://galge.seiya-saiga.com/game/kouryaku.html";
pub const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

type CachedIndex = (std::time::Instant, Vec<WalkthroughIndexEntry>);
static INDEX_CACHE: LazyLock<RwLock<Option<CachedIndex>>> = LazyLock::new(|| RwLock::new(None));

#[derive(Deserialize)]
struct CharItem {
    name: Option<String>,
    original: Option<String>,
}

/// Errors that can occur during walkthrough operations.
#[derive(Debug, thiserror::Error)]
pub enum WalkthroughError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("Database error: {0}")]
    Db(#[from] sqlx::Error),
    #[error("Parse error: {0}")]
    Parse(String),
    #[error("Translation error: {0}")]
    Translation(String),
    #[error("No walkthrough found for this game")]
    NotFound,
}

/// Consolidated view of a game's walkthrough for the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WalkthroughView {
    pub game_id: String,
    pub title: String,
    pub source_url: String,
    pub notes: Option<String>,
    pub routes: Vec<WalkthroughRoute>,
    pub checked_items: Vec<String>,
    /// Available translations: map of "`provider_lang`" => `Vec<WalkthroughRoute>`
    pub translations: HashMap<String, Vec<WalkthroughRoute>>,
    pub character_glossary: HashMap<String, String>,
}

/// Create an HTTP client configured to connect to Seiya-Saiga.
///
/// Direct IP resolution for `seiya-saiga.com` and `galge.seiya-saiga.com` defeats
/// ISP DNS tampering (such as Indonesian ISP Internet Positif redirection).
pub fn build_seiya_client() -> Result<reqwest::Client, WalkthroughError> {
    let mut builder = crate::vndb::http_client_builder()
        .user_agent(USER_AGENT)
        .danger_accept_invalid_certs(true)
        .timeout(Duration::from_secs(15));

    if let Ok(addr_443) = SEIYA_IP_PORT_443.parse::<SocketAddr>() {
        builder = builder
            .resolve("seiya-saiga.com", addr_443)
            .resolve("galge.seiya-saiga.com", addr_443);
    }

    builder.build().map_err(WalkthroughError::Http)
}

/// Fetch raw HTML from a Seiya-Saiga page, handling `Shift_JIS` decoding.
pub async fn fetch_seiya_html(
    client: &reqwest::Client,
    url: &str,
) -> Result<String, WalkthroughError> {
    let normalized = if url.starts_with("http://") {
        url.replacen("http://", "https://", 1)
    } else {
        url.to_string()
    };
    let resp = client.get(&normalized).send().await?.error_for_status()?;
    let bytes = resp.bytes().await?;
    Ok(decode_shift_jis(&bytes))
}

/// Fetch and parse the combined Seiya-Saiga master index across main, galge, and all-ages PC catalogs.
pub async fn fetch_master_index(
    client: &reqwest::Client,
) -> Result<Vec<WalkthroughIndexEntry>, WalkthroughError> {
    {
        let cache_read = INDEX_CACHE.read().await;
        if let Some((_, entries)) = cache_read
            .as_ref()
            .filter(|(t, e)| t.elapsed() < Duration::from_hours(6) && !e.is_empty())
        {
            return Ok(entries.clone());
        }
    }

    // Fetch all 4 index sources concurrently
    let (res_pc, res_galge, res_main, res_bekkan) = tokio::join!(
        fetch_seiya_html(client, SEIYA_PC_KOURYAKU_URL),
        fetch_seiya_html(client, SEIYA_GALGE_URL),
        fetch_seiya_html(client, SEIYA_MAIN_URL),
        fetch_seiya_html(client, SEIYA_BEKKAN_KOURYAKU_URL),
    );

    let mut combined = Vec::new();
    let mut seen_urls = std::collections::HashSet::new();

    if let Ok(html) = res_pc {
        for entry in parse_index_links(&html, "https://seiya-saiga.com/game/") {
            if seen_urls.insert(entry.url.clone()) {
                combined.push(entry);
            }
        }
    }
    if let Ok(html) = res_galge {
        for entry in parse_index_links(&html, "https://galge.seiya-saiga.com/game/") {
            if seen_urls.insert(entry.url.clone()) {
                combined.push(entry);
            }
        }
    }
    if let Ok(html) = res_main {
        for entry in parse_index_links(&html, "https://seiya-saiga.com/") {
            if seen_urls.insert(entry.url.clone()) {
                combined.push(entry);
            }
        }
    }
    if let Ok(html) = res_bekkan {
        for entry in parse_index_links(&html, "https://galge.seiya-saiga.com/game/") {
            if seen_urls.insert(entry.url.clone()) {
                combined.push(entry);
            }
        }
    }

    if combined.is_empty() {
        return Err(WalkthroughError::NotFound);
    }

    {
        let mut cache_write = INDEX_CACHE.write().await;
        *cache_write = Some((std::time::Instant::now(), combined.clone()));
    }

    Ok(combined)
}

/// Backwards-compatible alias for [`fetch_master_index`].
pub async fn fetch_kouryaku_index(
    client: &reqwest::Client,
) -> Result<Vec<WalkthroughIndexEntry>, WalkthroughError> {
    fetch_master_index(client).await
}

/// Retrieve the character glossary (original Kanji -> English/Romaji name) from VNDB details.
pub async fn load_character_glossary(
    pool: &SqlitePool,
    vn_id: Option<&str>,
) -> HashMap<String, String> {
    let mut glossary = HashMap::new();
    let Some(vn_id) = vn_id else {
        return glossary;
    };

    let row: Option<(String,)> =
        sqlx::query_as("SELECT characters_json FROM vndb_details WHERE vn_id = ?")
            .bind(vn_id)
            .fetch_optional(pool)
            .await
            .unwrap_or(None);

    let Some((json_str,)) = row else {
        return glossary;
    };

    if let Ok(chars) = serde_json::from_str::<Vec<CharItem>>(&json_str) {
        for c in chars {
            if let (Some(name), Some(orig)) = (c.name, c.original) {
                let clean_name = name.trim().to_string();
                let clean_orig = orig.trim().to_string();
                if !clean_name.is_empty() && !clean_orig.is_empty() {
                    glossary.insert(clean_orig.clone(), clean_name.clone());
                    // Also support single given name if space separated (e.g. "神戸 小鳥" -> "小鳥", "Kotori")
                    let orig_last = clean_orig.split([' ', '　']).next_back();
                    let name_first = clean_name.split(' ').next();
                    if let (Some(l), Some(f)) = (orig_last, name_first)
                        && l.len() >= 2
                        && !glossary.contains_key(l)
                    {
                        glossary.insert(l.to_string(), f.to_string());
                    }
                }
            }
        }
    }

    glossary
}

/// Load the complete walkthrough view from the database.
pub async fn get_walkthrough_view(
    pool: &SqlitePool,
    game_id: &str,
    vn_id: Option<&str>,
) -> Result<Option<WalkthroughView>, WalkthroughError> {
    let row: Option<(String, String, Option<String>, String, String)> = sqlx::query_as(
        "SELECT title, source_url, notes, routes_json, checked_items_json \
         FROM game_walkthroughs WHERE game_id = ?",
    )
    .bind(game_id)
    .fetch_optional(pool)
    .await?;

    let Some((title, source_url, notes, routes_json, checked_items_json)) = row else {
        return Ok(None);
    };

    let routes: Vec<WalkthroughRoute> =
        serde_json::from_str(&routes_json).map_err(|e| WalkthroughError::Parse(e.to_string()))?;
    let checked_items: Vec<String> = serde_json::from_str(&checked_items_json).unwrap_or_default();

    // Load translations
    let trans_rows: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT provider, target_lang, routes_json \
         FROM walkthrough_translations WHERE game_id = ?",
    )
    .bind(game_id)
    .fetch_all(pool)
    .await?;

    let mut translations = HashMap::new();
    for (provider, target_lang, t_json) in trans_rows {
        if let Ok(t_routes) = serde_json::from_str::<Vec<WalkthroughRoute>>(&t_json) {
            let key = format!("{provider}_{target_lang}");
            translations.insert(key, t_routes);
        }
    }

    let glossary = load_character_glossary(pool, vn_id).await;

    Ok(Some(WalkthroughView {
        game_id: game_id.to_string(),
        title,
        source_url,
        notes,
        routes,
        checked_items,
        translations,
        character_glossary: glossary,
    }))
}

/// Save a newly parsed walkthrough into the database.
pub async fn save_walkthrough(
    pool: &SqlitePool,
    game_id: &str,
    walkthrough: &Walkthrough,
) -> Result<(), WalkthroughError> {
    let routes_json = serde_json::to_string(&walkthrough.routes)
        .map_err(|e| WalkthroughError::Parse(e.to_string()))?;
    let now =
        vn_core::clock::Timestamp::now().map_err(|e| WalkthroughError::Parse(e.to_string()))?;
    let now_ms = now.as_millis();

    sqlx::query(
        "INSERT INTO game_walkthroughs (game_id, source_url, title, notes, routes_json, checked_items_json, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, '[]', ?, ?) \
         ON CONFLICT(game_id) DO UPDATE SET \
            source_url = excluded.source_url, \
            title = excluded.title, \
            notes = excluded.notes, \
            routes_json = excluded.routes_json, \
            updated_at = excluded.updated_at"
    )
    .bind(game_id)
    .bind(&walkthrough.source_url)
    .bind(&walkthrough.title)
    .bind(&walkthrough.notes)
    .bind(&routes_json)
    .bind(now_ms)
    .bind(now_ms)
    .execute(pool)
    .await?;

    Ok(())
}

/// Save a translation of a walkthrough into the database.
pub async fn save_translation(
    pool: &SqlitePool,
    game_id: &str,
    provider: &str,
    target_lang: &str,
    routes: &[WalkthroughRoute],
) -> Result<(), WalkthroughError> {
    let routes_json =
        serde_json::to_string(routes).map_err(|e| WalkthroughError::Parse(e.to_string()))?;
    let now =
        vn_core::clock::Timestamp::now().map_err(|e| WalkthroughError::Parse(e.to_string()))?;
    let now_ms = now.as_millis();

    sqlx::query(
        "INSERT INTO walkthrough_translations (game_id, provider, target_lang, routes_json, updated_at) \
         VALUES (?, ?, ?, ?, ?) \
         ON CONFLICT(game_id, provider, target_lang) DO UPDATE SET \
            routes_json = excluded.routes_json, \
            updated_at = excluded.updated_at"
    )
    .bind(game_id)
    .bind(provider)
    .bind(target_lang)
    .bind(&routes_json)
    .bind(now_ms)
    .execute(pool)
    .await?;

    Ok(())
}

/// Toggle a checked item for a game walkthrough.
pub async fn toggle_checked_item(
    pool: &SqlitePool,
    game_id: &str,
    item_id: &str,
) -> Result<Vec<String>, WalkthroughError> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT checked_items_json FROM game_walkthroughs WHERE game_id = ?")
            .bind(game_id)
            .fetch_optional(pool)
            .await?;

    let Some((json_str,)) = row else {
        return Err(WalkthroughError::NotFound);
    };

    let mut items: Vec<String> = serde_json::from_str(&json_str).unwrap_or_default();
    if let Some(pos) = items.iter().position(|x| x == item_id) {
        items.remove(pos);
    } else {
        items.push(item_id.to_string());
    }

    let next_json =
        serde_json::to_string(&items).map_err(|e| WalkthroughError::Parse(e.to_string()))?;
    sqlx::query("UPDATE game_walkthroughs SET checked_items_json = ? WHERE game_id = ?")
        .bind(&next_json)
        .bind(game_id)
        .execute(pool)
        .await?;

    Ok(items)
}

/// Clear all checked progress for a game walkthrough.
pub async fn clear_checked_items(pool: &SqlitePool, game_id: &str) -> Result<(), WalkthroughError> {
    sqlx::query("UPDATE game_walkthroughs SET checked_items_json = '[]' WHERE game_id = ?")
        .bind(game_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Translate text lines in batch using Google Translate's free API.
#[allow(clippy::implicit_hasher)]
pub async fn translate_batch_google(
    client: &reqwest::Client,
    texts: &[String],
    target_lang: &str,
    glossary: &HashMap<String, String>,
) -> Result<Vec<String>, WalkthroughError> {
    if texts.is_empty() {
        return Ok(Vec::new());
    }

    // Pre-substitute glossary terms in the Japanese text where possible
    let mut prepared = Vec::with_capacity(texts.len());
    for t in texts {
        let mut s = t.clone();
        for (jp, en) in glossary {
            if s.contains(jp) {
                s = s.replace(jp, en);
            }
        }
        prepared.push(s);
    }

    // Google Translate supports batching by joining with newlines
    let joined = prepared.join("\n");
    let url = format!(
        "https://translate.googleapis.com/translate_a/single?client=gtx&sl=ja&tl={}&dt=t",
        urlencoding(target_lang)
    );

    let resp = client
        .post(&url)
        .form(&[("q", &joined)])
        .send()
        .await
        .map_err(WalkthroughError::Http)?
        .error_for_status()
        .map_err(WalkthroughError::Http)?;

    let val: serde_json::Value = resp.json().await.map_err(WalkthroughError::Http)?;

    // Google Translate response format: [[["translated_segment", "src_segment", ...], ...], ...]
    let mut translated_full = String::new();
    if let Some(sentences) = val.get(0).and_then(|v| v.as_array()) {
        for item in sentences {
            if let Some(seg) = item.get(0).and_then(|v| v.as_str()) {
                translated_full.push_str(seg);
            }
        }
    }

    let mut result_lines: Vec<String> = translated_full
        .lines()
        .map(|s| s.trim().to_string())
        .collect();

    // Align length with original texts
    if result_lines.len() < texts.len() {
        while result_lines.len() < texts.len() {
            result_lines.push(texts[result_lines.len()].clone());
        }
    } else if result_lines.len() > texts.len() {
        result_lines.truncate(texts.len());
    }

    Ok(result_lines)
}

/// Translate text lines in batch using the configured AI provider, with VNDB character glossary.
#[allow(clippy::implicit_hasher)]
pub async fn translate_batch_ai(
    client: &reqwest::Client,
    config: &ChatConfig,
    key: Option<&ChatApiKey>,
    texts: &[String],
    target_lang: &str,
    glossary: &HashMap<String, String>,
) -> Result<Vec<String>, WalkthroughError> {
    if texts.is_empty() {
        return Ok(Vec::new());
    }

    let lang_name = match target_lang {
        "id" => "Indonesian",
        _ => "English",
    };

    let mut glossary_block = String::new();
    if !glossary.is_empty() {
        glossary_block.push_str("\nCharacter Name Glossary from VNDB (Adhere strictly):\n");
        for (jp, en) in glossary {
            let _ = writeln!(glossary_block, "- {jp} => {en}");
        }
    }

    let system_prompt = format!(
        "You are an expert visual novel translator. Translate the given walkthrough steps and choices from Japanese to {lang_name}.\n\
         Rules:\n\
         1. Keep choices clear, concise, and easy to read for a visual novel player.\n\
         2. Preserve dates (e.g. 10/21, October 21) and save point markers (e.g. Save 01, From Save 04).\n\
         3. Keep endings clean (e.g. Kotori END, Bad End).\n\
         {glossary_block}\n\
         IMPORTANT: The input is a JSON array of strings. You MUST reply with ONLY a valid JSON array of translated strings in the EXACT same order and length. Do not include markdown codeblocks or explanation."
    );

    let user_prompt = serde_json::to_string(texts).unwrap_or_default();

    let raw_reply = complete(client, config, key, &system_prompt, &user_prompt)
        .await
        .map_err(|e| WalkthroughError::Translation(e.to_string()))?;

    // Clean up potential markdown formatting (```json ... ```)
    let cleaned = raw_reply.trim();
    let json_str = if cleaned.starts_with("```") {
        let lines: Vec<&str> = cleaned.lines().collect();
        if lines.len() >= 2
            && lines.first().unwrap().starts_with("```")
            && lines.last().unwrap().starts_with("```")
        {
            lines[1..lines.len() - 1].join("\n")
        } else {
            cleaned.to_string()
        }
    } else {
        cleaned.to_string()
    };

    if let Some(translated_array) = serde_json::from_str::<Vec<String>>(&json_str)
        .ok()
        .filter(|arr| arr.len() == texts.len())
    {
        return Ok(translated_array);
    }

    // Fallback: line-by-line parsing if json array was malformed
    let mut lines = Vec::new();
    for line in json_str.lines() {
        let t = line
            .trim()
            .trim_start_matches(|c| {
                c == '-' || c == '*' || c == '"' || c == '[' || c == ']' || c == ','
            })
            .trim_end_matches([',', '"', ']'])
            .trim();
        if !t.is_empty() {
            lines.push(t.to_string());
        }
    }

    if lines.len() == texts.len() {
        return Ok(lines);
    }

    // Final fallback: return original texts if AI returned wrong format
    Ok(texts.to_vec())
}

/// Helper to url-encode a string slice.
fn urlencoding(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}
