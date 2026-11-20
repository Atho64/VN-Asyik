-- 0040_achievement_unlocks.sql
--
-- One row per earned achievement (PLAN-ACHIEVEMENTS.md, phase P1).
--
-- Storage identity is the catalog slug: `achievement_id` is UNIQUE and every
-- write upserts on it. The UUID `id` travels with the row for sync metadata
-- parity with every other synced table, but two devices unlocking the same
-- slug converge on one row, not two.
--
-- Unlocks are monotonic. The sync merge (vn_core::achievements::merge_unlocks)
-- keeps the earliest `unlocked_at` for a slug and a tombstone never beats a
-- live unlock, so `deleted` exists for payload parity only and stays 0 in
-- normal operation.
--
-- Timestamps are UTC milliseconds; `unlocked_at` is the real event date when
-- the history records one (first session, first finish, markers) and the scan
-- time for cumulative thresholds, which have no recorded crossing date.

CREATE TABLE achievement_unlocks (
    id             TEXT    PRIMARY KEY NOT NULL,
    achievement_id TEXT    NOT NULL UNIQUE,
    unlocked_at    INTEGER NOT NULL,
    updated_at     INTEGER NOT NULL,
    device_id      TEXT    NOT NULL,
    deleted        INTEGER NOT NULL DEFAULT 0 CHECK (deleted IN (0, 1))
);
