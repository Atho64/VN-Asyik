use tauri::State;
use vn_core::presence::{PresenceActivity, PresenceContext, PresenceSettings, build_activity};

use super::{CommandError, CommandResult};
use crate::db::queries;
use crate::presence;
use crate::state::AppState;

/// Get current Discord Rich Presence connection and status.
#[tauri::command]
pub async fn presence_status(
    state: State<'_, AppState>,
) -> CommandResult<crate::presence::PresenceStatus> {
    Ok(state.presence().status().await)
}

/// Get current Discord Rich Presence configuration.
#[tauri::command]
pub async fn presence_get_settings(state: State<'_, AppState>) -> CommandResult<PresenceSettings> {
    Ok(state.presence().get_settings().await)
}

/// Save Discord Rich Presence configuration.
#[tauri::command]
pub async fn presence_save_settings(
    state: State<'_, AppState>,
    mut settings: PresenceSettings,
) -> CommandResult<()> {
    settings.client_id = validate_client_id(&settings.client_id)?;
    settings.my_vndb_link = validate_profile_link(&settings.my_vndb_link)?;
    queries::save_presence_settings(state.db().pool(), &settings).await?;
    state.presence().set_settings(settings).await;
    super::tracking::sync_presence_now(&state).await;
    Ok(())
}

/// A Discord application id is a numeric snowflake. Anything else can never
/// connect, and would surface as "Discord is not running", which sends the user
/// looking in the wrong place. Blank is allowed and means the default id.
fn validate_client_id(raw: &str) -> CommandResult<String> {
    let id = raw.trim();
    if id.is_empty() {
        return Ok(String::new());
    }
    let numeric = id.bytes().all(|b| b.is_ascii_digit());
    if numeric && (15..=25).contains(&id.len()) {
        Ok(id.to_string())
    } else {
        Err(CommandError::new(
            "invalid_client_id",
            "The Discord application ID must be 15 to 25 digits.",
        ))
    }
}

/// Normalize the user's VNDB profile link, or reject it.
///
/// The link becomes the URL of a Discord button, so it is held to the same rules
/// as any other outbound link in this app: an `https` URL on `vndb.org`. Blank is
/// allowed and simply means the second button is not shown. Rejecting here rather
/// than only at render time is deliberate: a value that silently never appears on
/// the card leaves the user adjusting the wrong switch.
fn validate_profile_link(raw: &str) -> CommandResult<String> {
    let link = raw.trim();
    if link.is_empty() {
        return Ok(String::new());
    }
    // A bare `u123456` is what people paste from their profile page URL bar.
    if link.starts_with('u') && link[1..].bytes().all(|b| b.is_ascii_digit()) && link.len() > 1 {
        return Ok(format!("https://vndb.org/{link}"));
    }
    let Ok(parsed) = url::Url::parse(link) else {
        return Err(CommandError::new(
            "invalid_profile_link",
            "The VNDB profile link must be a full https://vndb.org/u... URL.",
        ));
    };
    let host_ok = parsed
        .host_str()
        .is_some_and(|h| h == "vndb.org" || h.ends_with(".vndb.org"));
    if parsed.scheme() != "https" || !host_ok {
        return Err(CommandError::new(
            "invalid_profile_link",
            "The VNDB profile link must be a full https://vndb.org/u... URL.",
        ));
    }
    Ok(parsed.to_string())
}

/// Set Discord Rich Presence enabled flag.
#[tauri::command]
pub async fn presence_set_enabled(
    state: State<'_, AppState>,
    enabled: bool,
) -> CommandResult<PresenceSettings> {
    let mut settings = state.presence().get_settings().await;
    settings.enabled = enabled;
    queries::save_presence_settings(state.db().pool(), &settings).await?;
    state.presence().set_settings(settings.clone()).await;
    if enabled {
        super::tracking::sync_presence_now(&state).await;
    }
    Ok(settings)
}

/// Manually clear Discord Rich Presence.
#[tauri::command]
pub async fn presence_clear(state: State<'_, AppState>) -> CommandResult<()> {
    state.presence().clear();
    Ok(())
}

/// Check if a Discord user token is stored in secure storage.
#[tauri::command]
pub async fn presence_has_token() -> CommandResult<bool> {
    Ok(crate::secrets::load_discord_token().is_ok())
}

/// Save Discord user token into secure credential storage.
#[tauri::command]
pub async fn presence_save_token(state: State<'_, AppState>, token: String) -> CommandResult<()> {
    let parsed = crate::secrets::DiscordToken::parse(&token)
        .map_err(|e| CommandError::new("invalid_token", e))?;
    crate::secrets::store_discord_token(&parsed)
        .map_err(|e| CommandError::new("secret_error", e.to_string()))?;
    super::tracking::sync_presence_now(&state).await;
    Ok(())
}

/// Clear Discord user token from secure credential storage and stop active presence.
#[tauri::command]
pub async fn presence_clear_token(state: State<'_, AppState>) -> CommandResult<()> {
    state.presence().clear();
    let _ = crate::platform::android_presence::stop();
    crate::secrets::clear_discord_token()
        .map_err(|e| CommandError::new("secret_error", e.to_string()))?;
    Ok(())
}

/// Return masked preview of stored Discord token, or None.
#[tauri::command]
pub async fn presence_token_preview() -> CommandResult<Option<String>> {
    match crate::secrets::load_discord_token() {
        Ok(t) => Ok(Some(t.masked())),
        Err(_) => Ok(None),
    }
}

/// Save the imgbb API key for the presence image host into secure storage.
#[tauri::command]
pub async fn presence_image_key_save(key: String) -> CommandResult<()> {
    let parsed = crate::secrets::ImgbbApiKey::parse(&key)
        .map_err(|e| CommandError::new("invalid_api_key", e))?;
    crate::secrets::store_imgbb_key(&parsed)
        .map_err(|e| CommandError::new("secret_error", e.to_string()))?;
    Ok(())
}

/// Remove the stored imgbb API key.
#[tauri::command]
pub async fn presence_image_key_clear() -> CommandResult<()> {
    // Clearing an absent key is fine: the UI treats both as "no key".
    let _ = crate::secrets::clear_imgbb_key();
    Ok(())
}

/// Return masked preview of the stored imgbb API key, or None.
#[tauri::command]
pub async fn presence_image_key_preview() -> CommandResult<Option<String>> {
    match crate::secrets::load_imgbb_key() {
        Ok(key) => Ok(Some(key.masked())),
        Err(_) => Ok(None),
    }
}

/// Live preview of an activity payload rendered with provided settings.
///
/// `cycle_index` / `state_cycle_index` let the settings screen step through a
/// rotation list without saving first, which is the only way a user can check
/// that a template list reads correctly on entry 3 and not just entry 1.
#[tauri::command]
pub async fn presence_preview(
    state: State<'_, AppState>,
    settings: PresenceSettings,
    game_id: Option<String>,
    cycle_index: Option<u32>,
    state_cycle_index: Option<u32>,
) -> CommandResult<PresenceActivity> {
    // The session is made up, but the clock is real: a hardcoded date would sit
    // still forever and read as a broken tag.
    let (date, time) = match vn_core::clock::Timestamp::now() {
        Ok(now) => crate::presence::local_date_time(now),
        Err(_) => (String::new(), String::new()),
    };
    let mut ctx = if let Some(gid) = game_id {
        let entity_id = vn_core::EntityId::parse(&gid)
            .map_err(|e| CommandError::new("invalid_id", e.to_string()))?;
        match queries::get_game(state.db().pool(), entity_id).await? {
            Some(game) => {
                let base = presence::context_for_game(state.db().pool(), &game).await;
                PresenceContext {
                    exe_name: "game.exe".to_string(),
                    elapsed_time: "01:23".to_string(),
                    chapter: Some("Chapter 1".to_string()),
                    route: Some("Common Route".to_string()),
                    session_start_epoch: Some(1_700_000_000),
                    ..base
                }
            }
            // A game row that vanished between the picker opening and the preview
            // is not an error worth a red banner; show the sample instead.
            None => sample_preview_context(),
        }
    } else {
        sample_preview_context()
    };
    ctx.date = date;
    ctx.time = time;
    ctx.cycle_index = cycle_index.unwrap_or(0);
    ctx.state_cycle_index = state_cycle_index.unwrap_or(0);

    Ok(build_activity(&settings, &ctx))
}

/// A made-up session for the settings preview.
///
/// It has no cover on purpose: a cover URL here would make the webview fetch an
/// image from VNDB's CDN every time the settings screen opens, for a game the
/// user may not own, which is a network request nobody asked for. `date` / `time`
/// are filled by the caller with the real clock, not stored here.
fn sample_preview_context() -> PresenceContext {
    PresenceContext {
        vn_name: "Steins;Gate".to_string(),
        activity_name: "Steins;Gate".to_string(),
        exe_name: "Gate.exe".to_string(),
        game_key: "steins-gate".to_string(),
        window_title: "STEINS;GATE".to_string(),
        large_text: "Steins;Gate".to_string(),
        vndb_id: Some("v2002".to_string()),
        vndb_url: Some("https://vndb.org/v2002".to_string()),
        vndb_rating: Some("8.86".to_string()),
        my_rating: Some("9.5".to_string()),
        vndb_developer: Some("5pb. / Nitroplus".to_string()),
        vndb_status: Some("Reading".to_string()),
        vndb_length: Some("Very Long (>50h)".to_string()),
        vndb_year: Some("2009".to_string()),
        vndb_released: Some("2009-10-15".to_string()),
        elapsed_time: "02:45".to_string(),
        date: String::new(),
        time: String::new(),
        chapter: Some("Chapter 3: Butterfly Effect's Divergence".to_string()),
        route: Some("Main Route".to_string()),
        cover_url: None,
        is_cover_explicit: false,
        rpc_image: None,
        session_start_epoch: Some(1_700_000_000),
        vndb_loaded: true,
        is_unfocused: false,
        cycle_index: 0,
        state_cycle_index: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_blank_client_id_means_the_default() {
        assert_eq!(validate_client_id("   ").expect("blank is allowed"), "");
    }

    #[test]
    fn a_numeric_snowflake_is_accepted_and_trimmed() {
        assert_eq!(
            validate_client_id(" 1563096795532230756 ").expect("valid"),
            "1563096795532230756"
        );
    }

    #[test]
    fn anything_that_cannot_connect_is_rejected_at_save() {
        for bad in [
            "abc",
            "12345",
            "1563096795532230756x",
            "1563 0967 9553 2230",
            "-1563096795532230756",
        ] {
            assert!(validate_client_id(bad).is_err(), "{bad:?} must be rejected");
        }
    }

    #[test]
    fn a_blank_profile_link_is_allowed_and_means_no_button() {
        assert_eq!(validate_profile_link("   ").expect("blank"), "");
    }

    #[test]
    fn a_bare_user_id_is_expanded_to_the_profile_url() {
        assert_eq!(
            validate_profile_link("u123456").expect("bare id"),
            "https://vndb.org/u123456"
        );
    }

    #[test]
    fn a_vndb_profile_url_is_accepted() {
        assert_eq!(
            validate_profile_link("https://vndb.org/u123456").expect("valid"),
            "https://vndb.org/u123456"
        );
    }

    #[test]
    fn anything_that_is_not_an_https_vndb_link_is_rejected_at_save() {
        // The link becomes a Discord button URL. Saving something that can never
        // render leaves the user fiddling with the wrong switch.
        for bad in [
            "javascript:alert(1)",
            "http://vndb.org/u1",
            "https://example.com/u1",
            "https://vndb.org.evil.test/u1",
            "file:///C:/secrets.txt",
            "u12x456",
        ] {
            assert!(
                validate_profile_link(bad).is_err(),
                "{bad:?} must be rejected"
            );
        }
    }

    #[test]
    fn the_preview_sample_never_points_at_a_remote_image() {
        let ctx = sample_preview_context();
        assert!(ctx.cover_url.is_none());
        let act = build_activity(&PresenceSettings::default(), &ctx);
        assert!(act.large_image.is_none());
    }
}
