//! Window-title chapter parsing, under user control.
//!
//! # Why
//!
//! Some engines write their progress into the window caption. Siglus does, and
//! the format is verified; other engines may too, in formats nobody here has
//! seen. Hardcoding one engine means a user with a different game gets nothing
//! and no way to say so.
//!
//! # The design rule
//!
//! A pattern that does not match must be **visible**, not silent. A user-supplied
//! regex that quietly never matches would look exactly like an engine that does
//! not write titles, and the user would have no way to tell which. So every
//! saved pattern is exercised against the tester before it is trusted, and the
//! live parser reports "pattern did not match" rather than "no chapter".
//!
//! # Safety of user patterns
//!
//! The `regex` crate guarantees linear-time matching, so a pathological pattern
//! cannot hang the tracking loop. Compiled patterns are length-capped anyway.

use serde::{Deserialize, Serialize};

/// Which built-in engine format to try.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TitleFormat {
    /// Do not read titles at all.
    Off,
    /// The verified Siglus caption format.
    Siglus,
    /// A pattern supplied by the user.
    Custom,
}

impl TitleFormat {
    /// The stored string form.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Siglus => "siglus",
            Self::Custom => "custom",
        }
    }

    /// Parse the stored form, falling back to `Off`.
    ///
    /// Falling back to `Off` rather than to `Siglus` is deliberate: an unknown
    /// value means this build does not know what the user chose, and silently
    /// guessing a format would attribute chapters from a format they did not
    /// pick.
    #[must_use]
    pub fn parse(value: &str) -> Self {
        match value {
            "siglus" => Self::Siglus,
            "custom" => Self::Custom,
            _ => Self::Off,
        }
    }
}

/// The user's title-reading configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TitleSettings {
    /// Which format to use.
    pub format: TitleFormat,
    /// The user's pattern, when `format` is `Custom`.
    ///
    /// Capture groups are taken positionally: the first is the route or section,
    /// the second is the position. One group is treated as the position alone.
    pub pattern: Option<String>,
}

impl Default for TitleSettings {
    fn default() -> Self {
        // The verified format, on: this is what the app shipped with, and turning
        // it off by default would silently regress a working feature.
        Self {
            format: TitleFormat::Siglus,
            pattern: None,
        }
    }
}

/// Maximum pattern length accepted.
///
/// A pattern longer than this is either a paste accident or an attempt to make
/// the tracker work hard; neither is worth supporting.
pub const MAX_PATTERN_LEN: usize = 512;

/// What a matching title yields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedTitle {
    /// Route or section label, when the pattern captured one.
    pub route: Option<String>,
    /// Position or date label.
    pub position: String,
}

/// A validated custom pattern.
#[derive(Debug, Clone)]
pub struct CustomPattern {
    regex: regex::Regex,
}

/// Why a pattern was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatternError {
    /// Empty or whitespace-only.
    Empty,
    /// Longer than [`MAX_PATTERN_LEN`].
    TooLong,
    /// Did not compile.
    Invalid(String),
    /// Compiled, but has no capture group to read.
    NoCapture,
}

impl std::fmt::Display for PatternError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => write!(f, "the pattern is empty"),
            Self::TooLong => write!(f, "the pattern is too long"),
            Self::Invalid(why) => write!(f, "the pattern is not valid: {why}"),
            Self::NoCapture => write!(
                f,
                "the pattern must have at least one (...) group to capture a chapter"
            ),
        }
    }
}

impl CustomPattern {
    /// Compile a user pattern.
    ///
    /// # Errors
    /// [`PatternError`] describing what is wrong, in words a user can act on.
    pub fn new(source: &str) -> Result<Self, PatternError> {
        let trimmed = source.trim();
        if trimmed.is_empty() {
            return Err(PatternError::Empty);
        }
        if trimmed.len() > MAX_PATTERN_LEN {
            return Err(PatternError::TooLong);
        }
        let regex =
            regex::Regex::new(trimmed).map_err(|err| PatternError::Invalid(err.to_string()))?;
        if regex.captures_len() < 2 {
            return Err(PatternError::NoCapture);
        }
        Ok(Self { regex })
    }

    /// Read a caption with this pattern.
    #[must_use]
    pub fn parse(&self, title: &str) -> Option<ParsedTitle> {
        let captures = self.regex.captures(title)?;
        let groups: Vec<&str> = captures
            .iter()
            .skip(1)
            .flatten()
            .map(|m| m.as_str())
            .collect();
        match groups.as_slice() {
            // Two or more groups: route then position.
            [route, position, ..] => Some(ParsedTitle {
                route: Some((*route).trim().to_owned()).filter(|text| !text.is_empty()),
                position: (*position).trim().to_owned(),
            }),
            // One group: it is the chapter itself.
            [only] => Some(ParsedTitle {
                route: None,
                position: (*only).trim().to_owned(),
            }),
            [] => None,
        }
    }
}

/// One sample the tester shows the user.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PatternTest {
    /// The caption that was tested.
    pub title: String,
    /// Whether it matched.
    pub matched: bool,
    /// What was read, when it matched.
    pub route: Option<String>,
    /// What was read, when it matched.
    pub position: Option<String>,
}

/// Run a pattern against sample captions, without saving it.
///
/// # Errors
/// [`PatternError`] when the pattern itself is unusable, so the UI can say why
/// rather than showing "no match" for a broken pattern.
pub fn test_pattern(source: &str, titles: &[String]) -> Result<Vec<PatternTest>, PatternError> {
    let pattern = CustomPattern::new(source)?;
    Ok(titles
        .iter()
        .map(|title| match pattern.parse(title) {
            Some(parsed) => PatternTest {
                title: title.clone(),
                matched: true,
                route: parsed.route,
                position: Some(parsed.position),
            },
            None => PatternTest {
                title: title.clone(),
                matched: false,
                route: None,
                position: None,
            },
        })
        .collect())
}

/// Read a caption using the user's settings.
///
/// Returns `None` both when the format is off and when nothing matched. The
/// caller distinguishes them by [`TitleSettings::format`], because "off" is a
/// choice and "no match" is a finding.
#[must_use]
pub fn parse_with(settings: &TitleSettings, title: &str, game_title: &str) -> Option<ParsedTitle> {
    match settings.format {
        TitleFormat::Off => None,
        TitleFormat::Siglus => match super::window_title::parse_siglus_title(title, game_title) {
            super::window_title::TitleChapter::Found {
                section, position, ..
            } => Some(ParsedTitle {
                route: section,
                position,
            }),
            _ => None,
        },
        TitleFormat::Custom => {
            let source = settings.pattern.as_deref()?;
            CustomPattern::new(source).ok()?.parse(title)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_groups_read_as_route_and_position() {
        let pattern = CustomPattern::new(r"^(.*?) - (.+)$").unwrap();
        let parsed = pattern.parse("Game - Chapter 3").unwrap();
        assert_eq!(parsed.route.as_deref(), Some("Game"));
        assert_eq!(parsed.position, "Chapter 3");
    }

    #[test]
    fn one_group_reads_as_the_position() {
        let pattern = CustomPattern::new(r"\[(.+)\]").unwrap();
        let parsed = pattern.parse("My Game [Chapter 3]").unwrap();
        assert_eq!(parsed.route, None);
        assert_eq!(parsed.position, "Chapter 3");
    }

    #[test]
    fn a_pattern_without_a_group_is_rejected_with_a_reason() {
        // A pattern that captures nothing can never produce a chapter, and
        // accepting it would look exactly like a game with no title support.
        let error = CustomPattern::new(r"^Chapter \d+$").unwrap_err();
        assert_eq!(error, PatternError::NoCapture);
        assert!(error.to_string().contains("group"));
    }

    #[test]
    fn a_broken_pattern_reports_why() {
        let error = CustomPattern::new(r"(unclosed").unwrap_err();
        assert!(matches!(error, PatternError::Invalid(_)));
    }

    #[test]
    fn an_empty_pattern_is_rejected() {
        assert_eq!(CustomPattern::new("   ").unwrap_err(), PatternError::Empty);
    }

    #[test]
    fn an_absurdly_long_pattern_is_rejected() {
        let long = format!("({})", "a".repeat(MAX_PATTERN_LEN));
        assert_eq!(
            CustomPattern::new(&long).unwrap_err(),
            PatternError::TooLong
        );
    }

    #[test]
    fn a_non_matching_title_returns_none_rather_than_empty_strings() {
        let pattern = CustomPattern::new(r"\[(.+)\]").unwrap();
        assert!(pattern.parse("Game with no marker").is_none());
    }

    #[test]
    fn the_tester_reports_each_sample_separately() {
        let samples = vec!["Game - One".to_owned(), "no marker".to_owned()];
        let results = test_pattern(r"^(.*?) - (.+)$", &samples).unwrap();
        assert_eq!(results.len(), 2);
        assert!(results[0].matched);
        assert!(!results[1].matched);
        assert_eq!(results[0].position.as_deref(), Some("One"));
    }

    #[test]
    fn a_broken_pattern_makes_the_tester_error_instead_of_reporting_no_matches() {
        // Otherwise a typo would read as "your game has no chapters".
        assert!(test_pattern(r"([", &["x".to_owned()]).is_err());
    }

    #[test]
    fn the_default_is_the_verified_format_and_on() {
        // Shipping with titles off would silently break a working feature.
        let settings = TitleSettings::default();
        assert_eq!(settings.format, TitleFormat::Siglus);
    }

    #[test]
    fn an_unknown_stored_value_falls_back_to_off_not_siglus() {
        // Guessing a format would attribute chapters from a format the user did
        // not choose.
        assert_eq!(TitleFormat::parse("wat"), TitleFormat::Off);
        assert_eq!(TitleFormat::parse(""), TitleFormat::Off);
    }

    #[test]
    fn format_strings_round_trip() {
        for format in [TitleFormat::Off, TitleFormat::Siglus, TitleFormat::Custom] {
            assert_eq!(TitleFormat::parse(format.as_str()), format);
        }
    }

    #[test]
    fn off_never_parses() {
        let settings = TitleSettings {
            format: TitleFormat::Off,
            pattern: Some(r".*".to_owned()),
        };
        assert!(parse_with(&settings, "anything", "anything").is_none());
    }

    #[test]
    fn custom_settings_are_used_when_selected() {
        let settings = TitleSettings {
            format: TitleFormat::Custom,
            pattern: Some(r"^(.*) - (.+)$".to_owned()),
        };
        let parsed = parse_with(&settings, "Game - Ch 2", "Game").unwrap();
        assert_eq!(parsed.position, "Ch 2");
    }

    #[test]
    fn custom_without_a_pattern_parses_nothing() {
        let settings = TitleSettings {
            format: TitleFormat::Custom,
            pattern: None,
        };
        assert!(parse_with(&settings, "Game - Ch 2", "Game").is_none());
    }

    #[test]
    fn siglus_settings_still_use_the_verified_parser() {
        let settings = TitleSettings::default();
        let parsed = parse_with(
            &settings,
            "Summer Pockets REFLECTION BLUE 静久 ８月７日",
            "Summer Pockets REFLECTION BLUE",
        );
        assert!(parsed.is_some(), "the verified format must keep working");
    }
}
