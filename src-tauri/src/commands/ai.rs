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

/// The single chat setting that existed before saved profiles.
///
/// Read once, to migrate an installation: every other caller goes through
/// [`crate::ai::profiles::resolve`], which knows about the saved list.
pub(crate) async fn read_legacy_config(pool: &sqlx::SqlitePool) -> crate::ai::chat::ChatConfig {
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

/// One saved endpoint as the settings screen shows it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileView {
    /// Stable id used by the edit and delete commands.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Wire format the profile speaks.
    pub provider: crate::ai::chat::Provider,
    /// Base URL, without the trailing endpoint.
    pub base_url: String,
    /// Model name.
    pub model: String,
    /// Whether a key is stored for this profile.
    pub has_key: bool,
    /// A masked hint of that key, when one is stored.
    pub key_hint: Option<String>,
}

/// The saved list, which profile is live, and whether keys can be stored here.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfilesView {
    /// Id of the profile in use, absent while the list is empty.
    pub active: Option<String>,
    /// Saved endpoints in the order they were added.
    pub profiles: Vec<ProfileView>,
    /// Whether this platform can store a key at all.
    pub supported: bool,
}

/// Build the view, reading each profile's key hint.
///
/// One credential-store read per profile: the list is capped at ten and this runs
/// only when the panel opens or a profile changes.
async fn profiles_view(state: &AppState) -> CommandResult<ProfilesView> {
    let saved = crate::ai::profiles::load(state.db().pool()).await;
    let rows: Vec<_> = saved
        .profiles
        .iter()
        .map(|profile| {
            (
                profile.id.clone(),
                profile.name.clone(),
                profile.provider,
                profile.base_url.clone(),
                profile.model.clone(),
            )
        })
        .collect();
    let checked = tauri::async_runtime::spawn_blocking(move || {
        rows.into_iter()
            .map(|(id, name, provider, base_url, model)| {
                let key = crate::secrets::load_chat_key(crate::ai::profiles::key_slot(&id)).ok();
                ProfileView {
                    id,
                    name,
                    provider,
                    base_url,
                    model,
                    has_key: key.is_some(),
                    key_hint: key.as_ref().map(crate::secrets::ChatApiKey::masked),
                }
            })
            .collect::<Vec<_>>()
    })
    .await
    .map_err(|err| CommandError::new("store_failed", err.to_string()))?;
    Ok(ProfilesView {
        active: saved.active,
        profiles: checked,
        supported: crate::secrets::is_supported(),
    })
}

/// List the saved LLM provider profiles.
///
/// # Errors
/// Database failures.
#[tauri::command]
pub async fn ai_llm_profiles(state: State<'_, AppState>) -> CommandResult<ProfilesView> {
    profiles_view(&state).await
}

/// One profile as the settings screen submits it. A missing id means "add".
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileInput {
    /// Existing profile to edit.
    pub id: Option<String>,
    /// Display name.
    pub name: String,
    /// Wire format: `openai` or `anthropic`.
    pub provider: String,
    /// Base URL, without the trailing endpoint.
    pub base_url: String,
    /// Model name.
    pub model: String,
    /// New key, when the user entered one. Empty keeps the stored key, so
    /// editing a model does not require re-entering it.
    pub key: Option<String>,
}

/// Save a profile and, when supplied, its key.
///
/// # Errors
/// `name_required`, `url_insecure`, `model_required`, `unknown_profile`,
/// `too_many_profiles`, `invalid_key` or `store_failed`.
#[tauri::command]
pub async fn ai_llm_save_profile(
    state: State<'_, AppState>,
    input: ProfileInput,
) -> CommandResult<ProfilesView> {
    let pool = state.db().pool();
    let mut saved = crate::ai::profiles::load(pool).await;
    let provider = crate::ai::chat::Provider::parse(&input.provider)
        .map_err(|msg| CommandError::new("invalid_provider", msg))?;
    let id = saved.upsert(&crate::ai::profiles::Input {
        id: input.id,
        name: input.name,
        provider,
        base_url: input.base_url,
        model: input.model,
    })?;
    if let Some(raw) = input.key.as_deref().filter(|text| !text.trim().is_empty()) {
        let key = crate::secrets::ChatApiKey::parse(raw)
            .map_err(|msg| CommandError::new("invalid_key", msg))?;
        let slot = crate::ai::profiles::key_slot(&id).to_owned();
        tauri::async_runtime::spawn_blocking(move || crate::secrets::store_chat_key(&slot, &key))
            .await
            .map_err(|err| CommandError::new("store_failed", err.to_string()))?
            .map_err(|err| match err {
                crate::secrets::SecretError::Unsupported => {
                    CommandError::new("unsupported", err.to_string())
                }
                other => CommandError::new("store_failed", other.to_string()),
            })?;
    }
    crate::ai::profiles::store(pool, &saved).await?;
    profiles_view(&state).await
}

/// Make a saved profile the live one.
///
/// # Errors
/// `unknown_profile` when the id is gone, or a database failure.
#[tauri::command]
pub async fn ai_llm_activate_profile(
    state: State<'_, AppState>,
    id: String,
) -> CommandResult<ProfilesView> {
    let pool = state.db().pool();
    let mut saved = crate::ai::profiles::load(pool).await;
    saved.activate(&id)?;
    crate::ai::profiles::store(pool, &saved).await?;
    profiles_view(&state).await
}

/// Delete a profile and its stored key.
///
/// # Errors
/// `unknown_profile` when the id is gone, or a database failure.
#[tauri::command]
pub async fn ai_llm_delete_profile(
    state: State<'_, AppState>,
    id: String,
) -> CommandResult<ProfilesView> {
    let pool = state.db().pool();
    let mut saved = crate::ai::profiles::load(pool).await;
    saved.remove(&id)?;
    crate::ai::profiles::store(pool, &saved).await?;
    // No profile can reach that key any more, so deleting the profile really
    // deletes the credential instead of orphaning it in the store.
    let slot = crate::ai::profiles::key_slot(&id).to_owned();
    let _ =
        tauri::async_runtime::spawn_blocking(move || crate::secrets::clear_chat_key(&slot)).await;
    profiles_view(&state).await
}

/// Remove one profile's stored key, keeping the profile.
///
/// # Errors
/// `unknown_profile` when the id is gone, `store_failed` when the store refuses.
#[tauri::command]
pub async fn ai_llm_clear_key(
    state: State<'_, AppState>,
    id: String,
) -> CommandResult<ProfilesView> {
    let saved = crate::ai::profiles::load(state.db().pool()).await;
    if saved.find(&id).is_none() {
        return Err(crate::ai::profiles::Invalid::UNKNOWN.into());
    }
    let slot = crate::ai::profiles::key_slot(&id).to_owned();
    match tauri::async_runtime::spawn_blocking(move || crate::secrets::clear_chat_key(&slot)).await
    {
        Ok(Err(err)) if !matches!(err, crate::secrets::SecretError::NotFound) => {
            return Err(CommandError::new("store_failed", err.to_string()));
        }
        Err(err) => return Err(CommandError::new("store_failed", err.to_string())),
        // Clearing a key that was never stored is the state the caller asked for.
        _ => {}
    }
    profiles_view(&state).await
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

/// Build the recommendation view from the live state.
///
/// The "a key is stored" flag is reported for the active profile only: a key on
/// an endpoint that would never be used is not a usable answer.
async fn recommend_settings(
    pool: &sqlx::SqlitePool,
    has_tavily_key: bool,
    tavily_key_hint: Option<String>,
) -> CommandResult<RecommendSettingsView> {
    let chat = crate::ai::profiles::resolve(pool).await;
    Ok(RecommendSettingsView {
        has_tavily_key,
        tavily_key_hint,
        has_chat_key: chat.key.is_some(),
        supported: crate::secrets::is_supported(),
        chat_model: chat.config.model,
    })
}

/// Report what the recommendation screen can do.
///
/// # Errors
/// `store_failed` when the credential store cannot be read.
#[tauri::command]
pub async fn ai_recommend_settings(
    state: State<'_, AppState>,
) -> CommandResult<RecommendSettingsView> {
    // Android Keystore resolves through the UI's native plugin bridge. Never
    // wait for that bridge from a synchronous command on the UI thread.
    let tavily = tauri::async_runtime::spawn_blocking(crate::secrets::load_tavily_key)
        .await
        .map_err(|err| CommandError::new("store_failed", err.to_string()))?
        .ok();
    let view = recommend_settings(
        state.db().pool(),
        tavily.is_some(),
        tavily.as_ref().map(crate::secrets::TavilyApiKey::masked),
    )
    .await?;
    Ok(view)
}

/// Store a Tavily API key in the OS credential store.
///
/// # Errors
/// A message describing an invalid key, or that this platform cannot store one.
#[tauri::command]
pub async fn ai_save_tavily_key(
    state: State<'_, AppState>,
    key: String,
) -> CommandResult<RecommendSettingsView> {
    let parsed = crate::secrets::TavilyApiKey::parse(&key)
        .map_err(|msg| CommandError::new("bad_key", msg))?;
    let hint = parsed.masked();
    tauri::async_runtime::spawn_blocking(move || {
        crate::secrets::store_tavily_key(&parsed).map_err(|err| match err {
            crate::secrets::SecretError::Unsupported => {
                CommandError::new("unsupported", err.to_string())
            }
            other => CommandError::new("store_failed", other.to_string()),
        })
    })
    .await
    .map_err(|err| CommandError::new("store_failed", err.to_string()))??;
    recommend_settings(state.db().pool(), true, Some(hint)).await
}

/// Remove the stored Tavily key.
///
/// # Errors
/// A message when the store refuses.
#[tauri::command]
pub async fn ai_clear_tavily_key(
    state: State<'_, AppState>,
) -> CommandResult<RecommendSettingsView> {
    tauri::async_runtime::spawn_blocking(|| match crate::secrets::clear_tavily_key() {
        Ok(()) | Err(crate::secrets::SecretError::NotFound) => Ok(()),
        Err(err) => Err(CommandError::new("store_failed", err.to_string())),
    })
    .await
    .map_err(|err| CommandError::new("store_failed", err.to_string()))??;
    recommend_settings(state.db().pool(), false, None).await
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
    let chat = crate::ai::profiles::resolve(pool).await;
    let Some(chat_key) = chat.key else {
        outcome.note = Some(if search.results.is_empty() {
            "The search returned nothing. Add a commentary key in Settings to name games."
                .to_owned()
        } else {
            "Showing search results. Add a commentary key in Settings to turn them into named suggestions."
                .to_owned()
        });
        return Ok(outcome);
    };

    let prompt = recommend::user_prompt(&request, &library, &search);
    match crate::ai::chat::complete(
        &client,
        &chat.config,
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
