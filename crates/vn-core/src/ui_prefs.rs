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
    /// Cover grid cards.
    #[default]
    Grid,
    /// Dense tabular rows.
    List,
    /// Spine/shelf rows.
    Shelf,
}

/// Snapshot of library category filters, view mode, and sort key persisted across launches.
///
/// Search text (`query`) is intentionally excluded so a forgotten search string
/// from a prior session never hides the library on fresh startup.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SavedLibraryBrowse {
    /// Selected status filter chips (empty = all).
    pub statuses: Vec<GameStatus>,
    /// Rated-only chip toggle.
    pub rated_only: bool,
    /// Active library layout (`grid`, `list`, `shelf`).
    pub view: LibraryViewMode,
    /// Active sort order (`recent`, `title`, `rating`, `playtime`, `started`).
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
    /// When `true`, restore the last selected library filters/view/sort on launch.
    pub library_remember_browse: bool,
    /// Default sort key applied on launch when `library_remember_browse` is `false`.
    pub library_default_sort: SortKey,
    /// Last persisted library browse state (restored when `library_remember_browse` is `true`).
    pub library_saved_browse: Option<SavedLibraryBrowse>,
}

/// Partial update applied to [`UiPrefs`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiPrefsPatch {
    /// Toggle desktop sidebar compact icon mode.
    pub sidebar_compact: Option<bool>,
    /// Toggle remembering library filters/view/sort across launches.
    pub library_remember_browse: Option<bool>,
    /// Change the default startup library sort key.
    pub library_default_sort: Option<SortKey>,
    /// Persist the current library filters/view/sort snapshot.
    pub library_saved_browse: Option<SavedLibraryBrowse>,
}

impl UiPrefs {
    /// Normalize nested fields in place.
    pub fn normalize(&mut self) {
        if let Some(browse) = &mut self.library_saved_browse {
            browse.normalize();
        }
    }

    /// Merge a partial update and normalize the resulting preferences.
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

    /// Compute the library browse state that should be active on app launch.
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
}
