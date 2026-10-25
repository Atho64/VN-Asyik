-- 0002_afk_and_vndb.sql
--
-- Adds two things that arrived as requirements after the Phase 1 baseline was
-- applied:
--
--   1. AFK accounting. Play sessions now record idle time separately from active
--      time. `counted_ms` keeps its meaning (active time only, the number the
--      library and stats headline), and `afk_ms` records the rest, so nothing is
--      discarded and an "include idle" toggle is a sum, not a lost value.
--
--   2. VNDB import metadata. Covers are cached locally with their content
--      ratings, and detail payloads (tags, releases, characters) are stored per
--      VN id rather than in the generic cache, so a game can be re-imported
--      offline.
--
-- Migration 0001 is already applied and must not be edited.

-- AFK time per session. SQLite cannot add a CHECK constraint to an existing
-- table, and the table is rebuilt rather than limited to a bare column, so the
-- invariant `afk_ms >= 0` is enforced by the app and asserted in tests instead of
-- by the schema. Default 0 reads existing rows as fully active, which is the only
-- honest interpretation: idle tracking did not exist when they were written.
ALTER TABLE play_sessions ADD COLUMN afk_ms INTEGER NOT NULL DEFAULT 0;

-- AFK threshold and behaviour live in local prefs, but the threshold needs a
-- validated integer range, so it gets its own columns rather than a generic blob.
CREATE TABLE afk_settings (
    id               INTEGER PRIMARY KEY CHECK (id = 1),
    threshold_ms     INTEGER NOT NULL DEFAULT 60000,
    exclude_afk      INTEGER NOT NULL DEFAULT 1,
    updated_at       INTEGER NOT NULL,
    CHECK (threshold_ms BETWEEN 15000 AND 1800000),
    CHECK (exclude_afk IN (0, 1))
);

INSERT INTO afk_settings (id, threshold_ms, exclude_afk, updated_at)
VALUES (1, 60000, 1, CAST(strftime('%s','now') AS INTEGER) * 1000);

-- Cached VNDB cover files, keyed by VN id so the lookup is by game, not by URL.
-- `sexual` and `violence` are VNDB averages in 0..2, stored as hundredths to keep
-- the fractional part the API actually returns.
CREATE TABLE vndb_images (
    vn_id        TEXT PRIMARY KEY NOT NULL,
    url          TEXT NOT NULL,
    local_path   TEXT,
    blur_path    TEXT,
    width        INTEGER,
    height       INTEGER,
    sexual       INTEGER,
    violence     INTEGER,
    fetched_at   INTEGER NOT NULL
);

-- One row per imported VN: the parts of the API payload the app actually shows.
-- Kept separate from `games` so a library entry can exist without VNDB data, and
-- so a re-sync never overwrites user-owned fields (status, rating, notes).
CREATE TABLE vndb_details (
    vn_id           TEXT PRIMARY KEY NOT NULL,
    title           TEXT NOT NULL,
    alttitle        TEXT,
    length_minutes  INTEGER,
    length          INTEGER,
    description     TEXT,
    rating          REAL,
    votecount       INTEGER,
    released        TEXT,
    olang           TEXT,
    tags_json       TEXT NOT NULL DEFAULT '[]',
    releases_json   TEXT NOT NULL DEFAULT '[]',
    characters_json TEXT NOT NULL DEFAULT '[]',
    screenshots_json TEXT NOT NULL DEFAULT '[]',
    fetched_at      INTEGER NOT NULL
);

CREATE INDEX idx_vndb_images_url ON vndb_images(url);
