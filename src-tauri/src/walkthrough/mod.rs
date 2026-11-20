use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::LazyLock;
use std::time::Duration;
use tokio::sync::RwLock;

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use vn_core::walkthrough::translation::{
    apply_translated_texts, character_glossary, replace_names,
};
use vn_core::walkthrough::{
    Walkthrough, WalkthroughIndexEntry, WalkthroughRoute, decode_html_bytes, parse_index_links,
};

const SEIYA_IP_PORT_443: &str = "157.7.107.46:443";
const SEIYA_PC_KOURYAKU_URL: &str = "https://seiya-saiga.com/game/kouryaku.html";
const SEIYA_GALGE_URL: &str = "https://galge.seiya-saiga.com/game/galge.html";
const SEIYA_MAIN_URL: &str = "https://seiya-saiga.com/";
const SEIYA_BEKKAN_KOURYAKU_URL: &str = "https://galge.seiya-saiga.com/game/kouryaku.html";
/// Identifies the app honestly; a faked browser user agent would misrepresent
/// who is fetching and could get the site's bots policy applied against it.
pub const USER_AGENT: &str = concat!(
    "VN-Tracker/",
    env!("CARGO_PKG_VERSION"),
    " (walkthrough fetcher; +https://seiya-saiga.com/)"
);

type CachedIndex = (std::time::Instant, Vec<WalkthroughIndexEntry>);
static INDEX_CACHE: LazyLock<RwLock<Option<CachedIndex>>> = LazyLock::new(|| RwLock::new(None));

#[derive(Deserialize)]
struct CharItem {
    name: Option<String>,
    original: Option<String>,
}

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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WalkthroughView {
    pub game_id: String,
    pub title: String,
    pub source_url: String,
    pub notes: Option<String>,
    pub routes: Vec<WalkthroughRoute>,
    pub checked_items: Vec<String>,
    /// Keyed by "`provider_lang`".
    pub translations: HashMap<String, Vec<WalkthroughRoute>>,
    pub character_glossary: HashMap<String, String>,
}

/// Create an HTTP client configured to connect to Seiya-Saiga.
///
/// Direct IP resolution for `seiya-saiga.com` and `galge.seiya-saiga.com` defeats
/// ISP DNS tampering (such as Indonesian ISP Internet Positif redirection). URLs
/// keep the real hostname, so certificate validation still checks the name the
/// server presents and must never be bypassed: the pin only replaces DNS.
pub fn build_seiya_client() -> Result<reqwest::Client, WalkthroughError> {
    let mut builder = crate::vndb::http_client_builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(15));

    if let Ok(addr_443) = SEIYA_IP_PORT_443.parse::<SocketAddr>() {
        builder = builder
            .resolve("seiya-saiga.com", addr_443)
            .resolve("galge.seiya-saiga.com", addr_443);
    }

    builder.build().map_err(WalkthroughError::Http)
}

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
    Ok(decode_html_bytes(&bytes))
}

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

pub async fn fetch_kouryaku_index(
    client: &reqwest::Client,
) -> Result<Vec<WalkthroughIndexEntry>, WalkthroughError> {
    fetch_master_index(client).await
}

pub async fn load_character_glossary(
    pool: &SqlitePool,
    vn_id: Option<&str>,
) -> HashMap<String, String> {
    let Some(vn_id) = vn_id else {
        return HashMap::new();
    };

    let row: Option<(String,)> =
        sqlx::query_as("SELECT characters_json FROM vndb_details WHERE vn_id = ?")
            .bind(vn_id)
            .fetch_optional(pool)
            .await
            .unwrap_or(None);

    let Some((json_str,)) = row else {
        return HashMap::new();
    };

    let chars = serde_json::from_str::<Vec<CharItem>>(&json_str).unwrap_or_default();
    character_glossary(
        chars
            .into_iter()
            .filter_map(|c| Some((c.original?, c.name?))),
    )
}

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

    let mut transaction = pool.begin().await?;
    sqlx::query(
        "DELETE FROM walkthrough_translations WHERE game_id = ? AND EXISTS \
         (SELECT 1 FROM game_walkthroughs WHERE game_id = ? AND (source_url != ? OR routes_json != ?))"
    )
    .bind(game_id).bind(game_id).bind(&walkthrough.source_url).bind(&routes_json)
    .execute(&mut *transaction).await?;
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
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;

    Ok(())
}

pub async fn save_translation(
    pool: &SqlitePool,
    original: &WalkthroughView,
    provider: &str,
    target_lang: &str,
    routes: &[WalkthroughRoute],
) -> Result<(), WalkthroughError> {
    let routes_json =
        serde_json::to_string(routes).map_err(|e| WalkthroughError::Parse(e.to_string()))?;
    let now =
        vn_core::clock::Timestamp::now().map_err(|e| WalkthroughError::Parse(e.to_string()))?;
    let now_ms = now.as_millis();

    let original_json = serde_json::to_string(&original.routes)
        .map_err(|e| WalkthroughError::Parse(e.to_string()))?;
    let result = sqlx::query(
        "INSERT INTO walkthrough_translations (game_id, provider, target_lang, routes_json, updated_at) \
         SELECT game_id, ?, ?, ?, ? FROM game_walkthroughs \
         WHERE game_id = ? AND source_url = ? AND routes_json = ? \
         ON CONFLICT(game_id, provider, target_lang) DO UPDATE SET \
            routes_json = excluded.routes_json, \
            updated_at = excluded.updated_at"
    )
    .bind(provider)
    .bind(target_lang)
    .bind(&routes_json)
    .bind(now_ms)
    .bind(&original.game_id)
    .bind(&original.source_url)
    .bind(original_json)
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(WalkthroughError::Translation(
            "The source guide changed during translation; refresh its translation again".into(),
        ));
    }

    Ok(())
}

pub async fn save_manual_translation(
    pool: &SqlitePool,
    game_id: &str,
    target_lang: &str,
    source_url: &str,
    source_routes: &[WalkthroughRoute],
    texts: &[String],
) -> Result<(), WalkthroughError> {
    if !matches!(target_lang, "en" | "id") {
        return Err(WalkthroughError::Translation(
            "Unsupported translation language".into(),
        ));
    }
    let original = get_walkthrough_view(pool, game_id, None)
        .await?
        .ok_or(WalkthroughError::NotFound)?;
    if original.source_url != source_url || original.routes != source_routes {
        return Err(WalkthroughError::Translation(
            "The source guide changed; reopen the translation editor".into(),
        ));
    }
    let routes = apply_translated_texts(&original.routes, texts)
        .map_err(|message| WalkthroughError::Translation(message.into()))?;
    save_translation(pool, &original, "manual", target_lang, &routes).await
}

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

pub async fn clear_checked_items(pool: &SqlitePool, game_id: &str) -> Result<(), WalkthroughError> {
    sqlx::query("UPDATE game_walkthroughs SET checked_items_json = '[]' WHERE game_id = ?")
        .bind(game_id)
        .execute(pool)
        .await?;
    Ok(())
}

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

    let prepared: Vec<_> = texts
        .iter()
        .map(|text| replace_names(text, glossary))
        .collect();

    // Google Translate batches newline-joined lines in a single request.
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

    parse_google_translation(&val, texts.len())
}

fn parse_google_translation(
    val: &serde_json::Value,
    expected: usize,
) -> Result<Vec<String>, WalkthroughError> {
    // Google Translate response format: [[["translated_segment", "src_segment", ...], ...], ...]
    let mut translated_full = String::new();
    if let Some(sentences) = val.get(0).and_then(|v| v.as_array()) {
        for item in sentences {
            if let Some(seg) = item.get(0).and_then(|v| v.as_str()) {
                translated_full.push_str(seg);
            }
        }
    }

    let result_lines: Vec<String> = translated_full
        .lines()
        .map(|s| s.trim().to_string())
        .collect();

    if result_lines.len() != expected || result_lines.iter().any(String::is_empty) {
        return Err(WalkthroughError::Translation(
            "Google returned incomplete or misaligned steps; no translation was saved".into(),
        ));
    }

    Ok(result_lines)
}

fn urlencoding(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{Db, queries};
    use vn_core::walkthrough::{ItemKind, WalkthroughItem};
    use vn_core::{
        clock::Timestamp,
        model::{Game, GameStatus},
    };

    #[test]
    fn google_segments_preserve_order_and_reject_incomplete_or_extra_lines() {
        let valid = serde_json::json!([[
            ["Kotori route\n", "小鳥ルート"],
            ["From Save 01", "セーブ01から"]
        ]]);
        assert_eq!(
            parse_google_translation(&valid, 2).unwrap(),
            ["Kotori route", "From Save 01"]
        );
        for malformed in [
            serde_json::json!([]),
            serde_json::json!([[["Only one line"]]]),
            serde_json::json!([[["One\nTwo\nThree"]]]),
            serde_json::json!([[["One\n\n"]]]),
        ] {
            assert!(parse_google_translation(&malformed, 2).is_err());
        }
    }

    async fn fixture() -> (Db, String, WalkthroughView) {
        let db = Db::open_memory().await.unwrap();
        let device = queries::ensure_device_id(db.pool()).await.unwrap();
        let game = Game::new(
            &device,
            Timestamp::from_millis(1_700_000_000_000),
            "Rewrite+",
            GameStatus::Playing,
        )
        .unwrap();
        queries::upsert_game(db.pool(), &game).await.unwrap();
        let id = game.meta.id.to_string();
        let source = Walkthrough {
            title: "Rewrite+".into(),
            source_url: "https://seiya-saiga.com/game/rewrite.html".into(),
            notes: None,
            routes: vec![WalkthroughRoute {
                id: "route_0".into(),
                title: "小鳥".into(),
                items: vec![WalkthroughItem {
                    id: "c_1".into(),
                    kind: ItemKind::Choice,
                    text: "小鳥を選ぶ".into(),
                    note: None,
                }],
            }],
        };
        save_walkthrough(db.pool(), &id, &source).await.unwrap();
        let view = get_walkthrough_view(db.pool(), &id, None)
            .await
            .unwrap()
            .unwrap();
        (db, id, view)
    }

    #[tokio::test]
    async fn manual_edits_are_local_separate_and_preserve_checked_choices() {
        let (db, id, original) = fixture().await;
        save_translation(db.pool(), &original, "google", "en", &original.routes)
            .await
            .unwrap();
        toggle_checked_item(db.pool(), &id, "c_1").await.unwrap();
        save_manual_translation(
            db.pool(),
            &id,
            "en",
            &original.source_url,
            &original.routes,
            &["Kotori route".into(), "Choose Kotori manually".into()],
        )
        .await
        .unwrap();
        let loaded = get_walkthrough_view(db.pool(), &id, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(loaded.routes, original.routes);
        assert_eq!(loaded.checked_items, ["c_1"]);
        assert_eq!(loaded.translations["google_en"], original.routes);
        assert_eq!(
            loaded.translations["manual_en"][0].items[0].text,
            "Choose Kotori manually"
        );
        assert_eq!(loaded.translations["manual_en"][0].items[0].id, "c_1");

        for (source, texts) in [
            (
                "https://seiya-saiga.com/other.html",
                vec!["Kotori".into(), "Wrong guide".into()],
            ),
            (original.source_url.as_str(), vec!["Kotori".into()]),
        ] {
            assert!(
                save_manual_translation(db.pool(), &id, "en", source, &original.routes, &texts)
                    .await
                    .is_err()
            );
        }
        let mut stale_routes = original.routes.clone();
        stale_routes[0].items[0].text = "old choice".into();
        assert!(
            save_manual_translation(
                db.pool(),
                &id,
                "en",
                &original.source_url,
                &stale_routes,
                &["Kotori".into(), "Old choice".into()]
            )
            .await
            .is_err()
        );
        let loaded = get_walkthrough_view(db.pool(), &id, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            loaded.translations["manual_en"][0].items[0].text,
            "Choose Kotori manually"
        );
    }

    #[tokio::test]
    async fn refreshing_translation_replaces_only_its_cache_and_preserves_original_and_progress() {
        let (db, id, original) = fixture().await;
        save_translation(db.pool(), &original, "manual", "en", &original.routes)
            .await
            .unwrap();
        save_translation(db.pool(), &original, "google", "id", &original.routes)
            .await
            .unwrap();
        toggle_checked_item(db.pool(), &id, "c_1").await.unwrap();
        let mut translated = original.routes.clone();
        translated[0].title = "Kotori route".into();
        save_translation(db.pool(), &original, "google", "en", &translated)
            .await
            .unwrap();
        translated[0].title = "Refreshed Kotori route".into();
        save_translation(db.pool(), &original, "google", "en", &translated)
            .await
            .unwrap();
        let loaded = get_walkthrough_view(db.pool(), &id, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(loaded.routes, original.routes);
        assert_eq!(loaded.checked_items, ["c_1"]);
        assert_eq!(
            loaded.translations["google_en"][0].title,
            "Refreshed Kotori route"
        );
        assert_eq!(loaded.translations["google_en"][0].items[0].id, "c_1");
        assert_eq!(loaded.translations["manual_en"], original.routes);
        assert_eq!(loaded.translations["google_id"], original.routes);
    }

    #[tokio::test]
    async fn changed_source_invalidates_cache_and_rejects_a_late_translation() {
        let (db, id, original) = fixture().await;
        save_translation(db.pool(), &original, "google", "en", &original.routes)
            .await
            .unwrap();
        let replacement = Walkthrough {
            title: "Other guide".into(),
            source_url: "https://seiya-saiga.com/game/other.html".into(),
            notes: None,
            routes: original.routes.clone(),
        };
        save_walkthrough(db.pool(), &id, &replacement)
            .await
            .unwrap();
        assert!(
            save_translation(db.pool(), &original, "google", "en", &original.routes)
                .await
                .is_err()
        );
        let loaded = get_walkthrough_view(db.pool(), &id, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(loaded.source_url, replacement.source_url);
        assert!(loaded.translations.is_empty());
    }

    #[tokio::test]
    async fn unchanged_source_keeps_translation_but_changed_steps_reject_stale_results() {
        let (db, id, original) = fixture().await;
        save_translation(db.pool(), &original, "google", "en", &original.routes)
            .await
            .unwrap();
        let mut source = Walkthrough {
            title: original.title.clone(),
            source_url: original.source_url.clone(),
            notes: None,
            routes: original.routes.clone(),
        };
        save_walkthrough(db.pool(), &id, &source).await.unwrap();
        assert!(
            !get_walkthrough_view(db.pool(), &id, None)
                .await
                .unwrap()
                .unwrap()
                .translations
                .is_empty()
        );
        source.routes[0].items[0].text = "違う選択肢".into();
        save_walkthrough(db.pool(), &id, &source).await.unwrap();
        assert!(
            save_translation(db.pool(), &original, "google", "en", &original.routes)
                .await
                .is_err()
        );
    }
}
