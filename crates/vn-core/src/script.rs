//! Extracted VN scripts: VNTP JSON import, normalization and compaction planning.
//!
//! Pure and synchronous so it tests fast. Imported scripts use the VNTP
//! name/message JSON array shape ([`ScriptFormat::JsonVntp`]:
//! `[{ "name": "…", "message": "…" }]`, `name` absent for narration) and normalize
//! into [`ScriptLine`] rows for storage, chunking and compaction.
//!
//! Siglus support was explicitly cancelled by the project owner (2026-10-24);
//! do not add it back without asking.

use serde::{Deserialize, Serialize};

/// Longest accepted dialogue line, in characters.
pub const MAX_LINE_CHARS: usize = 4_000;
/// Longest accepted speaker label, in characters.
pub const MAX_SPEAKER_CHARS: usize = 120;
/// Most accepted lines per document; past this the file is unlikely to be a script.
pub const MAX_LINES: usize = 300_000;
/// Default UTF-8 compaction chunk budget (~12k Japanese tokens; leaves 16k context room
/// for the outline). Pinned by a test because stored progress counts chunks against it.
pub const DEFAULT_CHUNK_BYTES: usize = 48_000;
/// Most summarizer agents one compaction runs in parallel (owner cap).
pub const MAX_AGENTS: usize = 10;
/// Default summarizer agent count; kept modest because each agent is a concurrent call.
pub const DEFAULT_AGENTS: usize = 3;
/// Compaction pipeline version; bumping restarts in-flight runs so outputs from
/// different prompts or context layouts never mix.
pub const COMPACTOR_VERSION: u32 = 3;
/// Characters of the previous chunk's outline passed to the next map call as rolling context.
pub const ROLLING_CONTEXT_CHARS: usize = 1_500;

/// One normalized dialogue line. `speaker` is `None` for narration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptLine {
    pub speaker: Option<String>,
    pub text: String,
}

/// Which importer produced a document. Stored in `script_documents.source_kind`
/// and constrained by a CHECK in migration 0024.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScriptFormat {
    JsonVntp,
}

impl ScriptFormat {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::JsonVntp => "json_vntp",
        }
    }
}

/// A parsed script plus where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedScript {
    pub format: ScriptFormat,
    /// Human-facing engine label ("user"). Stored for display only; never used
    /// for behavior.
    pub engine: String,
    pub lines: Vec<ScriptLine>,
}

/// Parse a VNTP JSON script (`[{ "name": "…", "message": "…" }]`, `name`
/// optional for narration).
///
/// Unknown object keys are tolerated so a newer extractor revision cannot break
/// import; non-JSON text returns `invalid_json`, and valid JSON that is not an
/// array of `{ "name"?, "message" }` objects returns `invalid_format`.
///
/// # Errors
/// Stable codes: `invalid_json`, `invalid_format`, `line_too_long`,
/// `speaker_too_long`, `too_many_lines`, `empty_script`.
pub fn parse(raw: &str) -> Result<ParsedScript, String> {
    let raw = raw.strip_prefix('\u{FEFF}').unwrap_or(raw);
    parse_vntp(raw)
}

fn parse_vntp(raw: &str) -> Result<ParsedScript, String> {
    #[derive(Deserialize)]
    struct Item {
        #[serde(default)]
        name: Option<String>,
        message: String,
    }
    let value: serde_json::Value = serde_json::from_str(raw).map_err(|_| "invalid_json")?;
    let serde_json::Value::Array(raw_items) = value else {
        return Err("invalid_format".into());
    };
    let mut items = Vec::with_capacity(raw_items.len());
    for raw_item in raw_items {
        let item: Item = serde_json::from_value(raw_item).map_err(|_| "invalid_format")?;
        items.push(item);
    }
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

fn build_line(speaker: Option<String>, text: &str) -> Result<ScriptLine, String> {
    let clean_text = text.trim();
    if clean_text.chars().count() > MAX_LINE_CHARS {
        return Err("line_too_long".into());
    }
    let clean_speaker = speaker.and_then(|value| {
        let collapsed = value.replace(['\r', '\n'], " ");
        let trimmed = collapsed.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_owned())
        }
    });
    if let Some(speaker) = &clean_speaker
        && speaker.chars().count() > MAX_SPEAKER_CHARS
    {
        return Err("speaker_too_long".into());
    }
    Ok(ScriptLine {
        speaker: clean_speaker,
        text: clean_text.to_owned(),
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
/// combined by hand first. Caps are re-checked on the combined line count.
///
/// # Errors
/// `empty_script` for no parts, `too_many_lines` when the parts together
/// exceed [`MAX_LINES`].
pub fn merge(parts: &[ParsedScript]) -> Result<ParsedScript, String> {
    if parts.is_empty() {
        return Err("empty_script".into());
    }
    let mut lines = Vec::new();
    for part in parts {
        lines.extend(part.lines.iter().cloned());
    }
    finish(ScriptFormat::JsonVntp, "user".to_owned(), lines)
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
    let mut count = 0_usize;
    for (index, line) in body.lines().enumerate() {
        let cost = line.len() + 1;
        if total > 0 && total + cost > budget {
            chunks.push((start, index));
            start = index;
            total = 0;
        }
        total += cost;
        count = index + 1;
    }
    if count > 0 {
        chunks.push((start, count));
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

/// How a stored digest differs from a run of the current pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DigestFreshness {
    /// Produced by this pipeline version and still complete.
    Current,
    /// Finished, but by an older pipeline: readable, and worth rebuilding.
    Outdated,
}

/// Decision on a finished digest. A stored 0 for a row that somehow recorded no
/// version is the phase 18/19B pipeline, not the pre-run sentinel.
#[must_use]
pub fn digest_freshness(stored_version: u32, planned_version: u32) -> DigestFreshness {
    if stored_version == planned_version {
        DigestFreshness::Current
    } else {
        DigestFreshness::Outdated
    }
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

/// Map-step prompt for one chunk (400-word cap scales with the 48 KB default
/// budget; forbids speculation and scopes rolling context to resolving references).
pub const MAP_SYSTEM_PROMPT: &str = "You create a factual story index for one chunk of a visual novel script. A later assistant answers detailed questions using only your index, so preserve evidence instead of writing a polished summary. Record in story order: exact speaker labels, named characters present, setting and scene transitions, important actions and revelations, choices shown to the player, and route or branch labels only when the text supplies them. Preserve short distinctive details, named objects, promises, conflicts, and changes in stated relationships. Distinguish dialogue, narration, dreams, recollections, and hypotheses; never turn a character's belief into a fact, and do not invent anything. When the text does not establish a motive, identity, chronology or route, write that it is unknown instead of inferring it. You may receive the preceding chunk's index for continuity only: use it to resolve references, never repeat or extend it. Do not add outside knowledge, predictions, literary analysis or invented quotations. Use compact plain text, no headers, at most 400 words.";

/// Segment-reduce prompt: merges one agent's segment outlines into its digest.
pub const SEGMENT_REDUCE_SYSTEM_PROMPT: &str = "You merge chunk indexes from one contiguous segment of a visual novel into a factual story index for later question answering. Preserve exact character names, chronology, scene transitions, player choices, explicit route or branch labels, important revelations, and unresolved questions. Keep a relationship change only when the evidence states or demonstrates it. Distinguish observed events from dialogue, recollections, dreams and speculation. Remove repeated wording without erasing distinct events or branches. If two indexes conflict, retain the conflict as unresolved. Do not add facts, motives, interpretations or outside knowledge. Use compact plain text, no headers, at most 700 words.";

/// Final-reduce prompt: merges parallel agents' segment digests and resolves overlaps.
pub const REDUCE_SYSTEM_PROMPT: &str = "You merge ordered segment indexes from one visual novel into a factual story index for later question answering. The segment writers could not see each other's material. Preserve exact character names, chronology, scene transitions, player choices, explicit route or branch labels, revelations, and unresolved questions. Do not merge separate branches into one timeline, and do not call a branch canonical unless the source says so. Distinguish observed events from dialogue, recollections, dreams and speculation. Resolve only genuine overlaps; retain conflicts as unresolved instead of choosing a version. Remove repetition without erasing distinct events, relationships or consequences. Do not add facts, motives, interpretations or outside knowledge. Use compact plain text with these labels where evidence exists: CHARACTERS, CHRONOLOGY, CHOICES AND BRANCHES, UNRESOLVED. Omit empty labels. At most 1200 words.";

/// Most name pairs carried in one reduce call.
pub const MAX_NAME_PAIRS: usize = 80;

/// Prepends VNDB character name pairs (`original` -> romaji `name`) for reduce calls
/// without altering the underlying chunk outlines. Returns empty when no pairs are usable.
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
        let first = parse(r#"[{"name":"雪々","message":"一つ目のファイル"}]"#).expect("parses");
        let second = parse(r#"[{"name":"雪々","message":"二つ目のファイル"}]"#).expect("parses");
        let merged = merge(&[first, second]).expect("merges");
        assert_eq!(merged.format, ScriptFormat::JsonVntp);
        assert_eq!(merged.engine, "user");
        assert_eq!(merged.lines.len(), 2);
        assert_eq!(merged.lines[1].text, "二つ目のファイル");

        let line = ScriptLine {
            speaker: None,
            text: "一行".to_owned(),
        };
        let half = ParsedScript {
            format: ScriptFormat::JsonVntp,
            engine: "user".to_owned(),
            lines: vec![line; MAX_LINES / 2 + 1],
        };
        assert_eq!(merge(&[half.clone(), half]).unwrap_err(), "too_many_lines");
        assert_eq!(merge(&[]).unwrap_err(), "empty_script");
    }

    #[test]
    fn natural_order_sorts_digit_runs_as_numbers() {
        let mut names = vec!["scene10.json", "scene2.json", "Scene1.json", "scene1b.json"];
        names.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(
            names,
            vec!["Scene1.json", "scene1b.json", "scene2.json", "scene10.json"]
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
        assert_eq!(
            plan_agents(0, 3).len(),
            0,
            "plan_agents(0, 3) should be empty"
        );
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
    fn non_vntp_or_broken_inputs_are_rejected() {
        assert_eq!(
            parse(r#"[{"name":"雪々","mess"#).unwrap_err(),
            "invalid_json"
        );
        assert_eq!(
            parse("雪々: ゆきゆきは、そう思う……\n").unwrap_err(),
            "invalid_json"
        );
        assert_eq!(
            parse(r#"{"engine":"kirikiri","lines":[{"text":"Hello."}]}"#).unwrap_err(),
            "invalid_format"
        );
        assert_eq!(
            parse(r#"[{"speaker":"Haru","text":"Hello."}]"#).unwrap_err(),
            "invalid_format"
        );
    }

    #[test]
    fn caps_are_enforced_for_vntp_scripts() {
        let too_long = "あ".repeat(MAX_LINE_CHARS + 1);
        assert_eq!(
            parse(&format!(r#"[{{"name":"雪々","message":"{too_long}"}}]"#)).unwrap_err(),
            "line_too_long"
        );
        let speaker = "名".repeat(MAX_SPEAKER_CHARS + 1);
        assert_eq!(
            parse(&format!(r#"[{{"name":"{speaker}","message":"hi"}}]"#)).unwrap_err(),
            "speaker_too_long"
        );
        let many: Vec<&str> = (0..=MAX_LINES).map(|_| r#"{"message":"一行"}"#).collect();
        let raw_many = format!("[{}]", many.join(","));
        assert_eq!(parse(&raw_many).unwrap_err(), "too_many_lines");
    }

    #[test]
    fn a_script_with_no_content_is_rejected() {
        assert_eq!(parse("[]").unwrap_err(), "empty_script");
        assert_eq!(
            parse(r#"[{"name":"雪々","message":"   \n\t  "}]"#).unwrap_err(),
            "empty_script"
        );
    }

    #[test]
    fn render_body_formats_speaker_and_narration_lines() {
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
    }

    #[test]
    fn digest_freshness_separates_old_pipelines() {
        // A digest from the previous prompts is readable but stale; the caller
        // offers a rebuild rather than silently serving it as current.
        assert_eq!(
            digest_freshness(COMPACTOR_VERSION, COMPACTOR_VERSION),
            DigestFreshness::Current
        );
        assert_eq!(
            digest_freshness(2, COMPACTOR_VERSION),
            DigestFreshness::Outdated
        );
        assert_eq!(
            digest_freshness(1, COMPACTOR_VERSION),
            DigestFreshness::Outdated
        );
        assert_eq!(
            digest_freshness(0, COMPACTOR_VERSION),
            DigestFreshness::Outdated
        );
    }

    #[test]
    fn the_compaction_prompts_keep_branches_and_uncertainty() {
        // A digest is the only story evidence the companion has. Shortening it
        // into a clean narrative is what made answers generic, so the prompts
        // must require branches, choices and explicit unknowns.
        for prompt in [
            MAP_SYSTEM_PROMPT,
            SEGMENT_REDUCE_SYSTEM_PROMPT,
            REDUCE_SYSTEM_PROMPT,
        ] {
            assert!(
                prompt.contains("choice") || prompt.contains("branch"),
                "{prompt}"
            );
            assert!(prompt.contains("Do not add"), "{prompt}");
        }
        assert!(REDUCE_SYSTEM_PROMPT.contains("UNRESOLVED"));
        assert!(MAP_SYSTEM_PROMPT.contains("unknown"));
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
        assert_eq!(
            plan_chunks("", DEFAULT_CHUNK_BYTES).len(),
            0,
            "plan_chunks(\"\", DEFAULT_CHUNK_BYTES) should be empty"
        );
    }

    #[test]
    fn compaction_prompts_forbid_inventing() {
        // The digest is evidence for the chat; the same no-invention rule as
        // the chat system prompt must reach every compaction call. The word cap
        // is pinned too: it scales with the chunk budget, and shrinking it
        // would silently thin every outline a digest is built from.
        assert!(MAP_SYSTEM_PROMPT.contains("Do not add outside knowledge"));
        assert!(MAP_SYSTEM_PROMPT.contains("at most 400 words"));
        // The rolling context is scoped: an agent's memory of the previous
        // chunk must resolve references, never leak into the new outline.
        assert!(MAP_SYSTEM_PROMPT.contains("preceding chunk"));
        assert!(MAP_SYSTEM_PROMPT.contains("never repeat or extend it"));
        assert!(SEGMENT_REDUCE_SYSTEM_PROMPT.contains("Do not add facts"));
        // The final reduce knows the segment indexes came from parallel agents
        // who could not see each other, so overlaps are resolved and separate
        // branches are not collapsed into one timeline.
        assert!(REDUCE_SYSTEM_PROMPT.contains("could not see each other's"));
        assert!(REDUCE_SYSTEM_PROMPT.contains("Do not add facts"));
        assert!(REDUCE_SYSTEM_PROMPT.contains("CHOICES AND BRANCHES"));
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
        assert_eq!(
            romaji_block(&[("Anna", "anna")]).len(),
            0,
            "romaji_block(&[(\"Anna\", \"anna\")]) should be empty"
        );
        assert_eq!(
            romaji_block(&[("", "Yukki"), ("雪々", " ")]).len(),
            0,
            "romaji_block(&[(\"\", \"Yukki\"), (\"雪々\", \" \")]) should be empty"
        );
        assert_eq!(
            romaji_block(&[]).len(),
            0,
            "romaji_block(&[]) should be empty"
        );
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

    #[test]
    fn parse_strips_utf8_bom_and_sanitizes_speaker_names() {
        let bom_vntp = "\u{FEFF}[{\"name\":\"Haru\\nSakura \",\"message\":\"Hi.\"}]";
        let parsed = parse(bom_vntp).expect("parses BOM VNTP JSON");
        assert_eq!(parsed.format, ScriptFormat::JsonVntp);
        assert_eq!(parsed.lines[0].speaker.as_deref(), Some("Haru Sakura"));
    }
}
