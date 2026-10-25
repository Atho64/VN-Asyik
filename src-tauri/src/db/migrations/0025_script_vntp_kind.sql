-- no-transaction
-- 0025_script_vntp_kind.sql
--
-- The `fvp_dialogue` import format is renamed `json_vntp`. The name/message
-- JSON array is a generic shape any extractor can emit (the owner's FVP tool's
-- `dialogue.json` is one producer), so the stored kind should name the shape,
-- not one tool.
--
-- --- why `-- no-transaction` (same reason as 0011) -------------------------
--
-- SQLite cannot alter a CHECK constraint, so the table is rebuilt. That requires
-- `PRAGMA foreign_keys = OFF`, which is a no-op inside a transaction, so this
-- migration cannot run inside one. `script_compaction` references
-- `script_documents(id)`; with enforcement off for the swap and re-enabled
-- after, the `PRAGMA foreign_key_check` below proves no reference was lost.

PRAGMA foreign_keys = OFF;

BEGIN;

CREATE TABLE script_documents_new (
    id TEXT PRIMARY KEY NOT NULL,
    game_id TEXT NOT NULL REFERENCES games(id) ON DELETE CASCADE,
    engine TEXT NOT NULL,
    source_kind TEXT NOT NULL CHECK(source_kind IN ('plain_text','json_vntp','asyik_script')),
    source_path TEXT NOT NULL,
    source_sha256 TEXT NOT NULL,
    line_count INTEGER NOT NULL,
    body TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

INSERT INTO script_documents_new (
    id, game_id, engine, source_kind, source_path, source_sha256,
    line_count, body, created_at, updated_at
)
SELECT
    id, game_id, engine,
    CASE source_kind WHEN 'fvp_dialogue' THEN 'json_vntp' ELSE source_kind END,
    source_path, source_sha256, line_count, body, created_at, updated_at
FROM script_documents;

DROP TABLE script_documents;

ALTER TABLE script_documents_new RENAME TO script_documents;

CREATE INDEX idx_script_documents_game ON script_documents(game_id);

COMMIT;

PRAGMA foreign_key_check;
PRAGMA foreign_keys = ON;
