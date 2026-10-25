-- 0001_init.sql
-- Phase 1 baseline schema.
--
-- Rules (AGENTS.md §5):
--   * Every synced table has: id (UUIDv7 text), updated_at (UTC ms),
--     device_id, deleted (soft delete).
--   * Timestamps are integers: UTC milliseconds since the Unix epoch.
--   * PlaySession is append-only for sync; rows are never overwritten by a merge.
--
-- This migration is applied once and must never be edited afterwards.

CREATE TABLE games (
    id                TEXT    PRIMARY KEY NOT NULL,
    title             TEXT    NOT NULL,
    title_original    TEXT,
    vndb_id           TEXT,
    status            TEXT    NOT NULL,
    rating            INTEGER,
    notes             TEXT,
    length_minutes    INTEGER,
    manual_playtime_ms INTEGER NOT NULL DEFAULT 0,
    accent_color      TEXT,
    cover_url         TEXT,
    cover_sexual      INTEGER,
    cover_violence    INTEGER,
    started_at        INTEGER,
    finished_at       INTEGER,
    updated_at        INTEGER NOT NULL,
    device_id         TEXT    NOT NULL,
    deleted           INTEGER NOT NULL DEFAULT 0,
    CHECK (length(trim(title)) > 0),
    CHECK (rating IS NULL OR (rating BETWEEN 1 AND 10)),
    CHECK (status IN ('wishlist','playing','completed','on_hold','dropped')),
    CHECK (deleted IN (0,1))
);

CREATE TABLE chapters (
    id             TEXT    PRIMARY KEY NOT NULL,
    game_id        TEXT    NOT NULL REFERENCES games(id) ON DELETE CASCADE,
    name           TEXT    NOT NULL,
    sort_index     INTEGER NOT NULL DEFAULT 0,
    reached        INTEGER NOT NULL DEFAULT 0,
    first_seen_at  INTEGER,
    updated_at     INTEGER NOT NULL,
    device_id      TEXT    NOT NULL,
    deleted        INTEGER NOT NULL DEFAULT 0,
    CHECK (reached IN (0,1)),
    CHECK (deleted IN (0,1))
);

CREATE TABLE play_sessions (
    id          TEXT    PRIMARY KEY NOT NULL,
    game_id     TEXT    NOT NULL REFERENCES games(id) ON DELETE CASCADE,
    chapter_id  TEXT             REFERENCES chapters(id) ON DELETE SET NULL,
    started_at  INTEGER NOT NULL,
    ended_at    INTEGER NOT NULL,
    counted_ms  INTEGER NOT NULL,
    source      TEXT    NOT NULL,
    updated_at  INTEGER NOT NULL,
    device_id   TEXT    NOT NULL,
    deleted     INTEGER NOT NULL DEFAULT 0,
    CHECK (ended_at >= started_at),
    CHECK (counted_ms >= 0),
    CHECK (source IN ('desktop','android_usage','manual','import')),
    CHECK (deleted IN (0,1))
);

CREATE TABLE endings (
    id           TEXT    PRIMARY KEY NOT NULL,
    game_id      TEXT    NOT NULL REFERENCES games(id) ON DELETE CASCADE,
    name         TEXT    NOT NULL,
    route        TEXT,
    achieved     INTEGER NOT NULL DEFAULT 0,
    achieved_at  INTEGER,
    updated_at   INTEGER NOT NULL,
    device_id    TEXT    NOT NULL,
    deleted      INTEGER NOT NULL DEFAULT 0,
    CHECK (achieved IN (0,1)),
    CHECK (deleted IN (0,1))
);

-- Cached VNDB responses. Not synced: it is a cache, safe to rebuild.
CREATE TABLE vndb_cache (
    cache_key   TEXT    PRIMARY KEY NOT NULL,
    payload     TEXT    NOT NULL,
    fetched_at  INTEGER NOT NULL,
    expires_at  INTEGER NOT NULL
);

-- Per-image metadata cache, so offline mode can still render art.
CREATE TABLE image_cache (
    url          TEXT    PRIMARY KEY NOT NULL,
    local_path   TEXT    NOT NULL,
    width        INTEGER,
    height       INTEGER,
    blur_path    TEXT,
    fetched_at   INTEGER NOT NULL
);

-- Local, unsynced key/value preferences (theme, language, NSFW mode, ...).
CREATE TABLE local_prefs (
    key    TEXT PRIMARY KEY NOT NULL,
    value  TEXT NOT NULL
);

-- Engine adapter profiles (chapter detection). Synced only when the user opts in.
CREATE TABLE adapter_profiles (
    id           TEXT    PRIMARY KEY NOT NULL,
    game_id      TEXT             REFERENCES games(id) ON DELETE CASCADE,
    engine       TEXT    NOT NULL,
    exe_hash     TEXT,
    profile_json TEXT    NOT NULL,
    confidence   TEXT    NOT NULL DEFAULT 'experimental',
    verified     INTEGER NOT NULL DEFAULT 0,
    updated_at   INTEGER NOT NULL,
    device_id    TEXT    NOT NULL,
    deleted      INTEGER NOT NULL DEFAULT 0,
    CHECK (confidence IN ('verified','experimental','stale')),
    CHECK (verified IN (0,1)),
    CHECK (deleted IN (0,1))
);

-- Sync bookkeeping. Never sent to a server; describes this device's progress.
CREATE TABLE sync_state (
    key    TEXT PRIMARY KEY NOT NULL,
    value  TEXT NOT NULL
);

CREATE INDEX idx_games_status        ON games(status) WHERE deleted = 0;
CREATE INDEX idx_games_updated       ON games(updated_at);
CREATE INDEX idx_chapters_game       ON chapters(game_id) WHERE deleted = 0;
CREATE INDEX idx_chapters_order      ON chapters(game_id, sort_index) WHERE deleted = 0;
CREATE INDEX idx_sessions_game_time  ON play_sessions(game_id, started_at) WHERE deleted = 0;
CREATE INDEX idx_sessions_chapter    ON play_sessions(chapter_id) WHERE deleted = 0;
CREATE INDEX idx_sessions_updated    ON play_sessions(updated_at);
CREATE INDEX idx_endings_game        ON endings(game_id) WHERE deleted = 0;
CREATE INDEX idx_profiles_game       ON adapter_profiles(game_id) WHERE deleted = 0;

-- Exactly one row: guards against two writers opening the same file twice.
CREATE TABLE schema_guard (
    id         INTEGER PRIMARY KEY CHECK (id = 1),
    created_at INTEGER NOT NULL
);

INSERT INTO schema_guard (id, created_at) VALUES (1, CAST(strftime('%s','now') AS INTEGER) * 1000);
