//! Reading the current chapter from an engine's window title.
//!
//! # Why the title
//!
//! Some engines put their progress in the caption. Siglus does, verified live on
//! two games:
//!
//! ```text
//! Summer Pockets REFLECTION BLUE Pocket ７月２６日
//! Summer Pockets REFLECTION BLUE 静久 ８月７日
//! Summer Pockets REFLECTION BLUE ７月３０日
//! Summer Pockets REFLECTION BLUE
//! anemoi Ｄａｙ １
//! ```
//!
//! The last sample is not a date: the marker is a label and a number, and every
//! character of it is full-width (`Ｄ` U+FF24, `１` U+FF11). Both shapes are read
//! here. Reading a caption needs no injection, no memory offsets and no unpacked
//! archives, so unlike an offset chain it keeps working across game updates.
//! Engines that put nothing useful in the title simply report no chapter.
//!
//! # What this module does not do
//!
//! It never guesses. A title with no recognisable marker yields [`TitleChapter::None`]
//! rather than a made-up chapter name, because a wrong chapter silently corrupts
//! playtime attribution and the user has no way to notice. A trailing number is
//! not enough on its own: `Save Slot 3` is a menu, so the number has to follow a
//! word that means a chapter, or a date unit.

use serde::{Deserialize, Serialize};

/// What a window title says about the current chapter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TitleChapter {
    /// A chapter marker was found.
    Found {
        /// The route or section word, when the title carries one.
        section: Option<String>,
        /// The date or position marker, as written, e.g. `７月２６日` or `Day 1`.
        position: String,
        /// Display name combining both, e.g. `Pocket ７月２６日`.
        name: String,
    },
    /// The title parsed but carries no chapter marker (a menu, a transition).
    None,
    /// The title does not belong to the game we are tracking.
    Foreign,
}

/// Fold the full-width forms a Japanese caption uses onto ASCII.
///
/// Titles write `７月２６日` and `Ｄａｙ １` with full-width characters, so a `\d`
/// or `Day` match finds nothing: `Ｄ` is U+FF24, not `D`. Full-width Latin,
/// digits and punctuation all sit in U+FF01..=U+FF5E, exactly 0xFEE0 above the
/// ASCII they stand for, and the ideographic space (U+3000) is a word break like
/// any other. Normalising first keeps every marker pattern simple.
fn normalise_width(text: &str) -> String {
    text.chars()
        .map(|ch| match ch {
            '\u{FF01}'..='\u{FF5E}' => char::from_u32(ch as u32 - 0xFEE0).unwrap_or(ch),
            '\u{3000}' => ' ',
            other => other,
        })
        .collect()
}

/// Whether a character can be part of a date marker.
fn is_marker_char(ch: char) -> bool {
    ch.is_ascii_digit() || ch.is_whitespace() || matches!(ch, '月' | '日' | '/' | '-')
}

/// Split a normalised title into `(before, marker)` for the trailing date.
///
/// Returns the text before the marker and the marker itself, or `None` when the
/// title does not end with a date.
fn split_trailing_date(normalised: &str) -> Option<(&str, &str)> {
    let trimmed = normalised.trim_end();
    let start = trimmed
        .char_indices()
        .rev()
        .take_while(|(_, ch)| is_marker_char(*ch))
        .last()
        .map(|(index, _)| index)?;
    let marker = &trimmed[start..];
    // A marker must contain a digit and one of the date units. Without this a
    // title ending in a bare number would be mistaken for a date.
    let has_digit = marker.chars().any(|ch| ch.is_ascii_digit());
    let has_unit = marker.contains('月') || marker.contains('/') || marker.contains('-');
    if !has_digit || !has_unit {
        return None;
    }
    Some((&trimmed[..start], marker))
}

/// Byte offset where the trailing run of ASCII digits starts.
///
/// The run is maximal, so the character before it is never a digit and may be a
/// space (`Day 1`) or a letter (`Day1`). `None` when the text does not end in a
/// digit at all.
fn trailing_digits_start(text: &str) -> Option<usize> {
    text.char_indices()
        .rev()
        .take_while(|(_, ch)| ch.is_ascii_digit())
        .last()
        .map(|(index, _)| index)
}

/// Words that turn a trailing number into a chapter position.
///
/// The label is what separates a chapter from a count: a caption ending in
/// `Slot 3` or `Save 2` is a menu, and crediting that number as a chapter would
/// attribute playtime to a chapter that does not exist. Only words whose meaning
/// is unambiguous are listed; `Day` is verified live on `anemoi`, the rest are
/// the same shape.
const COUNTER_LABELS: &[&str] = &[
    "day", "chapter", "ch", "ch.", "episode", "ep", "ep.", "part",
];

/// Split `(before, marker)` for a labelled counter such as `Day 1`.
///
/// Both `Day 1` and `Day1` are accepted, because the engine decides where the
/// space goes. The marker is returned whole, label included: the position is
/// what the UI shows, and `Day 1` is how the game names that chapter.
fn split_trailing_label(title: &str) -> Option<(&str, &str)> {
    let trimmed = title.trim_end();
    let number_start = trailing_digits_start(trimmed)?;
    let head = trimmed[..number_start].trim_end();
    let label_start = head
        .char_indices()
        .rev()
        .take_while(|(_, ch)| ch.is_ascii_alphabetic() || *ch == '.')
        .last()
        .map(|(index, _)| index)?;
    let label = &head[label_start..];
    if !COUNTER_LABELS.contains(&label.to_ascii_lowercase().as_str()) {
        return None;
    }
    let before = head[..label_start].trim_end();
    Some((before, &trimmed[before.len()..]))
}

/// Split `(before, marker)` for a Japanese counter: `第3話`, `第2章`, `2日目`.
///
/// `第` is required before an episode or chapter unit, so a menu caption such as
/// `全12話` (the episode total) does not become a chapter, and the day counter
/// only counts with `目` — which is what keeps it apart from the date `7月26日`.
fn split_trailing_counter(title: &str) -> Option<(&str, &str)> {
    let trimmed = title.trim_end();
    let (without_unit, unit) = trimmed
        .strip_suffix("日目")
        .map(|rest| (rest, "日目"))
        .or_else(|| {
            ["話", "章", "幕"]
                .into_iter()
                .find_map(|unit| trimmed.strip_suffix(unit).map(|rest| (rest, unit)))
        })?;
    let mut marker_start = trailing_digits_start(without_unit)?;
    if unit != "日目" {
        // `第` belongs to the marker, so the section is what comes before it.
        marker_start = without_unit[..marker_start].strip_suffix('第')?.len();
    }
    let before = trimmed[..marker_start].trim_end();
    Some((before, &trimmed[before.len()..]))
}

/// Split a normalised title into `(before, marker)` for a trailing chapter marker.
///
/// Dates are tried first because `７月２６日` is the older verified shape and must
/// keep parsing exactly as it did; then a labelled counter (`Day 1`), then a
/// Japanese counter (`第3話`).
fn split_trailing_marker(normalised: &str) -> Option<(&str, &str)> {
    split_trailing_date(normalised)
        .or_else(|| split_trailing_label(normalised))
        .or_else(|| split_trailing_counter(normalised))
}

/// Parse a Siglus window title.
///
/// `game_title` is the library's name for the game, used to strip the prefix the
/// engine puts in front of the chapter. Matching is lenient about case and
/// whitespace because the caption is not always byte-identical to the library
/// entry.
#[must_use]
pub fn parse_siglus_title(title: &str, game_title: &str) -> TitleChapter {
    let normalised = normalise_width(title);
    let trimmed = normalised.trim();

    if trimmed.is_empty() {
        return TitleChapter::None;
    }

    // Cut the game name off the front.
    //
    // The engine writes its own caption, which is not always byte-identical to the
    // library entry: it can be shorter ("Summer Pockets" for "Summer Pockets
    // REFLECTION BLUE") or written in full-width characters. Try the full name,
    // then the longest leading run of its words that the caption actually starts
    // with.
    let game = normalise_width(game_title.trim());
    let mut rest = strip_prefix_ignore_ascii_case(trimmed, &game).map(str::trim_start);
    if rest.is_none() {
        let mut words: Vec<&str> = game.split_whitespace().collect();
        while !words.is_empty() {
            let candidate = words.join(" ");
            if let Some(tail) = strip_prefix_ignore_ascii_case(trimmed, &candidate) {
                rest = Some(tail.trim_start());
                break;
            }
            words.pop();
        }
    }

    let Some(rest) = rest else {
        // The caption does not start with the game name, so it is another window.
        return TitleChapter::Foreign;
    };

    let Some((before, marker)) = split_trailing_marker(rest) else {
        // A bare game name means the engine is showing no chapter: a menu, a
        // transition, or the title screen.
        return TitleChapter::None;
    };

    let section = before.trim();
    let position = marker.trim().to_owned();
    let section_option = (!section.is_empty()).then(|| section.to_owned());
    let name = match &section_option {
        Some(section) => format!("{section} {position}"),
        None => position.clone(),
    };

    TitleChapter::Found {
        section: section_option,
        position,
        name,
    }
}

/// Strip `prefix` from the front of `text`, comparing ASCII case-insensitively.
///
/// Only ASCII is folded, so a byte offset from the original string stays valid;
/// folding a non-ASCII prefix could change its length and break the slice.
fn strip_prefix_ignore_ascii_case<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let head = text.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| &text[prefix.len()..])
}

/// Whether an engine's titles are known to carry chapter information.
///
/// Only Siglus has verified samples. Claiming the capability for an engine without
/// a sample would put a chapter on screen that came from parsing the game's name.
#[must_use]
pub fn engine_has_title_chapters(engine: &str) -> bool {
    matches!(engine, "siglus")
}

#[cfg(test)]
mod tests {
    use super::*;

    const GAME: &str = "Summer Pockets REFLECTION BLUE";

    #[test]
    fn a_section_and_date_are_both_captured() {
        // Observed live.
        assert_eq!(
            parse_siglus_title("Summer Pockets REFLECTION BLUE Pocket ７月２６日", GAME),
            TitleChapter::Found {
                section: Some("Pocket".to_owned()),
                position: "7月26日".to_owned(),
                name: "Pocket 7月26日".to_owned(),
            }
        );
    }

    #[test]
    fn a_different_section_is_captured() {
        // Observed live: the section word changes with the route.
        assert_eq!(
            parse_siglus_title("Summer Pockets REFLECTION BLUE 静久 ８月７日", GAME),
            TitleChapter::Found {
                section: Some("静久".to_owned()),
                position: "8月7日".to_owned(),
                name: "静久 8月7日".to_owned(),
            }
        );
    }

    #[test]
    fn a_date_without_a_section_is_captured() {
        // Observed live: the section is optional.
        assert_eq!(
            parse_siglus_title("Summer Pockets REFLECTION BLUE ７月３０日", GAME),
            TitleChapter::Found {
                section: None,
                position: "7月30日".to_owned(),
                name: "7月30日".to_owned(),
            }
        );
    }

    #[test]
    fn a_bare_game_name_means_no_chapter() {
        // Observed live at menus and transitions. Reporting the game name as a
        // chapter would attribute playtime to a chapter that does not exist.
        assert_eq!(
            parse_siglus_title("Summer Pockets REFLECTION BLUE", GAME),
            TitleChapter::None
        );
        assert_eq!(
            parse_siglus_title("Summer Pockets REFLECTION BLUE ", GAME),
            TitleChapter::None
        );
    }

    #[test]
    fn another_application_is_not_mistaken_for_the_game() {
        assert_eq!(
            parse_siglus_title("Windows PowerShell", GAME),
            TitleChapter::Foreign
        );
        assert_eq!(
            parse_siglus_title("Untitled - Notepad", GAME),
            TitleChapter::Foreign
        );
    }

    #[test]
    fn an_empty_title_is_no_chapter() {
        assert_eq!(parse_siglus_title("", GAME), TitleChapter::None);
        assert_eq!(parse_siglus_title("   ", GAME), TitleChapter::None);
    }

    #[test]
    fn a_trailing_number_without_a_date_unit_is_not_a_marker() {
        // `Save Slot 3` must not become a chapter.
        assert_eq!(
            parse_siglus_title("Summer Pockets REFLECTION BLUE 3", GAME),
            TitleChapter::None
        );
    }

    #[test]
    fn ascii_digits_parse_the_same_as_full_width() {
        // Localised builds may write half-width digits.
        assert_eq!(
            parse_siglus_title("Summer Pockets REFLECTION BLUE 7月26日", GAME),
            parse_siglus_title("Summer Pockets REFLECTION BLUE ７月２６日", GAME)
        );
    }

    #[test]
    fn the_game_prefix_match_is_case_insensitive() {
        assert!(matches!(
            parse_siglus_title("summer pockets reflection blue ７月２６日", GAME),
            TitleChapter::Found { .. }
        ));
    }

    #[test]
    fn the_game_prefix_may_be_cut_short() {
        // The engine's caption is not always identical to the library title.
        assert!(matches!(
            parse_siglus_title("Summer Pockets Pocket ７月２６日", GAME),
            TitleChapter::Found { .. }
        ));
    }

    #[test]
    fn a_section_containing_spaces_is_kept_whole() {
        assert_eq!(
            parse_siglus_title(
                "Summer Pockets REFLECTION BLUE Extra Story ７月２６日",
                GAME
            ),
            TitleChapter::Found {
                section: Some("Extra Story".to_owned()),
                position: "7月26日".to_owned(),
                name: "Extra Story 7月26日".to_owned(),
            }
        );
    }

    #[test]
    fn only_siglus_is_known_to_use_titles_for_chapters() {
        assert!(engine_has_title_chapters("siglus"));
        assert!(!engine_has_title_chapters("kirikiri"));
        assert!(!engine_has_title_chapters("artemis"));
        assert!(!engine_has_title_chapters("unknown"));
    }

    // --- labelled counters -------------------------------------------------
    //
    // `anemoi` writes a day number instead of a date, and writes the whole marker
    // in full-width characters. Observed live: `anemoi Ｄａｙ １`.

    const ANEMOI: &str = "anemoi";

    #[test]
    fn a_full_width_day_marker_is_captured() {
        assert_eq!(
            parse_siglus_title("anemoi Ｄａｙ １", ANEMOI),
            TitleChapter::Found {
                section: None,
                position: "Day 1".to_owned(),
                name: "Day 1".to_owned(),
            }
        );
    }

    #[test]
    fn the_day_marker_reads_the_same_in_half_width() {
        assert_eq!(
            parse_siglus_title("anemoi Day 1", ANEMOI),
            parse_siglus_title("anemoi Ｄａｙ １", ANEMOI)
        );
        // A lower-case label is recognised too, and the position keeps the case
        // the caption used, because that is what the user sees in the game.
        assert_eq!(
            parse_siglus_title("anemoi day 1", ANEMOI),
            TitleChapter::Found {
                section: None,
                position: "day 1".to_owned(),
                name: "day 1".to_owned(),
            }
        );
    }

    #[test]
    fn a_katakana_section_before_a_chapter_number_is_kept() {
        // Observed live on another route of `anemoi`: the caption names the route
        // in katakana and writes `Chapter` and the number full-width.
        assert_eq!(
            parse_siglus_title("anemoi アネモイ Ｃｈａｐｔｅｒ ０１", ANEMOI),
            TitleChapter::Found {
                section: Some("アネモイ".to_owned()),
                position: "Chapter 01".to_owned(),
                name: "アネモイ Chapter 01".to_owned(),
            }
        );
    }

    #[test]
    fn the_day_number_is_kept_with_its_label() {
        // The position is what the UI shows: `Day 12`, not `12`. A bare number
        // loses the only part that says what it counts.
        assert_eq!(
            parse_siglus_title("anemoi Ｄａｙ １２", ANEMOI),
            TitleChapter::Found {
                section: None,
                position: "Day 12".to_owned(),
                name: "Day 12".to_owned(),
            }
        );
    }

    #[test]
    fn a_section_before_a_day_marker_is_kept() {
        assert_eq!(
            parse_siglus_title("anemoi アオイ Ｄａｙ ３", ANEMOI),
            TitleChapter::Found {
                section: Some("アオイ".to_owned()),
                position: "Day 3".to_owned(),
                name: "アオイ Day 3".to_owned(),
            }
        );
    }

    #[test]
    fn other_chapter_words_work_the_same_way() {
        for (title, position) in [
            ("anemoi Chapter 2", "Chapter 2"),
            ("anemoi Episode 4", "Episode 4"),
            ("anemoi Part 5", "Part 5"),
            ("anemoi Ch. 2", "Ch. 2"),
            ("anemoi Ep 7", "Ep 7"),
        ] {
            assert_eq!(
                parse_siglus_title(title, ANEMOI),
                TitleChapter::Found {
                    section: None,
                    position: position.to_owned(),
                    name: position.to_owned(),
                },
                "{title}"
            );
        }
    }

    #[test]
    fn a_number_after_an_unrelated_word_is_not_a_chapter() {
        // `Save Slot 3` and `Volume 2` are menus. Only a word that means a
        // chapter turns a number into one; anything else stays silent, because a
        // made-up chapter corrupts attribution silently.
        assert_eq!(
            parse_siglus_title("anemoi Save Slot 3", ANEMOI),
            TitleChapter::None
        );
        assert_eq!(
            parse_siglus_title("anemoi Volume 2", ANEMOI),
            TitleChapter::None
        );
        // A label with no number behind it is not a position either.
        assert_eq!(parse_siglus_title("anemoi Day", ANEMOI), TitleChapter::None);
        assert_eq!(
            parse_siglus_title("anemoi Chapter", ANEMOI),
            TitleChapter::None
        );
    }

    #[test]
    fn a_japanese_counter_is_captured() {
        assert_eq!(
            parse_siglus_title("anemoi 第３話", ANEMOI),
            TitleChapter::Found {
                section: None,
                position: "第3話".to_owned(),
                name: "第3話".to_owned(),
            }
        );
        assert_eq!(
            parse_siglus_title("anemoi ２日目", ANEMOI),
            TitleChapter::Found {
                section: None,
                position: "2日目".to_owned(),
                name: "2日目".to_owned(),
            }
        );
        assert_eq!(
            parse_siglus_title("anemoi アオイ 第2章", ANEMOI),
            TitleChapter::Found {
                section: Some("アオイ".to_owned()),
                position: "第2章".to_owned(),
                name: "アオイ 第2章".to_owned(),
            }
        );
    }

    #[test]
    fn an_episode_total_is_not_a_chapter() {
        // `全12話` on a menu is a count, not the current episode: `第` is what
        // makes the unit a marker.
        assert_eq!(
            parse_siglus_title("anemoi 全12話", ANEMOI),
            TitleChapter::None
        );
    }

    #[test]
    fn a_full_width_game_name_still_matches() {
        // The caption and the library entry can both be full-width.
        assert!(matches!(
            parse_siglus_title("ａｎｅｍｏｉ Ｄａｙ １", ANEMOI),
            TitleChapter::Found { .. }
        ));
    }

    #[test]
    fn a_foreign_caption_is_still_foreign() {
        // The day marker must not make the parser accept another window.
        assert_eq!(
            parse_siglus_title("Day 1 - Notepad", ANEMOI),
            TitleChapter::Foreign
        );
    }
}
