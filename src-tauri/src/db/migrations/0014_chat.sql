-- Local chat history. Deliberately absent from sync and library export payloads.
-- Game ids are retained after library deletion so a scoped chat never silently
-- becomes an entire-library chat. Tools revalidate that the game is still live.
CREATE TABLE chat_threads (
    id TEXT PRIMARY KEY NOT NULL,
    title TEXT NOT NULL,
    options_json TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE TABLE chat_messages (
    id TEXT PRIMARY KEY NOT NULL,
    thread_id TEXT NOT NULL REFERENCES chat_threads(id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    role TEXT NOT NULL CHECK(role IN ('user','assistant')),
    content TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('complete','running','cancelled','error')),
    error TEXT,
    tools_json TEXT NOT NULL DEFAULT '[]',
    sources_json TEXT NOT NULL DEFAULT '[]',
    tokens INTEGER,
    created_at INTEGER NOT NULL,
    UNIQUE(thread_id, position)
);
CREATE INDEX idx_chat_threads_recent ON chat_threads(updated_at DESC);
