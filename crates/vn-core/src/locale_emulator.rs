//! Locale Emulator integration.
//!
//! Japanese visual novels written before Unicode commonly mojibake on a
//! non-Japanese Windows: they ask for the system ANSI code page and get the
//! local one. Locale Emulator runs such a game with a simulated Japanese
//! locale, which is what "run in Japanese" means for these titles.
//!
//! # What this module does and does not do
//!
//! It **detects** an installation and builds the arguments for it. It does not
//! ship, download, or patch anything. Locale Emulator is a separate program the
//! user installs; its core binaries have no stated redistribution terms, so
//! bundling them is not something this project can verify.
//!
//! Nothing here injects into another process: `LEProc.exe` is a launcher that
//! the operating system runs, exactly as the user would.

use std::path::{Path, PathBuf};

/// The launcher executable's file name.
pub const LE_PROC: &str = "LEProc.exe";

/// The profile file Locale Emulator writes beside `LEProc.exe`.
pub const LE_CONFIG: &str = "LEConfig.xml";

/// A profile as defined in `LEConfig.xml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeProfile {
    /// Display name, for the user to pick from.
    pub name: String,
    /// `Guid` attribute, which is what `-runas` takes.
    pub guid: String,
    /// `Location`, such as `ja-JP`.
    pub location: String,
}

/// Read the profiles from `LEConfig.xml` in `dir`.
///
/// This is the file Locale Emulator itself writes, so it is the only reliable
/// source for its profile GUIDs. Returns an empty list rather than an error when
/// the file is missing or unreadable: a malformed config means "no profiles
/// known", and the caller falls back to reporting that.
#[must_use]
pub fn read_profiles(dir: &Path) -> Vec<LeProfile> {
    let Ok(text) = std::fs::read_to_string(dir.join(LE_CONFIG)) else {
        return Vec::new();
    };
    parse_profiles(&text)
}

/// Extract profiles from `LEConfig.xml` contents.
///
/// Deliberately a small scan rather than an XML parser: the file has one flat
/// `<Profile ...>` list with no nesting quirks, and a dependency for eleven
/// attributes is not worth it. A `<Profile>` without a `Guid` is skipped, since
/// it cannot be launched.
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

/// The value of `key="..."` inside a tag, unescaped for the two entities the
/// config actually uses.
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

/// The GUID used when the user has not chosen a profile.
///
/// This was a zero GUID, on the theory that Locale Emulator would treat it as a
/// default. It does not: `LEProc.exe -runas {00000000-...}` exits immediately
/// and starts nothing, with no message, so pressing Play silently did nothing.
/// The caller now asks the user to pick a profile instead of guessing one.
pub const NO_PROFILE: &str = "";

/// Where Locale Emulator is found, and how confident that is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocaleEmulator {
    /// Full path to `LEProc.exe`.
    pub launcher: PathBuf,
    /// Which candidate location matched, for the UI to explain.
    pub source: InstallSource,
}

/// How the installation was located.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallSource {
    /// A directory the user configured.
    Configured,
    /// Next to the game's own executable, which some portable copies use.
    BesideGame,
}

/// Locate `LEProc.exe` without running anything.
///
/// Checked in order: the configured directory, then beside the game. Both are
/// compared case-insensitively on the file name because Windows file systems
/// are, and a portable copy may ship `leproc.exe`.
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

/// The launcher inside `dir`, if there is one.
fn launcher_in(dir: &Path) -> Option<PathBuf> {
    let direct = dir.join(LE_PROC);
    if direct.is_file() {
        return Some(direct);
    }
    // A portable copy may use different casing; read the directory rather than
    // guessing names.
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

/// The directory a game executable lives in.
#[must_use]
pub fn game_directory(exe: &Path) -> Option<&Path> {
    exe.parent()
}

/// How to launch a game, and whether a locale was simulated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchPlan {
    /// Program to run.
    pub program: PathBuf,
    /// Arguments, in order.
    pub args: Vec<String>,
    /// Whether this goes through Locale Emulator.
    pub localized: bool,
    /// One sentence the UI shows so the user knows which happened.
    pub explanation: String,
}

impl LaunchPlan {
    /// The arguments as a single display string, for logs and the UI.
    #[must_use]
    pub fn command_line(&self) -> String {
        if self.args.is_empty() {
            return self.program.display().to_string();
        }
        format!("{} {}", self.program.display(), self.args.join(" "))
    }
}

/// Build the command that starts `exe`.
///
/// With a locale emulator, the launch is `LEProc.exe -runas <guid> <exe>`;
/// without one it is the executable itself. The non-localized case is not an
/// error: a translated or Unicode build runs fine, and saying so is better than
/// refusing.
///
/// A localized plan with no `guid` is refused rather than guessed. Locale
/// Emulator accepts any well-formed GUID and exits silently when no profile
/// matches it, so a placeholder produces a Play button that does nothing.
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
        assert!(parse_profiles(xml).is_empty());
    }

    #[test]
    fn a_missing_config_file_yields_no_profiles() {
        let dir = temp_dir("no-config");
        assert!(read_profiles(&dir).is_empty());
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
        assert!(plan.args.is_empty());
        // The message must say what to do, not just that something is missing.
        assert!(plan.explanation.contains("Locale Emulator"));
    }

    #[test]
    fn the_game_directory_is_the_executables_parent() {
        assert_eq!(
            game_directory(Path::new(r"C:\Games\VN\game.exe")),
            Some(Path::new(r"C:\Games\VN"))
        );
    }

    #[test]
    fn a_bare_file_name_has_no_directory() {
        assert_eq!(game_directory(Path::new("game.exe")), Some(Path::new("")));
    }
}
