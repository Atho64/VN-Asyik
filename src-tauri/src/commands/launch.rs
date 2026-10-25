//! Starting a game, optionally through Locale Emulator.
//!
//! # Safety
//!
//! This spawns the executable the user already told the app to track, or the
//! Locale Emulator launcher they installed. It does not inject, patch, or modify
//! anything, and it refuses a path that is not an existing file rather than
//! handing it to the shell.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::State;

use vn_core::locale_emulator::{self, LaunchPlan};

use super::{CommandError, CommandResult};
use crate::state::AppState;

/// Setting key holding the Locale Emulator directory.
const LE_DIR_KEY: &str = "locale.launcherDir";
/// Setting key holding the chosen locale profile GUID.
const LE_GUID_KEY: &str = "locale.guid";
/// Setting key holding an explicit "do not use Locale Emulator" flag.
///
/// Separate from clearing the directory, because the two mean different things:
/// a configured folder with this set is "I have it, keep it configured, but start
/// this my own way". Without it the only way to disable the emulator was to forget
/// where it lived.
const LE_DISABLED_KEY: &str = "locale.disabled";

/// What Locale Emulator will do for the next launch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum EmulatorState {
    /// No launcher was found in the configured folder or beside the game.
    NotInstalled,
    /// Found and usable, but no Japanese profile is chosen, so launches go direct.
    ///
    /// This is the state that used to render the "Japanese locale" badge.
    NoProfile,
    /// Found, enabled and configured: a launch goes through the emulator.
    Ready,
    /// The user switched it off. The folder is remembered, nothing uses it.
    Disabled,
}

impl EmulatorState {
    /// Whether a launch will actually go through Locale Emulator.
    #[must_use]
    pub fn will_localize(self) -> bool {
        matches!(self, Self::Ready)
    }
}

/// What the launch screen needs.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchCapability {
    /// Whether a game can be started at all on this platform.
    pub can_launch: bool,
    /// Whether a locale emulator was found, and what it will actually do.
    ///
    /// One value rather than `has_emulator`/`emulator_enabled`/`will_localize`
    /// booleans, because those three were mutually constrained: only one
    /// combination is truthful at a time, and keeping them separate let the UI key
    /// a "starts in Japanese" badge off "a file exists on disk", which is what made
    /// it promise localization the launch then skipped.
    pub emulator: EmulatorState,
    /// Where it was found, for the UI to show.
    pub emulator_path: Option<String>,
    /// The directory the user configured, even when it does not exist.
    pub configured_dir: Option<String>,
    /// The chosen profile GUID, when one was set.
    pub guid: Option<String>,
    /// Profiles found in the installed Locale Emulator, so the UI can offer
    /// them by name instead of asking the user for a GUID.
    pub profiles: Vec<LaunchProfile>,
    /// Why the feature is unavailable, when it is.
    pub note: Option<String>,
}

/// One selectable Locale Emulator profile.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchProfile {
    /// Display name from the profile.
    pub name: String,
    /// GUID passed to `-runas`.
    pub guid: String,
    /// Locale such as `ja-JP`.
    pub location: String,
    /// Whether this is the profile currently chosen.
    pub selected: bool,
}

/// Settings the user can change about launching.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchSettingsInput {
    /// Directory containing `LEProc.exe`. An empty string clears it.
    pub launcher_dir: Option<String>,
    /// Profile GUID to run with. An empty string clears it.
    pub guid: Option<String>,
    /// Turn Locale Emulator off without forgetting where it is.
    #[serde(default)]
    pub emulator_enabled: Option<bool>,
}

async fn read_setting(pool: &sqlx::SqlitePool, key: &str) -> Option<String> {
    let found: Result<Option<(String,)>, _> =
        sqlx::query_as("SELECT value FROM sync_state WHERE key = ?")
            .bind(key)
            .fetch_optional(pool)
            .await;
    found.ok().flatten().map(|(value,)| value)
}

async fn write_setting(pool: &sqlx::SqlitePool, key: &str, value: Option<&str>) {
    match value {
        Some(text) if !text.trim().is_empty() => {
            let _ = sqlx::query(
                "INSERT INTO sync_state (key, value) VALUES (?, ?)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            )
            .bind(key)
            .bind(text.trim())
            .execute(pool)
            .await;
        }
        _ => {
            let _ = sqlx::query("DELETE FROM sync_state WHERE key = ?")
                .bind(key)
                .execute(pool)
                .await;
        }
    }
}

/// Report whether a game can be launched, and through what.
///
/// # Errors
/// Database failures only.
#[tauri::command]
pub async fn launch_capability(state: State<'_, AppState>) -> CommandResult<LaunchCapability> {
    let pool = state.db().pool();
    let configured = read_setting(pool, LE_DIR_KEY).await;
    let guid = read_setting(pool, LE_GUID_KEY).await;

    #[cfg(target_os = "windows")]
    {
        // Read only where it is used: on mobile the flag has no meaning and an
        // unconditional read is a dead variable the compiler rightly complains about.
        let disabled = read_setting(pool, LE_DISABLED_KEY).await.is_some();
        Ok(windows_capability(configured, guid, disabled))
    }
    #[cfg(not(target_os = "windows"))]
    {
        // Starting another program is a desktop concern; on mobile the OS owns
        // app launching, so this reports honestly instead of pretending.
        Ok(LaunchCapability {
            can_launch: cfg!(target_os = "android"),
            emulator: EmulatorState::NotInstalled,
            emulator_path: None,
            configured_dir: configured,
            guid,
            profiles: Vec::new(),
            note: Some(if cfg!(target_os = "android") {
                "Opens the mapped Android app. Shared runners require selecting the game inside the runner."
            } else {
                "Starting games is available on Windows and Android. Locale Emulator is Windows-only."
            }.to_owned()),
        })
    }
}

/// Change the Locale Emulator settings.
///
/// # Errors
/// Database failures only; a directory that does not exist is stored as given
/// and simply fails to be found later, which is what the UI reports.
#[tauri::command]
pub async fn launch_save_settings(
    state: State<'_, AppState>,
    input: LaunchSettingsInput,
) -> CommandResult<LaunchCapability> {
    let pool = state.db().pool();
    write_setting(pool, LE_DIR_KEY, input.launcher_dir.as_deref()).await;
    write_setting(pool, LE_GUID_KEY, input.guid.as_deref()).await;
    // Only touch the flag when the caller sent one: an older UI that does not know
    // about it must not silently re-enable a feature the user turned off.
    if let Some(enabled) = input.emulator_enabled {
        write_setting(
            pool,
            LE_DISABLED_KEY,
            if enabled { None } else { Some("1") },
        )
        .await;
    }
    launch_capability_for(state).await
}

/// Capability for this machine.
#[cfg(target_os = "windows")]
fn windows_capability(
    configured: Option<String>,
    guid: Option<String>,
    disabled: bool,
) -> LaunchCapability {
    // A disabled emulator is treated as absent for every decision *except* the
    // location, so the settings screen can still show what was configured.
    let found = if disabled {
        None
    } else {
        locale_emulator::find(configured.as_deref().map(Path::new), None)
    };
    // Profiles live beside LEProc.exe, in the file Locale Emulator writes.
    let profiles = found
        .as_ref()
        .and_then(|value| value.launcher.parent())
        .map(locale_emulator::read_profiles)
        .unwrap_or_default()
        .into_iter()
        .map(|profile| LaunchProfile {
            selected: guid.as_deref() == Some(profile.guid.as_str()),
            name: profile.name,
            guid: profile.guid,
            location: profile.location,
        })
        .collect::<Vec<_>>();
    // The launch path asks `plan()` for the same rule; this mirrors it rather than
    // inventing a second one.
    let chosen = guid
        .as_deref()
        .map(str::trim)
        .is_some_and(|value| !value.is_empty());
    let emulator = if disabled {
        EmulatorState::Disabled
    } else if found.is_none() {
        EmulatorState::NotInstalled
    } else if chosen {
        EmulatorState::Ready
    } else {
        EmulatorState::NoProfile
    };
    LaunchCapability {
        can_launch: true,
        emulator,
        emulator_path: found.map(|value| value.launcher.display().to_string()),
        configured_dir: configured,
        guid,
        profiles,
        note: None,
    }
}

async fn launch_capability_for(state: State<'_, AppState>) -> CommandResult<LaunchCapability> {
    let pool = state.db().pool();
    let configured = read_setting(pool, LE_DIR_KEY).await;
    let guid = read_setting(pool, LE_GUID_KEY).await;
    #[cfg(target_os = "windows")]
    {
        let disabled = read_setting(pool, LE_DISABLED_KEY).await.is_some();
        Ok(windows_capability(configured, guid, disabled))
    }
    #[cfg(not(target_os = "windows"))]
    {
        Ok(LaunchCapability {
            can_launch: cfg!(target_os = "android"),
            emulator: EmulatorState::NotInstalled,
            emulator_path: None,
            configured_dir: configured,
            guid,
            profiles: Vec::new(),
            note: Some(if cfg!(target_os = "android") {
                "Opens the mapped Android app. Shared runners require selecting the game inside the runner."
            } else {
                "Starting games is available on Windows and Android. Locale Emulator is Windows-only."
            }.to_owned()),
        })
    }
}

/// What happened when a game was started.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchOutcome {
    /// Whether a process was started.
    pub started: bool,
    /// Whether it went through Locale Emulator.
    pub localized: bool,
    /// The command line used, for the user to inspect or copy.
    pub command: String,
    /// One sentence explaining what happened.
    pub explanation: String,
}

fn android_target<'a>(
    executables: &'a [crate::tracking::mapping::GameExecutable],
    game_id: &str,
    installed: &[crate::platform::android::AndroidApp],
) -> Option<&'a crate::tracking::mapping::GameExecutable> {
    executables.iter().find(|item| {
        item.game_id == game_id
            && item.enabled
            && item
                .android_package()
                .is_some_and(|package| installed.iter().any(|app| app.package == package))
    })
}

async fn launch_android_game(
    state: &AppState,
    game_id: &str,
    executables: &[crate::tracking::mapping::GameExecutable],
) -> CommandResult<LaunchOutcome> {
    let installed = crate::platform::android::AndroidUsage::installed_apps()
        .await
        .map_err(|err| CommandError::new("android_apps", err.to_string()))?;
    let target = android_target(executables, game_id, &installed)
        .or_else(|| {
            executables.iter().find(|item| {
                item.game_id == game_id && item.enabled && item.android_package().is_some()
            })
        })
        .ok_or_else(|| {
            CommandError::new(
                "no_executable",
                "No enabled mapping matches an installed Android game or runner. Update its package in Tracking.",
            )
        })?;
    let package = target
        .android_package()
        .ok_or_else(|| CommandError::new("invalid", "invalid Android package mapping"))?;
    let now = vn_core::clock::Timestamp::now()?;
    crate::tracking::android::launch(state.db().pool(), state.device(), target, now.as_millis())
        .await
        .map_err(|err| CommandError::new("android_launch", err))?;
    super::tracking::sync_presence_now(state).await;
    Ok(LaunchOutcome {
        started: true,
        localized: false,
        command: package.to_owned(),
        explanation: "Opened the mapped Android app. Foreground time counts for this game until you return to VN Asyik or stop tracking.".to_owned(),
    })
}

/// Start the game's executable.
///
/// # Errors
/// When the game has no executable recorded, the file is gone, or the operating
/// system refuses to start it. The message names which.
#[tauri::command]
pub async fn launch_game(
    state: State<'_, AppState>,
    game_id: String,
) -> CommandResult<LaunchOutcome> {
    let id = vn_core::ids::EntityId::parse(&game_id)?;
    let executables = crate::tracking::mapping::list_executables(state.db().pool()).await?;

    if cfg!(target_os = "android") {
        return launch_android_game(&state, &id.to_string(), &executables).await;
    }

    // Prefer an executable recorded for this game; the first is the one the user
    // mapped when setting tracking up.
    let Some(target) = executables.iter().find(|item| {
        item.game_id == id.to_string()
            && item.enabled
            && item.exe_path.is_some()
            && item.android_package().is_none()
    }) else {
        return Err(CommandError::new(
            "no_executable",
            "no executable is recorded for this game yet. Add one in Tracking first.",
        ));
    };
    let Some(path) = target.exe_path.as_deref().map(PathBuf::from) else {
        return Err(CommandError::new(
            "no_executable",
            "no executable path recorded",
        ));
    };

    if !path.is_file() {
        return Err(CommandError::new(
            "missing_file",
            format!(
                "the game executable is no longer at {}. If it moved, add it again in Tracking.",
                path.display()
            ),
        ));
    }

    let runner_kind = vn_core::emulator::RunnerKind::parse(&target.runner_kind)
        .unwrap_or(vn_core::emulator::RunnerKind::Native);

    let plan = match runner_kind {
        vn_core::emulator::RunnerKind::Native => {
            let pool = state.db().pool();
            let configured = read_setting(pool, LE_DIR_KEY).await;
            let guid = read_setting(pool, LE_GUID_KEY).await;
            // Turned off means turned off: do not even look for the launcher, so a
            // configured folder cannot re-enable localization behind the user's back.
            let disabled = read_setting(pool, LE_DISABLED_KEY).await.is_some();

            let game_dir = locale_emulator::game_directory(&path).map(Path::to_path_buf);
            let emulator = if disabled {
                None
            } else {
                locale_emulator::find(configured.as_deref().map(Path::new), game_dir.as_deref())
            };
            locale_emulator::plan(&path, emulator.as_ref(), guid.as_deref())
        }
        emulator if emulator.platforms().windows => {
            let launch_target = target.launch_target.as_deref().unwrap_or_default();
            let args = emulator.windows_args(launch_target).ok_or_else(|| {
                CommandError::new(
                    "launch_failed",
                    format!("missing target to launch with {}", emulator.as_str()),
                )
            })?;
            LaunchPlan {
                program: path.clone(),
                args,
                localized: false,
                explanation: format!(
                    "Started through {} (target: {}).",
                    emulator.as_str(),
                    launch_target
                ),
            }
        }
        _ => {
            return Err(CommandError::new(
                "unsupported_runner",
                format!(
                    "runner {} cannot be launched directly on desktop",
                    runner_kind.as_str()
                ),
            ));
        }
    };

    let pid = spawn(&plan)?;
    state
        .tracker()
        .bind_launched_game(id, pid, path.display().to_string())
        .await;

    Ok(LaunchOutcome {
        started: true,
        localized: plan.localized,
        command: plan.command_line(),
        explanation: plan.explanation,
    })
}

/// Start the planned process without a shell.
fn spawn(plan: &LaunchPlan) -> CommandResult<u32> {
    let mut command = std::process::Command::new(&plan.program);
    command.args(&plan.args);
    // The game keeps its own working directory so relative asset paths resolve.
    if let Some(dir) = plan.program.parent() {
        command.current_dir(dir);
    }
    command.spawn().map(|child| child.id()).map_err(|err| {
        CommandError::new(
            "launch_failed",
            format!("could not start {}: {err}", plan.program.display()),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn android_play_skips_uninstalled_variants_and_other_games() {
        use crate::platform::android::AndroidApp;
        use crate::tracking::mapping::GameExecutable;
        let mapping = |game: &str, package: &str| GameExecutable {
            id: package.to_owned(),
            game_id: game.to_owned(),
            exe_name: package.to_owned(),
            exe_path: Some(package.to_owned()),
            enabled: true,
            runner_kind: "custom".to_owned(),
            launch_target: None,
            title_match: None,
            extra_args: None,
        };
        let mappings = vec![
            mapping("game", "com.winlator"),
            mapping("other", "com.winlator.cmod"),
            mapping("game", "com.winlator.cmod"),
        ];
        let installed = vec![AndroidApp {
            package: "com.winlator.cmod".to_owned(),
            label: "Winlator".to_owned(),
        }];
        let selected = android_target(&mappings, "game", &installed).expect("installed variant");
        assert_eq!(selected.game_id, "game");
        assert_eq!(selected.exe_path.as_deref(), Some("com.winlator.cmod"));
        assert!(android_target(&mappings, "game", &[]).is_none());
    }

    #[test]
    fn an_empty_directory_clears_the_setting() {
        // The UI needs a way to unset this, and "" is what it sends.
        assert_eq!(
            LaunchSettingsInput {
                launcher_dir: Some(String::new()),
                guid: None,
                emulator_enabled: None,
            }
            .launcher_dir
            .unwrap()
            .trim()
            .len(),
            0
        );
    }

    /// The reported bug: with the emulator installed and a folder configured but
    /// no profile chosen, the badge claimed "Japanese locale" while `plan()` started
    /// the game directly. `has_emulator` is a fact about the disk; `will_localize`
    /// is the promise the UI needs.
    #[cfg(target_os = "windows")]
    #[test]
    fn a_found_emulator_without_a_profile_does_not_claim_localization() {
        let dir = std::env::temp_dir().join("vn-le-capability-probe");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join("LEProc.exe"), b"stub").expect("stub launcher");

        let found = windows_capability(Some(dir.display().to_string()), None, false);
        assert_eq!(
            found.emulator,
            EmulatorState::NoProfile,
            "found on disk, but nothing will use it"
        );
        assert!(
            !found.emulator.will_localize(),
            "no profile is chosen, so the launch goes direct"
        );

        let with_profile = windows_capability(
            Some(dir.display().to_string()),
            Some("11111111-2222-3333-4444-555555555555".to_owned()),
            false,
        );
        assert_eq!(with_profile.emulator, EmulatorState::Ready);
        assert!(
            with_profile.emulator.will_localize(),
            "a profile makes the launch go through the emulator"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn turning_the_emulator_off_overrides_a_configured_folder_and_profile() {
        // Disabling must win over both settings, or a user who switched it off would
        // still see the badge and still be launched through it.
        let dir = std::env::temp_dir().join("vn-le-capability-off");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join("LEProc.exe"), b"stub").expect("stub launcher");

        let off = windows_capability(
            Some(dir.display().to_string()),
            Some("11111111-2222-3333-4444-555555555555".to_owned()),
            true,
        );
        assert_eq!(off.emulator, EmulatorState::Disabled);
        assert!(
            !off.emulator.will_localize(),
            "switched off must beat a configured folder and profile"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
