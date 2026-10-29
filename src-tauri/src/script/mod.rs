//! Extracted VN scripts: storage, import and map-reduce compaction.
//!
//! The parsing rules, chunk planning and prompts are pure and live in
//! `vn_core::script`. This module owns everything with side effects: reading
//! the file the user points at, storing the normalized body, and asking the
//! configured chat provider to compress it chunk by chunk until a single
//! digest exists for the chat tools to cite.
//!
//! Scripts are local-only (like chat history): never synced, never exported.

use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::platform::storage::ResolvedDocument;
use vn_core::script::{
    COMPACTOR_VERSION, CompactionKey, DEFAULT_AGENTS, DEFAULT_CHUNK_BYTES, DigestFreshness,
    MAP_SYSTEM_PROMPT, MAX_AGENTS, REDUCE_SYSTEM_PROMPT, ROLLING_CONTEXT_CHARS, Resume,
    SEGMENT_REDUCE_SYSTEM_PROMPT, digest_freshness, merge, natural_cmp, parse, plan_agents,
    plan_chunks, render_body, resume_decision,
};

/// Largest accepted import file, per file. A whole-game script is a few MB of
/// text; 64 MB means a wrong file (an archive, a disk image) fails fast
/// instead of being read into memory.
pub const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
/// Most stored documents per game. Re-imports dedupe by content hash, so this
/// only bounds deliberate imports of different revisions.
pub const MAX_DOCUMENTS_PER_GAME: i64 = 20;
/// Refuse to compact a document that would need more chunks than this. With the
/// 48 KB default budget this is ~190 MB of dialogue, far past any real script.
pub const MAX_CHUNKS: usize = 4_096;
/// Most chunks summarized per `script_compact` call. Keeps one command call
/// bounded; the caller repeats until `done_chunks == chunk_count`.
pub const MAX_CHUNKS_PER_CALL: u32 = 8;
/// Cap on one stored chunk summary, in characters.
const MAX_SUMMARY_CHARS: usize = 4_000;
/// Cap on the stored digest, in characters. The v2 reduce writes at most 1200
/// words of labelled sections, so this is a safety net, not the length target.
const MAX_DIGEST_CHARS: usize = 40_000;
/// Cap on the text sent to the reduce step, in characters.
const MAX_REDUCE_INPUT_CHARS: usize = 200_000;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptDocumentInfo {
    pub id: String,
    pub game_id: String,
    pub engine: String,
    pub source_kind: String,
    pub source_path: String,
    pub source_sha256: String,
    pub line_count: i64,
    pub byte_size: i64,
    /// Chunks this document splits into, known from import.
    pub chunk_count: i64,
    pub created_at: i64,
    pub compaction: Option<CompactionInfo>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompactionInfo {
    pub model_id: String,
    pub chunk_count: usize,
    pub done_chunks: usize,
    pub agent_count: usize,
    /// Pipeline that produced the stored progress.
    pub compactor_version: u32,
    /// Pipeline this build would run now.
    pub current_version: u32,
    pub digest: Option<String>,
    /// True for a finished digest from an older pipeline. The companion still
    /// reads it; the UI offers a rebuild instead of hiding that it is stale.
    pub digest_outdated: bool,
}

/// `sync_state` key holding the agent settings JSON blob.
pub const AGENT_SETTINGS_KEY: &str = "script.compaction";

/// How many summarizer agents one compaction runs in parallel. Each agent is
/// a concurrent provider call, so the number is a speed/rate-limit trade-off,
/// not a quality knob.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentSettings {
    pub agents: usize,
}

impl Default for AgentSettings {
    fn default() -> Self {
        Self {
            agents: DEFAULT_AGENTS,
        }
    }
}

impl AgentSettings {
    /// Clamp into `1..=MAX_AGENTS`; anything else is a typo or a hand-edited
    /// blob, not worth failing a save over.
    #[must_use]
    pub fn normalized(self) -> Self {
        Self {
            agents: self.agents.clamp(1, MAX_AGENTS),
        }
    }
}

/// Load agent settings from `sync_state`. A missing key answers the default;
/// a real database error or an unreadable blob is an error, never a silent
/// default — the agent count is part of the compaction resume key, so
/// compaction would quietly reset an in-flight run's stored progress instead
/// of mixing layouts. A corrupt blob is fixed by re-saving the setting in the
/// UI, which rewrites it.
///
/// # Errors
/// `db_error` when the store cannot be read, `agent_settings_invalid` when the
/// blob does not parse.
pub async fn load_agent_settings(pool: &SqlitePool) -> Result<AgentSettings, String> {
    let found: Result<Option<(String,)>, _> =
        sqlx::query_as("SELECT value FROM sync_state WHERE key = ?")
            .bind(AGENT_SETTINGS_KEY)
            .fetch_optional(pool)
            .await;
    match found {
        Ok(None) => Ok(AgentSettings::default()),
        Ok(Some((json,))) => serde_json::from_str::<AgentSettings>(&json)
            .map(AgentSettings::normalized)
            .map_err(|_| "agent_settings_invalid".to_owned()),
        Err(_) => Err("db_error".to_owned()),
    }
}

/// Normalize, persist and return the agent settings.
///
/// # Errors
/// `db_error`.
pub async fn save_agent_settings(
    pool: &SqlitePool,
    settings: AgentSettings,
) -> Result<AgentSettings, String> {
    let settings = settings.normalized();
    let json = serde_json::to_string(&settings).map_err(|_| "db_error".to_owned())?;
    sqlx::query(
        "INSERT INTO sync_state (key, value) VALUES (?, ?) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(AGENT_SETTINGS_KEY)
    .bind(&json)
    .execute(pool)
    .await
    .map_err(|_| "db_error".to_owned())?;
    Ok(settings)
}

fn now() -> i64 {
    time::OffsetDateTime::now_utc()
        .unix_timestamp_nanos()
        .checked_div(1_000_000)
        .and_then(|v| i64::try_from(v).ok())
        .unwrap_or(0)
}

/// Import one or many script files for a game as a single document, so a
/// script extracted as many part files imports in one pick instead of being
/// combined by hand first. The format of each file is detected from its
/// content (plain text, VNTP name/message JSON, or the Asyik JSON interchange
/// format); the parts are merged in natural display-name order (`scene2`
/// before `scene10`), which is the order extractors number their output in.
///
/// Importing the same set of files for the same game twice returns the
/// existing row (content hash over the files in merge order plus the rendered
/// body, which the concatenating hash alone cannot tell apart) instead of
/// storing a second copy.
///
/// # Errors
/// Stable codes: `no_files`, `game_missing`, `invalid_game`, `read_failed`,
/// `file_too_large`, `invalid_utf8`, plus the parse and merge codes from
/// `vn_core::script`, `db_error`, `script_limit`. Any bad file fails the whole
/// import; nothing is stored from a partial set.
pub async fn import(
    pool: &SqlitePool,
    game_id: &str,
    paths: &[String],
) -> Result<ScriptDocumentInfo, String> {
    if paths.is_empty() {
        return Err("no_files".into());
    }
    uuid::Uuid::parse_str(game_id).map_err(|_| "invalid_game")?;
    let live: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM games WHERE id=? AND deleted=0")
        .bind(game_id)
        .fetch_one(pool)
        .await
        .map_err(|_| "db_error")?;
    if live.0 != 1 {
        return Err("game_missing".into());
    }

    // Resolve what the picker returned before touching the filesystem: on
    // Android these are `content://` URIs, which the resolver copies through
    // the content resolver into cache files (AGENTS.md §9); on every other
    // platform the path passes through unchanged. The guards delete the
    // copies when they drop at the end of the import.
    let mut resolved = Vec::with_capacity(paths.len());
    for path in paths {
        let uri = path.clone();
        let guard = tauri::async_runtime::spawn_blocking(move || {
            crate::platform::storage::resolve_document_uri(&uri)
        })
        .await
        .map_err(|_| "read_failed".to_owned())?
        .map_err(|_| "read_failed".to_owned())?;
        resolved.push(guard);
    }

    // The parts are merged in the order the file manager shows, sorted by the
    // picker's display names — never by the raw picked strings, which on
    // Android are `content://` URIs whose numeric ids say nothing about the
    // files they name. `scene2` before `scene10` is the promise; a stable sort
    // keeps the picker's order when two files share a name.
    resolved.sort_by(|a, b| natural_cmp(a.display_name(), b.display_name()));

    let mut parts = Vec::with_capacity(resolved.len());
    let mut raws = Vec::with_capacity(resolved.len());
    for file in &resolved {
        let meta = tokio::fs::metadata(file.path())
            .await
            .map_err(|_| "read_failed")?;
        if meta.len() > MAX_FILE_BYTES {
            return Err("file_too_large".into());
        }
        let bytes = tokio::fs::read(file.path())
            .await
            .map_err(|_| "read_failed")?;
        let raw = String::from_utf8(bytes).map_err(|_| "invalid_utf8")?;
        parts.push(parse(&raw)?);
        raws.push(raw);
    }
    let parsed = merge(&parts)?;
    let body = render_body(&parsed.lines);
    let source_sha256 = sha256_hex(&raws);

    // Same content for the same game is the same script; return what exists so
    // re-importing after an interrupted compaction cannot fork progress. The
    // body answers the hash too: the hash covers concatenated file contents,
    // so two different sets (one file vs several that happen to concatenate
    // into the same bytes) can share one hash while rendering different
    // documents — only both agreeing means "the same script".
    if let Some(row) = sqlx::query(
        "SELECT d.id FROM script_documents d WHERE d.game_id=? AND d.source_sha256=? AND d.body=?",
    )
    .bind(game_id)
    .bind(&source_sha256)
    .bind(&body)
    .fetch_optional(pool)
    .await
    .map_err(|_| "db_error")?
    {
        let id: String = row.get("id");
        return get(pool, &id).await;
    }

    // The per-game cap sits behind the dedupe so re-importing a stored script
    // stays the no-op it promises to be even at the cap; only a genuinely new
    // document counts against it.
    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM script_documents WHERE game_id=?")
        .bind(game_id)
        .fetch_one(pool)
        .await
        .map_err(|_| "db_error")?;
    if count.0 >= MAX_DOCUMENTS_PER_GAME {
        return Err("script_limit".into());
    }

    let id = uuid::Uuid::new_v4().to_string();
    let line_count = i64::try_from(parsed.lines.len()).map_err(|_| "too_many_lines")?;
    // The chunk plan is derived from the body, which never changes after import,
    // so it is computed once here. Re-planning it on every compaction call meant
    // walking the whole script to answer a question about the next eight chunks.
    let plan = plan_chunks(&body, DEFAULT_CHUNK_BYTES);
    if plan.len() > MAX_CHUNKS {
        return Err("script_too_large".into());
    }
    let chunk_count = i64::try_from(plan.len()).map_err(|_| "script_too_large")?;
    // A JSON array of [start, end) line ranges: hundreds of numbers for a whole
    // game, and nothing joins on an individual chunk.
    let chunks_json = serde_json::to_string(&plan).map_err(|_| "db_error")?;
    sqlx::query(
        "INSERT INTO script_documents(id,game_id,engine,source_kind,source_path,source_sha256,line_count,body,chunk_plan_json,chunk_count,created_at,updated_at) \
         VALUES(?,?,?,?,?,?,?,?,?,?,?,?)",
    )
    .bind(&id)
    .bind(game_id)
    .bind(&parsed.engine)
    .bind(parsed.format.as_str())
    .bind(display_source_path(
        &resolved
            .iter()
            .map(ResolvedDocument::display_name)
            .collect::<Vec<_>>(),
    ))
    .bind(&source_sha256)
    .bind(line_count)
    .bind(&body)
    .bind(&chunks_json)
    .bind(chunk_count)
    .bind(now())
    .bind(now())
    .execute(pool)
    .await
    .map_err(|_| "db_error")?;
    get(pool, &id).await
}

/// Human-facing label for where a document came from: the picker's file names,
/// and from the fourth on a count, so a 30-part import does not store an essay.
fn display_source_path(names: &[&str]) -> String {
    if names.len() <= 3 {
        names.join(", ")
    } else {
        format!("{} +{} more", names[..3].join(", "), names.len() - 3)
    }
}

/// List stored scripts, newest first, optionally for one game. Bodies are not
/// returned; only their size.
///
/// # Errors
/// `invalid_game` or `db_error`.
pub async fn list(
    pool: &SqlitePool,
    game_id: Option<&str>,
) -> Result<Vec<ScriptDocumentInfo>, String> {
    if let Some(id) = game_id {
        uuid::Uuid::parse_str(id).map_err(|_| "invalid_game")?;
    }
    let rows = sqlx::query(
        "SELECT d.id,d.game_id,d.engine,d.source_kind,d.source_path,d.source_sha256,d.line_count, \
         LENGTH(d.body) AS byte_size,d.chunk_count AS d_chunks,d.created_at, \
         c.model_id AS c_model, c.chunk_count AS c_chunks, c.done_chunks AS c_done, \
         c.agent_count AS c_agents, c.compactor_version AS c_version, c.digest AS c_digest \
         FROM script_documents d \
         LEFT JOIN script_compaction c ON c.document_id = d.id \
         WHERE (? IS NULL OR d.game_id=?) ORDER BY d.created_at DESC, d.id LIMIT 100",
    )
    .bind(game_id)
    .bind(game_id)
    .fetch_all(pool)
    .await
    .map_err(|_| "db_error")?;
    Ok(rows.iter().map(info_row).collect())
}

/// Fetch one document. The body is not returned; only its size.
///
/// # Errors
/// `script_missing` or `db_error`.
pub async fn get(pool: &SqlitePool, id: &str) -> Result<ScriptDocumentInfo, String> {
    let row = sqlx::query(
        "SELECT d.id,d.game_id,d.engine,d.source_kind,d.source_path,d.source_sha256,d.line_count, \
         LENGTH(d.body) AS byte_size,d.chunk_count AS d_chunks,d.created_at, \
         c.model_id AS c_model, c.chunk_count AS c_chunks, c.done_chunks AS c_done, \
         c.agent_count AS c_agents, c.compactor_version AS c_version, c.digest AS c_digest \
         FROM script_documents d \
         LEFT JOIN script_compaction c ON c.document_id = d.id WHERE d.id=?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(|_| "db_error")?
    .ok_or("script_missing")?;
    Ok(info_row(&row))
}

fn info_row(row: &sqlx::sqlite::SqliteRow) -> ScriptDocumentInfo {
    // Chunk total exists from import, so progress is known before the first
    // compaction call; the compaction row takes over once it exists.
    let imported_chunks = row.get::<Option<i64>, _>("d_chunks").unwrap_or(0);
    let compaction = row.get::<Option<String>, _>("c_model").map(|model_id| {
        let compactor_version = u32::try_from(row.get::<i64, _>("c_version")).unwrap_or(u32::MAX);
        let digest: Option<String> = row.get("c_digest");
        let digest_outdated = digest.as_ref().is_some_and(|_| {
            digest_freshness(compactor_version, COMPACTOR_VERSION) == DigestFreshness::Outdated
        });
        CompactionInfo {
            model_id,
            chunk_count: usize::try_from(row.get::<i64, _>("c_chunks"))
                .unwrap_or(usize::try_from(imported_chunks).unwrap_or(0)),
            done_chunks: usize::try_from(row.get::<i64, _>("c_done")).unwrap_or(0),
            agent_count: usize::try_from(row.get::<i64, _>("c_agents").max(1)).unwrap_or(1),
            compactor_version,
            current_version: COMPACTOR_VERSION,
            digest,
            digest_outdated,
        }
    });
    ScriptDocumentInfo {
        id: row.get("id"),
        game_id: row.get("game_id"),
        engine: row.get("engine"),
        source_kind: row.get("source_kind"),
        source_path: row.get("source_path"),
        source_sha256: row.get("source_sha256"),
        line_count: row.get("line_count"),
        byte_size: row.get("byte_size"),
        chunk_count: imported_chunks,
        created_at: row.get("created_at"),
        compaction,
    }
}

/// Delete a stored script. Its compaction row cascades.
///
/// # Errors
/// `script_missing` or `db_error`.
pub async fn delete(pool: &SqlitePool, id: &str) -> Result<(), String> {
    let result = sqlx::query("DELETE FROM script_documents WHERE id=?")
        .bind(id)
        .execute(pool)
        .await
        .map_err(|_| "db_error")?;
    if result.rows_affected() == 0 {
        return Err("script_missing".into());
    }
    Ok(())
}

/// The compacted digest for a game's newest script, for the `script_digest`
/// chat tool. `None` fields distinguish "no script" from "not compacted yet".
///
/// # Errors
/// `db_error`.
pub async fn digest_for_game(
    pool: &SqlitePool,
    game_id: &str,
) -> Result<Option<DigestState>, String> {
    let row = sqlx::query(
        "SELECT c.chunk_count AS c_chunks, c.done_chunks AS c_done, c.compactor_version AS c_version, \
         c.digest AS c_digest \
         FROM script_documents d LEFT JOIN script_compaction c ON c.document_id = d.id \
         WHERE d.game_id=? ORDER BY d.created_at DESC, d.id LIMIT 1",
    )
    .bind(game_id)
    .fetch_optional(pool)
    .await
    .map_err(|_| "db_error")?;
    let Some(row) = row else {
        return Ok(None);
    };
    let digest: Option<String> = row.get("c_digest");
    let stored_version = row
        .get::<Option<i64>, _>("c_version")
        .and_then(|version| u32::try_from(version).ok())
        .unwrap_or(COMPACTOR_VERSION);
    let outdated = digest.as_ref().and_then(|_| {
        (digest_freshness(stored_version, COMPACTOR_VERSION) == DigestFreshness::Outdated)
            .then_some("built by an older summarizer, so it may be shorter and less specific than the script supports; rebuild it from the script card")
    });
    Ok(Some(DigestState {
        chunk_count: usize::try_from(row.get::<i64, _>("c_chunks")).unwrap_or(0),
        done_chunks: usize::try_from(row.get::<i64, _>("c_done")).unwrap_or(0),
        compactor_version: stored_version,
        digest,
        outdated,
    }))
}

/// What exists for one game: nothing, partial progress, or a finished digest.
#[derive(Debug, Clone)]
pub struct DigestState {
    pub chunk_count: usize,
    pub done_chunks: usize,
    pub compactor_version: u32,
    pub digest: Option<String>,
    /// Set when the finished digest came from an older pipeline.
    pub outdated: Option<&'static str>,
}

/// A stored run as loaded, before any resume decision.
struct StoredRun {
    key: CompactionKey,
    done: usize,
    summaries: Vec<Vec<String>>,
    segment_digests: Vec<Option<String>>,
    digest: Option<String>,
}

/// Row shape of `script_compaction` as [`load_run`] reads it.
type StoredRow = (
    String,
    i64,
    i64,
    i64,
    i64,
    String,
    Option<String>,
    Option<String>,
);

/// Load the stored row for a document. `None` when no run exists.
///
/// # Errors
/// `db_error` (including unreadable JSON: rows are only ever written by
/// `RunState::save`, so a parse failure is corruption, not a shape question).
async fn load_run(pool: &SqlitePool, id: &str) -> Result<Option<StoredRun>, String> {
    let row: Option<StoredRow> = sqlx::query_as(
        "SELECT model_id, chunk_count, agent_count, compactor_version, done_chunks, \
         summaries_json, segment_digests_json, digest \
         FROM script_compaction WHERE document_id=?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(|_| "db_error")?;
    let Some((
        model_id,
        chunk_count,
        agent_count,
        compactor_version,
        done,
        summaries,
        segment_digests,
        digest,
    )) = row
    else {
        return Ok(None);
    };
    let summaries: Vec<Vec<String>> = serde_json::from_str(&summaries).map_err(|_| "db_error")?;
    let segment_digests: Vec<Option<String>> = match segment_digests {
        Some(json) => serde_json::from_str(&json).map_err(|_| "db_error")?,
        None => Vec::new(),
    };
    Ok(Some(StoredRun {
        key: CompactionKey {
            model_id,
            chunk_count: usize::try_from(chunk_count).unwrap_or(usize::MAX),
            agent_count: usize::try_from(agent_count).unwrap_or(usize::MAX),
            compactor_version: u32::try_from(compactor_version).unwrap_or(u32::MAX),
        },
        done: usize::try_from(done).unwrap_or(usize::MAX),
        summaries,
        segment_digests,
        digest,
    }))
}

/// One compaction run's mutable state, shared by the agents through a lock.
#[derive(Debug, Clone)]
struct RunState {
    model_id: String,
    chunk_count: usize,
    agent_count: usize,
    /// Total summarized chunks across all agents (`done_chunks` in the row).
    done: usize,
    /// One summary vec per agent, in agent order.
    summaries: Vec<Vec<String>>,
    /// One digest per agent once its segment is complete.
    segment_digests: Vec<Option<String>>,
    digest: Option<String>,
}

impl RunState {
    fn fresh(model_id: String, chunk_count: usize, agent_count: usize) -> Self {
        Self {
            model_id,
            chunk_count,
            agent_count,
            done: 0,
            summaries: vec![Vec::new(); agent_count],
            segment_digests: vec![None; agent_count],
            digest: None,
        }
    }

    async fn save(&self, pool: &SqlitePool, id: &str) -> Result<(), String> {
        let summaries_json = serde_json::to_string(&self.summaries).map_err(|_| "db_error")?;
        let segments_json = serde_json::to_string(&self.segment_digests).map_err(|_| "db_error")?;
        sqlx::query(
            "INSERT INTO script_compaction(document_id,model_id,chunk_count,agent_count,compactor_version,done_chunks,summaries_json,segment_digests_json,digest,updated_at) \
             VALUES(?,?,?,?,?,?,?,?,?,?) \
             ON CONFLICT(document_id) DO UPDATE SET model_id=excluded.model_id, chunk_count=excluded.chunk_count, \
             agent_count=excluded.agent_count, compactor_version=excluded.compactor_version, \
             done_chunks=excluded.done_chunks, summaries_json=excluded.summaries_json, \
             segment_digests_json=excluded.segment_digests_json, digest=excluded.digest, updated_at=excluded.updated_at",
        )
        .bind(id)
        .bind(&self.model_id)
        .bind(i64::try_from(self.chunk_count).unwrap_or(i64::MAX))
        .bind(i64::try_from(self.agent_count).unwrap_or(i64::MAX))
        .bind(i64::from(COMPACTOR_VERSION))
        .bind(i64::try_from(self.done).unwrap_or(i64::MAX))
        .bind(&summaries_json)
        .bind(&segments_json)
        .bind(&self.digest)
        .bind(now())
        .execute(pool)
        .await
        .map_err(|_| "db_error")?;
        Ok(())
    }
}

/// The last `chars` characters of a summary, for the next call's rolling
/// context.
fn tail_chars(text: &str, chars: usize) -> String {
    let count = text.chars().count();
    text.chars().skip(count.saturating_sub(chars)).collect()
}

/// Cap a reduce input, and when material had to be dropped, say so: a digest
/// that confidently ends before the story does is worse than one that admits
/// the gap.
fn cap_reduce_input(text: String) -> String {
    if text.chars().count() <= MAX_REDUCE_INPUT_CHARS {
        return text;
    }
    let mut capped: String = text.chars().take(MAX_REDUCE_INPUT_CHARS).collect();
    capped.push_str(
        "\n\n[The remaining material was omitted for length. State in the digest that it does not reach the end of the script.]",
    );
    capped
}

/// Everything a compaction task needs besides its own segment: the provider
/// call surface and the shared run state. Exists to keep the task functions'
/// argument lists honest.
struct CallCtx<'a> {
    pool: &'a SqlitePool,
    client: &'a reqwest::Client,
    config: &'a crate::ai::chat::ChatConfig,
    key: Option<&'a crate::secrets::ChatApiKey>,
    id: &'a str,
    state: &'a Arc<Mutex<RunState>>,
    /// VNDB romaji mapping for the document's game, empty when the cache has
    /// none. Prepended to every reduce input so the digest writes names the
    /// way the user knows them (see [`vn_core::script::romaji_block`]).
    romaji: String,
}

impl CallCtx<'_> {
    /// Run a pure function over the shared state under the lock.
    async fn with_state<T>(&self, inspect: impl FnOnce(&RunState) -> T) -> T {
        let guard = self.state.lock().await;
        inspect(&guard)
    }
}

/// VNDB's cached character rows carry both spellings of every name (`original`
/// in the game's script, `name` romaji); the mapping block lets the digest
/// write the spelling the user knows. A game with no cache row, a cache
/// predating the characters field, or a cast with no original names simply
/// gets an empty block.
async fn character_romaji_block(pool: &SqlitePool, game_id: &str) -> String {
    let Ok(row) = sqlx::query(
        "SELECT d.characters_json AS characters_json \
         FROM vndb_details d JOIN games g ON g.vndb_id = d.vn_id \
         WHERE g.id = ?",
    )
    .bind(game_id)
    .fetch_optional(pool)
    .await
    else {
        return String::new();
    };
    let Some(json) = row.and_then(|row| row.try_get::<Option<String>, _>("characters_json").ok())
    else {
        return String::new();
    };
    let Ok(characters) =
        serde_json::from_str::<Vec<crate::vndb::dto::Character>>(json.as_deref().unwrap_or("[]"))
    else {
        return String::new();
    };
    let pairs: Vec<(&str, &str)> = characters
        .iter()
        .filter_map(|character| match (&character.original, &character.name) {
            (Some(original), Some(name)) => Some((original.as_str(), name.as_str())),
            _ => None,
        })
        .collect();
    vn_core::script::romaji_block(&pairs)
}

/// One agent's slice of the work.
struct AgentJob<'a> {
    index: usize,
    start: usize,
    end: usize,
    body: &'a str,
    chunks: &'a [(usize, usize)],
    chunk_count: usize,
    step: usize,
}

/// Whether every agent's outlines cover its whole segment.
fn outlines_complete(state: &RunState, plan: &[(usize, usize)]) -> bool {
    state
        .summaries
        .iter()
        .zip(plan)
        .all(|(agent_summaries, (start, end))| agent_summaries.len() == end - start)
}

/// The final reduce's input, or `None` while it cannot run yet. Pure so the
/// gating rules (outlines complete, digest absent, single- vs multi-agent
/// shape) are testable without a provider.
fn final_reduce_input(state: &RunState, plan: &[(usize, usize)]) -> Option<String> {
    if !outlines_complete(state, plan) || state.digest.is_some() {
        return None;
    }
    if state.agent_count == 1 {
        let joined = state.summaries[0]
            .iter()
            .enumerate()
            .map(|(index, summary)| format!("Chunk {}:\n{summary}", index + 1))
            .collect::<Vec<_>>()
            .join("\n\n");
        Some(cap_reduce_input(format!(
            "Chunk outlines, in order:\n\n{joined}"
        )))
    } else if state.segment_digests.iter().all(Option::is_some) {
        let joined = state
            .segment_digests
            .iter()
            .enumerate()
            .filter_map(|(index, digest)| {
                digest
                    .as_ref()
                    .map(|digest| format!("Segment {}:\n{digest}", index + 1))
            })
            .collect::<Vec<_>>()
            .join("\n\n");
        Some(cap_reduce_input(format!(
            "Segment digests, in story order:\n\n{joined}"
        )))
    } else {
        None
    }
}

/// Load stored progress and decide resume vs restart. Restarting deletes the
/// row so the fresh key is written together with the first saved chunk. The
/// structural check backs the key check: the summaries must be laid out
/// exactly as `plan` expects, and a segment digest may exist only for a
/// finished segment.
async fn initial_state(
    pool: &SqlitePool,
    id: &str,
    planned: &CompactionKey,
    plan: &[(usize, usize)],
) -> Result<RunState, String> {
    let agent_count = plan.len();
    let Some(stored) = load_run(pool, id).await? else {
        return Ok(RunState::fresh(
            planned.model_id.clone(),
            planned.chunk_count,
            agent_count,
        ));
    };
    let structural = stored.summaries.len() == agent_count
        && stored.segment_digests.len() <= agent_count
        && (0..agent_count).all(|k| {
            let segment_len = plan[k].1 - plan[k].0;
            let done = stored.summaries[k].len();
            let digest_ok = stored
                .segment_digests
                .get(k)
                .is_none_or(|digest| digest.is_none() || done == segment_len);
            done <= segment_len && digest_ok
        });
    let resumable = structural
        && matches!(
            resume_decision(
                &stored.key,
                planned,
                stored.done,
                stored.summaries.iter().map(Vec::len).sum(),
            ),
            Resume::Continue(_)
        );
    if !resumable {
        sqlx::query("DELETE FROM script_compaction WHERE document_id=?")
            .bind(id)
            .execute(pool)
            .await
            .map_err(|_| "db_error")?;
        return Ok(RunState::fresh(
            planned.model_id.clone(),
            planned.chunk_count,
            agent_count,
        ));
    }
    let mut segment_digests = vec![None; agent_count];
    for (index, digest) in stored.segment_digests.into_iter().enumerate() {
        if index < agent_count {
            segment_digests[index] = digest;
        }
    }
    Ok(RunState {
        model_id: planned.model_id.clone(),
        chunk_count: planned.chunk_count,
        agent_count,
        done: stored.done,
        summaries: stored.summaries,
        segment_digests,
        digest: stored.digest,
    })
}

/// Documents with a compaction call in flight, keyed by document id.
///
/// The UI allows one summarize loop per card instance, but a loop survives
/// unmount until its batch finishes — navigating away and back can start a
/// second call while the first is still in flight. Two concurrent calls each
/// evolve their own copy of the progress row and clobber each other's saves,
/// after which the resume guard discards the whole run and every provider call
/// spent on those chunks is wasted. The lease makes the second call report
/// `compaction_busy` instead of racing.
static ACTIVE_COMPACTIONS: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashSet<String>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashSet::new()));

/// Releases the document lease when dropped, however `compact` exits.
#[derive(Debug)]
struct CompactLease(String);

impl Drop for CompactLease {
    fn drop(&mut self) {
        if let Ok(mut active) = ACTIVE_COMPACTIONS.lock() {
            active.remove(&self.0);
        }
    }
}

/// Take the document's compaction lease, or fail with `compaction_busy` when
/// another call already holds it.
///
/// # Errors
/// `compaction_busy` when the document is already compacting.
fn try_acquire_lease(id: &str) -> Result<CompactLease, String> {
    let mut active = ACTIVE_COMPACTIONS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if !active.insert(id.to_owned()) {
        return Err("compaction_busy".into());
    }
    Ok(CompactLease(id.to_owned()))
}

/// What one compaction call may do. Grouped rather than passed as loose
/// arguments: `rebuild` is destructive, and a bare `true` at a call site is the
/// kind of flag that gets passed to the wrong function.
#[derive(Debug, Clone, Copy)]
pub struct RunOptions {
    /// Chunks summarized per agent in this call.
    pub step: u32,
    /// Summarizer agents (parallel provider calls).
    pub agents: usize,
    /// Discard stored progress first, including a finished digest.
    pub rebuild: bool,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            step: MAX_CHUNKS_PER_CALL,
            agents: DEFAULT_AGENTS,
            rebuild: false,
        }
    }
}

/// Advance compaction of one document by up to `step` chunks **per agent**,
/// then reduce when every chunk has a summary. Safe to call repeatedly;
/// progress persists after each chunk so an interrupted run resumes where it
/// stopped. One call per document at a time: a second concurrent call fails
/// with `compaction_busy` rather than racing the same progress row.
///
/// `rebuild` discards this document's stored run first, including a finished
/// digest. That is how a digest produced by an older pipeline is replaced: its
/// summaries must not be reused, because the prompts that produced them are
/// gone and their output reads differently from the current map step.
///
/// The story is split into `agents` contiguous segments. The agents summarize
/// their segments concurrently, each carrying the tail of its previous outline
/// as rolling context; when every segment is done, each agent compresses its
/// own outlines into a segment digest and one final call merges the segment
/// digests into the story digest. With one agent the middle step is skipped
/// and the outlines reduce directly.
///
/// A run belongs to one (model, chunk plan, agent layout, pipeline version)
/// key: any of those changing resets progress rather than mixing summaries
/// from two incompatible runs.
///
/// # Errors
/// `compaction_busy`, `script_missing`, `script_too_large`, `chat_key_missing`,
/// plus provider errors from [`crate::ai::chat::complete`], and `db_error`.
pub async fn compact(
    pool: &SqlitePool,
    client: &reqwest::Client,
    config: &crate::ai::chat::ChatConfig,
    key: Option<&crate::secrets::ChatApiKey>,
    id: &str,
    run: RunOptions,
) -> Result<CompactionInfo, String> {
    let step = run.step.clamp(1, MAX_CHUNKS_PER_CALL) as usize;
    let agents = run.agents.clamp(1, MAX_AGENTS);
    // Held for the whole call: a second call on this document waits for the
    // caller to repeat, by which time the stored progress has advanced.
    let _lease = try_acquire_lease(id)?;
    if run.rebuild {
        sqlx::query("DELETE FROM script_compaction WHERE document_id=?")
            .bind(id)
            .execute(pool)
            .await
            .map_err(|_| "db_error")?;
    }
    let (body, chunks, romaji) = load_document(pool, id).await?;
    let chunk_count = chunks.len();
    if chunk_count > MAX_CHUNKS {
        return Err("script_too_large".into());
    }
    let plan = plan_agents(chunk_count, agents);
    let agent_count = plan.len();
    let planned = CompactionKey {
        model_id: config.model.clone(),
        chunk_count,
        agent_count,
        compactor_version: COMPACTOR_VERSION,
    };
    let state = Arc::new(Mutex::new(initial_state(pool, id, &planned, &plan).await?));
    let call = CallCtx {
        pool,
        client,
        config,
        key,
        id,
        state: &state,
        romaji,
    };

    summarize_segments(&call, &body, &chunks, &plan, step).await?;

    // Segment digests: only for multi-agent runs, and only once every
    // outline exists. Each is stored as it is produced, so a failure in the
    // middle does not redo the earlier ones.
    if agent_count > 1 && call.with_state(|s| outlines_complete(s, &plan)).await {
        let mut tasks = Vec::with_capacity(agent_count);
        for index in 0..agent_count {
            tasks.push(make_segment_digest(&call, index));
        }
        let results = futures_util::future::join_all(tasks).await;
        if let Some(err) = results.into_iter().find_map(Result::err) {
            return Err(err);
        }
    }

    // Final reduce. A single agent reduces its outlines directly; several
    // agents merge their segment digests.
    if let Some(input) = call
        .with_state(|state| final_reduce_input(state, &plan))
        .await
    {
        let merged = crate::ai::chat::complete_with_budget(
            client,
            config,
            key,
            REDUCE_SYSTEM_PROMPT,
            &format!("{}{input}", call.romaji),
            8192,
        )
        .await
        .map_err(|err| err.to_string())?;
        let digest = merged.trim().chars().take(MAX_DIGEST_CHARS).collect();
        let mut guard = state.lock().await;
        guard.digest = Some(digest);
        guard.save(pool, id).await?;
    }

    let guard = state.lock().await;
    Ok(CompactionInfo {
        model_id: guard.model_id.clone(),
        chunk_count: guard.chunk_count,
        done_chunks: guard.done,
        agent_count: guard.agent_count,
        compactor_version: COMPACTOR_VERSION,
        current_version: COMPACTOR_VERSION,
        digest: guard.digest.clone(),
        digest_outdated: false,
    })
}

/// Run every agent over its own segment for this call. A failure is returned
/// once every agent has stopped: chunks finished before it are already
/// persisted, so the caller resumes instead of losing that work.
async fn summarize_segments(
    call: &CallCtx<'_>,
    body: &str,
    chunks: &[(usize, usize)],
    plan: &[(usize, usize)],
    step: usize,
) -> Result<(), String> {
    let chunk_count = chunks.len();
    let tasks = plan
        .iter()
        .enumerate()
        .map(|(index, &(start, end))| {
            run_agent(
                call,
                AgentJob {
                    index,
                    start,
                    end,
                    body,
                    chunks,
                    chunk_count,
                    step,
                },
            )
        })
        .collect::<Vec<_>>();
    futures_util::future::join_all(tasks)
        .await
        .into_iter()
        .find_map(Result::err)
        .map_or(Ok(()), Err)
}

/// Load the body, the chunk plan and the name mapping for one document.
///
/// The plan is stored at import, so a repeat call costs one row read instead of
/// re-splitting the script. Documents imported before the plan existed compute it
/// once and the plan is written back, which also keeps the stored chunk count in
/// step with what this run actually uses.
async fn load_document(
    pool: &SqlitePool,
    id: &str,
) -> Result<(String, Vec<(usize, usize)>, String), String> {
    let doc = sqlx::query("SELECT body, game_id, chunk_plan_json FROM script_documents WHERE id=?")
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|_| "db_error")?
        .ok_or("script_missing")?;
    let body: String = doc.get("body");
    let game_id: String = doc.get("game_id");
    let romaji = character_romaji_block(pool, &game_id).await;
    let stored_plan: Option<String> = doc.get("chunk_plan_json");
    let chunks = if let Some(plan) = stored_plan
        .as_deref()
        .and_then(|json| serde_json::from_str::<Vec<(usize, usize)>>(json).ok())
    {
        plan
    } else {
        let plan = plan_chunks(&body, DEFAULT_CHUNK_BYTES);
        let json = serde_json::to_string(&plan).map_err(|_| "db_error")?;
        sqlx::query("UPDATE script_documents SET chunk_plan_json=?, chunk_count=? WHERE id=?")
            .bind(&json)
            .bind(i64::try_from(plan.len()).unwrap_or(i64::MAX))
            .bind(id)
            .execute(pool)
            .await
            .map_err(|_| "db_error")?;
        plan
    };
    Ok((body, chunks, romaji))
}

/// One summarizer agent's loop over its segment: summarize up to `step` chunks
/// this call, each with the tail of the previous outline as rolling context,
/// saving after every chunk.
async fn run_agent(call: &CallCtx<'_>, job: AgentJob<'_>) -> Result<(), String> {
    let mut advanced = 0_usize;
    loop {
        // Pick the next chunk and its rolling context under the lock, then
        // release it for the provider call so the other agents keep moving.
        let (chunk, context) = {
            let guard = call.state.lock().await;
            let done_in_segment = guard.summaries[job.index].len();
            if done_in_segment >= job.end - job.start || advanced >= job.step {
                return Ok(());
            }
            let context = guard.summaries[job.index]
                .last()
                .map(|summary| tail_chars(summary, ROLLING_CONTEXT_CHARS))
                .unwrap_or_default();
            (job.start + done_in_segment, context)
        };
        let mut user = format!("Chunk {} of {}.\n\n", chunk + 1, job.chunk_count);
        if !context.is_empty() {
            user.push_str("Context, the previous chunk's outline, for continuity only:\n");
            user.push_str(&context);
            user.push_str("\n\n");
        }
        user.push_str(&vn_core::script::chunk_text(job.body, job.chunks[chunk]));
        let summary =
            crate::ai::chat::complete(call.client, call.config, call.key, MAP_SYSTEM_PROMPT, &user)
                .await
                .map_err(|err| err.to_string())?;
        let summary = summary
            .trim()
            .chars()
            .take(MAX_SUMMARY_CHARS)
            .collect::<String>();
        let mut guard = call.state.lock().await;
        guard.summaries[job.index].push(summary);
        guard.done += 1;
        advanced += 1;
        guard.save(call.pool, call.id).await?;
    }
}

/// Compress one agent's finished outlines into that segment's digest, skipping
/// agents that already have one.
async fn make_segment_digest(call: &CallCtx<'_>, index: usize) -> Result<(), String> {
    let outlines = {
        let guard = call.state.lock().await;
        if guard.segment_digests[index].is_some() {
            return Ok(());
        }
        guard.summaries[index].clone()
    };
    let joined = outlines
        .iter()
        .map(|outline| format!("- {outline}"))
        .collect::<Vec<_>>()
        .join("\n");
    let input = cap_reduce_input(format!(
        "Outlines of segment {index}, in story order:\n\n{joined}"
    ));
    let digest = crate::ai::chat::complete_with_budget(
        call.client,
        call.config,
        call.key,
        SEGMENT_REDUCE_SYSTEM_PROMPT,
        &format!("{}{input}", call.romaji),
        8192,
    )
    .await
    .map_err(|err| err.to_string())?;
    let digest = digest.trim().chars().take(MAX_DIGEST_CHARS).collect();
    let mut guard = call.state.lock().await;
    guard.segment_digests[index] = Some(digest);
    guard.save(call.pool, call.id).await?;
    Ok(())
}

/// SHA-256 over the concatenated file contents in merge order, hex encoded.
/// Contents, not paths: the same files re-imported from another folder are the
/// same script, and a renamed file is still the same script.
fn sha256_hex(parts: &[String]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part.as_bytes());
    }
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_hex_matches_the_known_vector() {
        // abc → ba7816bf... (FIPS 180-2 example). If dedupe ever hashes wrong,
        // every re-import stores a duplicate; pin the digest.
        assert_eq!(
            sha256_hex(&["abc".to_owned()]),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn the_source_path_label_ellipsises_long_file_sets() {
        // The names arrive already split (the resolver's display names), so the
        // label is a join with a count from the fourth on.
        assert_eq!(display_source_path(&["a.txt"]), "a.txt");
        assert_eq!(display_source_path(&["a.txt", "b.txt"]), "a.txt, b.txt");
        let names = [
            "part0.txt",
            "part1.txt",
            "part2.txt",
            "part3.txt",
            "part4.txt",
        ];
        assert_eq!(
            display_source_path(&names),
            "part0.txt, part1.txt, part2.txt +2 more"
        );
    }

    #[test]
    fn agent_settings_clamp_into_the_valid_range() {
        assert_eq!(AgentSettings { agents: 0 }.normalized().agents, 1);
        assert_eq!(AgentSettings { agents: 99 }.normalized().agents, MAX_AGENTS);
        assert_eq!(AgentSettings { agents: 4 }.normalized().agents, 4);
        assert_eq!(AgentSettings::default().agents, DEFAULT_AGENTS);
    }

    #[tokio::test]
    async fn agent_settings_error_when_the_store_is_missing() {
        // No migrations on this pool: the query itself must fail rather than
        // answer a default. The agent count is part of the compaction resume
        // key, so a silent default would reset an in-flight run's stored
        // progress instead of surfacing the problem.
        let pool = sqlx::SqlitePool::connect("sqlite::memory:")
            .await
            .expect("pool");
        assert_eq!(load_agent_settings(&pool).await, Err("db_error".to_owned()));
    }

    #[tokio::test]
    async fn agent_settings_error_when_the_blob_is_unreadable() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:")
            .await
            .expect("pool");
        sqlx::query("CREATE TABLE sync_state (key TEXT PRIMARY KEY NOT NULL, value TEXT NOT NULL)")
            .execute(&pool)
            .await
            .expect("table");
        sqlx::query(
            "INSERT INTO sync_state (key, value) VALUES ('script.compaction', '{not json')",
        )
        .execute(&pool)
        .await
        .expect("seed");
        // A corrupt blob errors too: compaction proceeding with the default
        // count would wipe stored progress. Re-saving the setting in the UI
        // rewrites the blob and clears the error.
        assert_eq!(
            load_agent_settings(&pool).await,
            Err("agent_settings_invalid".to_owned())
        );
    }

    #[test]
    fn the_compaction_lease_is_exclusive_until_dropped() {
        let held = try_acquire_lease("lease-test-doc-1").expect("first acquire");
        assert_eq!(
            try_acquire_lease("lease-test-doc-1").unwrap_err(),
            "compaction_busy",
            "a second call on the same document waits"
        );
        // A different document is unaffected.
        let other = try_acquire_lease("lease-test-doc-2").expect("second document");
        drop(other);
        drop(held);
        assert!(
            try_acquire_lease("lease-test-doc-1").is_ok(),
            "the lease is released on drop"
        );
    }

    #[test]
    fn the_final_reduce_waits_for_its_inputs() {
        let single = RunState {
            model_id: "qwen".into(),
            chunk_count: 2,
            agent_count: 1,
            done: 2,
            summaries: vec![vec!["one".into(), "two".into()]],
            segment_digests: vec![None],
            digest: None,
        };
        let input = final_reduce_input(&single, &[(0, 2)]).expect("ready");
        assert!(input.contains("Chunk 1:\none"));
        assert!(input.contains("Chunk 2:\ntwo"));

        // A finished digest must not be reduced again.
        let finished = RunState {
            digest: Some("done".into()),
            ..single.clone()
        };
        assert!(final_reduce_input(&finished, &[(0, 2)]).is_none());

        // One chunk missing: not ready.
        let partial = RunState {
            done: 1,
            summaries: vec![vec!["one".into()]],
            ..single
        };
        assert!(final_reduce_input(&partial, &[(0, 2)]).is_none());

        // Multi-agent: only when every segment digest exists, and the input
        // carries the segments in story order.
        let multi = RunState {
            model_id: "qwen".into(),
            chunk_count: 4,
            agent_count: 2,
            done: 4,
            summaries: vec![vec!["a".into(), "b".into()], vec!["c".into(), "d".into()]],
            segment_digests: vec![None, None],
            digest: None,
        };
        assert!(final_reduce_input(&multi, &[(0, 2), (2, 4)]).is_none());
        let ready = RunState {
            segment_digests: vec![Some("first half".into()), Some("second half".into())],
            ..multi
        };
        let input = final_reduce_input(&ready, &[(0, 2), (2, 4)]).expect("ready");
        assert!(input.contains("Segment 1:\nfirst half"));
        assert!(input.contains("Segment 2:\nsecond half"));
    }

    #[tokio::test]
    async fn import_rejects_an_unknown_game() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:")
            .await
            .expect("pool");
        // No migrations on this pool: the query itself must fail cleanly, and
        // the uuid guard runs first anyway.
        let err = import(&pool, "not-a-uuid", &["whatever.txt".to_owned()])
            .await
            .unwrap_err();
        assert_eq!(err, "invalid_game");
    }

    #[tokio::test]
    async fn import_rejects_an_empty_file_set() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:")
            .await
            .expect("pool");
        // The guard runs before any DB or file access, so no migrations needed.
        let err = import(&pool, "01a151db-3333-7000-8000-000000000003", &[])
            .await
            .unwrap_err();
        assert_eq!(err, "no_files");
    }
}

#[cfg(test)]
mod plan_tests {
    use super::*;

    /// The chunk plan is derived from a body that never changes after import, so
    /// it is stored and reused. Compaction used to re-plan the whole script on
    /// every call, which re-walked up to 300k lines to find the next eight chunks.
    #[tokio::test]
    async fn an_imported_document_keeps_its_chunk_plan() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:")
            .await
            .expect("pool");
        sqlx::migrate!("src/db/migrations")
            .run(&pool)
            .await
            .expect("migrate");
        let game = "01a151db-6666-7000-8000-000000000001";
        sqlx::query(
            "INSERT INTO games (id, title, status, updated_at, device_id) \
             VALUES (?, 'Saku', 'playing', 1, 'dev')",
        )
        .bind(game)
        .execute(&pool)
        .await
        .expect("insert game");

        // Long enough for more than one chunk at the 48 KB budget.
        let line = "雪々: ゆきゆきは、そう思う……\n";
        let body = line.repeat(12_000);
        let path = std::env::temp_dir().join(format!("vn-plan-{}.txt", std::process::id()));
        std::fs::write(&path, &body).expect("write script");
        let stored = import(&pool, game, &[path.to_string_lossy().into_owned()])
            .await
            .expect("import");
        std::fs::remove_file(&path).ok();

        let (plan_json, chunk_count): (Option<String>, Option<i64>) =
            sqlx::query_as("SELECT chunk_plan_json, chunk_count FROM script_documents WHERE id=?")
                .bind(&stored.id)
                .fetch_one(&pool)
                .await
                .expect("read plan");
        let plan: Vec<(usize, usize)> =
            serde_json::from_str(plan_json.as_deref().expect("stored plan")).expect("decode");
        assert_eq!(plan.len(), plan_chunks(&body, DEFAULT_CHUNK_BYTES).len());
        assert_eq!(
            chunk_count.and_then(|n| usize::try_from(n).ok()),
            Some(plan.len()),
            "the count is stored beside the plan"
        );
        // And the document info reports it before any compaction row exists.
        assert_eq!(usize::try_from(stored.chunk_count).unwrap_or(0), plan.len());
        assert!(stored.compaction.is_none());
    }
}
