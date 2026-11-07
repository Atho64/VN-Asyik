-- Local-only companion contexts, deliberately excluded from library sync/export.
-- Retain game_id on deletion, like chat_threads: deleting this owner row would
-- otherwise make its transcript look like an ordinary, editable chat. Reads
-- join the live game; missing/deleted games cannot resume or send.
CREATE TABLE companion_sessions (
    id TEXT PRIMARY KEY NOT NULL,
    thread_id TEXT NOT NULL UNIQUE REFERENCES chat_threads(id) ON DELETE CASCADE,
    game_id TEXT NOT NULL,
    playthrough_id TEXT NOT NULL,
    options_json TEXT NOT NULL,
    source_url TEXT,
    state TEXT NOT NULL CHECK(state IN ('active', 'paused', 'ended')),
    reason TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    ended_at INTEGER
);
CREATE INDEX idx_companion_sessions_recent ON companion_sessions(updated_at DESC);
