//! Ranking candidate executables found by a folder scan.
//!
//! A Visual Novel folder routinely holds dozens of `.exe` files that are not the
//! game: uninstallers, crash handlers, patch tools, launcher shims, and — in the
//! case that prompted this module — an entire vendored SDK build with its own
//! test binaries. The user then has to pick the right one by hand.
//!
//! Size alone is a poor guide. In a real folder the largest binary was a 31 MB
//! message-script editor and the game itself was 0.9 MB, ranked sixth. Name
//! patterns carry far more signal: `uninst`, `patch`, `tool`, `setup` and
//! `launcher` are almost never the game, while a bare title matching the folder
//! name usually is.
//!
//! This module is pure so the ranking is testable without a filesystem.

use serde::{Deserialize, Serialize};

/// Substrings that mark a binary as auxiliary rather than the game.
///
/// Kept lowercase; matching is done against a lowercased file name.
const UNLIKELY: &[&str] = &[
    // Real uninstaller names vary: uinst.exe, unins000.exe, uninst.exe.
    "uninst", "unins", "uinst", "setup", "install", "patch", "update", "updater", "launcher",
    "start", "config", "setting", "option", "tool", "editor", "test", "crash", "report", "helper",
    "service", "mesdis", "crack", "dll", "vcredist", "directx", "dotnet", "redist", "sdk",
    // Engine-adjacent utilities that live beside real games: KiriKiri archive
    // extractors and unpackers, script injectors.
    "brute", "extract", "unpack", "repack", "inject",
];

/// How much a candidate looks like the game it belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Plausibility {
    /// Almost certainly not the game: an installer, patch tool or test binary.
    Unlikely,
    /// No positive or negative signal.
    Neutral,
    /// Name resembles the folder it sits in, which usually means the game.
    Likely,
}

impl Plausibility {
    /// Sort key where higher sorts first.
    #[must_use]
    pub fn score(self) -> u8 {
        match self {
            Self::Unlikely => 0,
            Self::Neutral => 1,
            Self::Likely => 2,
        }
    }
}

/// Classify one executable by name, relative to the scanned folder.
///
/// `file_name` and `folder_name` are compared case-insensitively and without
/// their extension; punctuation and spaces are ignored, so `DC4.EXE` inside
/// `DC4` counts as a match and so does `Summer_Pockets.exe` in `Summer Pockets`.
#[must_use]
pub fn plausibility(file_name: &str, folder_name: &str) -> Plausibility {
    let stem = squash(trim_end_matches_case_insensitive(file_name, ".exe"));
    if stem.is_empty() {
        return Plausibility::Unlikely;
    }
    let lower = file_name.to_lowercase();
    if UNLIKELY.iter().any(|marker| lower.contains(marker)) {
        return Plausibility::Unlikely;
    }
    let folder = squash(folder_name);
    if !folder.is_empty()
        && (stem == folder || stem.starts_with(&folder) || folder.starts_with(&stem))
    {
        return Plausibility::Likely;
    }
    Plausibility::Neutral
}

/// Lowercase a name and drop everything that is not a letter or digit.
fn squash(text: &str) -> String {
    text.chars()
        .filter(|ch| ch.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// `str::trim_end_matches` with a case-insensitive needle, which std lacks.
fn trim_end_matches_case_insensitive<'t>(text: &'t str, suffix: &str) -> &'t str {
    let lower = text.to_lowercase();
    if lower.ends_with(suffix) {
        // The needle is ASCII, so byte slicing at the difference is safe.
        let cut = text.len().saturating_sub(suffix.len());
        if text.is_char_boundary(cut) {
            return &text[..cut];
        }
    }
    text
}

/// Sort candidates for display: plausibility first, then size, then path.
///
/// Larger binaries stay ahead within a plausibility band because a game
/// executable is usually bigger than the launcher next to it.
#[must_use]
pub fn order<'a, T>(
    items: impl IntoIterator<Item = &'a T>,
    fields: impl Fn(&'a T) -> (&'a str, &'a str, u64),
    folder_name: &str,
) -> Vec<&'a T> {
    // The closure borrows from `T`, so both lifetimes are tied to `'a`.
    let mut scored: Vec<(u8, u64, &'a T)> = items
        .into_iter()
        .map(|item| {
            let (name, _path, size) = fields(item);
            let grade = plausibility(name, folder_name);
            (grade.score(), size, item)
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
    scored.into_iter().map(|(_, _, item)| item).collect()
}

/// One executable found by a folder walk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// File name, lowercased.
    pub name: String,
    /// Full path.
    pub path: String,
    /// Size in bytes.
    pub size_bytes: u64,
    /// Directory depth below the scan root. 0 means it sat directly in the
    /// folder the user chose.
    pub level: u32,
}

/// Upper bound on results. A folder tree can hold thousands of executables, and
/// the list is for a human to pick from.
pub const MAX_FOUND: usize = 200;

/// Walk `root` and return every `.exe` found within `max_levels` of it.
///
/// Depth-first and breadth-limited. A directory that is too deep is skipped —
/// **not** treated as the end of the walk: `pop()` returns directories in LIFO
/// order, so one deep folder says nothing about the siblings still queued.
/// Returning early there was a real bug: scanning `F:\DC4 new` stopped inside a
/// vendored SDK and never reached the sibling folder holding the game.
#[must_use]
pub fn walk(root: &std::path::Path, max_levels: u32) -> Vec<Found> {
    let mut found = Vec::new();
    let mut stack = vec![(root.to_path_buf(), 0_u32)];
    while let Some((dir, level)) = stack.pop() {
        if level > max_levels {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            if file_type.is_dir() {
                stack.push((path, level + 1));
                continue;
            }
            // Case-insensitive per clippy: `.EXE` and `.Exe` are real extension
            // spellings on Windows. The lowercased name is still what the
            // plausibility check sees.
            let raw_name = entry.file_name().to_string_lossy().to_string();
            if !raw_name.to_ascii_lowercase().ends_with(".exe") {
                continue;
            }
            let name = raw_name.to_ascii_lowercase();
            if found.len() >= MAX_FOUND {
                return found;
            }
            found.push(Found {
                name,
                path: path.to_string_lossy().to_string(),
                size_bytes: entry.metadata().map_or(0, |meta| meta.len()),
                level,
            });
        }
    }
    found
}

/// Rank walk results for display.
///
/// Order: how much the name looks like a game, then whether it sits inside a
/// folder rather than loose at the scan root, then size, then path.
///
/// The root-is-not-a-game rule matters more than it looks. A user points this at
/// a shared `Visual Novels` directory, which is also where installers accumulate:
/// one real folder held 335 executables, 40 of them loose at the root, and the
/// largest by far were `Terabox`, `ProtonVPN` and a `.NET` runtime. Games live in their
/// own subfolder, so a loose root-level binary is demoted rather than listed
/// first.
#[must_use]
pub fn order_found<'f>(found: &'f [Found], folder_name: &str) -> Vec<&'f Found> {
    let mut scored: Vec<(u8, u8, u64, &'f Found)> = found
        .iter()
        .map(|item| {
            (
                plausibility(&item.name, folder_name).score(),
                u8::from(item.level > 0),
                item.size_bytes,
                item,
            )
        })
        .collect();
    scored.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then(b.1.cmp(&a.1))
            .then(b.2.cmp(&a.2))
            .then_with(|| a.3.path.cmp(&b.3.path))
    });
    scored.into_iter().map(|(_, _, _, item)| item).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_matching_its_folder_is_the_best_signal() {
        assert_eq!(plausibility("DC4.EXE", "DC4"), Plausibility::Likely);
        assert_eq!(
            plausibility("Summer_Pockets.exe", "Summer Pockets"),
            Plausibility::Likely
        );
        assert_eq!(plausibility("clannad.exe", "CLANNAD"), Plausibility::Likely);
    }

    #[test]
    fn installers_and_tools_are_ranked_last() {
        for name in [
            "uinst.exe",
            "unins000.exe",
            "setup.exe",
            "DC4Patch.exe",
            "DC4Launcher.exe",
            "crashreport.exe",
            "mesdis.exe",
            "vcredist_x64.exe",
            "xp3brute.exe",
            "xp3_extract.exe",
            "Textractor.exe",
        ] {
            assert_eq!(
                plausibility(name, "DC4"),
                Plausibility::Unlikely,
                "{name} should not look like the game"
            );
        }
    }

    #[test]
    fn size_alone_does_not_beat_a_name_match() {
        // Reproduces the real folder: a 31 MB editor tool outranked the 0.9 MB
        // game purely because it was bigger.
        let rows = [
            (
                "msg_tool.exe",
                r"F:\DC4\AdvData\GRP\EFF\msg_tool.exe",
                31_000_000u64,
            ),
            ("DC4.exe", r"F:\DC4\DC4.exe", 900_000),
        ];
        let ordered = order(rows.iter(), |r| (r.0, r.1, r.2), "DC4");
        assert_eq!(ordered[0].0, "DC4.exe");
    }

    #[test]
    fn a_plain_executable_stays_above_known_tools() {
        let rows = [
            ("somegame.exe", "a", 100u64),
            ("uninst.exe", "b", 9_000_000),
        ];
        let ordered = order(rows.iter(), |r| (r.0, r.1, r.2), "folder");
        assert_eq!(ordered[0].0, "somegame.exe");
    }

    /// Build a throwaway tree mirroring the folder that exposed the bug: a deep
    /// vendored SDK alongside a shallow folder holding the actual game.
    fn temp_tree() -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "vn-scan-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        let game = root.join("DC4");
        std::fs::create_dir_all(&game).unwrap();
        std::fs::write(game.join("DC4.EXE"), b"game").unwrap();
        std::fs::write(root.join("uinst.exe"), b"uninstaller").unwrap();
        // A sibling that sorts late in the LIFO stack and is deeper than allowed.
        let deep = root.join("sdk/third_party/lib/deeper");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(deep.join("tool.exe"), b"deep tool").unwrap();
        root
    }

    #[test]
    fn a_deep_directory_does_not_abandon_the_rest_of_the_walk() {
        let root = temp_tree();
        let found = walk(&root, 3);
        let names: Vec<&str> = found.iter().map(|f| f.name.as_str()).collect();
        // Regression: the walk used to `break` on the first too-deep directory,
        // which skipped the sibling folder containing the game.
        assert!(
            names.contains(&"dc4.exe"),
            "game executable was missed: {names:?}"
        );
        assert!(names.contains(&"uinst.exe"));
        assert!(
            !names.contains(&"tool.exe"),
            "beyond the depth limit but still returned: {names:?}"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_loose_installer_at_the_root_does_not_outrank_a_game_in_a_subfolder() {
        // Reproduces the shared-library folder: 161 MB Terabox and ProtonVPN
        // installers loose at the root, games one level down.
        let rows = [
            Found {
                name: "terabox_1.44.1.162.exe".into(),
                path: "F:/visual novels/Terabox_1.44.1.162.exe".into(),
                size_bytes: 161_000_000,
                level: 0,
            },
            Found {
                name: "nine_haruiro.exe".into(),
                path: "F:/visual novels/v64744/nine_haruiro.exe".into(),
                size_bytes: 4_000_000,
                level: 1,
            },
        ];
        let ordered = order_found(&rows, "visual novels");
        assert_eq!(ordered[0].name, "nine_haruiro.exe");
    }

    #[test]
    fn the_game_outranks_the_uninstaller_in_a_real_tree() {
        let root = temp_tree();
        let found = walk(&root, 3);
        let ordered = order_found(&found, "DC4 new");
        assert_eq!(ordered[0].name, "dc4.exe");
        assert_eq!(ordered.last().unwrap().name, "uinst.exe");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn an_empty_or_odd_name_never_panics() {
        assert_eq!(plausibility("", "DC4"), Plausibility::Unlikely);
        assert_eq!(plausibility(".exe", "DC4"), Plausibility::Unlikely);
        assert_eq!(plausibility("ゲーム.exe", "ゲーム"), Plausibility::Likely);
    }
}
