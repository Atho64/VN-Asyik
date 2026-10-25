//! Choosing which chapter-detection strategy to try, and in what order.
//!
//! # Why the order is data rather than `if` statements
//!
//! Chapter detection has several independent sources, and they are not
//! interchangeable. A window-title read is cheap, allocates nothing, and cannot
//! destabilise the game; a memory read opens a handle and walks engine
//! structures. Which sources exist at all depends on the engine, the platform,
//! and whether the user opted into hooks.
//!
//! Hardcoding the order in the poll loop made those trade-offs invisible and
//! untestable: every new source meant editing the loop and hoping. Here the
//! candidates are described as data, so the policy can be unit-tested on a
//! machine with no games on it, and an adapter that is unavailable on the
//! current platform reports that instead of silently contributing nothing.

use serde::{Deserialize, Serialize};

/// A source of chapter information.
///
/// Ordered by cost, cheapest first: the enum's declaration order is the order
/// they are tried in, and a test pins that so reordering is a deliberate act.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChapterSource {
    /// The focused window's caption. No injection, no offsets. Survives game
    /// updates that break offset chains, so it is tried first wherever an
    /// engine is known to write chapters there.
    WindowTitle,
    /// The engine's in-memory scene table. Needs a verified, versioned offset
    /// profile per engine build, and only where the engine keeps such a table.
    MemoryScene,
}

impl ChapterSource {
    /// Why this source might be unavailable, for the UI to explain rather than
    /// show a dead panel.
    pub fn unavailable_reason(self, platform: Platform) -> UnavailableReason {
        match (self, platform) {
            // Reading another process is a Windows-only capability here. It is
            // not that the code is missing: Android genuinely cannot do it.
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

/// The platform a decision is being made for.
///
/// Passed explicitly rather than read from `cfg!` so the policy can be tested
/// for every platform from one machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Platform {
    Windows,
    MacOs,
    Linux,
    Android,
}

impl Platform {
    /// The platform this build is running on.
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
    /// The OS does not let one process read another's memory.
    PlatformCannotReadOtherProcesses,
    /// The OS does not expose another app's window title.
    PlatformHasNoWindowTitles,
}

impl UnavailableReason {
    pub const fn is_available(self) -> bool {
        matches!(self, UnavailableReason::Available)
    }
}

/// The sources to try, in order, for one engine on one platform.
///
/// `engine` is the detected engine family (`siglus`, `kirikiri`, `artemis`), or
/// `None` when nothing was detected. An unknown engine still gets the
/// window-title source, because that needs nothing from the engine: it either
/// parses or it does not.
pub fn source_order(engine: Option<&str>, platform: Platform) -> Vec<ChapterSource> {
    let mut sources = Vec::new();

    // Title first, always: cheapest, and safest for the game process.
    if engine.is_none() || engine_writes_title_chapters(engine) {
        sources.push(ChapterSource::WindowTitle);
    }

    // Memory only for engines with a verified table, and only where the OS
    // permits it.
    if engine.is_some_and(engine_has_scene_table)
        && ChapterSource::MemoryScene
            .unavailable_reason(platform)
            .is_available()
    {
        sources.push(ChapterSource::MemoryScene);
    }

    sources
}

/// Engines this build can name at all.
///
/// Naming an engine is not the same as being able to read a chapter out of it.
/// Since the in-process hook was removed, no engine can be identified by scanning
/// a running process, so these names exist for the user's own engine selection and
/// for the window-title profiles. Nothing here implies detection.
pub const KNOWN_ENGINES: &[&str] = &["siglus", "kirikiri", "artemis"];

/// Whether `engine` is an engine family this build recognises.
#[must_use]
pub fn is_known_engine(engine: &str) -> bool {
    KNOWN_ENGINES
        .iter()
        .any(|known| known.eq_ignore_ascii_case(engine.trim()))
}

/// Engines verified to write a chapter marker into the window title.
///
/// Only Siglus, confirmed live against `Summer Pockets REFLECTION BLUE`, whose
/// caption is the game, an optional section, then a month and day in full-width
/// digits. `KiriKiri` sets its caption to the game name alone, so a title read
/// there would attribute nothing. Do not widen this without a live capture: a
/// wrong entry silently produces chapters named after the game itself.
pub fn engine_writes_title_chapters(engine: Option<&str>) -> bool {
    matches!(
        engine.map(str::to_ascii_lowercase).as_deref(),
        Some("siglus")
    )
}

/// Engines with a verified in-memory scene table.
///
/// Siglus only. Its table was reverse-engineered and confirmed live; see
/// `crate::scene` for the offsets and their provenance.
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
