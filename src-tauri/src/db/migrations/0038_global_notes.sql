-- Global and cross-game reading notes for the Timeline Notes tab.
-- `game_id` is nullable so a note can belong to the library as a whole or link
-- optionally to one visual novel (`ON DELETE SET NULL` keeps the note if the game row is hard-deleted).
CREATE TABLE global_notes (
    id         TEXT PRIMARY KEY NOT NULL,
    game_id    TEXT REFERENCES games(id) ON DELETE SET NULL,
    title      TEXT NOT NULL DEFAULT '',
    body       TEXT NOT NULL DEFAULT '',
    pinned     INTEGER NOT NULL DEFAULT 0 CHECK (pinned IN (0, 1)),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    device_id  TEXT NOT NULL,
    deleted    INTEGER NOT NULL DEFAULT 0 CHECK (deleted IN (0, 1))
);
CREATE INDEX idx_global_notes_order
    ON global_notes(pinned DESC, updated_at DESC, id DESC)
    WHERE deleted = 0;
