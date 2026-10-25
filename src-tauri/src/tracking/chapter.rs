//! Turning a scene name into a chapter.
//!
//! The name-to-chapter mapping is pure logic and lives in [`vn_core::chapter`];
//! this module is the thin reader trait the tracking loop calls.
//!
//! The in-memory reader that walked an engine's scene table went with the hook:
//! it read another process's memory at offsets derived from injected byte
//! patterns, which is precisely the mechanism that was removed. Chapter detection
//! now comes from the window-title adapter, which needs no reader at all, so
//! [`UnsupportedChapterReader`] is what every build provides.

use vn_core::chapter::{self, SceneRole};
use vn_core::scene::ChapterRead;

/// Why a chapter could not be read.
#[derive(Debug, thiserror::Error)]
pub enum ChapterError {
    /// This platform has no chapter reader.
    #[error("chapter detection is not supported on this platform")]
    Unsupported,
    /// No verified offset profile exists for this engine and module.
    #[error("no verified chapter profile for {engine} in {module}")]
    NoProfile {
        /// Engine family that was asked for.
        engine: String,
        /// Module that was inspected.
        module: String,
    },
    /// The process could not be opened or read.
    #[error("could not read the game: {0}")]
    Read(String),
}

/// A chapter attributed to a stretch of playtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chapter {
    /// The engine's raw scene name. Detection matches on this, and a rename never
    /// replaces it.
    pub raw_name: String,
    /// The name to show the user, with the route prefix stripped.
    pub name: String,
    /// The engine's ordering prefix, when it has one.
    pub order: Option<u32>,
    /// The route this chapter belongs to, when the engine's naming reveals one.
    pub route: Option<vn_core::chapter::Route>,
}

/// Reads chapters from one running game.
pub trait ChapterReader {
    /// The scene currently being played, or `None` if the engine is between
    /// scenes or at an interface screen the user would not call a chapter.
    fn current_chapter(&self) -> Result<Option<Chapter>, ChapterError>;

    /// The raw scene name, for diagnostics and for the profile creator UI.
    fn raw_scene(&self) -> Result<ChapterRead, ChapterError>;
}

/// Resolve a raw scene name into a chapter for `engine`.
///
/// Kept separate from any process access so the mapping rules are testable and
/// so a caller holding a name (from a log, or a test) gets the same answer.
#[must_use]
pub fn chapter_from_scene(engine: &str, scene: &str) -> Option<Chapter> {
    if !chapter::is_chapter(engine, scene) {
        return None;
    }
    match chapter::interpret_siglus(scene) {
        SceneRole::Chapter { name, order, route } => Some(Chapter {
            raw_name: scene.to_owned(),
            name,
            order,
            route,
        }),
        _ => None,
    }
}

/// A reader for builds with no implementation.
///
/// This is every platform now: chapter detection from engine memory went with the
/// hook. Reading a scene table means reading another process's memory at offsets
/// derived from injected patterns, and that is exactly what was removed. The
/// window-title adapter is the path that remains, and it needs no reader.
#[derive(Debug)]
pub struct UnsupportedChapterReader;

impl ChapterReader for UnsupportedChapterReader {
    fn current_chapter(&self) -> Result<Option<Chapter>, ChapterError> {
        Err(ChapterError::Unsupported)
    }

    fn raw_scene(&self) -> Result<ChapterRead, ChapterError> {
        Err(ChapterError::Unsupported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_story_scene_becomes_a_chapter() {
        let chapter = chapter_from_scene("siglus", "10_プロローグ0725").expect("chapter");
        assert_eq!(chapter.name, "プロローグ0725");
        assert_eq!(chapter.order, Some(10));
        // The raw name is retained as the identity detection matches on, so a
        // rename cannot break re-detection.
        assert_eq!(chapter.raw_name, "10_プロローグ0725");
    }

    #[test]
    fn a_chapter_carries_its_route() {
        let chapter = chapter_from_scene("siglus", "20_しろは0815").expect("chapter");
        let route = chapter.route.expect("route");
        assert_eq!(route.key, "route_20");
        assert_eq!(route.name.as_deref(), Some("Shiroha"));
    }

    #[test]
    fn the_title_menu_is_not_a_chapter() {
        assert_eq!(chapter_from_scene("siglus", "_rb_titlemenu"), None);
    }

    #[test]
    fn a_routing_scene_is_not_a_chapter() {
        assert_eq!(chapter_from_scene("siglus", "99_マップ選択"), None);
        assert_eq!(chapter_from_scene("siglus", "00_シナリオフロー"), None);
    }

    #[test]
    fn an_engine_without_a_profile_attributes_nothing() {
        assert_eq!(chapter_from_scene("kirikiri", "10_プロローグ0725"), None);
    }
}
