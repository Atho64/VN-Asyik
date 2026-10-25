//! Emulators and VN runners that host many games in one binary or package.
//!
//! A native game is one executable. An emulator is the opposite: `Vita3K.exe`,
//! `eden.exe` or the `com.winlator` package runs dozens of games, and the game is
//! chosen by an argument (a title id, a ROM path) rather than by which binary
//! started. This module holds the pure facts about those hosts: how to start a
//! given game, which Android packages they ship as, and how to tell which game is
//! running from a window caption.
//!
//! # Provenance and verification
//!
//! Command lines come from each emulator's public documentation and from
//! frontend integrations (`LaunchBox`, `EmulationStation`, the `PPSSPP` docs). None of
//! them has been run on this machine yet, so every entry reports
//! [`Verification::Documented`] and **not** `Verified`. A unit test enforces that
//! nothing claims `Verified` without a recorded build (AGENTS.md §8.3, §13).
//!
//! The `Winlator`, `Kirikiroid2`, `Tyranor`, `ONScripter`, `Artemis`, and `GameHub` launch
//! protocols and package lists are adapted from `KireiBox` (`Yukin0a/KireiBox`,
//! GPL-3.0, files `SupportedEngines.kt`, `EmulatorLauncher.kt`,
//! `external/winlator/WinlatorContract.kt` and `WinlatorLauncher.kt`); the Android
//! side lives in the Kotlin plugin, only the identifiers are mirrored here.
//!
//! Nothing here touches the filesystem, a process or the network.

use serde::{Deserialize, Serialize};

/// Android application IDs use dot-separated identifier components. Keeping
/// package IDs distinct from desktop paths prevents a Windows mapping from
/// accidentally being selected for Android launching after a backup import.
#[must_use]
pub fn is_android_package(value: &str) -> bool {
    let mut parts = value.split('.');
    let valid = |part: &str| {
        let mut chars = part.chars();
        chars.next().is_some_and(|ch| ch.is_ascii_alphabetic())
            && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    };
    parts.next().is_some_and(valid) && parts.clone().next().is_some() && parts.all(valid)
}

/// What kind of program runs a game.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunnerKind {
    /// A game that is its own executable. The default, and what every existing
    /// mapping is.
    Native,
    /// `Vita3K` (`PlayStation` Vita).
    Vita3k,
    /// RPCS3 (`PlayStation` 3).
    Rpcs3,
    /// PCSX2 (`PlayStation` 2).
    Pcsx2,
    /// Eden (Nintendo Switch).
    Eden,
    /// Citron / Citron Neo (Nintendo Switch).
    Citron,
    /// `shadPS4` (`PlayStation` 4). Windows only.
    ShadPs4,
    /// `PPSSPP` (`PlayStation` Portable).
    Ppsspp,
    /// `Kirikiroid2` (`KiriKiri` games on Android).
    Kirikiroid2,
    /// `Tyranor` / `TyranoEmu` (Tyrano games on Android).
    Tyranor,
    /// `ONScripter` / `ONScripter-Yuri` / `ONScripter Plus`.
    Onscripter,
    /// `Artemis Engine` runner.
    Artemis,
    /// Any `Winlator` build (Windows games on Android).
    Winlator,
    /// `GameHub` PC/container launcher on Android.
    GameHub,
    /// A game that is itself an Android application.
    #[serde(alias = "native_apk")]
    AndroidApk,
    /// Any installed Android launcher app or custom runner.
    Custom,
}

/// How far a launch recipe has been checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verification {
    /// Taken from documentation; never run here.
    Documented,
    /// Run against a real build on this project's machine.
    Verified,
}

/// Where a runner exists at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Platforms {
    /// Runs on Windows.
    pub windows: bool,
    /// Runs on Android.
    pub android: bool,
}

impl RunnerKind {
    /// Every kind, in the order the UI lists them.
    pub const ALL: [Self; 16] = [
        Self::Native,
        Self::Vita3k,
        Self::Rpcs3,
        Self::Pcsx2,
        Self::Eden,
        Self::Citron,
        Self::ShadPs4,
        Self::Ppsspp,
        Self::Kirikiroid2,
        Self::Tyranor,
        Self::Onscripter,
        Self::Artemis,
        Self::Winlator,
        Self::GameHub,
        Self::AndroidApk,
        Self::Custom,
    ];

    /// The stable string stored in the database.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Native => "native",
            Self::Vita3k => "vita3k",
            Self::Rpcs3 => "rpcs3",
            Self::Pcsx2 => "pcsx2",
            Self::Eden => "eden",
            Self::Citron => "citron",
            Self::ShadPs4 => "shadps4",
            Self::Ppsspp => "ppsspp",
            Self::Kirikiroid2 => "kirikiroid2",
            Self::Tyranor => "tyranor",
            Self::Onscripter => "onscripter",
            Self::Artemis => "artemis",
            Self::Winlator => "winlator",
            Self::GameHub => "gamehub",
            Self::AndroidApk => "android_apk",
            Self::Custom => "custom",
        }
    }

    /// Parse the stored string. Unknown values are `None`, never a guess.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "native_apk" => Some(Self::AndroidApk),
            "citron_neo" | "citron-neo" => Some(Self::Citron),
            "ons" | "onscripter_yuri" => Some(Self::Onscripter),
            "krkr2" | "krkr" => Some(Self::Kirikiroid2),
            "tyrano" | "tyranoemu" => Some(Self::Tyranor),
            "egggame" => Some(Self::GameHub),
            trimmed => Self::ALL.into_iter().find(|kind| kind.as_str() == trimmed),
        }
    }

    /// Where this runner exists.
    #[must_use]
    pub const fn platforms(self) -> Platforms {
        match self {
            Self::Vita3k
            | Self::Eden
            | Self::Citron
            | Self::Ppsspp
            | Self::Onscripter
            | Self::Artemis => Platforms {
                windows: true,
                android: true,
            },
            Self::Native | Self::Rpcs3 | Self::Pcsx2 | Self::ShadPs4 => Platforms {
                windows: true,
                android: false,
            },
            Self::Kirikiroid2
            | Self::Tyranor
            | Self::Winlator
            | Self::GameHub
            | Self::AndroidApk
            | Self::Custom => Platforms {
                windows: false,
                android: true,
            },
        }
    }

    /// Whether one binary or package hosts many games.
    #[must_use]
    pub const fn is_shared_host(self) -> bool {
        !matches!(self, Self::Native | Self::AndroidApk)
    }

    /// How well the Windows launch recipe is established.
    #[must_use]
    pub const fn verification(self) -> Verification {
        Verification::Documented
    }

    /// Lowercase executable names that identify this emulator on Windows.
    #[must_use]
    pub const fn windows_exe_names(self) -> &'static [&'static str] {
        match self {
            Self::Vita3k => &["vita3k.exe"],
            Self::Rpcs3 => &["rpcs3.exe"],
            Self::Pcsx2 => &["pcsx2-qt.exe", "pcsx2.exe"],
            Self::Eden => &["eden.exe"],
            Self::Citron => &["citron.exe", "citron-neo.exe"],
            Self::ShadPs4 => &["shadps4.exe"],
            Self::Ppsspp => &["ppssppwindows64.exe", "ppssppwindows.exe", "ppssppgold.exe"],
            Self::Onscripter => &["onscripter.exe", "onscripter-ru.exe", "nscripter.exe"],
            Self::Artemis => &["artemis.exe", "artemisengine.exe"],
            Self::Native
            | Self::Kirikiroid2
            | Self::Tyranor
            | Self::Winlator
            | Self::GameHub
            | Self::AndroidApk
            | Self::Custom => &[],
        }
    }

    /// Android package names this runner is known to ship under.
    ///
    /// Includes package identifiers verified from `Yukin0a/KireiBox` (GPL-3.0).
    #[must_use]
    pub const fn android_packages(self) -> &'static [&'static str] {
        match self {
            Self::Vita3k => &["org.vita3k.emulator"],
            Self::Eden => &[
                "dev.eden.eden_emulator",
                "dev.legacy.eden_emulator",
                "org.eden.emulator",
            ],
            Self::Citron => &[
                "org.citron.citron_emu",
                "org.citron.emulator",
                "org.citron_neo.emulator",
                "dev.citron.citron_emu",
            ],
            Self::Ppsspp => &["org.ppsspp.ppsspp", "org.ppsspp.ppssppgold"],
            Self::Kirikiroid2 => &[
                "org.tvp.kirikiri2",
                "org.tvp.kirikiri2_yuri_10309",
                "org.tvp.kirikiri2_yuri_10304",
                "org.tvp.kirikiri2_yuri_10206",
                "org.zeas.kirikiri2",
            ],
            Self::Tyranor => &[
                "com.akira.tyranoemu",
                "com.pseudogram.tyranor",
                "tyranor.emu",
            ],
            Self::Onscripter => &[
                "com.onscripter.plus",
                "cn.natdon.onscripterv2",
                "org.onscripter.plus",
                "com.yuri.onscripter",
            ],
            Self::Artemis => &["com.ies_net.artemis", "com.akira.tyranoemu"],
            Self::Winlator => &[
                "com.winlator",
                "com.winlator.cmod",
                "app.gamenative",
                "com.micewine.emu",
                "com.OxGames.Pluvia",
                "org.cypherpunk.winlator",
                "com.winlator.frost",
                "com.winlator.glibc",
                "com.winlator.ludashi",
            ],
            Self::GameHub => &["com.xiaoji.egggame", "com.xiaoji.gamehub"],
            Self::Native
            | Self::Rpcs3
            | Self::Pcsx2
            | Self::ShadPs4
            | Self::AndroidApk
            | Self::Custom => &[],
        }
    }

    /// Arguments that start `launch_target` in this emulator on Windows.
    #[must_use]
    pub fn windows_args(self, launch_target: &str) -> Option<Vec<String>> {
        let target = launch_target.trim();
        if target.is_empty() || !self.platforms().windows {
            return None;
        }
        let args: &[&str] = match self {
            // `-r <title id>` boots an installed title.
            Self::Vita3k => &["-r"],
            // `--no-gui <path>` skips the game list and boots the file.
            Self::Rpcs3 => &["--no-gui"],
            // `-batch` exits with the game; `--` ends options so a path that
            // starts with `-` is still a path.
            Self::Pcsx2 => &["-batch", "--"],
            // yuzu-lineage frontends and shadPS4 take `-g <path>`.
            Self::Eden | Self::Citron | Self::ShadPs4 => &["-g"],
            // PPSSPP, ONScripter, and Artemis take the file/folder as a bare argument.
            Self::Ppsspp | Self::Onscripter | Self::Artemis => &[],
            Self::Native
            | Self::Kirikiroid2
            | Self::Tyranor
            | Self::Winlator
            | Self::GameHub
            | Self::AndroidApk
            | Self::Custom => return None,
        };
        let mut out: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
        out.push(target.to_owned());
        Some(out)
    }
}

/// The text to look for in a window caption to recognise one game.
#[must_use]
pub fn title_needle(title_match: Option<&str>, launch_target: Option<&str>) -> Option<String> {
    if let Some(explicit) = title_match.map(str::trim).filter(|value| !value.is_empty()) {
        return Some(explicit.to_owned());
    }
    let target = launch_target
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let name = target.rsplit(['/', '\\']).next().unwrap_or(target);
    let stem = match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && ext.len() <= 5 && !ext.contains(' ') => stem,
        _ => name,
    };
    let stem = stem.trim();
    if stem.is_empty() || GENERIC_TARGET_NAMES.contains(&stem.to_lowercase().as_str()) {
        return None;
    }
    Some(stem.to_owned())
}

/// File names that appear in many games' folders and so say nothing about which.
const GENERIC_TARGET_NAMES: [&str; 5] = ["eboot", "game", "main", "default", "launcher"];

/// Whether a window caption belongs to the game described by `needle`.
#[must_use]
pub fn matches_window_title(title: &str, needle: &str) -> bool {
    let needle = needle.trim();
    if needle.is_empty() {
        return false;
    }
    title.to_lowercase().contains(&needle.to_lowercase())
}

/// Pick the game a caption belongs to from several candidates.
#[must_use]
pub fn best_title_match(title: &str, needles: &[Option<String>]) -> Option<usize> {
    let mut best: Option<(usize, usize)> = None;
    let mut tied = false;
    for (index, needle) in needles.iter().enumerate() {
        let Some(needle) = needle.as_deref() else {
            continue;
        };
        if !matches_window_title(title, needle) {
            continue;
        }
        let len = needle.trim().chars().count();
        match best {
            None => best = Some((index, len)),
            Some((_, best_len)) if len > best_len => {
                best = Some((index, len));
                tied = false;
            }
            Some((best_index, best_len)) if len == best_len => {
                let same = needles[best_index]
                    .as_deref()
                    .is_some_and(|other| other.trim().eq_ignore_ascii_case(needle.trim()));
                if !same {
                    tied = true;
                }
            }
            Some(_) => {}
        }
    }
    if tied {
        None
    } else {
        best.map(|(index, _)| index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installed_package_variants_are_not_limited_to_the_suggestion_list() {
        for package in [
            "com.winlator.cmod",
            "com.tyranor.next",
            "dev.eden.eden_emulator.nightly",
            "com.akira.tyranoemu",
        ] {
            assert!(is_android_package(package), "{package}");
        }
        for path in [
            r"C:\Games\game.exe",
            "game.exe/rom",
            "com..runner",
            " org.example.vn",
            "org.example.vn ",
        ] {
            assert!(!is_android_package(path), "{path}");
        }
        assert_eq!(
            RunnerKind::parse("native_apk"),
            Some(RunnerKind::AndroidApk)
        );
        assert_eq!(RunnerKind::parse("custom"), Some(RunnerKind::Custom));
        assert!(RunnerKind::Custom.platforms().android);
        assert_eq!(RunnerKind::Custom.windows_args("game.iso"), None);
    }

    #[test]
    fn every_kind_round_trips_through_its_stored_string() {
        for kind in RunnerKind::ALL {
            assert_eq!(RunnerKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(RunnerKind::parse("nonsense"), None);
        assert_eq!(RunnerKind::parse(""), None);
    }

    #[test]
    fn stored_strings_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for kind in RunnerKind::ALL {
            assert!(seen.insert(kind.as_str()), "{kind:?} reuses a string");
        }
    }

    #[test]
    fn no_runner_claims_verification_yet() {
        for kind in RunnerKind::ALL {
            assert_eq!(kind.verification(), Verification::Documented, "{kind:?}");
        }
    }

    #[test]
    fn shared_hosts_are_everything_but_native_and_apk() {
        assert!(!RunnerKind::Native.is_shared_host());
        assert!(!RunnerKind::AndroidApk.is_shared_host());
        assert!(RunnerKind::Vita3k.is_shared_host());
        assert!(RunnerKind::Winlator.is_shared_host());
    }

    #[test]
    fn platform_claims_match_what_the_plan_promises() {
        assert!(
            !RunnerKind::ShadPs4.platforms().android,
            "shadPS4 is Windows only"
        );
        for kind in [
            RunnerKind::Vita3k,
            RunnerKind::Eden,
            RunnerKind::Citron,
            RunnerKind::Ppsspp,
            RunnerKind::Onscripter,
            RunnerKind::Artemis,
        ] {
            let platforms = kind.platforms();
            assert!(platforms.windows && platforms.android, "{kind:?}");
        }
        for kind in [
            RunnerKind::Kirikiroid2,
            RunnerKind::Tyranor,
            RunnerKind::Winlator,
            RunnerKind::GameHub,
            RunnerKind::AndroidApk,
        ] {
            let platforms = kind.platforms();
            assert!(!platforms.windows && platforms.android, "{kind:?}");
        }
    }

    #[test]
    fn windows_runners_have_exe_names_and_android_ones_do_not() {
        for kind in RunnerKind::ALL {
            let has_names = !kind.windows_exe_names().is_empty();
            let expected = kind.platforms().windows && kind != RunnerKind::Native;
            assert_eq!(has_names, expected, "{kind:?}");
        }
        for kind in RunnerKind::ALL {
            for name in kind.windows_exe_names() {
                assert_eq!(*name, name.to_lowercase(), "names are stored lowercase");
            }
        }
    }

    #[test]
    fn vita3k_boots_by_title_id() {
        assert_eq!(
            RunnerKind::Vita3k.windows_args("PCSG00123"),
            Some(vec!["-r".to_owned(), "PCSG00123".to_owned()])
        );
    }

    #[test]
    fn rpcs3_skips_the_game_list() {
        assert_eq!(
            RunnerKind::Rpcs3.windows_args(r"C:\g\EBOOT.BIN"),
            Some(vec!["--no-gui".to_owned(), r"C:\g\EBOOT.BIN".to_owned()])
        );
    }

    #[test]
    fn pcsx2_ends_options_before_the_path() {
        let args = RunnerKind::Pcsx2
            .windows_args("-odd name.iso")
            .expect("args");
        assert_eq!(args, ["-batch", "--", "-odd name.iso"]);
    }

    #[test]
    fn switch_and_ps4_emulators_use_dash_g() {
        for kind in [RunnerKind::Eden, RunnerKind::Citron, RunnerKind::ShadPs4] {
            let args = kind.windows_args("game.nsp").expect("args");
            assert_eq!(args, ["-g", "game.nsp"], "{kind:?}");
        }
    }

    #[test]
    fn ppsspp_takes_the_file_bare() {
        assert_eq!(
            RunnerKind::Ppsspp.windows_args("a.iso"),
            Some(vec!["a.iso".to_owned()])
        );
    }

    #[test]
    fn a_target_with_spaces_stays_one_argument() {
        let args = RunnerKind::Eden
            .windows_args(r#"C:\My Games\a "quoted" b.nsp"#)
            .expect("args");
        assert_eq!(args.len(), 2, "the path must not be split");
    }

    #[test]
    fn no_launch_without_a_target_or_on_the_wrong_platform() {
        assert_eq!(RunnerKind::Vita3k.windows_args("   "), None);
        assert_eq!(RunnerKind::Vita3k.windows_args(""), None);
        assert_eq!(RunnerKind::Native.windows_args("x"), None);
        assert_eq!(RunnerKind::Winlator.windows_args("x"), None);
        assert_eq!(RunnerKind::Kirikiroid2.windows_args("x"), None);
        assert_eq!(RunnerKind::AndroidApk.windows_args("x"), None);
    }

    #[test]
    fn android_package_suggestions_cover_the_requested_runners() {
        assert!(
            RunnerKind::Winlator
                .android_packages()
                .contains(&"com.winlator")
        );
        assert!(
            RunnerKind::Kirikiroid2
                .android_packages()
                .contains(&"org.tvp.kirikiri2")
        );
        assert!(
            RunnerKind::Ppsspp
                .android_packages()
                .contains(&"org.ppsspp.ppsspp")
        );
        assert!(
            RunnerKind::Tyranor
                .android_packages()
                .contains(&"com.akira.tyranoemu")
        );
        assert!(
            RunnerKind::Onscripter
                .android_packages()
                .contains(&"cn.natdon.onscripterv2")
        );
        assert!(
            RunnerKind::GameHub
                .android_packages()
                .contains(&"com.xiaoji.egggame")
        );
    }

    #[test]
    fn needle_prefers_the_explicit_match() {
        assert_eq!(
            title_needle(Some("  Clannad "), Some("PCSG00123")),
            Some("Clannad".to_owned())
        );
    }

    #[test]
    fn needle_falls_back_to_the_file_stem_or_title_id() {
        assert_eq!(
            title_needle(None, Some(r"D:\roms\Final Fantasy X.iso")),
            Some("Final Fantasy X".to_owned())
        );
        assert_eq!(
            title_needle(Some(""), Some("PCSG00123")),
            Some("PCSG00123".to_owned())
        );
        assert_eq!(
            title_needle(None, Some("/roms/Game Title.nsp")),
            Some("Game Title".to_owned())
        );
    }

    #[test]
    fn a_generic_file_name_identifies_nothing() {
        assert_eq!(
            title_needle(None, Some(r"D:\x\PS3_GAME\USRDIR\EBOOT.BIN")),
            None
        );
        assert_eq!(title_needle(None, Some("game.exe")), None);
        assert_eq!(title_needle(None, None), None);
        assert_eq!(title_needle(Some("  "), Some("  ")), None);
    }

    #[test]
    fn caption_match_ignores_case_and_surrounding_text() {
        assert!(matches_window_title(
            "eden | v0.1 | Super Game [0100ABC]",
            "super game"
        ));
        assert!(!matches_window_title("Vita3K 0.2.0", "PCSG00123"));
        assert!(!matches_window_title("anything", ""));
        assert!(!matches_window_title("anything", "   "));
    }

    #[test]
    fn caption_match_handles_japanese_titles() {
        assert!(matches_window_title("PPSSPP - 白昼夢の青写真", "白昼夢"));
    }

    #[test]
    fn the_longest_needle_wins() {
        let needles = vec![Some("Fate".to_owned()), Some("Fate/stay night".to_owned())];
        assert_eq!(
            best_title_match("Vita3K | Fate/stay night [PCSG1]", &needles),
            Some(1)
        );
        assert_eq!(best_title_match("Vita3K | Fate Extra", &needles), Some(0));
    }

    #[test]
    fn two_equally_plausible_games_yield_no_attribution() {
        let needles = vec![Some("Alpha".to_owned()), Some("Gamma".to_owned())];
        assert_eq!(best_title_match("Alpha Gamma", &needles), None);
    }

    #[test]
    fn the_same_needle_twice_is_not_a_tie() {
        let needles = vec![Some("Alpha".to_owned()), Some("alpha".to_owned())];
        assert_eq!(best_title_match("Alpha", &needles), Some(0));
    }

    #[test]
    fn no_match_and_missing_needles_yield_none() {
        let needles = vec![None, Some("Alpha".to_owned())];
        assert_eq!(best_title_match("Beta", &needles), None);
        assert_eq!(best_title_match("Alpha", &[None, None]), None);
        assert_eq!(best_title_match("Alpha", &[]), None);
    }
}
