-- 0034_timeline_indexes.sql
--
-- The Timeline page walks every live session and journal entry across all games,
-- newest first, in (ts, id) keyset pages. Every index that existed before this
-- migration leads on `game_id`, so the cross-game walk had to sort each page from
-- scratch. Two partial indexes give the feed an order it can seek into, matching
-- the keyset predicate (`ts < ?` or `ts = ? AND id < ?`) over `deleted = 0` rows.
--
-- `IF NOT EXISTS` keeps a second device replaying a synced backup idempotent; the
-- tables themselves are untouched, so there is no backfill and nothing to dry-run.

CREATE INDEX IF NOT EXISTS idx_sessions_started
    ON play_sessions(started_at DESC, id DESC) WHERE deleted = 0;

CREATE INDEX IF NOT EXISTS idx_entries_created
    ON reading_entries(created_at DESC, id DESC) WHERE deleted = 0;
