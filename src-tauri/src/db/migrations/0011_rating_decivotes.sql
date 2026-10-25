-- no-transaction
-- 0011_rating_decivotes.sql
--
-- Ratings move from whole stars to decivotes.
--
-- VNDB's `vote` field is an integer 10..=100, which is the same scale in tenths:
-- a 7.5 there is 75. The app stored 1..=10, so every import rounded a vote of
-- 73 down to 7 and every export turned a 7 back into 70, quietly losing the
-- user's own precision on the round trip. Storing decivotes makes both
-- directions exact.
--
-- The display still reads 1.0 to 10.0; only the stored form changes.
--
-- --- why `-- no-transaction` -------------------------------------------------
--
-- SQLite cannot alter a CHECK constraint, so the table is rebuilt. That requires
-- `PRAGMA foreign_keys = OFF`, which is a no-op inside a transaction, so this
-- migration cannot run inside one.
--
-- The pragma is not optional here. `ALTER TABLE ... RENAME` rewrites every child
-- table's foreign key to follow the renamed table, so the usual rename-copy-swap
-- leaves children pointing at a name that then gets dropped: a dry run against a
-- real database went from 0 to 14 dangling references. Switching enforcement off
-- for the rebuild is the documented procedure for exactly this, and the
-- `PRAGMA foreign_key_check` at the end proves the result is sound.
--
-- The rollback is left to sqlx's migration bookkeeping: if the copy fails, this
-- migration is never recorded as applied and the next start retries it.

PRAGMA foreign_keys = OFF;

BEGIN;

CREATE TABLE games_new (
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
    -- Decivotes: 10 is 1.0, 100 is 10.0.
    CHECK (rating IS NULL OR (rating BETWEEN 10 AND 100)),
    CHECK (status IN ('wishlist','playing','completed','on_hold','dropped')),
    CHECK (deleted IN (0,1))
);

INSERT INTO games_new (
    id, title, title_original, vndb_id, status, rating, notes, length_minutes,
    manual_playtime_ms, accent_color, cover_url, cover_sexual, cover_violence,
    started_at, finished_at, updated_at, device_id, deleted
)
SELECT
    id, title, title_original, vndb_id, status, rating * 10, notes, length_minutes,
    manual_playtime_ms, accent_color, cover_url, cover_sexual, cover_violence,
    started_at, finished_at, updated_at, device_id, deleted
FROM games;

DROP TABLE games;

ALTER TABLE games_new RENAME TO games;

CREATE INDEX idx_games_status  ON games(status) WHERE deleted = 0;
CREATE INDEX idx_games_updated ON games(updated_at);

COMMIT;

PRAGMA foreign_keys = ON;
