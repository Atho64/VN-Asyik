-- 0003_vndb_sync.sql
--
-- Sync bookkeeping for the VNDB list: what was pulled, what is queued to push,
-- and what changed. The token is never stored here; it lives in the OS
-- credential store (see src/secrets.rs).
--
-- Migration 0001 and 0002 are applied and must not be edited.

-- The connected account. One row; absence means "not connected".
CREATE TABLE vndb_account (
    id            INTEGER PRIMARY KEY CHECK (id = 1),
    user_id       TEXT    NOT NULL,
    username      TEXT    NOT NULL,
    permissions   TEXT    NOT NULL DEFAULT '[]',
    connected_at  INTEGER NOT NULL,
    last_pull_at  INTEGER,
    last_push_at  INTEGER
);

-- Per-VN sync state: what VNDB last told us, so conflicts can be detected
-- without re-deriving it, and so an unchanged remote never re-triggers work.
CREATE TABLE vndb_sync_state (
    vn_id          TEXT PRIMARY KEY NOT NULL,
    game_id        TEXT,
    remote_lastmod INTEGER NOT NULL DEFAULT 0,
    remote_vote    INTEGER,
    remote_notes   TEXT,
    remote_started TEXT,
    remote_finished TEXT,
    remote_labels  TEXT NOT NULL DEFAULT '[]',
    pulled_at      INTEGER NOT NULL
);

-- Label mapping and default conflict policy, one row.
CREATE TABLE vndb_sync_settings (
    id               INTEGER PRIMARY KEY CHECK (id = 1),
    label_playing    INTEGER NOT NULL DEFAULT 1,
    label_completed  INTEGER NOT NULL DEFAULT 2,
    label_on_hold    INTEGER NOT NULL DEFAULT 3,
    label_dropped    INTEGER NOT NULL DEFAULT 4,
    label_wishlist   INTEGER NOT NULL DEFAULT 5,
    policy           TEXT    NOT NULL DEFAULT 'ask',
    updated_at       INTEGER NOT NULL,
    CHECK (policy IN ('ask','prefer_local','prefer_remote','newest'))
);

INSERT INTO vndb_sync_settings (id, updated_at)
VALUES (1, CAST(strftime('%s','now') AS INTEGER) * 1000);

-- Pending pushes. One row per VN: a newer edit replaces the queued patch, since
-- the patch describes the desired end state rather than a sequence of actions.
CREATE TABLE vndb_push_queue (
    vn_id      TEXT PRIMARY KEY NOT NULL,
    game_id    TEXT NOT NULL,
    patch_json TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    attempts   INTEGER NOT NULL DEFAULT 0,
    last_error TEXT,
    CHECK (attempts >= 0)
);

-- Human-readable sync log, capped by the app when it grows.
CREATE TABLE vndb_sync_log (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    at         INTEGER NOT NULL,
    kind       TEXT    NOT NULL,
    vn_id      TEXT,
    message    TEXT    NOT NULL,
    CHECK (kind IN ('pull','push','conflict','error','connect','disconnect'))
);

CREATE INDEX idx_sync_state_game ON vndb_sync_state(game_id);
CREATE INDEX idx_sync_log_at ON vndb_sync_log(at DESC);
