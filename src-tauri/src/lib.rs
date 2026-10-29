//! VN Asyik application core.
//!
//! Layout follows AGENTS.md §3. Everything user-visible goes through the
//! `commands` module; the frontend has no direct database or filesystem access.

pub mod ai;
pub mod commands;
pub mod db;
pub mod platform;
pub mod presence;
pub mod script;
pub mod secrets;
pub mod state;
pub mod sync;
pub mod tracking;
pub mod tray;
pub mod vndb;
pub mod walkthrough;

use tauri::Manager;

/// The Android usage-stats plugin, or an inert plugin elsewhere.
///
/// Tauri plugins are typed by runtime and cannot be conditionally absent from a
/// builder chain, so the non-Android arm registers a pass-through plugin under the
/// same name instead of `#[cfg]`-ing the call site.
fn android_usage_plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    #[cfg(target_os = "android")]
    {
        platform::android::init()
    }
    #[cfg(not(target_os = "android"))]
    {
        tauri::plugin::Builder::<tauri::Wry, ()>::new("usage-stats").build()
    }
}

/// Build and run the Tauri application.
///
/// # Panics
/// Panics only if Tauri itself fails to start, which is unrecoverable.
/// Every command the frontend may call, registered in one place.
// `generate_handler!` takes one flat list, so this cannot be split into modules
// the way the commands themselves are. The length is the IPC surface, not
// tangled logic.
#[allow(
    clippy::too_many_lines,
    reason = "generate_handler! requires every command in one macro invocation"
)]
fn handlers() -> impl Fn(tauri::ipc::Invoke) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        commands::library::list_games,
        commands::library::get_game,
        commands::library::create_game,
        commands::library::update_game,
        commands::library::delete_game,
        commands::library::list_chapters,
        commands::library::list_sessions,
        commands::library::list_game_sessions,
        commands::library::list_endings,
        commands::library::game_routes,
        commands::library::create_route,
        commands::library::rename_route,
        commands::library::merge_routes,
        commands::library::set_chapter_route,
        commands::library::set_ending_route,
        commands::library::create_ending,
        commands::library::update_ending,
        commands::library::delete_ending,
        commands::library::add_manual_session,
        commands::library::status_breakdown,
        commands::library::playtime_totals,
        commands::library::playtime_by_day,
        commands::library::get_afk_config,
        commands::library::set_afk_config,
        commands::library::set_game_accent,
        commands::library::game_custom_art,
        commands::library::set_game_custom_cover,
        commands::library::clear_game_custom_cover,
        commands::prefs::get_prefs,
        commands::prefs::set_theme,
        commands::prefs::set_language,
        commands::prefs::set_nsfw_mode,
        commands::prefs::set_reduce_effects,
        commands::backup::export_backup,
        commands::backup::import_backup,
        commands::backup::export_backup_file,
        commands::backup::import_backup_file,
        commands::data::data_open_file,
        commands::data::data_export_file,
        commands::data::data_share_file,
        commands::data::data_apply_import,
        commands::data::data_cancel_import,
        commands::data::share_backup_file,
        commands::system::platform_info,
        commands::system::open_external,
        commands::system::hide_window,
        commands::system::minimize_window,
        commands::system::show_window,
        commands::system::toggle_window,
        commands::system::quit_app,
        commands::vndb::vndb_search,
        commands::vndb::vndb_preview,
        commands::vndb::vndb_import,
        commands::vndb::vndb_import_many,
        commands::vndb::vndb_detail,
        commands::vndb::vndb_screenshots,
        commands::vndb::vndb_screenshot_fetch,
        commands::vndb::vndb_refresh_detail,
        commands::vndb::vndb_refresh_all,
        commands::vndb::vndb_local_art,
        commands::vndb_sync::vndb_status,
        commands::vndb_sync::vndb_connect,
        commands::vndb_sync::vndb_disconnect,
        commands::vndb_sync::vndb_token_hint,
        commands::vndb_sync::vndb_sync_settings,
        commands::vndb_sync::vndb_save_sync_settings,
        commands::vndb_sync::vndb_pull,
        commands::vndb_sync::vndb_apply_sync,
        commands::vndb_sync::vndb_push_pending,
        commands::vndb_sync::vndb_queue,
        commands::vndb_sync::vndb_log,
        commands::vndb_sync::vndb_labels,
        commands::tracking::tracking_capability,
        commands::tracking::tracking_status,
        commands::tracking::tracking_set_enabled,
        commands::tracking::tracking_map_executable,
        commands::tracking::tracking_map_runner_target,
        commands::tracking::tracking_list_executables,
        commands::tracking::tracking_list_chapters,
        commands::tracking::tracking_chapter_capability,
        commands::tracking::tracking_usage_access,
        commands::tracking::tracking_open_usage_settings,
        commands::tracking::tracking_android_apps,
        commands::tracking::tracking_android_status,
        commands::tracking::tracking_android_session,
        commands::tracking::tracking_android_stop,
        commands::sync::sync_status,
        commands::sync::sync_run,
        commands::sync::sync_conflicts,
        commands::sync::sync_export_file,
        commands::sync::sync_import_file,
        commands::sync::sync_share_file,
        commands::ai::ai_taste_profile,
        commands::ai::ai_chat_settings,
        commands::ai::ai_save_chat_settings,
        commands::ai::ai_clear_chat_key,
        commands::chat::chat_list,
        commands::chat::chat_create,
        commands::chat::chat_messages,
        commands::chat::chat_update,
        commands::chat::chat_delete,
        commands::chat::chat_send,
        commands::chat::chat_cancel,
        commands::script::script_import,
        commands::script::script_list,
        commands::script::script_delete,
        commands::script::script_compact,
        commands::script::script_agent_settings,
        commands::script::script_save_agent_settings,
        commands::luna::luna_settings,
        commands::luna::luna_save_settings,
        commands::luna::luna_status,
        commands::luna::luna_test,
        commands::launch::launch_capability,
        commands::launch::launch_save_settings,
        commands::launch::launch_game,
        commands::tracking::stats_derived,
        commands::tracking::tracking_title_config,
        commands::tracking::tracking_save_title_config,
        commands::tracking::tracking_test_title_pattern,
        commands::ai::ai_recommend_settings,
        commands::ai::ai_save_tavily_key,
        commands::ai::ai_clear_tavily_key,
        commands::ai::ai_recommend,
        commands::tracking::tracking_rename_chapter,
        commands::tracking::tracking_manual_status,
        commands::tracking::tracking_manual_start,
        commands::tracking::tracking_manual_start_route,
        commands::tracking::tracking_manual_pause,
        commands::tracking::tracking_manual_resume,
        commands::tracking::tracking_manual_stop,
        commands::tracking::tracking_create_chapter,
        commands::tracking::tracking_remove_executable,
        commands::tracking::tracking_pause,
        commands::tracking::tracking_resume,
        commands::tracking::tracking_stop,
        commands::tracking::tracking_list_scan_folders,
        commands::tracking::tracking_add_scan_folder,
        commands::tracking::tracking_remove_scan_folder,
        commands::tracking::tracking_scan_folder,
        commands::hook::hook_status,
        commands::presence::presence_status,
        commands::presence::presence_get_settings,
        commands::presence::presence_save_settings,
        commands::presence::presence_effective_settings,
        commands::presence::presence_reset_game_settings,
        commands::presence::presence_set_enabled,
        commands::presence::presence_clear,
        commands::presence::presence_preview,
        commands::presence::presence_has_token,
        commands::presence::presence_save_token,
        commands::presence::presence_clear_token,
        commands::presence::presence_token_preview,
        commands::presence::presence_image_key_save,
        commands::presence::presence_image_key_clear,
        commands::presence::presence_image_key_preview,
        commands::walkthrough::walkthrough_get,
        commands::walkthrough::walkthrough_search_candidate,
        commands::walkthrough::walkthrough_search,
        commands::walkthrough::walkthrough_fetch,
        commands::walkthrough::walkthrough_translate,
        commands::walkthrough::walkthrough_toggle_item,
        commands::walkthrough::walkthrough_clear_progress,
    ]
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        // Android only: registers the Kotlin usage-stats plugin so playtime can
        // come from the platform usage log. A no-op builder elsewhere.
        .plugin(android_usage_plugin())
        .plugin(platform::storage::init())
        .plugin(platform::android_presence::init())
        .setup(|app| {
            let handle = app.handle().clone();
            let state = tauri::async_runtime::block_on(state::AppState::boot(&handle))?;
            app.manage(state);
            // Start the playtime loop once state exists.
            commands::tracking::spawn_loop(handle.clone());
            // System tray icon for desktop background operation.
            tray::setup_tray(&handle)?;
            // Chromium's own right-click menu has no place in a desktop app; the
            // UI draws its own. No-op on platforms without WebView2.
            #[cfg(windows)]
            platform::context_menu::disable_default_context_menu(&handle);
            Ok(())
        })
        .on_window_event(|window, event| {
            #[cfg(desktop)]
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(handlers())
        .run(tauri::generate_context!())
        .expect("error while running VN Asyik");
}
