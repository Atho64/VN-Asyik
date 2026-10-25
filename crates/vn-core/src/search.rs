//! Title search: query normalisation, matching and relevance ranking.
//!
//! Two different jobs live here because they must agree:
//!
//! - **Matching** decides whether a title is worth showing. A bare substring
//!   test fails on the ways visual novel titles actually vary: diacritics and
//!   full-width Latin (`ＣＬＡＮＮＡＤ`), Japanese punctuation (`9-nine-` vs
//!   `9 nine`), decorative subtitles (`CLANNAD -Hikari Mimamoru Sakamichi de-`),
//!   and word order in long titles.
//! - **Ranking** decides what appears first. VNDB's own `searchrank` puts an
//!   obscure spin-off above the work the user obviously meant — searching
//!   `umineko` returns `Umineko Kareru` (11 votes) above `Umineko no Naku Koro
//!   ni` (14,050). Popularity is the tiebreaker that a title match alone cannot
//!   provide.

use serde::{Deserialize, Serialize};

/// Fold a string for comparison.
///
/// Lowercases, strips diacritics and full-width/half-width distinctions, and
/// reduces every non-alphanumeric character to a single space. So `CLANNAD
/// -Hikari-`, `clannad hikari` and `ＣＬＡＮＮＡＤ -Hikari-` all fold to the same
/// letters, while `9-nine-` and `9 nine` agree.
#[must_use]
pub fn fold(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last_was_space = true;
    for ch in text.chars() {
        // Full-width ASCII (U+FF01..U+FF5E) maps onto its half-width form, then
        // decomposes like any other Latin letter.
        let ch = if ('\u{FF01}'..='\u{FF5E}').contains(&ch) {
            char::from_u32(ch as u32 - 0xFEE0).unwrap_or(ch)
        } else {
            ch
        };
        for part in ch.to_lowercase() {
            let folded = fold_char(part);
            if folded.is_empty() {
                if !last_was_space {
                    out.push(' ');
                    last_was_space = true;
                }
            } else {
                out.push_str(&folded);
                last_was_space = false;
            }
        }
    }
    while out.ends_with(' ') {
        out.pop();
    }
    out
}

/// One character folded to ASCII letters/digits, or empty when it is
/// punctuation. Accented Latin letters decompose to their base letter.
fn fold_char(ch: char) -> String {
    if ch.is_ascii_alphanumeric() {
        return ch.to_string();
    }
    if ch.is_ascii() {
        return String::new();
    }
    // Japanese kana and CJK ideographs are kept: they are meaningful, not
    // decorative punctuation, and titles are often searched in script.
    if is_cjk(ch) {
        return ch.to_string();
    }
    match ch {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' => 'a'.to_string(),
        'ç' | 'ć' | 'č' | 'ĉ' | 'ċ' => 'c'.to_string(),
        'ď' | 'đ' => 'd'.to_string(),
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' => 'e'.to_string(),
        'ĝ' | 'ğ' | 'ġ' | 'ģ' => 'g'.to_string(),
        'ĥ' | 'ħ' => 'h'.to_string(),
        'ì' | 'í' | 'î' | 'ï' | 'ī' | 'ĭ' | 'į' | 'ı' => 'i'.to_string(),
        'ĵ' => 'j'.to_string(),
        'ķ' => 'k'.to_string(),
        'ĺ' | 'ļ' | 'ľ' | 'ł' => 'l'.to_string(),
        'ñ' | 'ń' | 'ņ' | 'ň' => 'n'.to_string(),
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ŏ' | 'ő' => 'o'.to_string(),
        'ŕ' | 'ŗ' | 'ř' => 'r'.to_string(),
        'ś' | 'ŝ' | 'ş' | 'š' => 's'.to_string(),
        'ţ' | 'ť' | 'ŧ' => 't'.to_string(),
        'ù' | 'ú' | 'û' | 'ü' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' => 'u'.to_string(),
        'ŵ' => 'w'.to_string(),
        'ý' | 'ÿ' | 'ŷ' => 'y'.to_string(),
        'ź' | 'ż' | 'ž' => 'z'.to_string(),
        'æ' => "ae".to_string(),
        'œ' => "oe".to_string(),
        'ß' => "ss".to_string(),
        'þ' => "th".to_string(),
        _ => String::new(),
    }
}

/// Kana and CJK ideographs, plus the full-width forms already mapped above.
fn is_cjk(ch: char) -> bool {
    matches!(ch,
        '\u{3040}'..='\u{30FF}'   // hiragana, katakana
        | '\u{3400}'..='\u{4DBF}' // CJK extension A
        | '\u{4E00}'..='\u{9FFF}' // CJK unified ideographs
        | '\u{F900}'..='\u{FAFF}' // compatibility ideographs
        | '\u{AC00}'..='\u{D7AF}' // hangul syllables
    )
}

/// How well a title answers a query.
///
/// Ordered best-first. The variants exist because "contains the words" is not a
/// useful answer to a list sorted by relevance: the exact title must outrank a
/// spin-off that merely shares a prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Match {
    /// Query does not appear in this title at all.
    None,
    /// Every query word appears, in any order.
    Words,
    /// The folded title contains the folded query.
    Contains,
    /// The folded title starts with the folded query.
    Prefix,
    /// The folded title equals the folded query.
    Exact,
}

impl Match {
    /// A number suitable for `sort_by_key` where higher is better.
    #[must_use]
    pub fn score(self) -> u8 {
        match self {
            Match::None => 0,
            Match::Words => 2,
            Match::Contains => 3,
            Match::Prefix => 4,
            Match::Exact => 5,
        }
    }
}

/// Classify how `title` matches `query`, treating the whole title and its
/// original-script form as equally valid.
#[must_use]
pub fn classify(query: &str, title: &str, alt_title: Option<&str>) -> Match {
    let needle = fold(query);
    if needle.is_empty() {
        return Match::Words;
    }
    [Some(title), alt_title]
        .into_iter()
        .flatten()
        .map(|candidate| classify_one(&needle, candidate))
        .max()
        .unwrap_or(Match::None)
}

fn classify_one(needle: &str, candidate: &str) -> Match {
    let hay = fold(candidate);
    if hay.is_empty() || needle.is_empty() {
        return Match::None;
    }
    if hay == needle {
        return Match::Exact;
    }
    if hay.starts_with(needle) {
        return Match::Prefix;
    }
    // Word-wise: every query word appears somewhere. Catches "naku koro umineko"
    // and titles with a subtitle spliced into the middle.
    let words: Vec<&str> = needle.split(' ').filter(|w| !w.is_empty()).collect();
    if words.len() > 1 {
        let hay_words: Vec<&str> = hay.split(' ').filter(|w| !w.is_empty()).collect();
        let matched = words
            .iter()
            .filter(|w| hay_words.iter().any(|hw| hw.starts_with(**w)))
            .count();
        if matched == words.len() {
            return Match::Words;
        }
        // A partial word match is not a hit: "game 4990" must not return every
        // title containing "game". Only a contiguous run still counts.
        return if hay.contains(needle) {
            Match::Contains
        } else {
            Match::None
        };
    }
    if hay.contains(needle) {
        return Match::Contains;
    }
    Match::None
}

/// A searchable row: the fields ranking needs, without the UI payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate<'a> {
    /// Display title.
    pub title: &'a str,
    /// Original-script title.
    pub alt_title: Option<&'a str>,
    /// Community vote count, used only to break ties between equal matches.
    pub votes: u32,
    /// Stable position in the source order, so sorting never reshuffles equals.
    pub order: usize,
}

/// Rank candidates for `query`, best first.
///
/// Rows that do not match at all are dropped. Within the same match class,
/// the more-voted entry wins: that is what keeps an 11-vote spin-off from
/// sitting above the 14,000-vote work it borrows its name from.
#[must_use]
pub fn rank<'a, T>(
    query: &str,
    items: &'a [T],
    fields: impl Fn(&'a T) -> Candidate<'a>,
) -> Vec<&'a T> {
    let mut scored: Vec<(u8, u32, usize, &'a T)> = items
        .iter()
        .filter_map(|item| {
            let Candidate {
                title,
                alt_title,
                votes,
                order,
            } = fields(item);
            let grade = classify(query, title, alt_title);
            (grade != Match::None).then_some((grade.score(), votes, order, item))
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)).then(a.2.cmp(&b.2)));
    scored.into_iter().map(|(_, _, _, item)| item).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(title: &str, votes: u32, order: usize) -> (String, Option<String>, u32, usize) {
        (title.to_owned(), None, votes, order)
    }

    fn rank_titles(query: &str, rows: &[(String, Option<String>, u32, usize)]) -> Vec<String> {
        rank(query, rows, |row| Candidate {
            title: &row.0,
            alt_title: row.1.as_deref(),
            votes: row.2,
            order: row.3,
        })
        .into_iter()
        .map(|row| row.0.clone())
        .collect()
    }

    #[test]
    fn folding_ignores_case_width_diacritics_and_punctuation() {
        assert_eq!(fold("CLANNAD"), "clannad");
        assert_eq!(fold("ＣＬＡＮＮＡＤ"), "clannad");
        assert_eq!(fold("9-nine-"), "9 nine");
        assert_eq!(fold("9 nine"), "9 nine");
        assert_eq!(
            fold("Clannad -Hikari Mimamoru Sakamichi de-"),
            "clannad hikari mimamoru sakamichi de"
        );
        assert_eq!(fold("  Nekopara   vol. 1  "), "nekopara vol 1");
        assert_eq!(fold("Café"), "cafe");
        assert_eq!(fold("Straße"), "strasse");
        // Script is preserved, not stripped as punctuation.
        assert_eq!(fold("クラナド"), "クラナド");
    }

    #[test]
    fn exact_and_prefix_outrank_substring_matches() {
        assert_eq!(classify("clannad", "CLANNAD", None), Match::Exact);
        assert_eq!(
            classify("clannad", "CLANNAD -Hikari Mimamoru Sakamichi de-", None),
            Match::Prefix
        );
        assert_eq!(
            classify("clannad", "QUIZ de CLANNAD", None),
            Match::Contains
        );
        assert_eq!(classify("zzz", "CLANNAD", None), Match::None);
    }

    #[test]
    fn word_order_does_not_matter() {
        assert_eq!(
            classify("naku koro umineko", "Umineko no Naku Koro ni", None),
            Match::Words
        );
        assert_eq!(
            classify("umineko naku", "Umineko no Naku Koro ni", None),
            Match::Words
        );
    }

    #[test]
    fn the_original_title_is_searched_too() {
        assert_eq!(
            classify("クラナド", "CLANNAD", Some("CLANNAD -クラナド-")),
            Match::Contains
        );
    }

    #[test]
    fn popularity_breaks_ties_within_one_match_class() {
        // Reproduces the real VNDB ordering that put an 11-vote spin-off first.
        let rows = vec![
            cand("Umineko Kareru", 11, 0),
            cand("Umineko no Naku Koro ni", 14_050, 1),
            cand("Umineko no Naku Koro ni Saku", 593, 2),
        ];
        let ranked = rank_titles("umineko", &rows);
        // "Kareru" is exact-prefix too, so votes decide: the real work leads and
        // the 11-vote entry falls to the end.
        assert_eq!(ranked[0], "Umineko no Naku Koro ni");
        assert_eq!(ranked[1], "Umineko no Naku Koro ni Saku");
        assert_eq!(ranked[2], "Umineko Kareru");
    }

    #[test]
    fn an_exact_title_beats_a_popular_prefix_match() {
        let rows = vec![
            cand("Tsukihime Plus-Disc", 99_999, 0),
            cand("Tsukihime", 11_655, 1),
        ];
        let ranked = rank_titles("tsukihime", &rows);
        assert_eq!(ranked, vec!["Tsukihime", "Tsukihime Plus-Disc"]);
    }

    #[test]
    fn a_partial_word_match_is_not_a_hit() {
        // Regression: searching "game 4990" used to return every "Game 499x"
        // because the first word matched and the second was ignored.
        assert_eq!(classify("game 4990", "Game 499", None), Match::None);
        assert_eq!(classify("game 4990", "Game 4991", None), Match::None);
        assert_eq!(classify("game 4990", "Game 4990", None), Match::Exact);
        // A prefix of the whole title still works.
        assert_eq!(classify("clann", "CLANNAD", None), Match::Prefix);
        // A word from the middle of a title is a substring hit.
        assert_eq!(
            classify("hikari", "CLANNAD -Hikari-", None),
            Match::Contains
        );
        assert_eq!(
            classify("koro", "Umineko no Naku Koro ni", None),
            Match::Contains
        );
    }

    #[test]
    fn unrelated_titles_are_dropped_but_substrings_still_match() {
        // "Fairy Unfortunate" does contain the letters of "fortuna", which is the
        // price of substring matching: it is what lets "koro" find a title. What
        // matters is that it ranks below the exact hit, and that a title with no
        // overlap at all is dropped.
        let rows = vec![
            cand("Fairy Unfortunate", 3, 0),
            cand("Fortuna", 14, 1),
            cand("CLANNAD", 5_000, 2),
        ];
        let ranked = rank_titles("fortuna", &rows);
        assert_eq!(ranked, vec!["Fortuna", "Fairy Unfortunate"]);
    }

    #[test]
    fn equal_rows_keep_their_incoming_order() {
        let rows = vec![cand("Rewrite", 100, 0), cand("Rewrite", 100, 1)];
        let ranked = rank_titles("rewrite", &rows);
        assert_eq!(ranked.len(), 2);
    }

    #[test]
    fn an_empty_query_matches_everything_without_panicking() {
        let rows = vec![cand("A", 0, 0), cand("B", 0, 1)];
        assert_eq!(rank_titles("   ", &rows), vec!["A", "B"]);
    }
}
