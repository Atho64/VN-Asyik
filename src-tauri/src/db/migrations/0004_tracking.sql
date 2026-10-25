-- 0004_tracking.sql
--
-- Playtime tracking: which executable belongs to which game, and which engine
-- adapter (if any) can attribute chapters for it.
--
-- Migration 0001-0003 are applied and must not be edited.

-- One row per executable a game is launched from. A visual novel often ships
-- several (a launcher, the game itself, a trial build), so this is a list.
CREATE TABLE game_executables (
    id          TEXT    PRIMARY KEY NOT NULL,
    game_id     TEXT    NOT NULL REFERENCES games(id) ON DELETE CASCADE,
    exe_name    TEXT    NOT NULL,
    exe_path    TEXT,
    added_at    INTEGER NOT NULL,
    -- Per-executable override of the tracking switch.
    enabled     INTEGER NOT NULL DEFAULT 1,
    updated_at  INTEGER NOT NULL,
    device_id   TEXT    NOT NULL,
    deleted     INTEGER NOT NULL DEFAULT 0,
    CHECK (enabled IN (0,1)),
    CHECK (deleted IN (0,1))
);

CREATE INDEX idx_exec_game ON game_executables(game_id) WHERE deleted = 0;
CREATE INDEX idx_exec_name ON game_executables(exe_name) WHERE deleted = 0;

-- Folders the user asked to scan for executables.
CREATE TABLE scan_folders (
    path        TEXT PRIMARY KEY NOT NULL,
    added_at    INTEGER NOT NULL,
    last_scan_at INTEGER
);

-- Tracker runtime state, so a crash can be recovered on next start.
-- One row: the open run, if any. Cleared when a run closes cleanly.
CREATE TABLE tracker_state (
    id              INTEGER PRIMARY KEY CHECK (id = 1),
    game_id         TEXT    NOT NULL,
    session_id      TEXT    NOT NULL,
    chapter_id      TEXT,
    started_at      INTEGER NOT NULL,
    active_ms       INTEGER NOT NULL,
    afk_ms          INTEGER NOT NULL,
    exe_path        TEXT    NOT NULL,
    checkpoint_at   INTEGER NOT NULL
);

-- Unsynced checkpoint rows awaiting promotion to real sessions on recovery.
-- A crash can leave a checkpoint whose run never closed; it is completed on the
-- next start rather than discarded.
CREATE TABLE pending_sessions (
    id           TEXT PRIMARY KEY NOT NULL,
    game_id      TEXT NOT NULL,
    chapter_id   TEXT,
    started_at   INTEGER NOT NULL,
    ended_at     INTEGER NOT NULL,
    counted_ms   INTEGER NOT NULL,
    afk_ms       INTEGER NOT NULL,
    source       TEXT NOT NULL,
    device_id    TEXT NOT NULL,
    written_at   INTEGER NOT NULL,
    CHECK (counted_ms >= 0),
    CHECK (afk_ms >= 0)
);

CREATE INDEX idx_pending_game ON pending_sessions(game_id);
