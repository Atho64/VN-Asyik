//! Choosing which chapter-detection strategy to try, and in what order.
//!
//! Describing candidates as data makes cost/safety trade-offs explicit and
//! unit-testable without a running game, and lets unavailable sources report
//! their platform reason instead of silently failing.

use serde::{Deserialize, Serialize};

/// A source of chapter information, ordered cheapest first (pinned by test).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChapterSource {
    /// Focused window caption: no injection or offsets; survives game updates.
    WindowTitle,
    /// Engine's in-memory scene table: needs a verified, versioned offset profile.
    MemoryScene,
}

impl ChapterSource {
    /// Why this source might be unavailable on `platform`, for UI reporting.
    pub fn unavailable_reason(self, platform: Platform) -> UnavailableReason {
        match (self, platform) {
            // Android and macOS prohibit cross-process memory reads.
            (ChapterSource::MemoryScene, Platform::Android | Platform::MacOs) => {
                UnavailableReason::PlatformCannotReadOtherProcesses
            }
            (ChapterSource::WindowTitle, Platform::Android) => {
                UnavailableReason::PlatformHasNoWindowTitles
            }
            _ => UnavailableReason::Available,
        }
    }
}

/// Target platform, passed explicitly so every platform's policy can be tested
/// from one machine without reading `cfg!`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Platform {
    Windows,
    MacOs,
    Linux,
    Android,
}

impl Platform {
    pub const fn current() -> Self {
        #[cfg(target_os = "windows")]
        {
            Platform::Windows
        }
        #[cfg(target_os = "macos")]
        {
            Platform::MacOs
        }
        #[cfg(target_os = "android")]
        {
            Platform::Android
        }
        #[cfg(all(
            not(target_os = "windows"),
            not(target_os = "macos"),
            not(target_os = "android")
        ))]
        {
            Platform::Linux
        }
    }
}

/// Whether a source can run here, and if not, why.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnavailableReason {
    Available,
    PlatformCannotReadOtherProcesses,
    PlatformHasNoWindowTitles,
}

impl UnavailableReason {
    pub const fn is_available(self) -> bool {
        matches!(self, UnavailableReason::Available)
    }
}

/// Sources to try in order for `engine` (`siglus`, `kirikiri`, `artemis`, or `None`)
/// on `platform`. Unknown engines still try window titles because parsing needs no engine support.
pub fn source_order(engine: Option<&str>, platform: Platform) -> Vec<ChapterSource> {
    let mut sources = Vec::new();

    // Title first: cheapest and safest for the game process.
    if engine.is_none() || engine_writes_title_chapters(engine) {
        sources.push(ChapterSource::WindowTitle);
    }

    if engine.is_some_and(engine_has_scene_table)
        && ChapterSource::MemoryScene
            .unavailable_reason(platform)
            .is_available()
    {
        sources.push(ChapterSource::MemoryScene);
    }

    sources
}

/// Engine families this build can name for user selection and window-title profiles.
/// Listing a name does not imply automatic process detection.
pub const KNOWN_ENGINES: &[&str] = &["siglus", "kirikiri", "artemis"];

#[must_use]
pub fn is_known_engine(engine: &str) -> bool {
    KNOWN_ENGINES
        .iter()
        .any(|known| known.eq_ignore_ascii_case(engine.trim()))
}

/// Engines verified to write a chapter marker into the window title.
///
/// Siglus only (`Summer Pockets REFLECTION BLUE`); `KiriKiri` sets its caption to
/// the game name alone. Do not widen without a live capture.
pub fn engine_writes_title_chapters(engine: Option<&str>) -> bool {
    matches!(
        engine.map(str::to_ascii_lowercase).as_deref(),
        Some("siglus")
    )
}

/// Engines with a verified in-memory scene table (Siglus only; see `crate::scene`).
pub fn engine_has_scene_table(engine: &str) -> bool {
    matches!(engine.to_ascii_lowercase().as_str(), "siglus")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_declaration_order_is_the_cheap_to_expensive_order() {
        // Chapter detection is attempted in enum order; a window-title read
        // costs one syscall and a memory read opens a handle. If someone
        // reorders the enum, this test says so rather than silently making
        // every poll more expensive.
        assert!(ChapterSource::WindowTitle < ChapterSource::MemoryScene);
    }

    #[test]
    fn siglus_tries_the_title_before_memory_on_desktop() {
        let order = source_order(Some("siglus"), Platform::Windows);
        assert_eq!(
            order,
            vec![ChapterSource::WindowTitle, ChapterSource::MemoryScene]
        );
    }

    #[test]
    fn an_unknown_engine_still_gets_the_window_title() {
        // Nothing was detected, but a title is free to try and either parses or
        // does not. Withholding it would give up before looking.
        let order = source_order(None, Platform::Windows);
        assert_eq!(order, vec![ChapterSource::WindowTitle]);
    }

    #[test]
    fn kirikiri_does_not_get_the_window_title_source() {
        // Its caption is the game name, so a title read would attribute
        // chapters named after the game itself.
        let order = source_order(Some("kirikiri"), Platform::Windows);
        assert!(order.is_empty(), "got {order:?}");
    }

    #[test]
    fn android_can_use_neither_source_and_says_why() {
        // Not a missing implementation: Android genuinely cannot read another
        // app's window or memory. The UI has to be able to explain that.
        assert_eq!(
            ChapterSource::MemoryScene.unavailable_reason(Platform::Android),
            UnavailableReason::PlatformCannotReadOtherProcesses
        );
        assert_eq!(
            ChapterSource::WindowTitle.unavailable_reason(Platform::Android),
            UnavailableReason::PlatformHasNoWindowTitles
        );
        assert!(
            !ChapterSource::WindowTitle
                .unavailable_reason(Platform::Android)
                .is_available()
        );
    }

    #[test]
    fn android_gets_no_memory_source_even_for_a_known_engine() {
        let order = source_order(Some("siglus"), Platform::Android);
        assert_eq!(order, vec![ChapterSource::WindowTitle]);
        assert!(!order.contains(&ChapterSource::MemoryScene));
    }

    #[test]
    fn macos_gets_no_memory_source() {
        let order = source_order(Some("siglus"), Platform::MacOs);
        assert!(!order.contains(&ChapterSource::MemoryScene));
    }

    #[test]
    fn windows_reports_every_source_as_available() {
        for source in [ChapterSource::WindowTitle, ChapterSource::MemoryScene] {
            assert!(source.unavailable_reason(Platform::Windows).is_available());
        }
    }

    #[test]
    fn engine_matching_is_case_insensitive() {
        // Engine names reach here from detection, which lowercases, but a
        // hand-written profile might not.
        assert!(engine_writes_title_chapters(Some("Siglus")));
        assert!(engine_has_scene_table("SIGLUS"));
        assert!(!engine_has_scene_table("kirikiri"));
    }

    #[test]
    fn only_siglus_is_known_to_write_title_chapters() {
        // Each name here needs a live capture, so this asserts outcomes
        // directly rather than filtering through the function under test: an
        // earlier version derived the expectation from
        // `engine_writes_title_chapters`, so widening that function widened the
        // expectation too and the test could never fail.
        assert!(engine_writes_title_chapters(Some("siglus")));
        assert!(!engine_writes_title_chapters(Some("kirikiri")));
        assert!(!engine_writes_title_chapters(Some("artemis")));
        assert!(!engine_writes_title_chapters(None));

        // And the order reflects it: KiriKiri gets no source at all on desktop,
        // because neither a title read nor a scene table applies to it.
        assert_eq!(
            source_order(Some("kirikiri"), Platform::Windows).len(),
            0,
            "source_order(Some(\"kirikiri\"), Platform::Windows) should be empty"
        );
    }

    #[test]
    fn only_siglus_has_a_scene_table() {
        assert!(engine_has_scene_table("siglus"));
        assert!(!engine_has_scene_table("kirikiri"));
        assert!(!engine_has_scene_table("artemis"));
    }

    #[test]
    fn a_recognised_engine_with_no_verified_source_offers_nothing() {
        // The capability report must be able to say "we know this engine, and we
        // still cannot read a chapter out of it". Reporting them as unknown would
        // be the wrong reason and would hide a real gap.
        for engine in ["kirikiri", "artemis"] {
            assert!(is_known_engine(engine), "{engine} should be recognised");
            assert_eq!(
                source_order(Some(engine), Platform::Windows).len(),
                0,
                "{engine} has no verified chapter source"
            );
        }
        assert!(is_known_engine("Siglus"));
        assert!(!is_known_engine("renpy"));
    }

    #[test]
    fn current_platform_is_one_of_the_known_ones() {
        assert!(matches!(
            Platform::current(),
            Platform::Windows | Platform::MacOs | Platform::Linux | Platform::Android
        ));
    }

    #[test]
    fn an_engine_without_a_scene_table_still_gets_nothing_from_memory() {
        let order = source_order(Some("artemis"), Platform::Windows);
        assert!(!order.contains(&ChapterSource::MemoryScene));
    }
}
