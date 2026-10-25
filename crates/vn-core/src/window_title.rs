//! Reading the current chapter from an engine's window title.
//!
//! # Why the title
//!
//! Some engines put their progress in the caption. Siglus does, verified live on
//! `Summer Pockets REFLECTION BLUE`:
//!
//! ```text
//! Summer Pockets REFLECTION BLUE Pocket ７月２６日
//! Summer Pockets REFLECTION BLUE 静久 ８月７日
//! Summer Pockets REFLECTION BLUE ７月３０日
//! Summer Pockets REFLECTION BLUE
//! ```
//!
//! Reading a caption needs no injection, no memory offsets and no unpacked
//! archives, so unlike an offset chain it keeps working across game updates.
//! Engines that put nothing useful in the title simply report no chapter.
//!
//! # What this module does not do
//!
//! It never guesses. A title with no recognisable marker yields [`TitleChapter::None`]
//! rather than a made-up chapter name, because a wrong chapter silently corrupts
//! playtime attribution and the user has no way to notice.

use serde::{Deserialize, Serialize};

/// What a window title says about the current chapter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TitleChapter {
    /// A chapter marker was found.
    Found {
        /// The route or section word, when the title carries one.
        section: Option<String>,
        /// The date or position marker, as written, e.g. `７月２６日`.
        position: String,
        /// Display name combining both, e.g. `Pocket ７月２６日`.
        name: String,
    },
    /// The title parsed but carries no chapter marker (a menu, a transition).
    None,
    /// The title does not belong to the game we are tracking.
    Foreign,
}

/// Full-width digits used by Japanese titles, mapped to ASCII.
///
/// Titles write `７月２６日` with full-width numerals, so a `\d` match finds
/// nothing. Normalising first keeps the marker pattern simple.
fn normalise_digits(text: &str) -> String {
    text.chars()
        .map(|ch| match ch {
            // Full-width ０..９ sit at U+FF10..U+FF19, ten in a row, so the offset
            // from the start of the run is the digit.
            '\u{FF10}'..='\u{FF19}' => char::from_u32(ch as u32 - 0xFF10 + 0x30).unwrap_or(ch),
            other => other,
        })
        .collect()
}

/// Whether a character can be part of a date marker.
fn is_marker_char(ch: char) -> bool {
    ch.is_ascii_digit() || ch.is_whitespace() || matches!(ch, '月' | '日' | '/' | '－' | '-')
}

/// Split a normalised title into `(before, marker)` for the trailing date.
///
/// Returns the text before the marker and the marker itself, or `None` when the
/// title does not end with a date.
fn split_trailing_marker(normalised: &str) -> Option<(&str, &str)> {
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

/// Parse a Siglus window title.
///
/// `game_title` is the library's name for the game, used to strip the prefix the
/// engine puts in front of the chapter. Matching is lenient about case and
/// whitespace because the caption is not always byte-identical to the library
/// entry.
#[must_use]
pub fn parse_siglus_title(title: &str, game_title: &str) -> TitleChapter {
    let normalised = normalise_digits(title);
    let trimmed = normalised.trim();

    if trimmed.is_empty() {
        return TitleChapter::None;
    }

    // Cut the game name off the front.
    //
    // The engine writes its own caption, which is not always byte-identical to the
    // library entry: it can be shorter ("Summer Pockets" for "Summer Pockets
    // REFLECTION BLUE"). Try the full name, then the longest leading run of its
    // words that the caption actually starts with.
    let game = game_title.trim();
    let mut rest = strip_prefix_ignore_ascii_case(trimmed, game).map(str::trim_start);
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
}
