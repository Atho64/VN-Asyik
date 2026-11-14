//! Locale Emulator integration (`LEProc.exe`).
//!
//! Detects an existing install and builds `-runas` launch plans for non-Unicode
//! Japanese games. Never bundles or patches binaries (unverified redistribution
//! terms) and never injects into processes. [`read_profiles`] and [`find`] are
//! the crate's sanctioned portable `std::fs` exceptions.

use std::path::{Path, PathBuf};

pub const LE_PROC: &str = "LEProc.exe";
pub const LE_CONFIG: &str = "LEConfig.xml";

/// A profile as defined in `LEConfig.xml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeProfile {
    pub name: String,
    /// `Guid` attribute passed to `-runas`.
    pub guid: String,
    /// `Location`, such as `ja-JP`.
    pub location: String,
}

/// Read profiles from `LEConfig.xml` in `dir`; returns empty if missing or unreadable.
#[must_use]
pub fn read_profiles(dir: &Path) -> Vec<LeProfile> {
    let Ok(text) = std::fs::read_to_string(dir.join(LE_CONFIG)) else {
        return Vec::new();
    };
    parse_profiles(&text)
}

/// Scan flat `<Profile>` tags from `LEConfig.xml` without a full XML parser;
/// entries missing `Guid` are skipped because they cannot be launched.
#[must_use]
pub fn parse_profiles(xml: &str) -> Vec<LeProfile> {
    let mut profiles = Vec::new();
    for chunk in xml.split("<Profile").skip(1) {
        let Some(tag) = chunk.split('>').next() else {
            continue;
        };
        let Some(guid) = attribute(tag, "Guid") else {
            continue;
        };
        let body = chunk.split("</Profile>").next().unwrap_or_default();
        let name = attribute(tag, "Name").unwrap_or_else(|| guid.clone());
        let location = element(body, "Location").unwrap_or_default();
        profiles.push(LeProfile {
            name,
            guid,
            location,
        });
    }
    profiles
}

/// Extract `key="..."` inside a tag and unescape XML entities.
fn attribute(tag: &str, key: &str) -> Option<String> {
    let needle = format!("{key}=\"");
    let start = tag.find(&needle)? + needle.len();
    let rest = &tag[start..];
    let end = rest.find('"')?;
    Some(unescape(&rest[..end]))
}

/// The text content of the first `<name>...</name>` in `body`.
fn element(body: &str, name: &str) -> Option<String> {
    let open = format!("<{name}>");
    let close = format!("</{name}>");
    let start = body.find(&open)? + open.len();
    let rest = &body[start..];
    let end = rest.find(&close)?;
    Some(unescape(&rest[..end]))
}

fn unescape(text: &str) -> String {
    text.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}

/// Empty sentinel when no profile is chosen; Locale Emulator exits silently on a zero GUID.
pub const NO_PROFILE: &str = "";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocaleEmulator {
    pub launcher: PathBuf,
    pub source: InstallSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallSource {
    Configured,
    /// Next to the game's executable (portable installs).
    BesideGame,
}

/// Locate `LEProc.exe` in `configured_dir` then `game_dir`, matching names case-insensitively.
#[must_use]
pub fn find(configured_dir: Option<&Path>, game_dir: Option<&Path>) -> Option<LocaleEmulator> {
    if let Some(dir) = configured_dir
        && let Some(launcher) = launcher_in(dir)
    {
        return Some(LocaleEmulator {
            launcher,
            source: InstallSource::Configured,
        });
    }
    if let Some(dir) = game_dir
        && let Some(launcher) = launcher_in(dir)
    {
        return Some(LocaleEmulator {
            launcher,
            source: InstallSource::BesideGame,
        });
    }
    None
}

fn launcher_in(dir: &Path) -> Option<PathBuf> {
    let direct = dir.join(LE_PROC);
    if direct.is_file() {
        return Some(direct);
    }
    // Portable copies may use different casing; scan entries rather than guessing.
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.eq_ignore_ascii_case(LE_PROC) {
            let path = entry.path();
            if path.is_file() {
                return Some(path);
            }
        }
    }
    None
}

#[must_use]
pub fn game_directory(exe: &Path) -> Option<&Path> {
    exe.parent()
}

/// Command and explanation for launching a game directly or via Locale Emulator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchPlan {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub localized: bool,
    pub explanation: String,
}

impl LaunchPlan {
    #[must_use]
    pub fn command_line(&self) -> String {
        if self.args.is_empty() {
            return self.program.display().to_string();
        }
        format!("{} {}", self.program.display(), self.args.join(" "))
    }
}

/// Build the launch command for `exe` (`LEProc.exe -runas <guid> <exe>` or direct).
/// Falls back to direct launch when `guid` is absent because Locale Emulator exits
/// silently on unknown GUIDs instead of falling back to a default profile.
#[must_use]
pub fn plan(exe: &Path, emulator: Option<&LocaleEmulator>, guid: Option<&str>) -> LaunchPlan {
    match emulator {
        Some(found) => {
            let chosen = guid.map(str::trim).filter(|value| !value.is_empty());
            let Some(guid) = chosen else {
                return LaunchPlan {
                    program: exe.to_path_buf(),
                    args: Vec::new(),
                    localized: false,
                    explanation: "No Japanese profile is chosen yet, so this started directly. Pick one in Settings to fix garbled text.".to_owned(),
                };
            };
            LaunchPlan {
                program: found.launcher.clone(),
                args: vec![
                    "-runas".to_owned(),
                    guid.to_owned(),
                    exe.display().to_string(),
                ],
                localized: true,
                explanation: match found.source {
                    InstallSource::Configured => {
                        "Started through Locale Emulator with a Japanese locale.".to_owned()
                    }
                    InstallSource::BesideGame => {
                        "Started through the Locale Emulator found next to the game.".to_owned()
                    }
                },
            }
        }
        None => LaunchPlan {
            program: exe.to_path_buf(),
            args: Vec::new(),
            localized: false,
            explanation:
                "Started directly. If the text looks garbled, install Locale Emulator and set its folder in Settings."
                    .to_owned(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("vn-locale-test-{name}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_configured_install_is_found_first() {
        let configured = temp_dir("configured");
        fs::write(configured.join(LE_PROC), b"x").unwrap();
        let game = temp_dir("game-a");
        fs::write(game.join(LE_PROC), b"x").unwrap();

        let found = find(Some(&configured), Some(&game)).unwrap();
        assert_eq!(found.source, InstallSource::Configured);
        assert_eq!(found.launcher, configured.join(LE_PROC));
    }

    #[test]
    fn a_portable_copy_beside_the_game_is_found() {
        let game = temp_dir("game-portable");
        fs::write(game.join("leproc.exe"), b"x").unwrap();

        let found = find(None, Some(&game)).unwrap();
        assert_eq!(found.source, InstallSource::BesideGame);
    }

    #[test]
    fn nothing_installed_means_none() {
        let game = temp_dir("game-empty");
        assert!(find(None, Some(&game)).is_none());
    }

    #[test]
    fn a_missing_directory_is_not_an_error() {
        // A configured path that no longer exists must degrade, not panic.
        let missing = PathBuf::from("/nonexistent/vn-tracker-le");
        assert!(find(Some(&missing), None).is_none());
    }

    #[test]
    fn a_directory_named_like_the_launcher_is_not_mistaken_for_one() {
        let dir = temp_dir("game-dirnamed");
        fs::create_dir_all(dir.join(LE_PROC)).unwrap();
        assert!(
            find(None, Some(&dir)).is_none(),
            "a directory must not be treated as the launcher"
        );
    }

    #[test]
    fn a_localized_plan_runs_the_launcher_with_the_game() {
        let plan = plan(
            Path::new(r"C:\Games\VN\game.exe"),
            Some(&LocaleEmulator {
                launcher: PathBuf::from(r"C:\LE\LEProc.exe"),
                source: InstallSource::Configured,
            }),
            Some("{abc}"),
        );
        assert!(plan.localized);
        assert_eq!(plan.program, PathBuf::from(r"C:\LE\LEProc.exe"));
        assert_eq!(plan.args[0], "-runas");
        assert_eq!(plan.args[1], "{abc}");
        assert_eq!(plan.args[2], r"C:\Games\VN\game.exe");
    }

    #[test]
    fn a_localized_plan_needs_a_profile_guid() {
        // Locale Emulator accepts a well-formed GUID with no matching profile and
        // exits silently. Passing a placeholder therefore made Play do nothing,
        // which is exactly what happened before this test existed.
        let plan = plan(
            Path::new(r"C:\Games\VN\game.exe"),
            Some(&LocaleEmulator {
                launcher: PathBuf::from(r"C:\LE\LEProc.exe"),
                source: InstallSource::Configured,
            }),
            None,
        );
        assert!(!plan.localized, "no profile means no emulator launch");
        assert_eq!(plan.program, PathBuf::from(r"C:\Games\VN\game.exe"));
        assert!(plan.explanation.contains("profile"));
    }

    #[test]
    fn an_empty_or_blank_guid_is_treated_as_no_profile() {
        for guid in ["", "   "] {
            let plan = plan(
                Path::new("game.exe"),
                Some(&LocaleEmulator {
                    launcher: PathBuf::from("LEProc.exe"),
                    source: InstallSource::Configured,
                }),
                Some(guid),
            );
            assert!(
                !plan.localized,
                "{guid:?} must not be launched as a profile"
            );
        }
    }

    #[test]
    fn profiles_are_read_from_the_config_locale_emulator_writes() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<LEConfig>
  <Profiles>
    <Profile Name="Run in Japanese" Guid="d95ea669-ef1f-4282-8cdf-90ca99d550fd" MainMenu="false">
      <Parameter></Parameter>
      <Location>ja-JP</Location>
    </Profile>
    <Profile Name="Run in Japanese (Admin)" Guid="8ea71f3b-e33a-4c6d-8d2c-fccc09d91a11" MainMenu="false">
      <Location>ja-JP</Location>
    </Profile>
  </Profiles>
</LEConfig>"#;
        let profiles = parse_profiles(xml);
        assert_eq!(profiles.len(), 2);
        assert_eq!(profiles[0].name, "Run in Japanese");
        assert_eq!(profiles[0].guid, "d95ea669-ef1f-4282-8cdf-90ca99d550fd");
        assert_eq!(profiles[0].location, "ja-JP");
        assert_eq!(profiles[1].name, "Run in Japanese (Admin)");
    }

    #[test]
    fn a_profile_without_a_guid_is_skipped() {
        // Without a GUID it cannot be passed to -runas, so it is not offerable.
        let xml =
            r#"<LEConfig><Profile Name="Broken"><Location>ja-JP</Location></Profile></LEConfig>"#;
        assert_eq!(
            parse_profiles(xml).len(),
            0,
            "parse_profiles(xml) should be empty"
        );
    }

    #[test]
    fn a_missing_config_file_yields_no_profiles() {
        let dir = temp_dir("no-config");
        assert_eq!(
            read_profiles(&dir).len(),
            0,
            "read_profiles(&dir) should be empty"
        );
    }

    #[test]
    fn a_profile_name_with_an_entity_is_unescaped() {
        let xml = r#"<LEConfig><Profile Name="Run &amp; Play" Guid="{abc}"><Location>ja-JP</Location></Profile></LEConfig>"#;
        assert_eq!(parse_profiles(xml)[0].name, "Run & Play");
    }

    #[test]
    fn without_an_emulator_the_game_runs_directly() {
        let plan = plan(Path::new("game.exe"), None, None);
        assert!(!plan.localized);
        assert_eq!(plan.program, PathBuf::from("game.exe"));
        assert_eq!(plan.args.len(), 0, "plan.args should be empty");
        // The message must say what to do, not just that something is missing.
        assert!(plan.explanation.contains("Locale Emulator"));
    }

    // Assert Windows path semantics only on Windows; on POSIX `C:\...` has no
    // separator so `parent()` returns `Some("")` and tests the host instead.
    #[cfg(windows)]
    #[test]
    fn the_game_directory_is_the_executables_parent() {
        assert_eq!(
            game_directory(Path::new(r"C:\Games\VN\game.exe")),
            Some(Path::new(r"C:\Games\VN"))
        );
    }

    #[cfg(windows)]
    #[test]
    fn a_bare_file_name_has_no_directory() {
        assert_eq!(game_directory(Path::new("game.exe")), Some(Path::new("")));
    }
}
