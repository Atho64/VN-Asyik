//! Walkthrough domain model and Seiya-Saiga HTML parser.
//!
//! Visual novel walkthroughs from Seiya-Saiga (`seiya-saiga.com`) are structured
//! by route (Common, Heroine routes, Bad Ends, After stories) and step items
//! (Choices with checkboxes, Save points, Dates, Actions, and Endings).

use serde::{Deserialize, Serialize};

/// Type of step in a walkthrough route.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemKind {
    /// An in-game choice to make (`<input type="checkbox">`).
    Choice,
    /// A save point marker (e.g. `◆セーブ04から`).
    Save,
    /// A date or chapter heading (e.g. `■１０月２１日(木)`).
    Date,
    /// A navigation or map action (e.g. `◆MAP10a 目的地に進む`).
    Action,
    /// An ending or branch result (e.g. `小鳥END`, `共通ルートBAD回収終了`).
    Ending,
    /// General notes or tips.
    Note,
}

/// One step in a walkthrough route.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WalkthroughItem {
    /// Stable item identifier within the walkthrough (e.g. `c_0`, `s_1`).
    pub id: String,
    /// The kind of item.
    pub kind: ItemKind,
    /// Text of the step in the original or translated language.
    pub text: String,
    /// Optional additional note or condition for this step.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// A route or section in a walkthrough.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WalkthroughRoute {
    /// Route identifier (slug or index-based).
    pub id: String,
    /// Title of the route (e.g. `共通・小鳥、ちはや`, `ルチア、朱音`).
    pub title: String,
    /// Steps in this route.
    pub items: Vec<WalkthroughItem>,
}

/// Parsed walkthrough for a visual novel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Walkthrough {
    /// Walkthrough title as given on the source page.
    pub title: String,
    /// URL where the walkthrough was fetched from.
    pub source_url: String,
    /// General notes, patch requirements, or walkthrough instructions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// Routes in this walkthrough.
    pub routes: Vec<WalkthroughRoute>,
}

/// An entry in the Seiya-Saiga master index (`kouryaku.html`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WalkthroughIndexEntry {
    /// Game title in Japanese as listed in the index.
    pub title: String,
    /// Absolute URL to the walkthrough page.
    pub url: String,
}

/// Errors that may occur while parsing a walkthrough.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WalkthroughParseError {
    #[error("walkthrough content is empty")]
    EmptyContent,
    #[error("no route sections found in walkthrough")]
    NoRoutesFound,
}

const SEIYA_BASE_URL: &str = "https://galge.seiya-saiga.com/game/";

/// Decode raw HTML bytes into a UTF-8 string by detecting the charset from `<meta ... charset=...>`.
///
/// Supports `EUC-JP`, `Shift_JIS`, and `UTF-8`. Falls back to `Shift_JIS` if no charset
/// is declared in the document.
#[must_use]
pub fn decode_html_bytes(bytes: &[u8]) -> String {
    let limit = bytes.len().min(2048);
    let head = &bytes[..limit];

    let encoding = detect_charset(head)
        .and_then(|label| encoding_rs::Encoding::for_label(label.as_bytes()))
        .unwrap_or(encoding_rs::SHIFT_JIS);

    let (cow, _encoding, _had_errors) = encoding.decode(bytes);
    cow.into_owned()
}

fn detect_charset(head: &[u8]) -> Option<String> {
    let s: String = head
        .iter()
        .map(|&b| if b.is_ascii() { (b as char).to_ascii_lowercase() } else { ' ' })
        .collect();
    if let Some(pos) = s.find("charset") {
        let rest = s[pos + 7..].trim_start();
        if let Some(after_eq) = rest.strip_prefix('=') {
            let trimmed = after_eq.trim_start_matches(['"', '\'', ' ']);
            let end = trimmed.find(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == ';' || c == '>');
            let label = if let Some(e) = end {
                &trimmed[..e]
            } else {
                trimmed
            };
            let label = label.trim();
            if !label.is_empty() {
                return Some(label.to_string());
            }
        }
    }
    None
}

/// Backwards-compatible alias for [`decode_html_bytes`].
#[must_use]
pub fn decode_shift_jis(bytes: &[u8]) -> String {
    decode_html_bytes(bytes)
}

/// Parse walkthrough links from any Seiya-Saiga index page (home, galge, kouryaku).
#[must_use]
pub fn parse_index_links(html: &str, base_url: &str) -> Vec<WalkthroughIndexEntry> {
    let re = regex::Regex::new(r#"(?is)<a[^>]*href=["']?([^"'#\s>]+)["']?[^>]*>(?P<title>.*?)</a>"#)
        .expect("valid link regex");

    let mut entries = Vec::new();
    let mut seen_urls = std::collections::HashSet::new();

    for cap in re.captures_iter(html) {
        let href = cap[1].trim();
        let raw_title = clean_html_tags(&cap["title"]);

        if !href.ends_with(".html") {
            continue;
        }

        let lower_href = href.to_ascii_lowercase();
        if lower_href.contains("fanza")
            || lower_href.contains("dmm")
            || lower_href.contains("dlsite")
            || lower_href.contains("amazon")
            || lower_href.contains("twitter")
            || lower_href.contains("x.com")
            || lower_href.contains("information.html")
            || lower_href.contains("save.html")
            || lower_href.contains("log.html")
        {
            continue;
        }

        let clean_title = raw_title
            .replace("：攻略", "")
            .replace(":攻略", "")
            .replace("【全年齢向け】", "")
            .replace("【国際版】", "")
            .replace("【全年齢対象版】", "")
            .replace("【DLsite専売】", "")
            .trim()
            .to_string();

        if clean_title.is_empty()
            || clean_title.starts_with("http")
            || clean_title.len() > 100
            || clean_title == "Tweet"
            || clean_title == "Home"
            || clean_title == "TOP"
            || clean_title == "誠也の部屋"
            || clean_title == "PCゲーム攻略一覧"
        {
            continue;
        }

        let full_url = if href.starts_with("http://") || href.starts_with("https://") {
            href.replacen("http://", "https://", 1)
        } else {
            let base = base_url.trim_end_matches('/');
            let rel = href.trim_start_matches('/');
            format!("{base}/{rel}")
        };

        if seen_urls.insert(full_url.clone()) {
            entries.push(WalkthroughIndexEntry {
                title: clean_title,
                url: full_url,
            });
        }
    }

    entries
}

/// Parse the Seiya-Saiga `kouryaku.html` master index.
#[must_use]
pub fn parse_kouryaku_index(html: &str) -> Vec<WalkthroughIndexEntry> {
    parse_index_links(html, SEIYA_BASE_URL)
}

/// Normalize a title for fuzzy matching across VNDB and Seiya-Saiga.
fn normalize_for_match(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            // Full-width space, half-width space, punctuation, brackets, quotes, wave dashes
            ' ' | '\u{3000}' | '-' | '－' | '・' | '\u{ff65}' | ':' | '：' | '！' | '!' | '？' | '?'
            | '~' | '～' | '〜' | '_' | '/' | '／' | '\\' | '.' | '。' | ',' | '、' | '，'
            | '+' | '＋' | '\'' | '’' | '‘' | '"' | '“' | '”' | '(' | ')' | '（' | '）'
            | '【' | '】' | '『' | '』' | '「' | '」' | '［' | '］' | '[' | ']' => {}
            // Normalize case
            other => {
                for lower in other.to_lowercase() {
                    out.push(lower);
                }
            }
        }
    }
    out
}

/// Score-based search across index entries, matching strictly against title content.
#[must_use]
pub fn search_index(
    entries: &[WalkthroughIndexEntry],
    query: &str,
    limit: usize,
) -> Vec<WalkthroughIndexEntry> {
    let clean_query = query.trim();
    if clean_query.is_empty() || entries.is_empty() {
        return Vec::new();
    }

    let norm_query = normalize_for_match(clean_query);
    let query_lower = clean_query.to_lowercase();
    let query_tokens: Vec<&str> = clean_query.split_whitespace().collect();

    let mut scored: Vec<(u32, &WalkthroughIndexEntry)> = Vec::new();

    for entry in entries {
        let norm_title = normalize_for_match(&entry.title);
        let title_lower = entry.title.to_lowercase();

        let mut score: u32 = 0;

        // Search strictly from TITLE CONTENT:
        if !norm_query.is_empty() && !norm_title.is_empty() {
            if norm_title == norm_query {
                score += 100;
            } else if norm_title.starts_with(&norm_query) || norm_query.starts_with(&norm_title) {
                score += 80;
            } else if norm_title.contains(&norm_query) || norm_query.contains(&norm_title) {
                score += 60;
            }
        }

        if score == 0 {
            if title_lower.contains(&query_lower) {
                score += 50;
            } else if !query_tokens.is_empty()
                && query_tokens.iter().all(|tok| title_lower.contains(&tok.to_lowercase()))
            {
                score += 40;
            } else if query_tokens.iter().any(|tok| tok.len() >= 2 && title_lower.contains(&tok.to_lowercase())) {
                score += 20;
            }
        }

        if score > 0 {
            scored.push((score, entry));
        }
    }

    scored.sort_by(|a, b| b.0.cmp(&a.0));
    scored.into_iter().take(limit).map(|(_, e)| e.clone()).collect()
}

/// Find matching candidate entries for a game based on all its titles and aliases.
#[must_use]
pub fn find_candidate_matches(
    entries: &[WalkthroughIndexEntry],
    candidates: &[String],
    limit: usize,
) -> Vec<WalkthroughIndexEntry> {
    let mut matches = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for cand in candidates {
        let results = search_index(entries, cand, limit);
        for res in results {
            if seen.insert(res.url.clone()) {
                matches.push(res);
                if matches.len() >= limit {
                    return matches;
                }
            }
        }
    }

    matches
}

/// Find the best matching Seiya-Saiga index entry given candidate titles (VNDB title, alt title, aliases).
#[must_use]
pub fn find_best_match<'a>(
    entries: &'a [WalkthroughIndexEntry],
    candidates: &[String],
) -> Option<&'a WalkthroughIndexEntry> {
    if entries.is_empty() || candidates.is_empty() {
        return None;
    }

    let norm_candidates: Vec<String> = candidates
        .iter()
        .map(|c| normalize_for_match(c))
        .filter(|c| !c.is_empty())
        .collect();

    // 1. Exact normalized match
    for cand in &norm_candidates {
        for entry in entries {
            let norm_entry = normalize_for_match(&entry.title);
            if norm_entry == *cand {
                return Some(entry);
            }
        }
    }

    // 2. Prefix or substring match (candidate starts with entry or vice-versa)
    for cand in &norm_candidates {
        if cand.len() < 3 {
            continue;
        }
        for entry in entries {
            let norm_entry = normalize_for_match(&entry.title);
            if norm_entry.len() >= 3 && (norm_entry.contains(cand) || cand.contains(&norm_entry)) {
                return Some(entry);
            }
        }
    }

    None
}

/// Parse a full Seiya-Saiga game walkthrough HTML page into structured [`Walkthrough`].
///
/// # Errors
/// Returns [`WalkthroughParseError`] if the HTML cannot be parsed or contains no routes.
pub fn parse_seiya_walkthrough(
    html: &str,
    source_url: &str,
) -> Result<Walkthrough, WalkthroughParseError> {
    if html.trim().is_empty() {
        return Err(WalkthroughParseError::EmptyContent);
    }

    // 1. Extract Title
    let title = extract_title(html).unwrap_or_else(|| "Visual Novel Walkthrough".to_string());

    // 2. Extract top notes (instructions, disks, save slots, general notices)
    let notes = extract_notes(html);

    // 3. Extract Routes:
    // Seiya-Saiga routes are formatted in tables where a header row has cells
    // (typically bgcolor="#ccffff" or similar) with route titles, followed by
    // a row containing the corresponding steps for each column.
    let tr_re = regex::Regex::new(r"(?is)<tr[^>]*>(?P<tr>.*?)</tr>").expect("valid tr_re");
    let td_re = regex::Regex::new(r#"(?is)<t[dh](?P<attrs>[^>]*)>(?P<content>.*?)</t[dh]>"#).expect("valid td_re");

    let tr_caps: Vec<_> = tr_re.captures_iter(html).collect();
    let mut routes = Vec::new();
    let mut item_counter: usize = 0;
    let mut row_idx = 0;

    while row_idx < tr_caps.len() {
        let tr_str = &tr_caps[row_idx]["tr"];
        let header_tds: Vec<_> = td_re.captures_iter(tr_str).collect();

        let is_header_row = !header_tds.is_empty() && header_tds.iter().any(|td| {
            let attrs = td["attrs"].to_ascii_lowercase();
            attrs.contains("ccffff") || attrs.contains("ccccff") || attrs.contains("99ccff") || attrs.contains("ccffcc")
        });

        if is_header_row && row_idx + 1 < tr_caps.len() {
            let body_tr_str = &tr_caps[row_idx + 1]["tr"];
            let body_tds: Vec<_> = td_re.captures_iter(body_tr_str).collect();

            for (col_idx, h_cap) in header_tds.iter().enumerate() {
                let raw_title = clean_html_tags(&h_cap["content"]);
                let title_clean = raw_title.trim();
                if title_clean.is_empty() {
                    continue;
                }

                if let Some(b_cap) = body_tds.get(col_idx) {
                    let body = &b_cap["content"];
                    let items = parse_route_items(body, &mut item_counter);
                    if !items.is_empty() {
                        let route_id = format!("route_{}", routes.len());
                        routes.push(WalkthroughRoute {
                            id: route_id,
                            title: title_clean.to_string(),
                            items,
                        });
                    }
                }
            }

            row_idx += 2;
            continue;
        }

        row_idx += 1;
    }

    if routes.is_empty() {
        return Err(WalkthroughParseError::NoRoutesFound);
    }

    Ok(Walkthrough {
        title,
        source_url: source_url.to_string(),
        notes,
        routes,
    })
}

/// Strip basic HTML tags and normalize whitespace.
fn clean_html_tags(s: &str) -> String {
    let re = regex::Regex::new(r"<[^>]+>").expect("valid tag re");
    let unescaped = re.replace_all(s, " ");
    unescaped
        .replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Extract page title from either the white title header or `<title>`.
fn extract_title(html: &str) -> Option<String> {
    let header_re = regex::Regex::new(r#"(?is)<FONT[^>]*size=["']?6["']?[^>]*>(?P<title>.*?)</FONT>"#)
        .expect("valid header_re");
    if let Some(cap) = header_re.captures(html) {
        let clean = clean_html_tags(&cap["title"]);
        let trimmed = clean.trim_matches(|c| c == '◆' || c == '★' || c == '■' || c == ' ');
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    let title_re = regex::Regex::new(r#"(?is)<title>(?P<title>.*?)</title>"#).expect("valid title_re");
    if let Some(cap) = title_re.captures(html) {
        let clean = clean_html_tags(&cap["title"]);
        let stripped = clean
            .split([':', '：'])
            .last()
            .unwrap_or(&clean)
            .trim_matches(|c| c == '【' || c == '】' || c == '攻' || c == '略' || c == ' ' || c == '◆');
        if !stripped.is_empty() {
            return Some(stripped.to_string());
        }
    }

    None
}

/// Extract summary/instructions notes at the top of the table.
fn extract_notes(html: &str) -> Option<String> {
    let notes_re = regex::Regex::new(
        r#"(?is)<td\s+align=["']?left["']?\s+colspan=["']?[45]["']?[^>]*>(?P<notes>.*?)</td>"#,
    )
    .expect("valid notes_re");

    let mut lines = Vec::new();
    for cap in notes_re.captures_iter(html) {
        let text = clean_html_tags(&cap["notes"]);
        if !text.is_empty() && !lines.contains(&text) {
            lines.push(text);
        }
    }

    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n\n"))
    }
}

/// Parse the step items inside a route's table cell.
fn parse_route_items(body: &str, counter: &mut usize) -> Vec<WalkthroughItem> {
    // Split on `<br>`, `<hr>`, or newlines
    let line_re = regex::Regex::new(r#"(?i)<br\s*/?>|<hr\s*/?>|\n"#).expect("valid split re");
    let mut items = Vec::new();

    for raw_part in line_re.split(body) {
        let part = raw_part.trim();
        if part.is_empty() {
            continue;
        }

        // 1. Choice: contains <input type="checkbox">
        if part.to_ascii_lowercase().contains("type=\"checkbox\"")
            || part.to_ascii_lowercase().contains("type='checkbox'")
            || part.to_ascii_lowercase().contains("type=checkbox")
        {
            let text = clean_html_tags(part);
            if !text.is_empty() {
                *counter += 1;
                items.push(WalkthroughItem {
                    id: format!("c_{counter}"),
                    kind: ItemKind::Choice,
                    text,
                    note: None,
                });
            }
            continue;
        }

        let clean = clean_html_tags(part);
        if clean.is_empty() {
            continue;
        }

        // 2. Save point marker
        if clean.contains("セーブ") || clean.starts_with("◆セーブ") || clean.starts_with("セーブ") {
            *counter += 1;
            items.push(WalkthroughItem {
                id: format!("s_{counter}"),
                kind: ItemKind::Save,
                text: clean,
                note: None,
            });
            continue;
        }

        // 3. Ending marker
        if clean.contains("END") || clean.contains("BAD") || clean.contains("TrueEnd") || clean.ends_with("エンド") {
            *counter += 1;
            items.push(WalkthroughItem {
                id: format!("e_{counter}"),
                kind: ItemKind::Ending,
                text: clean,
                note: None,
            });
            continue;
        }

        // 4. Date marker (e.g. ■１０月２１日, ■5月12日)
        if clean.starts_with('■') || (clean.contains('月') && clean.contains('日')) {
            *counter += 1;
            items.push(WalkthroughItem {
                id: format!("d_{counter}"),
                kind: ItemKind::Date,
                text: clean,
                note: None,
            });
            continue;
        }

        // 5. Action / Navigation marker (e.g. ◆MAP, 目的地に進む)
        if clean.starts_with('◆') || clean.contains("目的地") || clean.contains("MAP") {
            *counter += 1;
            items.push(WalkthroughItem {
                id: format!("a_{counter}"),
                kind: ItemKind::Action,
                text: clean,
                note: None,
            });
            continue;
        }

        // 6. Generic note / Condition
        *counter += 1;
        items.push(WalkthroughItem {
            id: format!("n_{counter}"),
            kind: ItemKind::Note,
            text: clean,
            note: None,
        });
    }

    items
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_kouryaku_entries_correctly() {
        let html = r#"
        <table border="1">
          <tr>
            <td align="left"><B><A href="key/air_me.html">AIR 全年齢版</A></B></td>
            <td align="left"><B><A href="https://galge.seiya-saiga.com/game/key/kanon_me.html">Kanon メモリアル</A></B></td>
          </tr>
        </table>
        "#;
        let entries = parse_kouryaku_index(html);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].title, "AIR 全年齢版");
        assert_eq!(entries[0].url, "https://galge.seiya-saiga.com/game/key/air_me.html");
        assert_eq!(entries[1].title, "Kanon メモリアル");
        assert_eq!(entries[1].url, "https://galge.seiya-saiga.com/game/key/kanon_me.html");
    }

    #[test]
    fn matches_candidates_cleanly() {
        let entries = vec![
            WalkthroughIndexEntry {
                title: "Rewrite+".to_string(),
                url: "https://galge.seiya-saiga.com/game/key/rewrite_plus.html".to_string(),
            },
            WalkthroughIndexEntry {
                title: "AIR 全年齢版".to_string(),
                url: "https://galge.seiya-saiga.com/game/key/air_me.html".to_string(),
            },
        ];

        let matched = find_best_match(&entries, &["Rewrite+".to_string()]);
        assert_eq!(matched.unwrap().title, "Rewrite+");

        let matched_case = find_best_match(&entries, &["rewrite +".to_string()]);
        assert_eq!(matched_case.unwrap().title, "Rewrite+");

        let matched_alt = find_best_match(&entries, &["AIR".to_string()]);
        assert_eq!(matched_alt.unwrap().title, "AIR 全年齢版");
    }

    #[test]
    fn parses_route_and_items_from_html_snippet() {
        let html = r##"
        <html>
        <head><title>攻略:【Rewrite+】</title></head>
        <body>
        <FONT color="#ffffff" size="6">◆Rewrite+◆</FONT>
        <table border="1">
          <tr>
            <td align="left" colspan="4">1周目は小鳥、ちはや攻略可能。</td>
          </tr>
          <tr>
            <td align="center" width="400" bgcolor="#ccffff"><b>共通・小鳥、ちはや</b></td>
          </tr>
          <tr>
            <td align="left" width="400" valign="top">
              <font color="#ff0000">◆セーブ01</font>から<br>
              <input type="checkbox">男の態度が気に食わないのでもう帰る<br>
              ■１０月２１日(木)<br>
              ◆MAP10a<br>
              目的地に進む<br>
              <input type="checkbox">吉野とコラボる<br>
              <font color="#0000ff">小鳥END</font><br>
            </td>
          </tr>
        </table>
        </body>
        </html>
        "##;

        let res = parse_seiya_walkthrough(html, "https://galge.seiya-saiga.com/game/key/rewrite_plus.html");
        assert!(res.is_ok());
        let wt = res.unwrap();

        assert_eq!(wt.title, "Rewrite+");
        assert_eq!(wt.routes.len(), 1);

        let route = &wt.routes[0];
        assert_eq!(route.title, "共通・小鳥、ちはや");
        assert_eq!(route.items.len(), 7);

        assert_eq!(route.items[0].kind, ItemKind::Save);
        assert!(route.items[0].text.contains("セーブ01"));

        assert_eq!(route.items[1].kind, ItemKind::Choice);
        assert_eq!(route.items[1].text, "男の態度が気に食わないのでもう帰る");

        assert_eq!(route.items[2].kind, ItemKind::Date);
        assert!(route.items[2].text.contains("１０月２１日"));

        assert_eq!(route.items[3].kind, ItemKind::Action);
        assert_eq!(route.items[4].kind, ItemKind::Action);

        assert_eq!(route.items[5].kind, ItemKind::Choice);
        assert_eq!(route.items[5].text, "吉野とコラボる");

        assert_eq!(route.items[6].kind, ItemKind::Ending);
        assert_eq!(route.items[6].text, "小鳥END");
    }

    #[test]
    fn decodes_shift_jis_bytes() {
        let (cow, _, _) = encoding_rs::SHIFT_JIS.encode("小鳥");
        let decoded = decode_shift_jis(&cow);
        assert_eq!(decoded, "小鳥");
    }

    #[test]
    fn decodes_euc_jp_meta_bytes() {
        let (cow, _, _) = encoding_rs::EUC_JP.encode("<html><head><meta http-equiv=\"content-type\" content=\"text/html; charset=EUC-JP\"></head><body>さくら、もゆ。</body></html>");
        let decoded = decode_html_bytes(&cow);
        assert!(decoded.contains("さくら、もゆ。"));
    }

    #[test]
    fn parses_multi_column_routes() {
        let html = r##"
        <html>
        <head><title>誠也の部屋【さくら、もゆ。 攻略】</title></head>
        <body>
        <table>
          <tr>
            <td align="center" width="266" bgcolor="#ccffff"><b>夜月 姫織</b></td>
            <td align="center" width="266" bgcolor="#ccffff"><b>杏藤 千和</b></td>
            <td align="center" width="266" bgcolor="#ccffff"><b>柊 ハル</b></td>
          </tr>
          <tr>
            <td align="left" valign="top">
              ■NEW GAME<br>
              ◆セーブ01<br>
              <font color="#0000ff">姫織END</font>
            </td>
            <td align="left" valign="top">
              ◆セーブ01から<br>
              <input type="checkbox">大通りを散策<br>
              <font color="#0000ff">千和END</font>
            </td>
            <td align="left" valign="top">
              ◆セーブ01から<br>
              <input type="checkbox">小さな石橋<br>
              <font color="#0000ff">ハルEND</font>
            </td>
          </tr>
        </table>
        </body>
        </html>
        "##;

        let res = parse_seiya_walkthrough(html, "https://seiya-saiga.com/game/favorite/sakuramoyu.html");
        assert!(res.is_ok());
        let wt = res.unwrap();
        assert_eq!(wt.routes.len(), 3);
        assert_eq!(wt.routes[0].title, "夜月 姫織");
        assert_eq!(wt.routes[1].title, "杏藤 千和");
        assert_eq!(wt.routes[2].title, "柊 ハル");

        assert_eq!(wt.routes[0].items.len(), 3);
        assert_eq!(wt.routes[1].items.len(), 3);
        assert_eq!(wt.routes[2].items.len(), 3);
    }

    #[test]
    fn searches_strictly_by_title_content_and_normalizes_japanese() {
        let entries = vec![
            WalkthroughIndexEntry {
                title: "サノバウィッチ".to_string(),
                url: "https://seiya-saiga.com/game/yuzu-soft/sothewitch.html".to_string(),
            },
            WalkthroughIndexEntry {
                title: "さくら、もゆ。 -as the Night’s，Reincarnation-".to_string(),
                url: "https://seiya-saiga.com/game/favorite/sakuramoyu.html".to_string(),
            },
            WalkthroughIndexEntry {
                title: "リトルバスターズ！".to_string(),
                url: "https://galge.seiya-saiga.com/game/key/littlebusters.html".to_string(),
            },
        ];

        // 1. Matches Japanese title content even if URL is completely different (sothewitch vs サノバウィッチ)
        let res1 = search_index(&entries, "サノバウィッチ", 5);
        assert_eq!(res1.len(), 1);
        assert_eq!(res1[0].title, "サノバウィッチ");

        // 2. Normalizes fullwidth quotes and wave dashes across VNDB and Seiya-Saiga
        let res2 = search_index(&entries, "さくら、もゆ。-as the Night's, Reincarnation-", 5);
        assert_eq!(res2.len(), 1);
        assert_eq!(res2[0].title, "さくら、もゆ。 -as the Night’s，Reincarnation-");

        // 3. Partial / prefix search on Japanese title
        let res3 = search_index(&entries, "リトルバスターズ", 5);
        assert_eq!(res3.len(), 1);
        assert_eq!(res3[0].title, "リトルバスターズ！");
    }
}
