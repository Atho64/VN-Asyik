//! AI commands: taste and recommendations.
//!
//! `ai_taste_profile` counts the VNDB tags the library carries most, entirely
//! locally and deterministically: a plain list a user can check against their
//! own games.
//!
//! `ai_recommend` reaches the network through Tavily, and is the only one that
//! spends credits.
//!
//! The local embedding index that used to live here was removed on 2026-10-22;
//! see `docs/archive/local-embeddings/` for what it was and why.

use serde::Serialize;
use tauri::State;

use super::{CommandError, CommandResult};
use crate::state::AppState;

/// The library's taste, computed from its own data.
///
/// Deterministic and offline: this is the answer a user gets with no API key and
/// no network.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TasteView {
    /// The computed profile: the tags the library carries most.
    pub profile: vn_core::taste::TasteProfile,
}

/// Compute the taste profile.
///
/// # Errors
/// Database failures.
#[tauri::command]
pub async fn ai_taste_profile(state: State<'_, AppState>) -> CommandResult<TasteView> {
    let pool = state.db().pool();
    let games = profiled_games(pool)
        .await
        .map_err(|err| CommandError::new("db", err.to_string()))?;
    let profile = vn_core::taste::profile(&games);
    Ok(TasteView { profile })
}

/// Every live game, in the shape the taste profile expects.
///
/// Shared by the profile and the recommendation query so both read the same
/// tags: a recommendation built from a different view of the library than the
/// profile shown beside it would be two answers to one question.
async fn profiled_games(
    pool: &sqlx::SqlitePool,
) -> Result<Vec<vn_core::taste::ProfiledGame>, crate::db::DbError> {
    use sqlx::Row as _;
    let rows = sqlx::query(
        "SELECT g.id AS id, g.title AS title, g.status AS status, g.rating AS rating, \
                g.vndb_id AS vndb_id, d.tags_json AS tags_json \
         FROM games g LEFT JOIN vndb_details d ON d.vn_id = g.vndb_id \
         WHERE g.deleted = 0 ORDER BY g.title",
    )
    .fetch_all(pool)
    .await?;

    let games: Vec<vn_core::taste::ProfiledGame> = rows
        .into_iter()
        .map(|row| {
            let json: Option<String> = row.get("tags_json");
            // A malformed cache row must not break the whole profile; that game
            // then simply has no tags and is reported as undetermined.
            let tags: Vec<vn_core::taste::TagWeight> = json
                .and_then(|text| serde_json::from_str(&text).ok())
                .unwrap_or_default();
            vn_core::taste::ProfiledGame {
                id: row.get("id"),
                title: row.get("title"),
                status: row.get("status"),
                rating: row
                    .get::<Option<i64>, _>("rating")
                    .and_then(|value| u8::try_from(value).ok()),
                tags,
                work_id: row.get::<Option<String>, _>("vndb_id"),
            }
        })
        .collect();
    Ok(games)
}

/// Whether an `http://` base URL points at this machine.
///
/// `127.0.0.1` and `[::1]` are as local as `localhost`, and local runtimes
/// (Ollama, LM Studio) are commonly configured with them, so accepting only the
/// hostname would reject a correct setup.
pub(crate) fn is_local_endpoint(base: &str) -> bool {
    let Some(rest) = base.strip_prefix("http://") else {
        return false;
    };
    ["localhost", "127.0.0.1", "[::1]"].iter().any(|host| {
        rest == *host
            || rest.starts_with(&format!("{host}:"))
            || rest.starts_with(&format!("{host}/"))
    })
}

/// Chat provider settings, without the key.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatSettingsView {
    /// Base URL of an OpenAI-compatible endpoint.
    pub base_url: String,
    /// Model name.
    pub model: String,
    /// Whether a key is stored, and where it is stored.
    pub has_key: bool,
    /// A masked hint of the key, when one is stored.
    pub key_hint: Option<String>,
    /// Whether this platform can store a key at all.
    pub supported: bool,
}

pub(crate) async fn read_settings(pool: &sqlx::SqlitePool) -> crate::ai::chat::ChatConfig {
    // Stored as a plain JSON blob in `sync_state`: it holds no secrets, and a
    // separate table for two strings would be noise.
    let defaults = crate::ai::chat::ChatConfig::default();
    let found: Result<Option<(String,)>, _> =
        sqlx::query_as("SELECT value FROM sync_state WHERE key = 'chat.config'")
            .fetch_optional(pool)
            .await;
    match found {
        Ok(Some((json,))) => serde_json::from_str(&json).unwrap_or(defaults),
        _ => defaults,
    }
}

/// Report chat settings and whether a key is stored.
///
/// # Errors
/// Database failures.
#[tauri::command]
pub async fn ai_chat_settings(state: State<'_, AppState>) -> CommandResult<ChatSettingsView> {
    let config = read_settings(state.db().pool()).await;
    let key = crate::secrets::load_chat_key().ok();
    Ok(ChatSettingsView {
        base_url: config.base_url,
        model: config.model,
        has_key: key.is_some(),
        key_hint: key.as_ref().map(crate::secrets::ChatApiKey::masked),
        supported: crate::secrets::is_supported(),
    })
}

/// Payload for saving chat settings. A `None` key leaves the stored one alone,
/// so editing the model does not require re-entering the key.
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatSettingsInput {
    /// Base URL of an OpenAI-compatible endpoint.
    pub base_url: String,
    /// Model name.
    pub model: String,
    /// New key, when the user entered one.
    pub key: Option<String>,
}

/// Save chat settings and, when supplied, the key.
///
/// # Errors
/// `invalid` for a malformed URL or key, `unsupported` where no credential store
/// exists.
#[tauri::command]
pub async fn ai_save_chat_settings(
    state: State<'_, AppState>,
    input: ChatSettingsInput,
) -> CommandResult<ChatSettingsView> {
    let base = input.base_url.trim().trim_end_matches('/').to_owned();
    if !base.starts_with("https://") && !is_local_endpoint(&base) {
        // Plain http would send the key in clear text over a network. A loopback
        // address is allowed because a local model runtime is the offline case.
        return Err(CommandError::new(
            "invalid",
            "the endpoint must use https, or http on localhost",
        ));
    }
    if input.model.trim().is_empty() {
        return Err(CommandError::new("invalid", "model must not be empty"));
    }

    if let Some(raw) = input.key.as_deref().filter(|text| !text.trim().is_empty()) {
        let key = crate::secrets::ChatApiKey::parse(raw)
            .map_err(|err| CommandError::new("invalid", err))?;
        crate::secrets::store_chat_key(&key).map_err(|err| match err {
            crate::secrets::SecretError::Unsupported => {
                CommandError::new("unsupported", "this platform cannot store the key securely")
            }
            other => CommandError::new("store_failed", other.to_string()),
        })?;
    }

    let config = crate::ai::chat::ChatConfig {
        base_url: base,
        model: input.model.trim().to_owned(),
    };
    let json = serde_json::to_string(&config)
        .map_err(|err| CommandError::new("invalid", err.to_string()))?;
    sqlx::query(
        "INSERT INTO sync_state (key, value) VALUES ('chat.config', ?)          ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(json)
    .execute(state.db().pool())
    .await
    .map_err(|err| CommandError::new("db", err.to_string()))?;

    ai_chat_settings(state).await
}

/// Remove the stored chat key.
///
/// # Errors
/// `remove_failed` when the credential store refuses.
#[tauri::command]
pub async fn ai_clear_chat_key(state: State<'_, AppState>) -> CommandResult<ChatSettingsView> {
    match crate::secrets::clear_chat_key() {
        Ok(()) | Err(crate::secrets::SecretError::NotFound) => {}
        Err(err) => return Err(CommandError::new("remove_failed", err.to_string())),
    }
    ai_chat_settings(state).await
}

// Recommendations

/// What the recommendation screen needs to render its controls.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecommendSettingsView {
    /// Whether a Tavily key is stored, and where.
    pub has_tavily_key: bool,
    /// A masked hint of the Tavily key.
    pub tavily_key_hint: Option<String>,
    /// Whether a chat key is stored: the step that names games needs one.
    pub has_chat_key: bool,
    /// Whether this platform can store keys at all.
    pub supported: bool,
    /// Chat provider base URL and model, for display.
    pub chat_model: String,
}

/// Report what the recommendation screen can do.
#[tauri::command]
pub async fn ai_recommend_settings(
    state: State<'_, AppState>,
) -> CommandResult<RecommendSettingsView> {
    // Android Keystore resolves through the UI's native plugin bridge. Never
    // wait for that bridge from a synchronous command on the UI thread.
    let (tavily, has_chat_key) = tauri::async_runtime::spawn_blocking(|| {
        (
            crate::secrets::load_tavily_key().ok(),
            crate::secrets::load_chat_key().is_ok(),
        )
    })
    .await
    .map_err(|err| CommandError::new("store_failed", err.to_string()))?;
    let config = read_settings(state.db().pool()).await;
    Ok(RecommendSettingsView {
        has_tavily_key: tavily.is_some(),
        tavily_key_hint: tavily.as_ref().map(crate::secrets::TavilyApiKey::masked),
        has_chat_key,
        supported: crate::secrets::is_supported(),
        chat_model: config.model,
    })
}

/// Store a Tavily API key in the OS credential store.
///
/// # Errors
/// A message describing an invalid key, or that this platform cannot store one.
#[tauri::command]
pub async fn ai_save_tavily_key(key: String) -> CommandResult<RecommendSettingsView> {
    let parsed = crate::secrets::TavilyApiKey::parse(&key)
        .map_err(|msg| CommandError::new("bad_key", msg))?;
    tauri::async_runtime::spawn_blocking(move || {
        crate::secrets::store_tavily_key(&parsed).map_err(|err| match err {
            crate::secrets::SecretError::Unsupported => {
                CommandError::new("unsupported", err.to_string())
            }
            other => CommandError::new("store_failed", other.to_string()),
        })?;
        Ok(RecommendSettingsView {
            has_tavily_key: true,
            tavily_key_hint: Some(parsed.masked()),
            has_chat_key: crate::secrets::load_chat_key().is_ok(),
            supported: crate::secrets::is_supported(),
            chat_model: crate::ai::chat::ChatConfig::default().model,
        })
    })
    .await
    .map_err(|err| CommandError::new("store_failed", err.to_string()))?
}

/// Remove the stored Tavily key.
///
/// # Errors
/// A message when the store refuses.
#[tauri::command]
pub async fn ai_clear_tavily_key() -> CommandResult<RecommendSettingsView> {
    tauri::async_runtime::spawn_blocking(|| {
        match crate::secrets::clear_tavily_key() {
            Ok(()) | Err(crate::secrets::SecretError::NotFound) => {}
            Err(err) => return Err(CommandError::new("store_failed", err.to_string())),
        }
        Ok(RecommendSettingsView {
            has_tavily_key: false,
            tavily_key_hint: None,
            has_chat_key: crate::secrets::load_chat_key().is_ok(),
            supported: crate::secrets::is_supported(),
            chat_model: crate::ai::chat::ChatConfig::default().model,
        })
    })
    .await
    .map_err(|err| CommandError::new("store_failed", err.to_string()))?
}

/// Ask what to read next.
///
/// Never fails on a search or model problem: the caller always gets the search
/// results and a note explaining what happened, because "no recommendation" with
/// a reason is useful and an error dialog is not.
///
/// # Errors
/// Only when the database cannot be read or no Tavily key is stored.
#[tauri::command]
pub async fn ai_recommend(
    state: State<'_, AppState>,
    request: String,
    depth: String,
) -> CommandResult<crate::ai::recommend::RecommendationOutcome> {
    use crate::ai::recommend;
    use sqlx::Row as _;

    let key = crate::secrets::load_tavily_key().map_err(|err| {
        CommandError::new("no_key", format!("{err}. Add a Tavily key in Settings."))
    })?;

    let pool = state.db().pool();
    let rows = sqlx::query("SELECT title FROM games WHERE deleted = 0")
        .fetch_all(pool)
        .await
        .map_err(|err| CommandError::new("db", err.to_string()))?;
    let library: Vec<String> = rows.iter().map(|row| row.get("title")).collect();

    let search_depth = if depth == "advanced" {
        crate::ai::tavily::Depth::Advanced
    } else {
        crate::ai::tavily::Depth::Basic
    };
    // Steer the search with the user's own strongest themes. Without this the
    // query is just the seed title, which finds the game's store page rather than
    // readers discussing what someone who liked it would enjoy next.
    let taste = profiled_games(pool)
        .await
        .map(|games| {
            vn_core::taste::profile(&games)
                .themes
                .iter()
                .map(|theme| theme.name.clone())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let query = recommend::build_query(&request, &taste);

    let client = crate::vndb::http_client_builder()
        .build()
        .map_err(|err| CommandError::new("http", err.to_string()))?;
    let search = match crate::ai::tavily::search(&client, &key, &query, 8, search_depth).await {
        Ok(found) => found,
        Err(err) => {
            // The search itself failed, so there is nothing to cite.
            return Ok(recommend::RecommendationOutcome {
                query,
                games: Vec::new(),
                answer: None,
                sources: Vec::new(),
                note: Some(err.to_string()),
                credits: 0,
            });
        }
    };

    let mut outcome = recommend::RecommendationOutcome {
        query: query.clone(),
        games: Vec::new(),
        answer: search.answer.clone(),
        sources: search.results.clone(),
        note: None,
        credits: search.credits,
    };

    // Naming games needs a chat model. Without one the search results are still
    // useful, so this degrades rather than failing.
    let Ok(chat_key) = crate::secrets::load_chat_key() else {
        outcome.note = Some(if search.results.is_empty() {
            "The search returned nothing. Add a commentary key in Settings to name games."
                .to_owned()
        } else {
            "Showing search results. Add a commentary key in Settings to turn them into named suggestions."
                .to_owned()
        });
        return Ok(outcome);
    };

    let config = read_settings(pool).await;
    let prompt = recommend::user_prompt(&request, &library, &search);
    match crate::ai::chat::complete(
        &client,
        &config,
        Some(&chat_key),
        recommend::system_prompt(),
        &prompt,
    )
    .await
    {
        Ok(reply) => match recommend::parse_suggestions(&reply) {
            Ok(parsed) => {
                // The seed game is excluded: suggesting the game just asked about
                // is the most obvious way to look broken.
                outcome.games = recommend::assemble_excluding(
                    &parsed.games,
                    &search.results,
                    &library,
                    std::slice::from_ref(&request),
                );
                let owned = outcome.games.iter().filter(|game| game.owned).count();
                outcome.note =
                    recommend::note_for(outcome.games.len(), search.results.len(), owned);
            }
            Err(err) => {
                outcome.note = Some(format!("The model's reply could not be read: {err}"));
            }
        },
        Err(err) => {
            outcome.note = Some(format!("{err}. The search results above are unaffected."));
        }
    }

    Ok(outcome)
}

#[cfg(test)]
mod local_endpoint_tests {
    use super::is_local_endpoint;

    #[test]
    fn the_documented_hostname_is_local() {
        // Configuring a local runtime as http://localhost:11434 must work.
        assert!(is_local_endpoint("http://localhost:11434/v1"));
        assert!(is_local_endpoint("http://localhost"));
    }

    #[test]
    fn a_loopback_ip_is_as_local_as_the_hostname() {
        // Ollama and LM Studio commonly print 127.0.0.1, so accepting only the
        // name would reject a correct configuration.
        assert!(is_local_endpoint("http://127.0.0.1:1234/v1"));
        assert!(is_local_endpoint("http://[::1]:8080/v1"));
    }

    #[test]
    fn a_remote_host_is_rejected() {
        // Plain http to another machine would put the key on the wire.
        assert!(!is_local_endpoint("http://example.com/v1"));
        assert!(!is_local_endpoint("http://192.168.1.10:11434/v1"));
    }

    #[test]
    fn a_lookalike_hostname_is_rejected() {
        // "localhost.evil.com" starts with "localhost" but is not this machine.
        assert!(!is_local_endpoint("http://localhost.evil.com/v1"));
        assert!(!is_local_endpoint("http://127.0.0.1.evil.com/v1"));
    }

    #[test]
    fn only_the_http_prefix_is_considered() {
        // https:// is accepted elsewhere; this guard is only about plain http.
        assert!(!is_local_endpoint("https://api.openai.com/v1"));
    }
}
