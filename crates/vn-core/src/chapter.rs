//! Turning an engine's scene identity into a chapter the user recognises.
//!
//! # Why this is not just "pass the string through"
//!
//! Engines name their internal scenes for their own convenience. Siglus in
//! `Summer Pockets` yields a mix of real chapters (`10_プロローグ0725`), story
//! routing tables (`00_シナリオフロー`), and screens the user would never call a
//! chapter (`_rb_titlemenu`, `99_マップ選択`). Playing the title screen must not
//! create a chapter called `_rb_titlemenu` in the library.
//!
//! So the raw scene name is interpreted, not displayed. The rules below are
//! derived from the names actually present in that game and are unit-tested
//! against them; a name the rules do not recognise is reported as *unattributed*
//! rather than guessed at.

use serde::{Deserialize, Serialize};

/// What a raw engine scene name means for playtime attribution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SceneRole {
    /// A story chapter. This is the only role that earns per-chapter time.
    Chapter {
        /// The name to store, with engine prefixes stripped.
        name: String,
        /// The engine's ordering prefix, when it has one, so the UI can sort
        /// chapters the way the game presents them rather than alphabetically.
        order: Option<u32>,
        /// The route this chapter belongs to, when the engine's naming reveals
        /// one.
        route: Option<Route>,
    },
    /// A screen the engine draws outside the story: title, menus, maps.
    Interface,
    /// A routing or index table rather than a place the player visits.
    Routing,
    /// Recognisably a scene, but not one we can attribute honestly.
    Unknown,
}

/// Siglus scene names that are navigation, not chapters.
///
/// Verified against `Summer Pockets REFLECTION BLUE`: these appear in the scene
/// table and the player passes through them constantly, so counting them as
/// chapters would fragment every session into meaningless slivers.
const SIGLUS_ROUTING_SUFFIXES: &[&str] = &["シナリオフロー", "マップ選択"];

/// A story route, as the engine's naming encodes it.
///
/// In `Summer Pockets` the numeric prefix is not a chapter number: every scene
/// of one character's story shares a prefix, so the prefix identifies the route
/// and the rest names the scene within it. Grouping by route is how a player
/// thinks about the game, and it keeps the chapter list from interleaving five
/// routes by date.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Route {
    /// The numeric code the engine uses.
    pub code: u32,
    /// A stable identifier for storage, e.g. `route_20`.
    pub key: String,
    /// The route's name, when the engine gives one; otherwise the code.
    pub name: Option<String>,
}

/// Route names verified for Summer Pockets REFLECTION BLUE.
///
/// The game never writes these labels into a scene name, so they cannot be read
/// from the engine. They are recorded here from the game's own structure: the
/// scenario flow lists the routes, and these are its codes. Any code not listed
/// keeps its number rather than being given an invented name.
const SIGLUS_ROUTE_NAMES: &[(u32, &str)] = &[
    (10, "Common"),
    (20, "Shiroha"),
    (30, "Ao"),
    (40, "Kamome"),
    (50, "Tsumugi"),
    (60, "Umi"),
    (70, "Final"),
];

/// Build a route from its numeric code.
#[must_use]
pub fn route_from_code(code: u32) -> Route {
    let name = SIGLUS_ROUTE_NAMES
        .iter()
        .find(|(candidate, _)| *candidate == code)
        .map(|(_, label)| (*label).to_owned());
    Route {
        key: format!("route_{code}"),
        code,
        name,
    }
}

/// A stable key for a route the user named rather than the engine.
///
/// The prefix keeps manual and engine routes in separate key spaces: an engine
/// route is always `route_<digits>`, so a manual route can never collide with one
/// by sharing a label.
#[must_use]
pub fn manual_route_key(label: &str) -> String {
    format!("route:{}", label.trim())
}

/// The label to show for a route code, when it is not one of [`SIGLUS_ROUTE_NAMES`].
///
/// Returning `None` here is what keeps [`route_from_code`] honest: an unrecognised
/// code keeps its number instead of being given an invented name. The UI still has
/// to render *something*, and a bare number reads as a bug, so this is the one place
/// that decides how an unnamed route is titled.
#[must_use]
pub fn route_label(key: Option<&str>, name: Option<&str>) -> String {
    if let Some(name) = name.map(str::trim).filter(|name| !name.is_empty()) {
        return name.to_owned();
    }
    let Some(key) = key else {
        return String::new();
    };
    // `route_42` is the engine's own code for a route whose name is not recorded.
    // Showing the code is truthful; showing nothing makes it indistinguishable
    // from a chapter with no route at all.
    if let Some(code) = key.strip_prefix("route_") {
        return format!("Route {code}");
    }
    // A manual route key already carries its label.
    key.strip_prefix("route:").unwrap_or(key).to_owned()
}

/// Interpret a raw Siglus scene name.
///
/// Siglus names scenes `<digits>_<name>`, where the digits are a route or order
/// code rather than a chapter number, and prefixes screenshots and utility
/// scenes with `_`. Both facts are from the real scene table of the reference
/// game, not from documentation.
#[must_use]
pub fn interpret_siglus(scene: &str) -> SceneRole {
    let scene = scene.trim();
    if scene.is_empty() {
        return SceneRole::Unknown;
    }

    // A leading underscore marks an engine-internal screen: `_rb_titlemenu`,
    // `_rb_mng_table_tennis_user_data`. These are menus and management tables.
    if scene.starts_with('_') {
        return SceneRole::Interface;
    }

    for suffix in SIGLUS_ROUTING_SUFFIXES {
        if scene.ends_with(suffix) {
            return SceneRole::Routing;
        }
    }

    // `<digits>_<rest>`: the digits order the script, the rest names it.
    if let Some((prefix, rest)) = scene.split_once('_') {
        let rest = rest.trim();
        if prefix.is_empty() || rest.is_empty() || !prefix.bytes().all(|b| b.is_ascii_digit()) {
            return SceneRole::Unknown;
        }
        let Ok(code) = prefix.parse::<u32>() else {
            return SceneRole::Unknown;
        };
        return SceneRole::Chapter {
            name: rest.to_owned(),
            order: Some(code),
            route: Some(route_from_code(code)),
        };
    }

    // A bare name with no prefix is a scene we cannot place. Reporting it as
    // unknown keeps it out of the chapter list instead of inventing an order.
    SceneRole::Unknown
}

/// Whether a scene should start a chapter segment for the given engine.
#[must_use]
pub fn is_chapter(engine: &str, scene: &str) -> bool {
    match engine {
        "siglus" => matches!(interpret_siglus(scene), SceneRole::Chapter { .. }),
        // Any other engine's scenes are not interpreted yet. Returning false
        // keeps playtime at game level, which the UI states plainly.
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_story_scene_becomes_a_chapter() {
        assert_eq!(
            interpret_siglus("10_プロローグ0725"),
            SceneRole::Chapter {
                name: "プロローグ0725".to_owned(),
                order: Some(10),
                route: Some(route_from_code(10)),
            }
        );
    }

    #[test]
    fn a_known_route_code_gets_its_name() {
        let route = route_from_code(20);
        assert_eq!(route.key, "route_20");
        assert_eq!(route.name.as_deref(), Some("Shiroha"));
    }

    #[test]
    fn an_unknown_route_code_keeps_its_number() {
        // Inventing a label for an unrecognised code would be a guess.
        let route = route_from_code(42);
        assert_eq!(route.code, 42);
        assert_eq!(route.key, "route_42");
        assert_eq!(route.name, None);
    }

    #[test]
    fn an_unnamed_route_is_labelled_by_its_code() {
        // Regression: the UI rendered the "No route" label for these, because it
        // grouped on `route_key` but took the label from `route_name`, which is
        // deliberately `None` for a code we do not recognise.
        assert_eq!(route_label(Some("route_42"), None), "Route 42");
    }

    #[test]
    fn a_named_route_uses_its_name() {
        assert_eq!(route_label(Some("route_20"), Some("Shiroha")), "Shiroha");
    }

    #[test]
    fn a_chapter_with_no_route_has_no_label() {
        assert_eq!(route_label(None, None), "");
    }

    #[test]
    fn a_manual_route_key_survives_as_its_label() {
        assert_eq!(
            route_label(Some(&manual_route_key("Mio")), None),
            "Mio",
            "a manual key is the label, not a code to decorate"
        );
    }

    #[test]
    fn an_empty_name_is_not_a_label() {
        // A stored empty string would otherwise render a blank route heading.
        assert_eq!(route_label(Some("route_42"), Some("   ")), "Route 42");
    }

    #[test]
    fn every_known_route_has_a_stable_key() {
        for (code, _) in SIGLUS_ROUTE_NAMES {
            assert_eq!(route_from_code(*code).key, format!("route_{code}"));
        }
    }

    #[test]
    fn the_route_code_is_kept_as_the_order() {
        // Real names from Summer Pockets RB: the prefix distinguishes routes.
        for (scene, order) in [
            ("20_しろは0815", 20u32),
            ("30_蒼0807", 30),
            ("40_鴎0810", 40),
            ("50_紬0812", 50),
            ("60_うみ0820", 60),
            ("70_終章_0810", 70),
        ] {
            match interpret_siglus(scene) {
                SceneRole::Chapter {
                    order: got, route, ..
                } => {
                    assert_eq!(got, Some(order), "{scene}");
                    assert_eq!(
                        route.map(|route| route.code),
                        Some(order),
                        "{scene} keeps its route code"
                    );
                }
                other => panic!("{scene} should be a chapter, got {other:?}"),
            }
        }
    }

    #[test]
    fn a_final_chapter_keeps_its_underscore_in_the_name() {
        match interpret_siglus("70_終章_0810") {
            SceneRole::Chapter { name, .. } => assert_eq!(name, "終章_0810"),
            other => panic!("expected a chapter, got {other:?}"),
        }
    }

    #[test]
    fn the_title_menu_is_not_a_chapter() {
        assert_eq!(interpret_siglus("_rb_titlemenu"), SceneRole::Interface);
    }

    #[test]
    fn engine_management_tables_are_not_chapters() {
        assert_eq!(
            interpret_siglus("_rb_mng_table_tennis_user_data"),
            SceneRole::Interface
        );
    }

    #[test]
    fn the_scenario_flow_index_is_routing() {
        assert_eq!(interpret_siglus("00_シナリオフロー"), SceneRole::Routing);
    }

    #[test]
    fn the_map_select_screen_is_routing() {
        assert_eq!(interpret_siglus("99_マップ選択"), SceneRole::Routing);
    }

    #[test]
    fn an_unprefixed_name_is_not_attributed() {
        // Guessing an order here would create phantom chapters.
        assert_eq!(interpret_siglus("somethingelse"), SceneRole::Unknown);
    }

    #[test]
    fn an_empty_scene_is_unknown() {
        assert_eq!(interpret_siglus(""), SceneRole::Unknown);
        assert_eq!(interpret_siglus("   "), SceneRole::Unknown);
    }

    #[test]
    fn a_non_numeric_prefix_is_not_a_chapter() {
        assert_eq!(interpret_siglus("ab_thing"), SceneRole::Unknown);
    }

    #[test]
    fn the_engine_gate_accepts_only_chapter_scenes() {
        assert!(is_chapter("siglus", "10_プロローグ0725"));
        assert!(!is_chapter("siglus", "_rb_titlemenu"));
        assert!(!is_chapter("siglus", "99_マップ選択"));
    }

    #[test]
    fn an_unimplemented_engine_never_attributes_chapters() {
        // A name that would be a chapter under Siglus must not be attributed by
        // an engine whose scene format we have not verified.
        assert!(!is_chapter("kirikiri", "10_プロローグ0725"));
        assert!(!is_chapter("unknown", "10_プロローグ0725"));
    }
}

/// Time and progress for one route.
///
/// Routes are the unit a player actually tracks: "I finished Rin's route" is a
/// meaningful statement, "I was in chapter 14" usually is not. Chapters remain
/// the detected detail that route totals are built from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteTotal {
    /// Stable key, empty for chapters with no route.
    pub key: String,
    /// Label to show.
    pub label: String,
    /// Chapters under this route.
    pub chapters: usize,
    /// Chapters reached, for a progress fraction.
    pub reached: usize,
    /// Time summed across the route's chapters.
    pub playtime_ms: i64,
}

impl RouteTotal {
    /// Fraction of the route's chapters reached, 0 when there are none.
    ///
    /// Integer maths then a single division: a route cannot have billions of
    /// chapters, and this avoids a float cast of an unbounded width.
    #[must_use]
    pub fn progress(&self) -> f64 {
        if self.chapters == 0 {
            return 0.0;
        }
        let reached = u32::try_from(self.reached).unwrap_or(u32::MAX);
        let total = u32::try_from(self.chapters).unwrap_or(u32::MAX);
        f64::from(reached) / f64::from(total)
    }
}

/// One chapter, as far as route totals care.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChapterTime<'a> {
    /// Route key, empty when the engine's naming revealed none.
    pub route_key: &'a str,
    /// Route label.
    pub route_label: &'a str,
    /// Whether it was reached.
    pub reached: bool,
    /// Time attributed to it.
    pub playtime_ms: i64,
}

/// Group chapters into route totals, largest time first.
///
/// Ordering by time rather than by name puts the routes someone actually played
/// at the top, which is what they came to look at. Routes with no time yet keep
/// their insertion order after the played ones.
#[must_use]
pub fn route_totals(chapters: &[ChapterTime<'_>]) -> Vec<RouteTotal> {
    let mut out: Vec<RouteTotal> = Vec::new();
    for chapter in chapters {
        match out.iter_mut().find(|total| total.key == chapter.route_key) {
            Some(existing) => {
                existing.chapters += 1;
                if chapter.reached {
                    existing.reached += 1;
                }
                existing.playtime_ms += chapter.playtime_ms;
            }
            None => out.push(RouteTotal {
                key: chapter.route_key.to_owned(),
                label: chapter.route_label.to_owned(),
                chapters: 1,
                reached: usize::from(chapter.reached),
                playtime_ms: chapter.playtime_ms,
            }),
        }
    }
    // Stable sort keeps first-seen order among equal totals.
    out.sort_by_key(|total| -total.playtime_ms);
    out
}

#[cfg(test)]
mod route_total_tests {
    use super::*;

    fn chapter<'a>(route: &'a str, label: &'a str, reached: bool, ms: i64) -> ChapterTime<'a> {
        ChapterTime {
            route_key: route,
            route_label: label,
            reached,
            playtime_ms: ms,
        }
    }

    #[test]
    fn chapters_roll_up_into_their_route() {
        let chapters = [
            chapter("shizuku", "Shizuku", true, 3_600_000),
            chapter("shizuku", "Shizuku", true, 1_800_000),
        ];
        let totals = route_totals(&chapters);
        assert_eq!(totals.len(), 1);
        assert_eq!(totals[0].chapters, 2);
        assert_eq!(totals[0].playtime_ms, 5_400_000);
    }

    #[test]
    fn routes_are_ordered_by_time_played() {
        // What someone played most is what they came to see.
        let chapters = [
            chapter("common", "Common", true, 60_000),
            chapter("rin", "Rin", true, 7_200_000),
            chapter("mai", "Mai", true, 300_000),
        ];
        let totals = route_totals(&chapters);
        assert_eq!(
            totals.iter().map(|t| t.key.as_str()).collect::<Vec<_>>(),
            vec!["rin", "mai", "common"]
        );
    }

    #[test]
    fn progress_counts_only_reached_chapters() {
        let chapters = [
            chapter("rin", "Rin", true, 1000),
            chapter("rin", "Rin", true, 1000),
            chapter("rin", "Rin", false, 0),
            chapter("rin", "Rin", false, 0),
        ];
        let totals = route_totals(&chapters);
        assert_eq!(totals[0].reached, 2);
        assert_eq!(totals[0].chapters, 4);
        assert!((totals[0].progress() - 0.5).abs() < 1e-9);
    }

    #[test]
    fn chapters_with_no_route_share_one_unlabelled_group() {
        let chapters = [
            chapter("", "No route", true, 1000),
            chapter("", "No route", true, 2000),
        ];
        let totals = route_totals(&chapters);
        assert_eq!(totals.len(), 1);
        assert_eq!(totals[0].key.len(), 0, "totals[0].key should be empty");
        assert_eq!(totals[0].playtime_ms, 3000);
    }

    #[test]
    fn an_empty_route_set_has_no_totals() {
        assert_eq!(
            route_totals(&[]).len(),
            0,
            "route_totals(&[]) should be empty"
        );
    }

    #[test]
    fn progress_of_an_empty_route_is_zero_not_a_division() {
        // A route with no chapters cannot be 100% done.
        let total = RouteTotal {
            key: "x".to_owned(),
            label: "X".to_owned(),
            chapters: 0,
            reached: 0,
            playtime_ms: 0,
        };
        assert!(total.progress().abs() < f64::EPSILON);
    }
}
