//! Extracted VN scripts: import formats, normalization and compaction planning.
//!
//! Pure and synchronous so it tests fast. Every import format shares one
//! normalized shape ([`ScriptLine`]) so storage, chunking and the chat tools
//! never care where a script came from:
//!
//! - [`ScriptFormat::PlainText`]: anything the user extracted themselves with
//!   any tool, saved as UTF-8 text. This is the first-class "disassemble it
//!   yourself" path: no tooling is required on our side and nothing about the
//!   source engine is assumed.
//! - [`ScriptFormat::JsonVntp`]: the generic VNTP name/message JSON array
//!   (`[{ "name": "…", "message": "…" }]`, `name` absent for narration), used
//!   verbatim so an extractor's output never needs converting by hand. The
//!   owner's FVP/FAVORITE extractor (`fvp_tool`, `dialogue.json`) is one
//!   producer of this shape; the kind names the shape, not the tool.
//! - [`ScriptFormat::AsyikScript`]: this app's structured interchange format.
//!   Built-in extractors (GARbro-derived Kirikiri/Artemis, FVP) emit it in
//!   later phases; a user can also write it by hand for full fidelity.
//!
//! Siglus support was explicitly cancelled by the project owner (2026-10-24);
//! do not add it back without asking.

use serde::{Deserialize, Serialize};

/// Longest accepted dialogue line, in characters.
pub const MAX_LINE_CHARS: usize = 4_000;
/// Longest accepted speaker label, in characters.
pub const MAX_SPEAKER_CHARS: usize = 120;
/// Most accepted lines per document. Real whole-game scripts reach a few
/// hundred thousand lines; past that the file is almost certainly not a script.
pub const MAX_LINES: usize = 300_000;
/// Default chunk budget for compaction, in UTF-8 bytes. Roughly 12k tokens of
/// Japanese text. Each chunk costs one provider call, and whole-game scripts
/// were spending hundreds of calls at the old 12 KB (3k tokens) budget, so the
/// budget was quadrupled (phase 19B): 12k tokens still leaves a 16k-token
/// context room for the outline. Pinned by a test because stored compaction
/// progress counts chunks against this budget.
pub const DEFAULT_CHUNK_BYTES: usize = 48_000;
/// Most summarizer agents one compaction runs in parallel (owner cap).
pub const MAX_AGENTS: usize = 10;
/// Default summarizer agent count when the user has not chosen one. Modest on
/// purpose: every agent is a concurrent provider call, and small endpoints
/// rate-limit long before ten agents pay off.
pub const DEFAULT_AGENTS: usize = 3;
/// Version of the compaction pipeline (prompts, context strategy, agent
/// layout). Stored with each run's progress: bumping it restarts in-flight
/// compactions exactly like a model change does, so summaries from two
/// pipelines never mix. 1 = the phase 19C multi-agent pipeline with rolling
/// context; 0 marks phase 18/19B single-threaded rows, which restart once.
pub const COMPACTOR_VERSION: u32 = 1;
/// Characters of the previous chunk's outline passed to the next map call as
/// rolling context: enough to carry names and scene threads across a chunk
/// boundary without re-feeding whole outlines.
pub const ROLLING_CONTEXT_CHARS: usize = 1_500;

/// One normalized dialogue line. `speaker` is `None` for narration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptLine {
    pub speaker: Option<String>,
    pub text: String,
}

/// Which importer produced a document. The strings are stored in
/// `script_documents.source_kind` and constrained by a CHECK in migration 0024.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScriptFormat {
    PlainText,
    JsonVntp,
    AsyikScript,
}

impl ScriptFormat {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PlainText => "plain_text",
            Self::JsonVntp => "json_vntp",
            Self::AsyikScript => "asyik_script",
        }
    }
}

/// A parsed script plus where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedScript {
    pub format: ScriptFormat,
    /// Human-facing engine label ("user", or the value from an Asyik
    /// document). Stored for display only; never used for behavior.
    pub engine: String,
    pub lines: Vec<ScriptLine>,
}

/// Detect and parse any supported import format.
///
/// JSON is never silently treated as text: a file that looks like JSON but
/// fails to parse is a broken export, and importing it as "text" would store
/// punctuation noise as dialogue.
///
/// # Errors
/// Stable codes: `invalid_json`, `invalid_format`, `line_too_long`,
/// `speaker_too_long`, `too_many_lines`, `empty_script`.
pub fn parse(raw: &str) -> Result<ParsedScript, String> {
    let trimmed = raw.trim_start();
    if trimmed.starts_with('[') {
        parse_vntp(raw)
    } else if trimmed.starts_with('{') {
        parse_asyik(raw)
    } else {
        parse_plain(raw)
    }
}

/// VNTP name/message JSON: an array of `{ "name": "…", "message": "…" }`
/// objects where `name` is absent for narration. Any extractor that emits this
/// shape imports as-is (the owner's FVP tool's `dialogue.json` is one producer).
/// Unknown keys are tolerated so a newer tool revision cannot break import; a
/// missing `message` is a malformed export and fails.
fn parse_vntp(raw: &str) -> Result<ParsedScript, String> {
    #[derive(Deserialize)]
    struct Item {
        #[serde(default)]
        name: Option<String>,
        message: String,
    }
    let items: Vec<Item> = serde_json::from_str(raw).map_err(|_| "invalid_json")?;
    let lines = items
        .into_iter()
        .filter(|item| !item.message.trim().is_empty())
        .map(|item| {
            let speaker = item.name.and_then(|name| {
                let trimmed = name.trim();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_owned())
                }
            });
            build_line(speaker, &item.message)
        })
        .collect::<Result<Vec<_>, String>>()?;
    finish(ScriptFormat::JsonVntp, "user".to_owned(), lines)
}

/// The app's interchange format:
/// `{ "engine": "…", "lines": [{ "speaker": "…", "text": "…" }] }` with
/// `speaker` optional for narration.
fn parse_asyik(raw: &str) -> Result<ParsedScript, String> {
    #[derive(Deserialize)]
    struct Doc {
        #[serde(default)]
        engine: String,
        lines: Vec<RawLine>,
    }
    #[derive(Deserialize)]
    struct RawLine {
        #[serde(default)]
        speaker: Option<String>,
        text: String,
    }
    let doc: Doc = serde_json::from_str(raw).map_err(|_| "invalid_json")?;
    if doc.engine.trim().is_empty() {
        return Err("invalid_format".into());
    }
    let engine = doc.engine.trim().chars().take(60).collect::<String>();
    let lines = doc
        .lines
        .into_iter()
        .filter(|line| !line.text.trim().is_empty())
        .map(|line| {
            let speaker = line.speaker.and_then(|name| {
                let trimmed = name.trim();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_owned())
                }
            });
            build_line(speaker, &line.text)
        })
        .collect::<Result<Vec<_>, String>>()?;
    finish(ScriptFormat::AsyikScript, engine, lines)
}

/// Plain UTF-8 text, one dialogue line per non-empty row. A row may carry a
/// speaker as a `Name:` or `Name：` prefix; the prefix becomes the speaker only
/// when it is short (≤ 24 chars), because mid-line colons are common in prose
/// and mislabeling those would corrupt every digest built from the body.
fn parse_plain(raw: &str) -> Result<ParsedScript, String> {
    let lines = raw
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let (speaker, text) = split_speaker(line);
            build_line(speaker.map(str::to_owned), text)
        })
        .collect::<Result<Vec<_>, String>>()?;
    finish(ScriptFormat::PlainText, "user".to_owned(), lines)
}

/// Split an optional `Name:` / `Name：` prefix. Returns the raw line when there
/// is no plausible speaker prefix.
///
/// The rule is deliberately biased toward splitting: a short, clean prefix
/// before a colon is treated as a speaker even though prose can produce the
/// same shape ("彼はこう言った: …"). A wrong label on one narration line costs
/// little in a digest, while dropping every real speaker name would cost the
/// whole character list. Exact speakers come from the JSON import formats;
/// plain text is best-effort by design.
fn split_speaker(line: &str) -> (Option<&str>, &str) {
    let Some(colon) = line.find([':', '：']) else {
        return (None, line);
    };
    // The colon may be one byte (':') or three ('：'); step past the whole
    // character, not one byte.
    let after = colon + line[colon..].chars().next().map_or(1, char::len_utf8);
    let name = &line[..colon];
    let rest = line[after..].trim_start();
    let plausible = !name.is_empty()
        && name.chars().count() <= 24
        && !name.chars().any(|c| "。．.,、!？?「」…".contains(c))
        && !rest.is_empty();
    if plausible {
        (Some(name), rest)
    } else {
        (None, line)
    }
}

fn build_line(speaker: Option<String>, text: &str) -> Result<ScriptLine, String> {
    if text.chars().count() > MAX_LINE_CHARS {
        return Err("line_too_long".into());
    }
    if let Some(speaker) = &speaker
        && speaker.chars().count() > MAX_SPEAKER_CHARS
    {
        return Err("speaker_too_long".into());
    }
    Ok(ScriptLine {
        speaker,
        text: text.trim().to_owned(),
    })
}

fn finish(
    format: ScriptFormat,
    engine: String,
    lines: Vec<ScriptLine>,
) -> Result<ParsedScript, String> {
    if lines.is_empty() {
        return Err("empty_script".into());
    }
    if lines.len() > MAX_LINES {
        return Err("too_many_lines".into());
    }
    Ok(ParsedScript {
        format,
        engine,
        lines,
    })
}

/// Merge several parsed scripts, in the given order, into one document, so a
/// story extracted as many part files imports as one script instead of being
/// combined by hand first.
///
/// The format and engine are kept only when every part agrees; a mix becomes
/// the generic `plain_text`/"user" document, which is what the merged body
/// honestly is. Caps are re-checked on the combined line count.
///
/// # Errors
/// `empty_script` for no parts, `too_many_lines` when the parts together
/// exceed [`MAX_LINES`].
pub fn merge(parts: &[ParsedScript]) -> Result<ParsedScript, String> {
    let Some(first) = parts.first() else {
        return Err("empty_script".into());
    };
    let format = if parts.iter().all(|part| part.format == first.format) {
        first.format
    } else {
        ScriptFormat::PlainText
    };
    let engine = if parts.iter().all(|part| part.engine == first.engine) {
        first.engine.clone()
    } else {
        "user".to_owned()
    };
    let mut lines = Vec::new();
    for part in parts {
        lines.extend(part.lines.iter().cloned());
    }
    finish(format, engine, lines)
}

/// Compare strings the way a person reads file names: runs of ASCII digits
/// compare as numbers, so `scene2` sorts before `scene10`, and letters compare
/// case-insensitively. Equal-valued digit runs (`02` vs `2`) and full ties keep
/// the input order (a stable sort decides).
#[must_use]
pub fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    fn digit_run(text: &str) -> (&str, &str) {
        let end = text
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(text.len());
        text.split_at(end)
    }
    let mut a = a;
    let mut b = b;
    loop {
        let (Some(ca), Some(cb)) = (a.chars().next(), b.chars().next()) else {
            return match (a.is_empty(), b.is_empty()) {
                (true, true) => Ordering::Equal,
                (true, false) => Ordering::Less,
                (false, _) => Ordering::Greater,
            };
        };
        if ca.is_ascii_digit() && cb.is_ascii_digit() {
            let (run_a, rest_a) = digit_run(a);
            let (run_b, rest_b) = digit_run(b);
            // Compare digit runs by numeric value: strip leading zeros, then
            // longer wins, then lexicographic on equal lengths.
            let num_a = run_a.trim_start_matches('0');
            let num_b = run_b.trim_start_matches('0');
            let by_value = num_a.len().cmp(&num_b.len()).then_with(|| num_a.cmp(num_b));
            if by_value != Ordering::Equal {
                return by_value;
            }
            a = rest_a;
            b = rest_b;
        } else {
            let fa = ca.to_ascii_lowercase();
            let fb = cb.to_ascii_lowercase();
            if fa != fb {
                return fa.cmp(&fb);
            }
            a = a.strip_prefix(ca).unwrap_or(a);
            b = b.strip_prefix(cb).unwrap_or(b);
        }
    }
}

/// Render lines to the storage body: `Speaker: text` or bare `text`, one per
/// row. The body is the single source compaction re-chunks from, so it must be
/// deterministic and lossless enough to re-derive speakers via
/// [`split_speaker`].
#[must_use]
pub fn render_body(lines: &[ScriptLine]) -> String {
    let mut body = String::new();
    for line in lines {
        if let Some(speaker) = &line.speaker {
            body.push_str(speaker);
            body.push_str(": ");
        }
        body.push_str(&line.text);
        body.push('\n');
    }
    body
}

/// Plan compaction chunks over a rendered body: a list of `(start, end)` line
/// index ranges (end exclusive), in order, covering every line exactly once.
///
/// A chunk stops before its byte total (UTF-8 length + newline) would exceed
/// `budget`. A single line longer than the budget becomes its own chunk, since
/// splitting a line would corrupt the text the summarizer reads.
#[must_use]
pub fn plan_chunks(body: &str, budget: usize) -> Vec<(usize, usize)> {
    let mut chunks = Vec::new();
    let mut start = 0_usize;
    let mut total = 0_usize;
    for (index, line) in body.lines().enumerate() {
        let cost = line.len() + 1;
        if total > 0 && total + cost > budget {
            chunks.push((start, index));
            start = index;
            total = 0;
        }
        total += cost;
    }
    if body.lines().next().is_some() {
        chunks.push((start, body.lines().count()));
    }
    chunks
}

/// The text of one chunk, as the map step reads it.
#[must_use]
pub fn chunk_text(body: &str, range: (usize, usize)) -> String {
    let mut text = String::new();
    for line in body
        .lines()
        .skip(range.0)
        .take(range.1.saturating_sub(range.0))
    {
        text.push_str(line);
        text.push('\n');
    }
    text
}

/// Whether stored compaction progress can be reused by a new run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resume {
    /// Continue from this many finished chunks.
    Continue(usize),
    /// The stored progress cannot be trusted; start over from zero.
    Restart,
}

/// The identity a stored compaction run must match to be resumable. Progress
/// belongs to exactly one of these: a different model reads differently, a
/// different chunk plan means the stored summaries describe chunks that no
/// longer exist, a different agent layout means they belong to other agents'
/// segments, and a different compactor version means a different pipeline
/// (prompts, context strategy) produced them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactionKey {
    pub model_id: String,
    pub chunk_count: usize,
    pub agent_count: usize,
    pub compactor_version: u32,
}

/// Decide whether an interrupted compaction run can resume. The chunk counter
/// and the stored summary count must also agree; if they disagree the row is
/// untrustworthy and restarting is cheaper than guessing which half is right.
#[must_use]
pub fn resume_decision(
    stored: &CompactionKey,
    planned: &CompactionKey,
    stored_done: usize,
    stored_summary_count: usize,
) -> Resume {
    if *stored != *planned
        || stored_done != stored_summary_count
        || stored_done > planned.chunk_count
    {
        return Resume::Restart;
    }
    Resume::Continue(stored_done)
}

/// Split `chunk_count` chunks over `agents` summarizers: contiguous, balanced,
/// end-exclusive chunk index ranges that cover every chunk exactly once, in
/// order. Agents whose share would be empty (fewer chunks than agents) are
/// dropped rather than run for nothing.
#[must_use]
pub fn plan_agents(chunk_count: usize, agents: usize) -> Vec<(usize, usize)> {
    let agents = agents.max(1);
    if chunk_count == 0 {
        return Vec::new();
    }
    let base = chunk_count / agents;
    let extra = chunk_count % agents;
    let mut ranges = Vec::with_capacity(agents.min(chunk_count));
    let mut start = 0;
    for index in 0..agents {
        let len = base + usize::from(index < extra);
        if len == 0 {
            continue;
        }
        ranges.push((start, start + len));
        start += len;
    }
    ranges
}

/// Map-step prompt: compress one chunk. Summaries feed later question
/// answering, so the prompt forbids interpretation for the same reason the
/// chat system prompt does — an invented motive is indistinguishable from a
/// real one once it is in the digest. The word cap scales with the chunk
/// budget (400 words for the 48 KB default) so a bigger chunk does not silently
/// lose the density the reduce step depends on. Each call after the first in a
/// segment also receives the previous chunk's outline as rolling context; the
/// prompt scopes it to resolving who is being discussed, never re-summarizing
/// it, so an agent's memory cannot leak into another chunk's outline.
pub const MAP_SYSTEM_PROMPT: &str = "You compress one chunk of a visual novel script into a dense factual outline. List only what the chunk shows, in order: which characters speak (use their exact name labels), key events, reveals, and scene changes. You may receive the outline of the preceding chunk as context for continuity: use it only to resolve who or what the current chunk refers to, and never summarize, repeat, or extend it. Do not interpret motives, do not predict what happens next, do not add outside knowledge, and do not invent anything. Plain text, no headers, at most 400 words.";

/// Segment-reduce prompt: one summarizer agent merges its own segment's
/// outlines into that segment's digest.
pub const SEGMENT_REDUCE_SYSTEM_PROMPT: &str = "You merge the chunk-by-chunk outlines you wrote for one segment of a visual novel script into a single chronological segment digest used for later question answering. Keep every named character and every event you can place in order; drop repetitions. Plain prose, no headers, at most 400 words. Do not add anything not present in the outlines.";

/// Final-reduce prompt: the segment digests of several summarizer agents, who
/// worked in parallel and cannot see each other's segments, become one story
/// digest. Overlapping accounts of the same events are expected and must be
/// resolved, not averaged.
pub const REDUCE_SYSTEM_PROMPT: &str = "You merge the segment digests of one visual novel script, written in parallel by several summarizers who each saw only their own segment, into a single chronological story digest used for later question answering. The segments are given in story order; resolve overlapping accounts of the same events, keep every named character and every event you can place in order, and drop repetitions. Plain prose, no headers, at most 600 words. Do not add anything not present in the segment digests.";

/// Most name pairs one mapping block carries. A whole-game cast can be large,
/// but the digest only needs the names the script actually uses, and the block
/// rides along on every reduce call.
pub const MAX_NAME_PAIRS: usize = 80;

/// Build the name-mapping paragraph prepended to a reduce call's input.
///
/// Extracted scripts write character names the way the engine does — in the
/// game's original script — so a digest that quotes them verbatim mentions
/// names the user knows in another form. The pairs come from VNDB's character
/// data (`original` → romaji `name`), and the instruction scopes them to the
/// digest being written: the outlines themselves stay untouched, so a mapping
/// can never rewrite what a chunk actually said.
///
/// Empty input, or no usable pairs, produces an empty string so callers can
/// skip the block entirely.
#[must_use]
pub fn romaji_block(pairs: &[(&str, &str)]) -> String {
    let usable: Vec<(&str, &str)> = pairs
        .iter()
        .take(MAX_NAME_PAIRS)
        .filter(|(original, romaji)| {
            !original.trim().is_empty()
                && !romaji.trim().is_empty()
                && !original.trim().eq_ignore_ascii_case(romaji.trim())
        })
        .copied()
        .collect();
    if usable.is_empty() {
        return String::new();
    }
    let mut block = String::from(
        "Names: the outlines write character names as the game's script spells them. \
         When writing the digest below, write each listed name using its official \
         romaji instead, and keep this list itself out of the digest.\n\n",
    );
    for (original, romaji) in usable {
        block.push_str(original.trim());
        block.push_str(" = ");
        block.push_str(romaji.trim());
        block.push('\n');
    }
    block.push('\n');
    block
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_imports_user_extracted_scripts() {
        let parsed = parse("雪々: ゆきゆきは、そう思う……\n\nただの地の文です。\n").expect("parses");
        assert_eq!(parsed.format, ScriptFormat::PlainText);
        assert_eq!(parsed.engine, "user");
        assert_eq!(
            parsed.lines,
            vec![
                ScriptLine {
                    speaker: Some("雪々".into()),
                    text: "ゆきゆきは、そう思う……".into()
                },
                ScriptLine {
                    speaker: None,
                    text: "ただの地の文です。".into()
                },
            ]
        );
    }

    #[test]
    fn a_fullwidth_colon_also_marks_a_speaker() {
        let (speaker, text) = split_speaker("ひなた：この雪ですね");
        assert_eq!(speaker, Some("ひなた"));
        assert_eq!(text, "この雪ですね");
    }

    #[test]
    fn a_short_clean_prefix_before_an_ascii_colon_is_a_speaker() {
        // Prose can produce the same shape as a nameplate ("彼はこう言った:
        // …"), and no cheap rule can tell them apart. Splitting keeps every
        // real name at the cost of a rare wrong label, which is the documented
        // plain-text trade-off; the JSON formats carry exact speakers.
        let (speaker, text) = split_speaker("彼はこう言った: 「こんにちは」と、雪の中で。");
        assert_eq!(speaker, Some("彼はこう言った"));
        assert_eq!(text, "「こんにちは」と、雪の中で。");
    }

    #[test]
    fn a_sentence_like_prefix_is_not_a_speaker() {
        let (speaker, _) = split_speaker("わからない。: それでも歩く。");
        assert_eq!(speaker, None);
    }

    #[test]
    fn vntp_name_message_json_imports_verbatim() {
        let raw = r#"[
            {"name":"雪々","message":"テストルートを選んでください"},
            {"message":"ここは自由に駆け回れる庭だった。"},
            {"name":"  ","message":"名前が空白なら地の文"}
        ]"#;
        let parsed = parse(raw).expect("parses");
        assert_eq!(parsed.format, ScriptFormat::JsonVntp);
        assert_eq!(parsed.engine, "user");
        assert_eq!(parsed.lines.len(), 3);
        assert_eq!(parsed.lines[0].speaker.as_deref(), Some("雪々"));
        assert_eq!(parsed.lines[1].speaker, None);
        // A whitespace-only name is narration, not an empty speaker label.
        assert_eq!(parsed.lines[2].speaker, None);
    }

    #[test]
    fn merge_combines_part_files_in_order_and_enforces_caps() {
        let first = parse("雪々: 一つ目のファイル\n").expect("parses");
        let second = parse("雪々: 二つ目のファイル\n").expect("parses");
        let merged = merge(&[first, second]).expect("merges");
        assert_eq!(merged.format, ScriptFormat::PlainText);
        assert_eq!(merged.engine, "user");
        assert_eq!(merged.lines.len(), 2);
        assert_eq!(merged.lines[1].text, "二つ目のファイル");

        let line = ScriptLine {
            speaker: None,
            text: "一行".to_owned(),
        };
        let half = ParsedScript {
            format: ScriptFormat::PlainText,
            engine: "user".to_owned(),
            lines: vec![line; MAX_LINES / 2 + 1],
        };
        assert_eq!(merge(&[half.clone(), half]).unwrap_err(), "too_many_lines");
        assert_eq!(merge(&[]).unwrap_err(), "empty_script");
    }

    #[test]
    fn merge_of_mixed_formats_is_the_generic_document() {
        let plain = parse("ただの地の文\n").expect("parses");
        let asyik = parse(r#"{"engine":"kirikiri","lines":[{"text":"Hello."}]}"#).expect("parses");
        let merged = merge(&[plain, asyik]).expect("merges");
        // The parts disagree about their shape, so the merged document is
        // honestly the generic one rather than claiming either engine.
        assert_eq!(merged.format, ScriptFormat::PlainText);
        assert_eq!(merged.engine, "user");
        assert_eq!(merged.lines.len(), 2);
    }

    #[test]
    fn natural_order_sorts_digit_runs_as_numbers() {
        let mut names = vec!["scene10.txt", "scene2.txt", "Scene1.txt", "scene1b.txt"];
        names.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(
            names,
            vec!["Scene1.txt", "scene1b.txt", "scene2.txt", "scene10.txt"]
        );
        // Equal-valued runs with different zero padding do not split order
        // beyond the tie: both sort after 1 and before 3.
        let mut padded = ["part03", "part002", "part1"];
        padded.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(padded[..1], ["part1"]);
    }

    #[test]
    fn resume_requires_the_same_model_plan_and_counter() {
        let key = |model: &str, chunks: usize, agents: usize, version: u32| CompactionKey {
            model_id: model.to_owned(),
            chunk_count: chunks,
            agent_count: agents,
            compactor_version: version,
        };
        // Same key, counter matching the stored summaries.
        assert_eq!(
            resume_decision(&key("qwen", 10, 3, 1), &key("qwen", 10, 3, 1), 3, 3),
            Resume::Continue(3)
        );
        // Different model: summaries describe another model's reading.
        assert_eq!(
            resume_decision(&key("qwen", 10, 3, 1), &key("gpt", 10, 3, 1), 3, 3),
            Resume::Restart
        );
        // Different chunk plan: the budget changed, the stored summaries
        // describe chunks that no longer exist.
        assert_eq!(
            resume_decision(&key("qwen", 10, 3, 1), &key("qwen", 42, 3, 1), 3, 3),
            Resume::Restart
        );
        // Different agent layout: the summaries belong to other agents'
        // segments.
        assert_eq!(
            resume_decision(&key("qwen", 10, 3, 1), &key("qwen", 10, 5, 1), 3, 3),
            Resume::Restart
        );
        // Different pipeline version: the prompts or context strategy changed.
        assert_eq!(
            resume_decision(&key("qwen", 10, 3, 0), &key("qwen", 10, 3, 1), 3, 3),
            Resume::Restart
        );
        // Counter disagrees with the stored summaries: untrustworthy row.
        assert_eq!(
            resume_decision(&key("qwen", 10, 3, 1), &key("qwen", 10, 3, 1), 3, 4),
            Resume::Restart
        );
        // Done beyond the plan: corrupt.
        assert_eq!(
            resume_decision(&key("qwen", 10, 3, 1), &key("qwen", 10, 3, 1), 11, 11),
            Resume::Restart
        );
    }

    #[test]
    fn agents_cover_every_chunk_once_with_balanced_shares() {
        let plan = plan_agents(10, 3);
        assert_eq!(plan, vec![(0, 4), (4, 7), (7, 10)], "remainder goes left");

        let plan = plan_agents(9, 3);
        assert_eq!(plan, vec![(0, 3), (3, 6), (6, 9)]);

        // One agent is the whole story; zero chunks plan nothing; more agents
        // than chunks drops the empty shares instead of running them.
        assert_eq!(plan_agents(7, 1), vec![(0, 7)]);
        assert!(plan_agents(0, 3).is_empty());
        assert_eq!(plan_agents(2, 5), vec![(0, 1), (1, 2)]);
        assert_eq!(plan_agents(4, 0), vec![(0, 4)], "zero agents means one");
    }

    #[test]
    fn the_chunk_budget_is_pinned() {
        // Stored compaction progress counts chunks against this budget: a
        // silent change restarts every in-flight compaction (the resume guard
        // makes that safe, not free). Change it only with that reset in mind.
        assert_eq!(DEFAULT_CHUNK_BYTES, 48_000);
    }

    #[test]
    fn asyik_script_requires_an_engine_and_keeps_speakers() {
        let raw = r#"{"engine":"kirikiri","lines":[
            {"speaker":"Haru","text":"Hello."},
            {"text":"Narration."}
        ]}"#;
        let parsed = parse(raw).expect("parses");
        assert_eq!(parsed.format, ScriptFormat::AsyikScript);
        assert_eq!(parsed.engine, "kirikiri");
        assert_eq!(parsed.lines[0].speaker.as_deref(), Some("Haru"));
        assert_eq!(parsed.lines[1].speaker, None);
    }

    #[test]
    fn broken_json_is_an_error_never_text() {
        // A truncated export must not be imported as "plain text" punctuation.
        assert_eq!(
            parse(r#"[{"name":"雪々","mess"#).unwrap_err(),
            "invalid_json"
        );
        assert_eq!(
            parse(r#"{"lines":"not an array"}"#).unwrap_err(),
            "invalid_json"
        );
    }

    #[test]
    fn caps_are_enforced_for_every_format() {
        let too_long = "あ".repeat(MAX_LINE_CHARS + 1);
        assert_eq!(
            parse(&format!("雪々: {too_long}")).unwrap_err(),
            "line_too_long"
        );
        let speaker = "名".repeat(MAX_SPEAKER_CHARS + 1);
        assert_eq!(
            parse_asyik(&format!(
                r#"{{"engine":"x","lines":[{{"speaker":"{speaker}","text":"hi"}}]}}"#
            ))
            .unwrap_err(),
            "speaker_too_long"
        );
        let many: Vec<&str> = (0..=MAX_LINES).map(|_| "一行").collect();
        assert_eq!(parse(&many.join("\n")).unwrap_err(), "too_many_lines");
    }

    #[test]
    fn a_script_with_no_content_is_rejected() {
        assert_eq!(parse("   \n\n  ").unwrap_err(), "empty_script");
        assert_eq!(parse("[]").unwrap_err(), "empty_script");
        assert_eq!(
            parse(r#"{"engine":"x","lines":[]}"#).unwrap_err(),
            "empty_script"
        );
    }

    #[test]
    fn the_body_round_trips_through_the_plaintext_convention() {
        let lines = vec![
            ScriptLine {
                speaker: Some("雪々".into()),
                text: "そう思う……".into(),
            },
            ScriptLine {
                speaker: None,
                text: "地の文。".into(),
            },
        ];
        let body = render_body(&lines);
        assert_eq!(body, "雪々: そう思う……\n地の文。\n");
        let parsed = parse(&body).expect("parses");
        assert_eq!(parsed.lines, lines);
    }

    #[test]
    fn chunks_cover_every_line_exactly_once_within_budget() {
        let body: Vec<String> = (0..200)
            .map(|index| format!("line {index:03} with some bytes"))
            .collect();
        let body = body.join("\n") + "\n";
        let line_count = body.lines().count();
        let budget = 120_usize;
        let chunks = plan_chunks(&body, budget);
        assert_eq!(chunks.first().unwrap().0, 0);
        assert_eq!(chunks.last().unwrap().1, line_count);
        // Contiguous and gapless.
        for pair in chunks.windows(2) {
            assert_eq!(pair[0].1, pair[1].0);
        }
        // Every chunk within budget, and no line lost or duplicated: the
        // concatenation of chunk texts must rebuild the body.
        let mut rebuilt = String::new();
        for range in &chunks {
            let text = chunk_text(&body, *range);
            assert!(text.len() <= budget || text.lines().count() == 1);
            rebuilt.push_str(&text);
        }
        assert_eq!(rebuilt, body);
    }

    #[test]
    fn a_line_longer_than_the_budget_is_its_own_chunk() {
        let huge = "x".repeat(DEFAULT_CHUNK_BYTES * 2);
        let body = format!("short\n{huge}\nshort again\n");
        let chunks = plan_chunks(&body, DEFAULT_CHUNK_BYTES);
        assert!(chunks.len() >= 3);
        let oversized = &chunks[1];
        assert_eq!(oversized.1 - oversized.0, 1);
        assert_eq!(chunk_text(&body, *oversized).trim(), huge);
    }

    #[test]
    fn an_empty_body_plans_no_chunks() {
        assert!(plan_chunks("", DEFAULT_CHUNK_BYTES).is_empty());
    }

    #[test]
    fn compaction_prompts_forbid_inventing() {
        // The digest is evidence for the chat; the same no-invention rule as
        // the chat system prompt must reach every compaction call. The word cap
        // is pinned too: it scales with the chunk budget, and shrinking it
        // would silently thin every outline a digest is built from.
        assert!(MAP_SYSTEM_PROMPT.contains("do not invent"));
        assert!(MAP_SYSTEM_PROMPT.contains("in order"));
        assert!(MAP_SYSTEM_PROMPT.contains("at most 400 words"));
        // The rolling context is scoped: an agent's memory of the previous
        // chunk must resolve references, never leak into the new outline.
        assert!(MAP_SYSTEM_PROMPT.contains("preceding chunk"));
        assert!(MAP_SYSTEM_PROMPT.contains("never summarize, repeat, or extend"));
        assert!(SEGMENT_REDUCE_SYSTEM_PROMPT.contains("Do not add anything not present"));
        // The final reduce knows the segment digests came from parallel agents
        // who could not see each other, so overlaps are resolved, not averaged.
        assert!(REDUCE_SYSTEM_PROMPT.contains("in parallel"));
        assert!(REDUCE_SYSTEM_PROMPT.contains("Do not add anything not present"));
    }

    #[test]
    fn the_romaji_mapping_scopes_itself_and_survives_edge_cases() {
        let block = romaji_block(&[("雪々", "Yukki"), ("大輔", "Daisuke")]);
        assert!(block.contains("雪々 = Yukki"));
        assert!(block.contains("大輔 = Daisuke"));
        // The instruction binds the mapping to the digest being written: the
        // outlines stay verbatim, so the mapping cannot rewrite what a chunk said.
        assert!(block.contains("digest below"));
        assert!(block.contains("keep this list itself out of the digest"));

        // A pair where both sides already agree adds noise, not information.
        assert!(romaji_block(&[("Anna", "anna")]).is_empty());
        assert!(romaji_block(&[("", "Yukki"), ("雪々", " ")]).is_empty());
        assert!(romaji_block(&[]).is_empty());
        // The cap keeps a huge cast from riding on every reduce call.
        let many: Vec<(&str, &str)> = (0..200).map(|_| ("x", "y")).collect();
        assert_eq!(
            romaji_block(&many)
                .lines()
                .filter(|line| line.contains(" = "))
                .count(),
            MAX_NAME_PAIRS
        );
    }
}
