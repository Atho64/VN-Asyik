//! Describing a library's taste from its own data.
//!
//! # Why this is deterministic
//!
//! The panel is a plain count of the VNDB tags the library carries most, with no
//! model and no network, so it works with no API key and says exactly what it
//! counts. Every tag names the games behind it.
//!
//! # What makes a claim honest
//!
//! Every statement this module produces is traceable to a count. Nothing here
//! guesses, and nothing is invented to fill a paragraph.
//!
//! # Spoilers
//!
//! VNDB tags carry a `spoiler` level. A tag that only exists to reveal plot is
//! excluded from the profile, because a taste summary is shown regardless of how
//! far through a game the user is.

use serde::{Deserialize, Serialize};

/// A tag as it contributes to a profile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TagWeight {
    /// Tag name as VNDB labels it.
    pub name: String,
    /// VNDB's own weight for how much this tag defines the work, 0 to 3.
    pub rating: f32,
    /// VNDB spoiler level: 0 none, 1 mild, 2 major. Above zero is excluded.
    pub spoiler: u8,
}

/// One game as the profile needs it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfiledGame {
    /// Stable id, so a statement can point at the game it came from.
    pub id: String,
    /// Display title.
    pub title: String,
    /// `playing`, `completed`, `wishlist` and so on.
    pub status: String,
    /// 1 to 10, when rated.
    pub rating: Option<u8>,
    /// Tags from VNDB, if the game has been described.
    pub tags: Vec<TagWeight>,
    /// VNDB id when linked, used to collapse duplicates.
    ///
    /// A library can hold the same work twice under different titles, which is
    /// exactly what this library did. Two entries for one game must not double a
    /// tag's weight and invent a preference out of a single work.
    pub work_id: Option<String>,
}

/// A theme the library leans toward.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Theme {
    /// Tag name.
    pub name: String,
    /// How many games carry it.
    pub count: usize,
    /// Titles of the games, newest evidence first, capped for display.
    pub examples: Vec<String>,
    /// Share of the library, 0 to 1.
    pub share: f32,
}

/// The computed profile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TasteProfile {
    /// Games considered, after excluding anything unusable.
    pub considered: usize,
    /// Games skipped for having no tags, with titles.
    pub undetermined: Vec<String>,
    /// Tags, most games carrying them first.
    pub themes: Vec<Theme>,
}

/// Tags below this VNDB rating are technical boilerplate.
///
/// Measured on a real library: `ADV` sits at 2.86, `Built-in Encyclopedia` and
/// `Protagonist with a Face` lower still, and all of them appear on nearly every
/// visual novel. Counting them made the profile read "100% Genre: visual novel",
/// which describes the medium rather than the reader. Real taste signal sits
/// higher: `Nakige` at 2.98, `Family` at 2.94.
const BOILERPLATE_BELOW: f32 = 2.88;

/// Minimum absolute count, so a one-game library produces no themes at all rather
/// than a confident statement built on nothing.
const MINIMUM_COUNT: usize = 2;

/// How many tags to report, strongest first.
const MAX_THEMES: usize = 12;

/// How many example titles per tag.
const MAX_EXAMPLES: usize = 3;

/// Share of a whole, as a fraction.
///
/// One place for the narrowing, so the reason is stated once: library sizes are
/// thousands at most, far inside `f32`'s exact integer range.
fn share_of(count: usize, whole: usize) -> f32 {
    #[allow(clippy::cast_precision_loss)]
    let (count, whole) = (count as f32, whole as f32);
    if whole <= 0.0 { 0.0 } else { count / whole }
}

/// Build a taste profile.
///
/// Games with no usable tags are reported in `undetermined` rather than silently
/// dropped, because a library of ten games where six lack metadata must not read
/// as a profile of four.
#[must_use]
pub fn profile(games: &[ProfiledGame]) -> TasteProfile {
    let mut undetermined = Vec::new();
    let mut usable: Vec<&ProfiledGame> = Vec::new();

    // Collapse duplicate entries for the same work before counting anything.
    let mut seen_works: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut distinct: Vec<&ProfiledGame> = Vec::new();
    for game in games {
        if let Some(work) = &game.work_id
            && !seen_works.insert(work.clone())
        {
            continue;
        }
        distinct.push(game);
    }
    let games: &[ProfiledGame] = &distinct
        .iter()
        .map(|game| (*game).clone())
        .collect::<Vec<_>>();

    for game in games {
        let has_tags = game
            .tags
            .iter()
            .any(|tag| tag.spoiler == 0 && tag.rating >= BOILERPLATE_BELOW);
        if has_tags {
            usable.push(game);
        } else {
            undetermined.push(game.title.clone());
        }
    }

    // Per-tag evidence: count and examples.
    let mut tally: Vec<(String, usize, Vec<String>)> = Vec::new();
    for game in &usable {
        let mut seen_here: Vec<&str> = Vec::new();
        for tag in &game.tags {
            // Spoiler tags reveal plot, and low-rated tags describe the medium
            // rather than the reader's preference.
            if tag.spoiler != 0 || tag.rating < BOILERPLATE_BELOW {
                continue;
            }
            // One game counts once per tag, however many times it lists it.
            if seen_here.contains(&tag.name.as_str()) {
                continue;
            }
            seen_here.push(tag.name.as_str());
            match tally.iter_mut().find(|(name, _, _)| name == &tag.name) {
                Some((_, count, examples)) => {
                    *count += 1;
                    if examples.len() < MAX_EXAMPLES {
                        examples.push(game.title.clone());
                    }
                }
                None => tally.push((tag.name.clone(), 1, vec![game.title.clone()])),
            }
        }
    }

    let considered = usable.len();
    // Ordered by how many games carry the tag. A tag on every game at the minimum
    // rating is weaker evidence than one on half the library at the maximum, but
    // the panel shows counts, so the count is the sort; ties break by name so the
    // output is stable between runs.
    let mut themes: Vec<Theme> = tally
        .into_iter()
        // A single shared game is a coincidence, not a pattern, however big the
        // library is.
        .filter(|(_, count, _)| *count >= MINIMUM_COUNT)
        .map(|(name, count, examples)| Theme {
            name,
            count,
            examples,
            share: share_of(count, considered),
        })
        .collect();

    themes.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.name.cmp(&b.name)));
    themes.truncate(MAX_THEMES);

    TasteProfile {
        considered,
        undetermined,
        themes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag(name: &str, rating: f32) -> TagWeight {
        TagWeight {
            name: name.to_owned(),
            rating,
            spoiler: 0,
        }
    }

    fn spoiler_tag(name: &str) -> TagWeight {
        TagWeight {
            name: name.to_owned(),
            rating: 2.5,
            spoiler: 2,
        }
    }

    fn game(id: &str, title: &str, tags: Vec<TagWeight>) -> ProfiledGame {
        ProfiledGame {
            id: id.to_owned(),
            title: title.to_owned(),
            status: "completed".to_owned(),
            rating: None,
            tags,
            work_id: None,
        }
    }

    #[test]
    fn the_wire_format_is_camel_case() {
        // The frontend parses these keys directly. A snake_case field here would
        // arrive as undefined in the UI and render as a blank, which has happened
        // twice elsewhere in this codebase for exactly this reason.
        let games = vec![
            game("1", "A", vec![tag("Nakige", 2.98)]),
            game("2", "B", vec![tag("Nakige", 2.98)]),
        ];
        let text = serde_json::to_string(&profile(&games)).expect("serialise");
        assert!(text.contains("\"themes\""), "got {text}");
        assert!(text.contains("\"undetermined\""), "got {text}");

        let tag_text = serde_json::to_string(&tag("Nakige", 2.98)).expect("tag");
        assert!(tag_text.contains("\"rating\""), "got {tag_text}");
        assert!(tag_text.contains("\"spoiler\""), "got {tag_text}");
    }

    /// A game linked to a specific VNDB work.
    fn linked(id: &str, title: &str, work: &str, tags: Vec<TagWeight>) -> ProfiledGame {
        ProfiledGame {
            work_id: Some(work.to_owned()),
            ..game(id, title, tags)
        }
    }

    #[test]
    fn two_entries_for_one_work_do_not_invent_a_preference() {
        // A real library held Umineko twice, under its English and Japanese
        // titles. Counting both made a single work look like a pattern across two
        // games, which is the difference between describing a taste and inventing
        // one.
        let games = vec![
            linked(
                "1",
                "Umineko no Naku Koro ni",
                "v24",
                vec![tag("Battle of Wits", 2.97)],
            ),
            linked(
                "2",
                "うみねこのなく頃に",
                "v24",
                vec![tag("Battle of Wits", 2.97)],
            ),
            linked("3", "CLANNAD", "v4", vec![tag("Nakige", 2.98)]),
        ];
        let built = profile(&games);
        assert_eq!(built.considered, 2, "two distinct works, not three entries");
        let names: Vec<(&str, usize)> = built
            .themes
            .iter()
            .map(|t| (t.name.as_str(), t.count))
            .collect();
        assert!(
            !names.contains(&("Battle of Wits", 2)),
            "one work must not count twice, got {names:?}",
        );
    }

    #[test]
    fn a_shared_tag_becomes_a_theme() {
        let games = vec![
            game("1", "A", vec![tag("Nakige", 2.98)]),
            game("2", "B", vec![tag("Nakige", 2.94)]),
        ];
        let built = profile(&games);
        assert_eq!(built.considered, 2);
        assert_eq!(built.themes.len(), 1);
        assert_eq!(built.themes[0].name, "Nakige");
        assert_eq!(built.themes[0].count, 2);
        assert!((built.themes[0].share - 1.0).abs() < 1e-6);
        assert_eq!(built.themes[0].examples, vec!["A", "B"]);
    }

    #[test]
    fn boilerplate_tags_describe_the_medium_not_the_reader() {
        // Measured on a real library: ADV, Built-in Encyclopedia and Protagonist
        // with a Face appear on nearly every visual novel, and counting them made
        // the profile read "100% this is a visual novel".
        let games = vec![
            game(
                "1",
                "A",
                vec![
                    tag("ADV", 2.86),
                    tag("Built-in Encyclopedia", 1.4),
                    tag("Nakige", 2.98),
                ],
            ),
            game(
                "2",
                "B",
                vec![
                    tag("ADV", 2.87),
                    tag("Built-in Encyclopedia", 1.5),
                    tag("Nakige", 2.95),
                ],
            ),
        ];
        let built = profile(&games);
        let names: Vec<&str> = built.themes.iter().map(|t| t.name.as_str()).collect();
        assert!(!names.contains(&"ADV"), "got {names:?}");
        assert!(!names.contains(&"Built-in Encyclopedia"), "got {names:?}");
        assert!(names.contains(&"Nakige"), "got {names:?}");
    }

    #[test]
    fn a_game_with_only_boilerplate_counts_as_undetermined() {
        // If every tag is technical, the game says nothing about taste, and must be
        // reported rather than contributing a fake preference.
        let games = vec![
            game("1", "Real A", vec![tag("Nakige", 2.98)]),
            game("2", "Real B", vec![tag("Nakige", 2.96)]),
            game(
                "3",
                "Tech Only",
                vec![tag("ADV", 2.86), tag("Multiple Endings", 2.82)],
            ),
        ];
        let built = profile(&games);
        assert_eq!(built.considered, 2);
        assert_eq!(built.undetermined, vec!["Tech Only"]);
    }

    #[test]
    fn a_tag_on_one_game_is_not_a_preference() {
        // Describing a library is not the same as inventing one. With two games,
        // one shared and one unique, only the shared tag is a pattern.
        let games = vec![
            game("1", "A", vec![tag("Nakige", 2.98), tag("Unique", 2.97)]),
            game("2", "B", vec![tag("Nakige", 2.98)]),
        ];
        let built = profile(&games);
        assert_eq!(built.themes.len(), 1);
        assert_eq!(built.themes[0].name, "Nakige");
    }

    #[test]
    fn a_single_game_library_produces_no_themes() {
        // One game cannot show a pattern, however tempting the arithmetic.
        let games = vec![game("1", "Only", vec![tag("Nakige", 2.98)])];
        let built = profile(&games);
        assert_eq!(built.considered, 1);
        assert!(built.themes.is_empty());
    }

    #[test]
    fn spoiler_tags_are_excluded_from_taste() {
        // A plot-revealing tag is shown to a user who may not have finished the
        // game, so it is not taste.
        let games = vec![
            game(
                "1",
                "A",
                vec![tag("Nakige", 2.98), spoiler_tag("Heroine Dies")],
            ),
            game(
                "2",
                "B",
                vec![tag("Nakige", 2.98), spoiler_tag("Heroine Dies")],
            ),
        ];
        let built = profile(&games);
        let names: Vec<&str> = built.themes.iter().map(|t| t.name.as_str()).collect();
        assert!(!names.contains(&"Heroine Dies"), "got {names:?}");
        assert!(names.contains(&"Nakige"));
    }

    #[test]
    fn games_without_tags_are_reported_rather_than_silently_dropped() {
        // A library of ten where six lack metadata must not read as a profile of
        // four with no warning.
        let games = vec![
            game("1", "Described A", vec![tag("Nakige", 2.98)]),
            game("2", "Described B", vec![tag("Nakige", 2.98)]),
            game("3", "Bare One", vec![]),
            game("4", "Bare Two", vec![]),
        ];
        let built = profile(&games);
        assert_eq!(built.considered, 2);
        assert_eq!(built.undetermined, vec!["Bare One", "Bare Two"]);
    }

    #[test]
    fn a_repeated_tag_on_one_game_counts_once() {
        // VNDB should not repeat a tag, but a weighted average must not be
        // inflated if it ever does.
        let games = vec![
            game("1", "A", vec![tag("Nakige", 2.98), tag("Nakige", 2.90)]),
            game("2", "B", vec![tag("Nakige", 2.98)]),
        ];
        let built = profile(&games);
        assert_eq!(built.themes[0].count, 2, "two games, not three mentions");
        assert!((built.themes[0].share - 1.0).abs() < 1e-6);
    }

    #[test]
    fn an_empty_library_says_so_instead_of_crashing() {
        let built = profile(&[]);
        assert_eq!(built.considered, 0);
        assert!(built.themes.is_empty());
    }

    #[test]
    fn themes_are_ordered_strongest_first_and_stable() {
        let games = vec![
            game("1", "A", vec![tag("Rare", 2.98), tag("Common", 2.96)]),
            game("2", "B", vec![tag("Common", 2.96)]),
            game("3", "C", vec![tag("Common", 2.96)]),
        ];
        let built = profile(&games);
        assert_eq!(built.themes[0].name, "Common");
        // Same input, same order, so a re-render does not shuffle the summary.
        let again = profile(&games);
        assert_eq!(built.themes, again.themes);
    }

    #[test]
    fn a_tag_below_a_quarter_of_the_library_is_still_reported() {
        // The panel is a plain count, not a claim about "your type": a tag on
        // two games of a ten-game library is exactly what the list says it is.
        // The old 25% share floor hid such tags and made the panel read empty.
        let games = (0..10)
            .map(|index| {
                let tags = if index < 2 {
                    vec![tag("Rare", 2.98)]
                } else {
                    vec![tag("Common", 2.96)]
                };
                game(&(index + 1).to_string(), &format!("G{index}"), tags)
            })
            .collect::<Vec<_>>();
        let built = profile(&games);
        let names: Vec<&str> = built.themes.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"Rare"), "got {names:?}");
    }
}
