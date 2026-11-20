-- Custom reading goals (automatic playtime / completed VNs, or manual targets).
CREATE TABLE IF NOT EXISTS reading_goals (
    id              TEXT PRIMARY KEY NOT NULL,
    title           TEXT NOT NULL,
    kind            TEXT NOT NULL,
    period          TEXT NOT NULL,
    target_value    INTEGER NOT NULL,
    manual_progress INTEGER NOT NULL DEFAULT 0,
    unit            TEXT,
    pinned          INTEGER NOT NULL DEFAULT 0,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    device_id       TEXT NOT NULL,
    deleted         INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_reading_goals_order
    ON reading_goals(pinned DESC, updated_at DESC, id DESC)
    WHERE deleted = 0;
