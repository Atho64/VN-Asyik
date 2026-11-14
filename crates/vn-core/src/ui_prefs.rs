//! Pure UI navigation and library startup browsing preferences.
//!
//! These are local presentation/browsing defaults stored outside synced domain
//! models so existing backup contracts stay unchanged and sanitization can run in
//! `vn-core` tests without Tauri or `SQLite`.

use serde::{Deserialize, Serialize};

use crate::library::SortKey;
use crate::model::GameStatus;

/// Visual collection layout in the library screen.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LibraryViewMode {
    #[default]
    Grid,
    List,
    Shelf,
}

/// Snapshot of library category filters, view mode, and sort key persisted across launches.
///
/// Search text (`query`) is excluded so a prior session's query never hides the library on startup.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SavedLibraryBrowse {
    /// Selected status filter chips (empty = all).
    pub statuses: Vec<GameStatus>,
    pub rated_only: bool,
    pub view: LibraryViewMode,
    pub sort: SortKey,
}

impl SavedLibraryBrowse {
    /// Deduplicate status filters while preserving user selection order.
    pub fn normalize(&mut self) {
        let mut unique = Vec::with_capacity(self.statuses.len());
        for &status in &self.statuses {
            if !unique.contains(&status) {
                unique.push(status);
            }
        }
        self.statuses = unique;
    }
}

/// Persisted UI shell and library startup preferences.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiPrefs {
    /// Desktop navigation rail collapsed into compact icon-only mode.
    pub sidebar_compact: bool,
    pub library_remember_browse: bool,
    pub library_default_sort: SortKey,
    pub library_saved_browse: Option<SavedLibraryBrowse>,
}

/// What the app does with its window on launch.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StartupMode {
    /// Show the normal window at the saved position.
    #[default]
    Manual,
    /// Open into Big Picture, fullscreen.
    BigPicture,
    /// Stay hidden in the tray until opened by the user.
    Tray,
}

/// Device-local window behaviour: launch presentation and close action.
///
/// Autostart is OS state (a Run-key entry), not a stored pref; the toggle
/// calls the OS directly so the two cannot disagree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct StartupPrefs {
    pub startup_mode: StartupMode,
    /// Hide to the tray on close instead of quitting.
    pub close_to_tray: bool,
}

impl Default for StartupPrefs {
    fn default() -> Self {
        Self {
            startup_mode: StartupMode::Manual,
            // Preserve hide-to-tray when a row is missing or partial.
            close_to_tray: true,
        }
    }
}

/// Partial update applied to [`StartupPrefs`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct StartupPrefsPatch {
    pub startup_mode: Option<StartupMode>,
    pub close_to_tray: Option<bool>,
}

impl StartupPrefs {
    pub fn apply_patch(&mut self, patch: StartupPrefsPatch) {
        if let Some(mode) = patch.startup_mode {
            self.startup_mode = mode;
        }
        if let Some(tray) = patch.close_to_tray {
            self.close_to_tray = tray;
        }
    }
}

/// Partial update applied to [`UiPrefs`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiPrefsPatch {
    pub sidebar_compact: Option<bool>,
    pub library_remember_browse: Option<bool>,
    pub library_default_sort: Option<SortKey>,
    pub library_saved_browse: Option<SavedLibraryBrowse>,
}

impl UiPrefs {
    pub fn normalize(&mut self) {
        if let Some(browse) = &mut self.library_saved_browse {
            browse.normalize();
        }
    }

    pub fn apply_patch(&mut self, patch: UiPrefsPatch) {
        if let Some(compact) = patch.sidebar_compact {
            self.sidebar_compact = compact;
        }
        if let Some(remember) = patch.library_remember_browse {
            self.library_remember_browse = remember;
        }
        if let Some(default_sort) = patch.library_default_sort {
            self.library_default_sort = default_sort;
        }
        if let Some(mut saved) = patch.library_saved_browse {
            saved.normalize();
            self.library_saved_browse = Some(saved);
        }
        self.normalize();
    }

    /// Active library browse state on app launch.
    #[must_use]
    pub fn effective_library_browse(&self) -> SavedLibraryBrowse {
        if self.library_remember_browse
            && let Some(saved) = &self.library_saved_browse
        {
            return saved.clone();
        }
        SavedLibraryBrowse {
            statuses: Vec::new(),
            rated_only: false,
            view: LibraryViewMode::Grid,
            sort: self.library_default_sort,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_existing_app_behavior() {
        let prefs = UiPrefs::default();
        assert!(!prefs.sidebar_compact);
        assert!(!prefs.library_remember_browse);
        assert_eq!(prefs.library_default_sort, SortKey::Recent);
        assert_eq!(prefs.effective_library_browse().sort, SortKey::Recent);
        assert_eq!(prefs.effective_library_browse().view, LibraryViewMode::Grid);
        assert!(prefs.effective_library_browse().statuses.is_empty());
        assert!(!prefs.effective_library_browse().rated_only);
    }

    #[test]
    fn default_sort_applies_when_remember_browse_is_off() {
        let mut prefs = UiPrefs::default();
        prefs.apply_patch(UiPrefsPatch {
            library_default_sort: Some(SortKey::Playtime),
            library_saved_browse: Some(SavedLibraryBrowse {
                statuses: vec![GameStatus::Completed],
                rated_only: true,
                view: LibraryViewMode::List,
                sort: SortKey::Title,
            }),
            ..Default::default()
        });

        let effective = prefs.effective_library_browse();
        assert_eq!(effective.sort, SortKey::Playtime);
        assert_eq!(effective.view, LibraryViewMode::Grid);
        assert!(effective.statuses.is_empty());
        assert!(!effective.rated_only);
    }

    #[test]
    fn remember_browse_restores_saved_filters_and_deduplicates_statuses() {
        let mut prefs = UiPrefs::default();
        prefs.apply_patch(UiPrefsPatch {
            library_remember_browse: Some(true),
            library_default_sort: Some(SortKey::Recent),
            library_saved_browse: Some(SavedLibraryBrowse {
                statuses: vec![
                    GameStatus::Playing,
                    GameStatus::Wishlist,
                    GameStatus::Playing,
                ],
                rated_only: true,
                view: LibraryViewMode::Shelf,
                sort: SortKey::Rating,
            }),
            ..Default::default()
        });

        let effective = prefs.effective_library_browse();
        assert_eq!(
            effective.statuses,
            vec![GameStatus::Playing, GameStatus::Wishlist]
        );
        assert!(effective.rated_only);
        assert_eq!(effective.view, LibraryViewMode::Shelf);
        assert_eq!(effective.sort, SortKey::Rating);
    }

    #[test]
    fn patch_preserves_unspecified_fields() {
        let mut prefs = UiPrefs {
            sidebar_compact: true,
            library_remember_browse: true,
            library_default_sort: SortKey::Title,
            library_saved_browse: None,
        };
        prefs.apply_patch(UiPrefsPatch {
            sidebar_compact: Some(false),
            ..Default::default()
        });

        assert!(!prefs.sidebar_compact);
        assert!(prefs.library_remember_browse);
        assert_eq!(prefs.library_default_sort, SortKey::Title);
    }

    #[test]
    fn startup_defaults_preserve_today_behavior() {
        // Manual launch, close hides to the tray: the settings may exist, but
        // turning nothing on must not change what the app already did.
        let prefs = StartupPrefs::default();
        assert_eq!(prefs.startup_mode, StartupMode::Manual);
        assert!(prefs.close_to_tray);
    }

    #[test]
    fn startup_wire_format_is_snake_case() {
        // The frontend parses these keys directly; a camelCase rename would
        // arrive as undefined and silently reset the mode.
        let text = serde_json::to_string(&StartupPrefs {
            startup_mode: StartupMode::BigPicture,
            close_to_tray: false,
        })
        .expect("serialise");
        assert!(
            text.contains("\"startup_mode\":\"big_picture\""),
            "got {text}"
        );
        assert!(text.contains("\"close_to_tray\":false"), "got {text}");
    }

    #[test]
    fn a_startup_row_from_before_the_fields_existed_decodes_to_defaults() {
        let old_row = "{}";
        let decoded: StartupPrefs = serde_json::from_str(old_row).expect("decode");
        assert_eq!(decoded, StartupPrefs::default());
    }

    #[test]
    fn startup_patch_changes_only_what_it_names() {
        let mut prefs = StartupPrefs {
            startup_mode: StartupMode::Tray,
            close_to_tray: true,
        };
        prefs.apply_patch(StartupPrefsPatch {
            close_to_tray: Some(false),
            ..Default::default()
        });
        assert_eq!(prefs.startup_mode, StartupMode::Tray);
        assert!(!prefs.close_to_tray);
    }
}
