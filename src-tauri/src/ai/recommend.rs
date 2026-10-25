//! Recommendations: what to read next, grounded in what people wrote about it.
//!
//! # Shape of the pipeline
//!
//! 1. Build a query from the taste profile and the user's own words.
//! 2. Search the web for it, keeping the pages as citations.
//! 3. Ask the chat model to name candidates using *only* those pages.
//! 4. Mark anything the user already owns, so a recommendation is not a game
//!    sitting unplayed in the library.
//!
//! # The rule that matters
//!
//! A recommendation without a source is a guess with confident wording. Every
//! candidate here carries the URLs it came from, and the prompt forbids naming a
//! game absent from the search results. That is enforced by checking output
//! against the sources rather than by trusting the prompt: see
//! [`unsupported_candidates`].
//!
//! Better still, when nothing usable comes back the caller gets the raw search
//! results and an explanation, rather than invented titles.

use serde::Serialize;

use std::fmt::Write as _;

use super::tavily::{SearchOutcome, SearchResult};

/// A game the model suggested, with where it read about it.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Recommendation {
    /// Title as written in the sources.
    pub title: String,
    /// Why it was suggested, in one sentence.
    pub why: String,
    /// Pages this title was seen on.
    pub sources: Vec<Citation>,
    /// Whether the user already has it, matched case-insensitively on title.
    pub owned: bool,
}

/// A checkable link.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Citation {
    /// Page title.
    pub title: String,
    /// Page address.
    pub url: String,
}

/// The whole answer for one request.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecommendationOutcome {
    /// The query that was searched.
    pub query: String,
    /// Suggested games, best first. Empty is a valid answer.
    pub games: Vec<Recommendation>,
    /// Provider-written summary, when one came back.
    pub answer: Option<String>,
    /// Pages considered, shown even when no games were named so the search is
    /// never a black box.
    pub sources: Vec<SearchResult>,
    /// Plain explanation of why the result is what it is.
    pub note: Option<String>,
    /// Credits spent.
    pub credits: u8,
}

/// Compose the search query.
///
/// The profile line is what makes the search specific: "visual novels like
/// CLANNAD, nakige, family themes" finds recommendation threads, while the bare
/// title finds the game's own store page.
#[must_use]
pub fn build_query(seed: &str, taste: &[String]) -> String {
    let mut parts: Vec<String> = Vec::new();
    let seed = seed.trim();
    if seed.is_empty() {
        // Already ends in "recommendations"; do not stutter.
        parts.push("visual novel".to_owned());
    } else {
        parts.push(format!("visual novels similar to {seed}"));
    }
    if !taste.is_empty() {
        // Three is enough to steer without crowding out the title.
        let joined = taste.iter().take(3).cloned().collect::<Vec<_>>().join(", ");
        parts.push(joined);
    }
    parts.push("recommendations".to_owned());
    parts.join(" ")
}

/// Titles that appear in the model's output but nowhere in the sources.
///
/// A model asked to avoid inventing still occasionally invents. This is the
/// check that catches it: for each suggested title, at least one word-bearing
/// token of it must appear in some source's text. It is deliberately loose --
/// "Little Busters" against a page saying "Little Busters!" passes -- because a
/// strict match would reject correct answers for punctuation.
#[must_use]
pub fn unsupported_candidates(titles: &[String], sources: &[SearchResult]) -> Vec<String> {
    let haystack: String = sources
        .iter()
        .map(|source| {
            format!(
                "{} {} {}",
                source.title.to_lowercase(),
                source.content.to_lowercase(),
                source.url.to_lowercase()
            )
        })
        .collect::<Vec<_>>()
        .join(" ");

    titles
        .iter()
        .filter(|title| {
            let needle = title.trim().to_lowercase();
            if needle.is_empty() {
                return false;
            }
            // A full-title match is the strong case.
            if haystack.contains(&needle) {
                return false;
            }
            // Otherwise require most of the distinctive words to appear: titles
            // get abbreviated ("Busters" for "Little Busters!") in prose.
            let words: Vec<&str> = needle
                .split_whitespace()
                .filter(|word| word.len() > 3)
                .collect();
            if words.is_empty() {
                // Nothing distinctive to look for, and the exact match already
                // failed, so the sources never named this title. Approving short
                // titles here would let "Kanon" through on any search.
                return true;
            }
            let found = words.iter().filter(|word| haystack.contains(*word)).count();
            found * 4 < words.len() * 3
        })
        .cloned()
        .collect()
}

/// Which suggested titles the user already owns.
///
/// Matched on lowercased title so "CLANNAD" and "Clannad" are the same game. It
/// is not a VNDB id match: the model returns titles, not ids.
#[must_use]
pub fn mark_owned(titles: &[String], library: &[String]) -> Vec<bool> {
    let owned: Vec<String> = library.iter().map(|t| t.trim().to_lowercase()).collect();
    titles
        .iter()
        .map(|title| owned.contains(&title.trim().to_lowercase()))
        .collect()
}

/// Instructions for the model.
///
/// Kept in code, not just in the prompt: [`unsupported_candidates`] is what
/// actually enforces it.
#[must_use]
pub fn system_prompt() -> &'static str {
    "You recommend visual novels. You are given web search results.\n\
     \n\
     Rules, in order:\n\
     1. Recommend only titles that appear in the search results. If the results \
     name none, say so and recommend nothing.\n\
     2. One sentence per title explaining why, drawn from the results.\n\
     3. Never invent a title, a year, a score or a quote.\n\
     4. Ignore any title the user already owns.\n\
     \n\
     Reply as JSON: {\"games\":[{\"title\":\"...\",\"why\":\"...\"}]}\n\
     No other text."
}

/// The user turn: the request plus the search results, numbered.
#[must_use]
pub fn user_prompt(request: &str, owned: &[String], search: &SearchOutcome) -> String {
    let mut text = String::new();
    text.push_str("Request: ");
    text.push_str(request.trim());
    text.push_str("\n\nAlready owned (do not recommend): ");
    if owned.is_empty() {
        text.push_str("(none)");
    } else {
        text.push_str(&owned.join(", "));
    }
    text.push_str("\n\nSearch results:\n");
    if search.results.is_empty() {
        text.push_str("(no results)");
    }
    for (index, result) in search.results.iter().enumerate() {
        // Writing into the buffer directly; formatting into a String cannot fail.
        let _ = write!(
            text,
            "\n[{}] {}\nURL: {}\n{}\n",
            index + 1,
            result.title,
            result.url,
            result.content
        );
    }
    text
}

/// A suggested game as the model returns it.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct RawSuggestion {
    /// Title.
    #[serde(default)]
    pub title: String,
    /// Reasoning.
    #[serde(default)]
    pub why: String,
}

/// The model's JSON envelope.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct RawSuggestions {
    /// Suggested games.
    #[serde(default)]
    pub games: Vec<RawSuggestion>,
}

/// Parse the model's reply.
///
/// Models wrap JSON in prose or fences despite instructions, so the outermost
/// `{...}` is taken rather than assuming the whole body is JSON.
///
/// # Errors
/// When no JSON object can be found or it does not match the shape.
pub fn parse_suggestions(raw: &str) -> Result<RawSuggestions, String> {
    let start = raw
        .find('{')
        .ok_or_else(|| "no JSON object in reply".to_owned())?;
    let end = raw
        .rfind('}')
        .ok_or_else(|| "unterminated JSON in reply".to_owned())?;
    if end < start {
        return Err("malformed JSON in reply".to_owned());
    }
    serde_json::from_str(&raw[start..=end]).map_err(|err| format!("unreadable reply: {err}"))
}

/// Remove markdown emphasis that a model adds to plain prose.
///
/// The UI renders text, not markdown, so `**CLANNAD**` would show its asterisks.
/// Only the inline emphasis markers are removed; anything else is left alone.
#[must_use]
pub fn strip_emphasis(text: &str) -> String {
    text.replace("**", "").replace("__", "")
}

/// Turn parsed suggestions into recommendations, dropping unsupported ones.
///
/// Dropping rather than flagging is deliberate: a name the sources do not
/// contain is unusable, so showing it would be worse than showing nothing.
#[must_use]
pub fn assemble(
    suggestions: &[RawSuggestion],
    sources: &[SearchResult],
    library: &[String],
) -> Vec<Recommendation> {
    assemble_excluding(suggestions, sources, library, &[])
}

/// As [`assemble`], also dropping titles named in `exclude`.
///
/// The seed game is excluded because recommending the game someone just asked
/// about is the most obvious way to look broken.
#[must_use]
pub fn assemble_excluding(
    suggestions: &[RawSuggestion],
    sources: &[SearchResult],
    library: &[String],
    exclude: &[String],
) -> Vec<Recommendation> {
    let skip: Vec<String> = exclude
        .iter()
        .map(|value| value.trim().to_lowercase())
        .collect();
    let titles: Vec<String> = suggestions
        .iter()
        .map(|item| item.title.trim().to_owned())
        .filter(|title| !title.is_empty())
        .collect();
    let rejected = unsupported_candidates(&titles, sources);
    let owned_flags = mark_owned(&titles, library);

    titles
        .into_iter()
        .zip(suggestions.iter())
        .zip(owned_flags)
        .filter(|((title, _), _)| !rejected.contains(title))
        .filter(|((title, _), _)| !skip.contains(&title.trim().to_lowercase()))
        .map(|((title, suggestion), owned)| {
            // Cite only the pages that actually mention this title.
            let needle = title.to_lowercase();
            let sources: Vec<Citation> = sources
                .iter()
                .filter(|source| {
                    let hay = format!(
                        "{} {}",
                        source.title.to_lowercase(),
                        source.content.to_lowercase()
                    );
                    hay.contains(&needle)
                        || needle
                            .split_whitespace()
                            .filter(|word| word.len() > 3)
                            .any(|word| hay.contains(word))
                })
                .take(3)
                .map(|source| Citation {
                    title: source.title.clone(),
                    url: source.url.clone(),
                })
                .collect();
            Recommendation {
                title,
                why: strip_emphasis(suggestion.why.trim()),
                sources,
                owned,
            }
        })
        .collect()
}

/// Note shown beside the results.
#[must_use]
pub fn note_for(games: usize, sources: usize, owned: usize) -> Option<String> {
    if games == 0 {
        return Some(if sources == 0 {
            "The search returned nothing, so there is nothing to recommend.".to_owned()
        } else {
            "The search found pages but no titles it could verify. Try naming a game you liked."
                .to_owned()
        });
    }
    let ownable = games.saturating_sub(owned);
    if ownable == 0 {
        return Some("Every title suggested is already in your library.".to_owned());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(title: &str, url: &str, content: &str) -> SearchResult {
        SearchResult {
            title: title.to_owned(),
            url: url.to_owned(),
            content: content.to_owned(),
            relevance: 0.8,
        }
    }

    fn outcome(results: Vec<SearchResult>) -> SearchOutcome {
        SearchOutcome {
            query: "q".to_owned(),
            answer: None,
            results,
            credits: 1,
        }
    }

    #[test]
    fn the_query_names_the_seed_game_and_the_taste() {
        let q = build_query("CLANNAD", &["Nakige".to_owned(), "Family".to_owned()]);
        assert!(q.contains("similar to CLANNAD"), "{q}");
        assert!(q.contains("Nakige"), "{q}");
        assert!(q.ends_with("recommendations"), "{q}");
    }

    #[test]
    fn the_taste_themes_reach_the_query() {
        // The command used to call build_query with an empty taste slice, so the
        // search was the seed title alone and found the game's own store page
        // instead of what to read next. This is the wiring that was missing.
        let themes = ["Nakige", "Family", "School"]
            .iter()
            .map(|name| vn_core::taste::Theme {
                name: (*name).to_owned(),
                count: 3,
                examples: Vec::new(),
                share: 0.5,
            })
            .collect::<Vec<_>>();
        let names: Vec<String> = themes.into_iter().map(|theme| theme.name).collect();
        let q = build_query("CLANNAD", &names);
        assert!(q.contains("similar to CLANNAD"), "{q}");
        assert!(q.contains("Nakige, Family, School"), "{q}");
    }

    #[test]
    fn an_empty_seed_still_produces_a_usable_query() {
        // The screen opens with no seed chosen; the query must still be real.
        let q = build_query("   ", &[]);
        assert_eq!(
            q,
            "visual novel recommendations recommendations"
                .replace(" recommendations recommendations", " recommendations")
        );
        assert!(!q.is_empty(), "a seedless query is still a query");
    }

    #[test]
    fn only_the_first_three_taste_themes_steer_the_search() {
        // Beyond three, the title gets crowded out of the query.
        let taste: Vec<String> = (0..6).map(|i| format!("Theme{i}")).collect();
        let q = build_query("X", &taste);
        assert!(q.contains("Theme2"));
        assert!(!q.contains("Theme3"), "{q}");
    }

    #[test]
    fn a_title_present_in_a_source_is_supported() {
        let sources = vec![source(
            "Recommendations",
            "https://example.com/a",
            "If you liked CLANNAD, try Little Busters!",
        )];
        assert_eq!(
            unsupported_candidates(&["Little Busters".to_owned()], &sources).len(),
            0,
            "unsupported_candidates(&[\"Little Busters\".to_owned()], &sources) should be empty"
        );
    }

    #[test]
    fn a_title_absent_from_every_source_is_rejected() {
        // This is the check that stops an invented recommendation.
        let sources = vec![source("Recs", "https://example.com", "Try Kanon")];
        let bad = unsupported_candidates(&["Fate/stay night".to_owned()], &sources);
        assert_eq!(bad, vec!["Fate/stay night".to_owned()]);
    }

    #[test]
    fn punctuation_differences_do_not_reject_a_real_title() {
        // "Little Busters!" in a source must support "Little Busters".
        let sources = vec![source("x", "https://e.com", "Little Busters! is great")];
        assert_eq!(
            unsupported_candidates(&["Little Busters".to_owned()], &sources).len(),
            0,
            "unsupported_candidates(&[\"Little Busters\".to_owned()], &sources) should be empty"
        );
    }

    #[test]
    fn a_matching_word_in_the_url_counts_as_support() {
        let sources = vec![source("Tag", "https://vndb.org/g596", "")];
        assert_eq!(
            unsupported_candidates(&["g596".to_owned()], &sources).len(),
            0,
            "unsupported_candidates(&[\"g596\".to_owned()], &sources) should be empty"
        );
    }

    #[test]
    fn support_is_judged_across_all_sources_together() {
        // A title may be split across two pages; it is still supported.
        let sources = vec![
            source("a", "https://e.com/1", "Little"),
            source("b", "https://e.com/2", "Busters"),
        ];
        assert_eq!(
            unsupported_candidates(&["Little Busters".to_owned()], &sources).len(),
            0,
            "unsupported_candidates(&[\"Little Busters\".to_owned()], &sources) should be empty"
        );
    }

    #[test]
    fn a_short_title_needs_an_exact_match() {
        let sources = vec![source("x", "https://e.com", "Kanon is a classic")];
        assert_eq!(
            unsupported_candidates(&["Kanon".to_owned()], &sources).len(),
            0,
            "unsupported_candidates(&[\"Kanon\".to_owned()], &sources) should be empty"
        );

        let unrelated = vec![source("y", "https://e.com", "Rewrite is a classic")];
        assert_eq!(
            unsupported_candidates(&["Kanon".to_owned()], &unrelated),
            vec!["Kanon".to_owned()],
            "a title absent from every source must be rejected"
        );
    }

    #[test]
    fn a_title_too_short_to_have_distinctive_words_is_still_checked() {
        // "Ai" has no word longer than three letters. If the fallback approved
        // such titles, any search would support any short name, and the check
        // would stop doing its job exactly where models invent most.
        let sources = vec![source("x", "https://e.com", "Rewrite is a classic")];
        assert_eq!(
            unsupported_candidates(&["Ai".to_owned()], &sources),
            vec!["Ai".to_owned()]
        );
        // But a source that does name it still supports it.
        let named = vec![source("y", "https://e.com", "Ai is a classic")];
        assert_eq!(
            unsupported_candidates(&["Ai".to_owned()], &named).len(),
            0,
            "unsupported_candidates(&[\"Ai\".to_owned()], &named) should be empty"
        );
    }

    #[test]
    fn the_exact_match_shortcut_beats_a_partial_word_overlap() {
        // "Summer Pockets" against a page mentioning only "Summer" and
        // "Pockets" separately is supported by the fallback; the shortcut is
        // what makes the full phrase count as stronger evidence.
        let together = vec![source("x", "https://e.com", "Summer Pockets is great")];
        let apart = vec![source(
            "y",
            "https://e.com",
            "Summer is warm, pockets hold keys",
        )];
        for sources in [together, apart] {
            assert!(
                unsupported_candidates(&["Summer Pockets".to_owned()], &sources).is_empty(),
                "both phrasings must support the title"
            );
        }
    }

    #[test]
    fn an_empty_title_is_not_reported_as_unsupported() {
        // Nothing to check; it is dropped elsewhere.
        let sources = vec![source("a", "https://e.com", "text")];
        assert_eq!(
            unsupported_candidates(&["   ".to_owned()], &sources).len(),
            0,
            "unsupported_candidates(&[\"   \".to_owned()], &sources) should be empty"
        );
    }

    #[test]
    fn owned_matching_ignores_case_and_spacing() {
        let flags = mark_owned(
            &["clannad".to_owned(), "New Game".to_owned()],
            &["CLANNAD ".to_owned()],
        );
        assert_eq!(flags, vec![true, false]);
    }

    #[test]
    fn assembling_drops_unsupported_titles_and_keeps_citations() {
        let sources = vec![source(
            "Recs",
            "https://e.com/recs",
            "Little Busters and Kanon are similar to CLANNAD",
        )];
        let suggestions = vec![
            RawSuggestion {
                title: "Little Busters".to_owned(),
                why: "same emotional arc".to_owned(),
            },
            RawSuggestion {
                title: "Fate/stay night".to_owned(),
                why: "invented".to_owned(),
            },
        ];
        let games = assemble(&suggestions, &sources, &[]);
        assert_eq!(games.len(), 1, "{games:?}");
        assert_eq!(games[0].title, "Little Busters");
        assert_eq!(games[0].sources[0].url, "https://e.com/recs");
    }

    #[test]
    fn assembling_marks_a_suggestion_the_user_already_owns() {
        let sources = vec![source("a", "https://e.com", "CLANNAD and Kanon")];
        let suggestions = vec![RawSuggestion {
            title: "CLANNAD".to_owned(),
            why: "already yours".to_owned(),
        }];
        let games = assemble(&suggestions, &sources, &["CLANNAD".to_owned()]);
        assert!(games[0].owned);
    }

    #[test]
    fn a_fenced_json_reply_is_parsed() {
        // Models wrap JSON in fences even when told not to.
        let raw = "```json\n{\"games\":[{\"title\":\"Kanon\",\"why\":\"w\"}]}\n```";
        let parsed = parse_suggestions(raw).unwrap();
        assert_eq!(parsed.games[0].title, "Kanon");
    }

    #[test]
    fn a_reply_with_prose_around_the_json_is_parsed() {
        let raw = "Here you go: {\"games\":[]} hope that helps";
        assert_eq!(
            parse_suggestions(raw).unwrap().games.len(),
            0,
            "parse_suggestions(raw).unwrap().games should be empty"
        );
    }

    #[test]
    fn a_reply_without_json_is_an_error_not_a_panic() {
        assert!(parse_suggestions("I cannot help with that").is_err());
    }

    #[test]
    fn the_user_prompt_lists_owned_titles_and_numbered_sources() {
        let search = outcome(vec![source("T", "https://e.com", "body")]);
        let prompt = user_prompt("like CLANNAD", &["Umineko".to_owned()], &search);
        assert!(prompt.contains("like CLANNAD"));
        assert!(prompt.contains("Umineko"));
        assert!(prompt.contains("[1] T"));
        assert!(prompt.contains("https://e.com"));
    }

    #[test]
    fn the_user_prompt_says_so_when_there_are_no_results() {
        let prompt = user_prompt("q", &[], &outcome(Vec::new()));
        assert!(prompt.contains("(no results)"));
        assert!(prompt.contains("(none)"));
    }

    #[test]
    fn no_games_and_no_sources_explains_the_search_was_empty() {
        let note = note_for(0, 0, 0).unwrap();
        assert!(note.contains("nothing to recommend"));
    }

    #[test]
    fn sources_without_verifiable_titles_explains_that() {
        let note = note_for(0, 3, 0).unwrap();
        assert!(note.contains("no titles it could verify"));
    }

    #[test]
    fn the_seed_game_is_never_recommended_back() {
        // Suggesting the game someone just asked about is the clearest sign the
        // feature is not working, and models do it.
        let sources = vec![source("a", "https://e.com", "Summer Pockets and Kanon")];
        let suggestions = vec![
            RawSuggestion {
                title: "Summer Pockets".to_owned(),
                why: "the seed".to_owned(),
            },
            RawSuggestion {
                title: "Kanon".to_owned(),
                why: "similar".to_owned(),
            },
        ];
        let games = assemble_excluding(&suggestions, &sources, &[], &["summer pockets".to_owned()]);
        assert_eq!(games.len(), 1, "{games:?}");
        assert_eq!(games[0].title, "Kanon");
    }

    #[test]
    fn markdown_emphasis_is_removed_from_the_reason() {
        // The UI renders text, so **asterisks** would appear literally.
        assert_eq!(
            strip_emphasis("from the **Key** studio"),
            "from the Key studio"
        );
        assert_eq!(strip_emphasis("plain text"), "plain text");
    }

    #[test]
    fn a_result_where_everything_is_owned_says_so() {
        let note = note_for(2, 3, 2).unwrap();
        assert!(note.contains("already in your library"));
    }

    #[test]
    fn a_result_with_something_new_has_no_note() {
        assert!(note_for(2, 3, 1).is_none());
    }
}
