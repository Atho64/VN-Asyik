-- 0022_session_routes.sql
--
-- A play session can now belong to a route rather than a chapter.
--
-- Until now every session pointed at a chapter (or nowhere at all), so "record an
-- hour on the Shiroha route" was impossible: the user had to invent a chapter to
-- blame it on, which polluted the chapter list with rows that exist only to hold
-- time. `chapter_id` was already nullable, so this adds the missing half rather than
-- loosening a constraint.
--
-- Exactly one of the two should be set, and a session with neither is still legal:
-- that is what detected time looks like when the adapter knows the game but no
-- chapter, and it is what every session written before chapter detection existed
-- looks like. The "not both" rule is enforced in Rust (`Segment` carries one target)
-- rather than as a table CHECK, because SQLite cannot add a CHECK to an existing
-- table without a full rebuild, and a rebuild of the largest table in the database
-- is a worse trade than one constructor.
--
-- Backfill: a session that already has a chapter inherits that chapter's `route_id`,
-- because the chapter is the more specific fact and its route is already known. This
-- is not a guess -- it reads the link migration 0021 established. A session with no
-- chapter keeps `route_id = NULL` rather than being reassigned to some route.
--
-- The index is partial, matching `idx_sessions_chapter`: the route rollup reads live
-- rows only, and most rows are live, so a partial index costs nothing and keeps the
-- planner on it.

ALTER TABLE play_sessions ADD COLUMN route_id TEXT;

CREATE INDEX IF NOT EXISTS idx_sessions_route ON play_sessions(route_id) WHERE deleted = 0;

-- Inherit the route of the chapter each session was already attributed to.
--
-- Guarded by EXISTS on `games` for the same reason 0021 is: an orphan session whose
-- game row is gone is reachable in a real database, and nothing here should touch a
-- row that no screen can see.
UPDATE play_sessions
SET route_id = (
    SELECT c.route_id
    FROM chapters c
    WHERE c.id = play_sessions.chapter_id
)
WHERE deleted = 0
  AND chapter_id IS NOT NULL
  AND route_id IS NULL
  AND EXISTS (SELECT 1 FROM games g WHERE g.id = play_sessions.game_id);
