-- Extracted VN scripts and their compaction state. Local-only like chat
-- history: source paths are device-specific and bodies are large, so they are
-- absent from sync and library export payloads. Deleting a game removes its
-- scripts; the chat script_digest tool revalidates the game before reading.
CREATE TABLE script_documents (
    id TEXT PRIMARY KEY NOT NULL,
    game_id TEXT NOT NULL REFERENCES games(id) ON DELETE CASCADE,
    engine TEXT NOT NULL,
    source_kind TEXT NOT NULL CHECK(source_kind IN ('plain_text','fvp_dialogue','asyik_script')),
    source_path TEXT NOT NULL,
    source_sha256 TEXT NOT NULL,
    line_count INTEGER NOT NULL,
    body TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE INDEX idx_script_documents_game ON script_documents(game_id);
-- Map-reduce compaction progress. Summaries accumulate one chunk at a time so
-- an interrupted run resumes; the digest appears only when every chunk is
-- summarized. Rows are keyed to the model that produced the summaries: a model
-- change resets progress rather than serving stale summaries (same rule as the
-- removed vector store).
CREATE TABLE script_compaction (
    document_id TEXT PRIMARY KEY NOT NULL REFERENCES script_documents(id) ON DELETE CASCADE,
    model_id TEXT NOT NULL,
    chunk_count INTEGER NOT NULL,
    done_chunks INTEGER NOT NULL DEFAULT 0,
    summaries_json TEXT NOT NULL DEFAULT '[]',
    digest TEXT,
    updated_at INTEGER NOT NULL
);
