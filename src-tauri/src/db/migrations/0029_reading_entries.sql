-- Journal checkpoints are independent of append-only session accounting.
-- Names are snapshots, not route/chapter identifiers: manual positions work on
-- every platform and historical notes survive later route renames and merges.
CREATE TABLE reading_entries (
    id TEXT PRIMARY KEY NOT NULL,
    game_id TEXT NOT NULL REFERENCES games(id) ON DELETE CASCADE,
    session_id TEXT REFERENCES play_sessions(id) ON DELETE SET NULL,
    created_at INTEGER NOT NULL,
    played_at INTEGER,
    note TEXT NOT NULL DEFAULT '',
    save_slot TEXT NOT NULL DEFAULT '',
    route_name TEXT NOT NULL DEFAULT '',
    chapter_name TEXT NOT NULL DEFAULT '',
    updated_at INTEGER NOT NULL,
    device_id TEXT NOT NULL,
    deleted INTEGER NOT NULL DEFAULT 0 CHECK (deleted IN (0, 1))
);
CREATE INDEX idx_reading_entries_game ON reading_entries(game_id, created_at DESC, id DESC);
