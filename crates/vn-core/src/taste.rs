//! Describing a library's taste deterministically from its own VNDB tags.
//!
//! Every claim is traceable to a direct tag count (no model, no network).
//! Spoiler tags (`spoiler > 0`) are excluded because taste summaries show
//! regardless of reading progress.

use serde::{Deserialize, Serialize};

/// A tag as it contributes to a profile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TagWeight {
    /// Tag id, e.g. `g505` (counting uses `name` so aliases do not split; "For you" uses `id`).
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    /// VNDB weight (0 to 3) for how strongly this tag defines the work.
    pub rating: f32,
    /// VNDB spoiler level: 0 none, 1 mild, 2 major (> 0 excluded).
    pub spoiler: u8,
}

/// One named person or company identified by its stable VNDB id (`p8`, `s123`),
/// because display names change via aliases and romanisations.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NamedEntity {
    pub id: String,
    pub name: String,
}

/// A staff credit carrying `role` (composer vs scenario writer are distinct signals).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StaffCredit {
    pub id: String,
    pub name: String,
    pub role: String,
}

/// One game as the profile needs it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfiledGame {
    /// Stable id so a claim can point at the source game.
    pub id: String,
    pub title: String,
    pub status: String,
    /// 1 to 10, when rated.
    pub rating: Option<u8>,
    pub tags: Vec<TagWeight>,
    /// VNDB work id used to collapse duplicate entries for the same work before tallying.
    pub work_id: Option<String>,
    #[serde(default)]
    pub developers: Vec<NamedEntity>,
    #[serde(default)]
    pub staff: Vec<StaffCredit>,
    #[serde(default)]
    pub voices: Vec<NamedEntity>,
}

/// A theme the library leans toward.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Theme {
    pub name: String,
    pub count: usize,
    /// Game titles, newest evidence first, capped for display.
    pub examples: Vec<String>,
    /// Share of considered games, 0 to 1.
    pub share: f32,
}

/// The computed taste profile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TasteProfile {
    pub considered: usize,
    /// Titles skipped for having no usable tags.
    pub undetermined: Vec<String>,
    pub themes: Vec<Theme>,
}

/// Tags below this VNDB rating are medium boilerplate (`ADV` 2.86, `Built-in Encyclopedia` < 2.86;
/// real taste signals sit higher: `Nakige` 2.98, `Family` 2.94).
pub const BOILERPLATE_BELOW: f32 = 2.88;

/// Minimum game count so a single-game library never invents a pattern.
pub(crate) const MINIMUM_COUNT: usize = 2;

const MAX_THEMES: usize = 12;
const MAX_EXAMPLES: usize = 3;

/// Library sizes are thousands at most, well inside `f32`'s exact integer range.
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
    // Sort by game count descending (matching displayed counts), breaking ties by name.
    let mut themes: Vec<Theme> = tally
        .into_iter()
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
            id: None,
            name: name.to_owned(),
            rating,
            spoiler: 0,
        }
    }

    fn spoiler_tag(name: &str) -> TagWeight {
        TagWeight {
            id: None,
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
            developers: Vec::new(),
            staff: Vec::new(),
            voices: Vec::new(),
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
        assert_eq!(built.themes.len(), 0, "built.themes should be empty");
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
        assert_eq!(built.themes.len(), 0, "built.themes should be empty");
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
