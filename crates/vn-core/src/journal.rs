//! Readable journal documents: one game's reading checkpoints as Markdown or HTML.
//!
//! Deliberately separate from library sync exports (`commands::backup`,
//! `commands::data`), which carry tombstones and device ids for restoration.
//!
//! Dates arrive pre-formatted because the local calendar belongs to `SQLite`'s
//! `'localtime'` modifier (same authority as `playtime_by_day` and the heatmap);
//! `vn-core` has no timezone and must not grow one.
//!
//! ## Promises
//!
//! * **Self-contained.** Photos embed as `data:` URIs, never links, because the
//!   app is offline-first (`AGENTS.md` §2.6).
//! * **Content setting holds.** Blurred photos embed their pre-blurred copy;
//!   hidden photos are omitted. The caller resolves that policy so the threshold
//!   cannot drift here (`AGENTS.md` §6).
//! * **User text is literal.** Notes are escaped so `*` or `<script>` render verbatim.

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

/// Which document to render.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JournalFormat {
    /// Plain Markdown, for editing or pasting anywhere.
    Markdown,
    /// A standalone HTML file with its own styling, for reading and printing.
    Html,
}

impl JournalFormat {
    /// Parse the value that arrives from the frontend.
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "markdown" | "md" => Ok(Self::Markdown),
            "html" => Ok(Self::Html),
            other => Err(format!("unknown journal format: {other}")),
        }
    }

    /// File extension without the dot.
    #[must_use]
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Markdown => "md",
            Self::Html => "html",
        }
    }

    /// MIME type, so the platform document picker labels the file correctly.
    #[must_use]
    pub const fn mime(self) -> &'static str {
        match self {
            Self::Markdown => "text/markdown",
            Self::Html => "text/html",
        }
    }

    /// Human name for the picker's filter.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Markdown => "Markdown",
            Self::Html => "HTML",
        }
    }
}

/// Why a checkpoint's photo is not in the document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhotoGap {
    /// The content setting hides this image, so nothing is written.
    Concealed,
    /// The image was never cached (or the cache was cleared).
    NotCached,
}

/// A pinned screenshot as the document can show it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JournalPhoto {
    /// Bytes to embed. `concealed` marks the pre-blurred copy, so the caption can
    /// say so instead of presenting a soft image as the real thing.
    Embedded {
        /// `data:image/jpeg;base64,…`
        data_url: String,
        /// True when these bytes are the blurred copy.
        concealed: bool,
    },
    /// Nothing to embed, with the reason stated in the document.
    Missing(PhotoGap),
}

/// One checkpoint, with its dates already in the reader's local calendar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalEntryView {
    /// Local calendar date, `YYYY-MM-DD`.
    pub date: String,
    /// Local clock time, `HH:MM`.
    pub time: String,
    /// Local date of the reading session this checkpoint was written in, when it
    /// has one. A checkpoint can be manual, so this is optional.
    pub played_date: Option<String>,
    pub note: String,
    pub save_slot: String,
    pub route_name: String,
    pub chapter_name: String,
    pub photo: Option<JournalPhoto>,
}

/// The whole document: one game and its checkpoints in reading order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalDocument {
    /// Display title at export time.
    pub title: String,
    /// Original-language title, when the game has one.
    pub title_original: Option<String>,
    /// VNDB id, when the game is linked.
    pub vndb_id: Option<String>,
    /// Local date the document was written, `YYYY-MM-DD`.
    pub exported_on: String,
    /// Checkpoints, oldest first.
    pub entries: Vec<JournalEntryView>,
}

impl JournalDocument {
    #[must_use]
    pub fn render(&self, format: JournalFormat) -> String {
        match format {
            JournalFormat::Markdown => self.to_markdown(),
            JournalFormat::Html => self.to_html(),
        }
    }

    /// Suggested file name without a directory, sanitized identically to backup
    /// exports (preserving Unicode letters while stripping path separators and `:`).
    #[must_use]
    pub fn file_stem(&self) -> String {
        let mut stem = String::with_capacity(self.title.len());
        for ch in self.title.chars() {
            stem.push(if ch.is_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else if ch.is_whitespace() {
                '-'
            } else {
                // Dropped rather than dashed so punctuation-only titles don't become "---".
                continue;
            });
        }
        let trimmed = stem.trim_matches('-').to_owned();
        let collapsed = trimmed
            .split('-')
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join("-");
        if collapsed.is_empty() {
            "journal".to_owned()
        } else {
            collapsed
        }
    }

    #[must_use]
    pub fn file_name(&self, format: JournalFormat) -> String {
        format!("{}-journal.{}", self.file_stem(), format.extension())
    }

    fn to_markdown(&self) -> String {
        let mut out = String::new();
        write_markdown_head(&mut out, self);
        if self.entries.is_empty() {
            out.push_str("\nNo checkpoints have been written for this game yet.\n");
            return out;
        }
        let mut current_date: Option<&str> = None;
        for entry in &self.entries {
            current_date = write_markdown_entry(&mut out, entry, current_date);
        }
        out
    }

    /// Standalone dark-surface HTML with inline screen and print styles.
    fn to_html(&self) -> String {
        let mut out = String::with_capacity(4096 + self.entries.len() * 512);
        write_html_head(&mut out, self);
        if self.entries.is_empty() {
            out.push_str(
                "<p class=\"journal__empty\">No checkpoints have been written for this game yet.</p>\n",
            );
        } else {
            let mut current_date: Option<&str> = None;
            for entry in &self.entries {
                current_date = write_html_entry(&mut out, entry, current_date);
            }
        }
        out.push_str("</main>\n</body>\n</html>\n");
        out
    }
}

/// `checkpoint` or `checkpoints`, for the header line both formats share.
fn checkpoint_count(document: &JournalDocument) -> &'static str {
    if document.entries.len() == 1 {
        "checkpoint"
    } else {
        "checkpoints"
    }
}

/// Title block, common to both formats.
fn write_markdown_head(out: &mut String, document: &JournalDocument) {
    out.push_str("# ");
    out.push_str(&escape_markdown_inline(&document.title));
    out.push_str("\n\n");
    if let Some(original) = document
        .title_original
        .as_deref()
        .filter(|text| !text.trim().is_empty())
    {
        out.push('*');
        out.push_str(&escape_markdown_inline(original));
        out.push_str("*\n\n");
    }
    if let Some(vndb) = document.vndb_id.as_deref() {
        out.push_str("[VNDB](https://vndb.org/");
        out.push_str(&escape_markdown_url(vndb));
        out.push_str(")\n\n");
    }
    let _ = writeln!(
        out,
        "Exported from VN Asyik on {}. {} {}.",
        document.exported_on,
        document.entries.len(),
        checkpoint_count(document)
    );
}

/// One checkpoint as Markdown, returning the day heading it last wrote.
fn write_markdown_entry<'a>(
    out: &mut String,
    entry: &'a JournalEntryView,
    current_date: Option<&'a str>,
) -> Option<&'a str> {
    let mut current_date = current_date;
    if current_date != Some(entry.date.as_str()) {
        out.push_str("\n## ");
        out.push_str(&escape_markdown_inline(&entry.date));
        out.push('\n');
        current_date = Some(entry.date.as_str());
    }
    out.push_str("\n### ");
    out.push_str(&escape_markdown_inline(&entry.time));
    let position = position_line(entry);
    if !position.is_empty() {
        out.push_str(" · ");
        out.push_str(&escape_markdown_inline(&position));
    }
    out.push_str("\n\n");

    if let Some(photo) = &entry.photo {
        write_markdown_photo(out, photo);
    }
    for line in entry.note.lines() {
        out.push_str(&escape_markdown_line(line));
        out.push('\n');
    }
    if !entry.note.is_empty() {
        out.push('\n');
    }
    let facts = facts_line(entry);
    if !facts.is_empty() {
        out.push('_');
        out.push_str(&escape_markdown_inline(&facts));
        out.push_str("_\n\n");
    }
    current_date
}

/// The photo block, or the reason there is none.
fn write_markdown_photo(out: &mut String, photo: &JournalPhoto) {
    match photo {
        JournalPhoto::Embedded {
            data_url,
            concealed,
        } => {
            out.push_str(if *concealed {
                "![Pinned screenshot, blurred](<"
            } else {
                "![Pinned screenshot](<"
            });
            // Angle brackets around the destination keep a `)` inside the base64
            // from ending the link early.
            out.push_str(data_url);
            out.push_str(">)\n\n");
        }
        JournalPhoto::Missing(PhotoGap::Concealed) => {
            out.push_str("*Photo concealed by your content setting.*\n\n");
        }
        JournalPhoto::Missing(PhotoGap::NotCached) => {
            out.push_str("*Photo is not in the local cache.*\n\n");
        }
    }
}

/// Document head, including the inline stylesheet.
fn write_html_head(out: &mut String, document: &JournalDocument) {
    out.push_str(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<title>",
    );
    out.push_str(&escape_html(&document.title));
    out.push_str(" - journal</title>\n<style>\n");
    out.push_str(HTML_STYLE);
    out.push_str("</style>\n</head>\n<body>\n<main class=\"journal\">\n");
    out.push_str("<header class=\"journal__head\">\n<h1>");
    out.push_str(&escape_html(&document.title));
    out.push_str("</h1>\n");
    if let Some(original) = document
        .title_original
        .as_deref()
        .filter(|text| !text.trim().is_empty())
    {
        out.push_str("<p class=\"journal__original\">");
        out.push_str(&escape_html(original));
        out.push_str("</p>\n");
    }
    if let Some(vndb) = document.vndb_id.as_deref() {
        out.push_str("<p class=\"journal__vndb\"><a href=\"https://vndb.org/");
        out.push_str(&escape_html_attr(vndb));
        out.push_str("\">");
        out.push_str(&escape_html(vndb));
        out.push_str("</a></p>\n");
    }
    let _ = writeln!(
        out,
        "<p class=\"journal__meta\">Exported from VN Asyik on {}. {} {}.</p>\n</header>",
        escape_html(&document.exported_on),
        document.entries.len(),
        checkpoint_count(document)
    );
}

/// One checkpoint as HTML, returning the day heading it last wrote.
fn write_html_entry<'a>(
    out: &mut String,
    entry: &'a JournalEntryView,
    current_date: Option<&'a str>,
) -> Option<&'a str> {
    let mut current_date = current_date;
    if current_date != Some(entry.date.as_str()) {
        out.push_str("<h2 class=\"journal__day\">");
        out.push_str(&escape_html(&entry.date));
        out.push_str("</h2>\n");
        current_date = Some(entry.date.as_str());
    }
    out.push_str("<article class=\"entry\">\n<p class=\"entry__stamp\"><time>");
    out.push_str(&escape_html(&entry.time));
    out.push_str("</time>");
    let position = position_line(entry);
    if !position.is_empty() {
        out.push_str("<span class=\"entry__position\">");
        out.push_str(&escape_html(&position));
        out.push_str("</span>");
    }
    out.push_str("</p>\n");

    if let Some(photo) = &entry.photo {
        write_html_photo(out, photo);
    }
    if !entry.note.is_empty() {
        out.push_str("<div class=\"entry__note\">");
        // Line breaks are preserved here because HTML, unlike Markdown, has a way
        // to say so without inventing syntax.
        for (index, line) in entry.note.lines().enumerate() {
            if index > 0 {
                out.push_str("<br>");
            }
            out.push_str(&escape_html(line));
        }
        out.push_str("</div>\n");
    }
    let facts = facts_line(entry);
    if !facts.is_empty() {
        out.push_str("<p class=\"entry__facts\">");
        out.push_str(&escape_html(&facts));
        out.push_str("</p>\n");
    }
    out.push_str("</article>\n");
    current_date
}

/// The photo figure, or the reason there is none.
fn write_html_photo(out: &mut String, photo: &JournalPhoto) {
    match photo {
        JournalPhoto::Embedded {
            data_url,
            concealed,
        } => {
            out.push_str("<figure class=\"entry__photo\"><img src=\"");
            out.push_str(&escape_html_attr(data_url));
            out.push_str("\" alt=\"");
            out.push_str(if *concealed {
                "Pinned screenshot, blurred"
            } else {
                "Pinned screenshot"
            });
            out.push_str("\" loading=\"lazy\" decoding=\"async\">");
            if *concealed {
                out.push_str("<figcaption>Blurred by your content setting.</figcaption>");
            }
            out.push_str("</figure>\n");
        }
        JournalPhoto::Missing(PhotoGap::Concealed) => {
            out.push_str("<p class=\"entry__gap\">Photo concealed by your content setting.</p>\n");
        }
        JournalPhoto::Missing(PhotoGap::NotCached) => {
            out.push_str("<p class=\"entry__gap\">Photo is not in the local cache.</p>\n");
        }
    }
}

/// `save slot 12 · session 2026-11-07`, or empty when the checkpoint has neither.
///
/// Unescaped: each renderer escapes the whole line for its own syntax.
fn facts_line(entry: &JournalEntryView) -> String {
    let mut facts: Vec<String> = Vec::new();
    if !entry.save_slot.is_empty() {
        facts.push(format!("save slot {}", entry.save_slot));
    }
    if let Some(date) = entry.played_date.as_deref() {
        facts.push(format!("session {date}"));
    }
    facts.join(" · ")
}

/// `Route · Chapter 3`, or whichever half the checkpoint has.
fn position_line(entry: &JournalEntryView) -> String {
    let mut parts: Vec<&str> = Vec::new();
    if !entry.route_name.is_empty() {
        parts.push(&entry.route_name);
    }
    if !entry.chapter_name.is_empty() {
        parts.push(&entry.chapter_name);
    }
    parts.join(" · ")
}

/// Escapes `CommonMark` inline punctuation and raw HTML (`<`, `>`) so notes render verbatim.
fn escape_markdown_inline(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        if matches!(
            ch,
            '\\' | '`' | '*' | '_' | '[' | ']' | '<' | '>' | '&' | '|' | '~'
        ) {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// Escapes inline syntax plus block-start markers (`#`, `-`, `>`, `1.`) so prose stays prose.
fn escape_markdown_line(line: &str) -> String {
    let escaped = escape_markdown_inline(line);
    let trimmed = line.trim_start();
    let starts_block = trimmed.starts_with('#')
        || trimmed.starts_with('-')
        || trimmed.starts_with('+')
        || trimmed.starts_with('>')
        || trimmed.starts_with('=')
        || ordered_list_marker(trimmed);
    if starts_block {
        format!("\\{escaped}")
    } else {
        escaped
    }
}

/// Whether a line begins with `12.` or `12)`, which Markdown reads as a list.
fn ordered_list_marker(line: &str) -> bool {
    let digits: String = line.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return false;
    }
    matches!(line[digits.len()..].chars().next(), Some('.' | ')'))
}

/// Strips whitespace and control characters from a Markdown link destination.
fn escape_markdown_url(url: &str) -> String {
    url.chars()
        .filter(|ch| !ch.is_whitespace() && !ch.is_control())
        .collect()
}

/// Escape text for an HTML text node.
fn escape_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
    out
}

/// Same escaping as [`escape_html`], kept separate for attribute boundaries (`data:` URIs).
fn escape_html_attr(text: &str) -> String {
    escape_html(text)
}

/// The document's own styling: dark, readable, printable.
const HTML_STYLE: &str = "\
:root { color-scheme: dark; }
* { box-sizing: border-box; }
body {
  margin: 0;
  padding: 2.5rem 1.25rem 4rem;
  background: #101719;
  color: #e6edf0;
  font: 16px/1.65 \"Segoe UI\", system-ui, -apple-system, sans-serif;
}
.journal { max-width: 44rem; margin: 0 auto; }
.journal__head { border-bottom: 1px solid #243236; padding-bottom: 1.25rem; margin-bottom: 2rem; }
h1 { font-size: 1.9rem; line-height: 1.2; margin: 0 0 .5rem; }
.journal__original { margin: 0 0 .5rem; color: #9fb0b6; font-style: italic; }
.journal__vndb { margin: 0 0 .75rem; font-size: .9rem; }
.journal__vndb a { color: #7aa2f7; }
.journal__meta, .journal__empty { margin: 0; color: #9fb0b6; font-size: .9rem; }
.journal__day { font-size: 1.1rem; margin: 2.5rem 0 .75rem; color: #9fb0b6; font-weight: 600; }
.entry { margin: 0 0 2rem; padding: 0 0 1.5rem; border-bottom: 1px solid #1b2629; }
.entry:last-child { border-bottom: 0; }
.entry__stamp { margin: 0 0 .75rem; font-variant-numeric: tabular-nums; color: #9fb0b6; font-size: .9rem; }
.entry__position { margin-left: .75rem; color: #c8d6db; }
.entry__photo { margin: 0 0 1rem; }
.entry__photo img { display: block; width: 100%; height: auto; border-radius: 10px; }
.entry__photo figcaption { margin-top: .5rem; color: #9fb0b6; font-size: .8rem; }
.entry__gap { margin: 0 0 1rem; color: #9fb0b6; font-style: italic; }
.entry__note { white-space: normal; }
.entry__facts { margin: 1rem 0 0; color: #9fb0b6; font-size: .85rem; }
@media print {
  body { background: #fff; color: #111; padding: 0; }
  .journal__head { border-color: #ccc; }
  .journal__original, .journal__meta, .journal__day, .entry__stamp, .entry__facts { color: #555; }
  .entry { border-color: #ddd; page-break-inside: avoid; }
  .entry__photo img { max-width: 100%; }
}
";

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(date: &str, time: &str, note: &str) -> JournalEntryView {
        JournalEntryView {
            date: date.to_owned(),
            time: time.to_owned(),
            played_date: None,
            note: note.to_owned(),
            save_slot: String::new(),
            route_name: String::new(),
            chapter_name: String::new(),
            photo: None,
        }
    }

    fn document(entries: Vec<JournalEntryView>) -> JournalDocument {
        JournalDocument {
            title: "Symphonic Rain".to_owned(),
            title_original: Some("シンフォニックレイン".to_owned()),
            vndb_id: Some("v17".to_owned()),
            exported_on: "2026-11-08".to_owned(),
            entries,
        }
    }

    #[test]
    fn entries_are_grouped_by_day_in_reading_order() {
        let doc = document(vec![
            entry("2026-11-07", "21:14", "At the station."),
            entry("2026-11-07", "22:40", "Before the choice."),
            entry("2026-11-09", "08:02", "Next morning."),
        ]);
        let md = doc.render(JournalFormat::Markdown);
        let first_day = md.find("## 2026-11-07").expect("first day heading");
        let second_day = md.find("## 2026-11-09").expect("second day heading");
        let station = md.find("At the station.").expect("first note");
        let choice = md.find("Before the choice.").expect("second note");
        assert!(first_day < station && station < choice && choice < second_day);
        // One heading per day, not one per entry.
        assert_eq!(md.matches("## 2026-11-07").count(), 1);
    }

    #[test]
    fn a_note_is_escaped_rather_than_interpreted() {
        let doc = document(vec![entry(
            "2026-11-07",
            "21:14",
            "*not emphasis* [not a link](x) <script>alert(1)</script> R&D",
        )]);
        let md = doc.render(JournalFormat::Markdown);
        assert!(md.contains("\\*not emphasis\\*"), "emphasis escaped: {md}");
        assert!(md.contains("\\[not a link\\]"), "brackets escaped: {md}");
        assert!(md.contains("\\<script\\>"), "raw HTML escaped: {md}");

        let html = doc.render(JournalFormat::Html);
        assert!(!html.contains("<script>"), "no raw script tag: {html}");
        assert!(html.contains("&lt;script&gt;"));
        assert!(html.contains("&amp;"), "ampersands escaped");
    }

    #[test]
    fn a_note_beginning_with_a_block_marker_does_not_become_one() {
        for (note, expected) in [
            ("# 3 is the good route", "\\# 3 is the good route"),
            ("- not a bullet", "\\- not a bullet"),
            ("1. not a list", "\\1. not a list"),
            ("> not a quote", "\\> not a quote"),
        ] {
            let doc = document(vec![entry("2026-11-07", "21:14", note)]);
            let md = doc.render(JournalFormat::Markdown);
            assert!(md.contains(expected), "{note:?} must stay prose: {md}");
        }
    }

    #[test]
    fn an_embedded_photo_is_self_contained_in_both_formats() {
        let mut with_photo = entry("2026-11-07", "21:14", "The pier.");
        with_photo.photo = Some(JournalPhoto::Embedded {
            data_url: "data:image/jpeg;base64,AAAA".to_owned(),
            concealed: false,
        });
        let doc = document(vec![with_photo]);
        let md = doc.render(JournalFormat::Markdown);
        assert!(md.contains("![Pinned screenshot](<data:image/jpeg;base64,AAAA>)"));
        // No remote URL: the document must not need the network to show its photos.
        assert!(!md.contains("t.vndb.org"));
        let html = doc.render(JournalFormat::Html);
        assert!(html.contains("src=\"data:image/jpeg;base64,AAAA\""));
    }

    #[test]
    fn a_blurred_photo_says_so_instead_of_pretending_to_be_sharp() {
        let mut with_photo = entry("2026-11-07", "21:14", "");
        with_photo.photo = Some(JournalPhoto::Embedded {
            data_url: "data:image/jpeg;base64,BBBB".to_owned(),
            concealed: true,
        });
        let doc = document(vec![with_photo]);
        let md = doc.render(JournalFormat::Markdown);
        assert!(md.contains("![Pinned screenshot, blurred]"));
        let html = doc.render(JournalFormat::Html);
        assert!(html.contains("Blurred by your content setting."));
    }

    #[test]
    fn a_hidden_photo_embeds_nothing() {
        let mut with_photo = entry("2026-11-07", "21:14", "The pier.");
        with_photo.photo = Some(JournalPhoto::Missing(PhotoGap::Concealed));
        let doc = document(vec![with_photo]);
        let md = doc.render(JournalFormat::Markdown);
        assert!(md.contains("*Photo concealed by your content setting.*"));
        assert!(!md.contains("data:image"));
        let html = doc.render(JournalFormat::Html);
        assert!(!html.contains("<img"));
        assert!(html.contains("Photo concealed by your content setting."));
    }

    #[test]
    fn a_photo_that_was_never_cached_is_reported_not_invented() {
        let mut with_photo = entry("2026-11-07", "21:14", "");
        with_photo.photo = Some(JournalPhoto::Missing(PhotoGap::NotCached));
        let doc = document(vec![with_photo]);
        assert!(
            doc.render(JournalFormat::Markdown)
                .contains("*Photo is not in the local cache.*")
        );
    }

    #[test]
    fn a_photo_only_checkpoint_still_renders() {
        // The save form accepts a photo with no caption; the document must not
        // produce a heading with nothing under it.
        let mut with_photo = entry("2026-11-07", "21:14", "");
        with_photo.photo = Some(JournalPhoto::Embedded {
            data_url: "data:image/jpeg;base64,AAAA".to_owned(),
            concealed: false,
        });
        let doc = document(vec![with_photo]);
        let html = doc.render(JournalFormat::Html);
        assert!(html.contains("<article class=\"entry\">"));
        assert!(html.contains("<img"));
    }

    #[test]
    fn an_empty_journal_says_so_instead_of_rendering_a_bare_header() {
        let doc = document(Vec::new());
        let md = doc.render(JournalFormat::Markdown);
        assert!(md.contains("No checkpoints have been written"));
        let html = doc.render(JournalFormat::Html);
        assert!(html.contains("No checkpoints have been written"));
        assert!(html.ends_with("</html>\n"));
    }

    #[test]
    fn the_file_name_is_safe_on_every_platform() {
        let mut doc = document(Vec::new());
        doc.title = "Symphonic Rain: A/B".to_owned();
        assert_eq!(doc.file_stem(), "Symphonic-Rain-AB");
        assert_eq!(
            doc.file_name(JournalFormat::Markdown),
            "Symphonic-Rain-AB-journal.md"
        );
        doc.title = "うみねこのなく頃に".to_owned();
        assert_eq!(doc.file_stem(), "うみねこのなく頃に");
        doc.title = "///".to_owned();
        assert_eq!(doc.file_stem(), "journal");
    }

    #[test]
    fn the_position_line_uses_whichever_half_exists() {
        let mut with_route = entry("2026-11-07", "21:14", "x");
        with_route.route_name = "Mio".to_owned();
        with_route.chapter_name = "Chapter 3".to_owned();
        assert_eq!(position_line(&with_route), "Mio · Chapter 3");
        with_route.chapter_name.clear();
        assert_eq!(position_line(&with_route), "Mio");
        with_route.route_name.clear();
        assert_eq!(position_line(&with_route), "");
    }

    #[test]
    fn format_parsing_rejects_a_made_up_value() {
        assert_eq!(
            JournalFormat::parse("markdown"),
            Ok(JournalFormat::Markdown)
        );
        assert_eq!(JournalFormat::parse("html"), Ok(JournalFormat::Html));
        assert!(JournalFormat::parse("pdf").is_err());
        assert_eq!(JournalFormat::Html.extension(), "html");
        assert_eq!(JournalFormat::Markdown.mime(), "text/markdown");
    }
}
