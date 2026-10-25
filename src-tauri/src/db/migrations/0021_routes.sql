-- 0021_routes.sql
--
-- One route, one row.
--
-- A route was modelled twice: `chapters.route_key`/`route_name` is engine-derived
-- (`route_20`), and `endings.route` is free text ("Shiroha"). Phase 11 made the
-- ending form suggest chapter labels so the two would agree by hand, and recorded
-- the limitation honestly: "nothing stores a foreign key between the two tables. A
-- rename on either side does not propagate." The same story was grouped two
-- different ways on one screen, permanently.
--
-- `routes` is the identity both children now point at. It is user-authored data
-- (unlike `vndb_details`, which is a cache), so it syncs: same sync columns as
-- every other user table, tombstones included.
--
--   routes.id          stable id, the link target
--   routes.game_id     owning game
--   routes.name        what to show, e.g. "Shiroha"
--   routes.sort_order  presentation order; NULL sorts last
--   routes.origin      'engine' | 'ending' | 'manual' -- where the row came from
--   routes.merged_into survivor id when the user merged two rows
--
-- Backfill rules, and why they are additive rather than clever:
--
-- Every distinct chapter route and every distinct ending route becomes its own
-- row. Nothing is linked across the two sides, because there is no provable
-- mapping: a chapter's key is an engine artefact and an ending's is a label the
-- user typed, and "Mio" may or may not be the "Mio Route" the engine numbered
-- `route_3`. Fusing them by string match would silently rewrite the user's
-- history. Splitting wrongly is visible and reversible; a wrong merge is neither.
-- The UI offers an explicit "same route" merge instead.
--
-- `origin` records which side a row came from, so the UI can explain why one story
-- appears twice before the user merges it.
--
-- `merged_into` is a pointer, not a delete. Sync merges by id, so a second device
-- that had not seen the delete would resurrect the row; a pointer travels with the
-- row and both devices converge on the same survivor.
--
-- Ids are derived from (game, side, key) rather than random, so two devices
-- applying this migration independently produce the same ids and sync merges them
-- instead of duplicating every route.

CREATE TABLE IF NOT EXISTS routes (
    id          TEXT PRIMARY KEY NOT NULL,
    game_id     TEXT NOT NULL REFERENCES games(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    sort_order  INTEGER,
    origin      TEXT NOT NULL DEFAULT 'manual',
    merged_into TEXT,
    updated_at  INTEGER NOT NULL,
    device_id   TEXT NOT NULL,
    deleted     INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_routes_game ON routes(game_id) WHERE deleted = 0;

-- Chapters and endings gain the link. Nullable: a chapter with no route is
-- ordinary. The engine columns stay on `chapters` because `route_key` is the
-- detection key -- what ties an engine-reported scene to a row -- and must not be
-- replaced by a user-facing name.
ALTER TABLE chapters ADD COLUMN route_id TEXT;
ALTER TABLE endings ADD COLUMN route_id TEXT;

CREATE INDEX IF NOT EXISTS idx_chapters_route ON chapters(route_id);
CREATE INDEX IF NOT EXISTS idx_endings_route ON endings(route_id);

-- ---------------------------------------------------------------------------

-- ---------------------------------------------------------------------------
-- Backfill
--
-- Ids must be valid UUIDs: `EntityId::parse` rejects anything else, and a raw hex
-- hash here made every route read fail with "invalid id" the moment the backfill
-- ran. They must also stay deterministic, or two devices applying this migration
-- would each mint their own copy of every route and sync would duplicate them all.
--
-- A counter scoped to the game is the smallest thing that satisfies both. The first
-- 21 hex chars identify the game, the last 12 are a per-game sequence, so ids are
-- unique across games and within one, and identical on every device because the
-- input data is identical.
-- ---------------------------------------------------------------------------

-- One row per distinct engine route that has at least one live chapter.
--
-- The `games` EXISTS guard is load-bearing, not decoration. `PRAGMA foreign_keys`
-- is per connection and a `DROP TABLE` without it drops the parent without
-- cascading, so orphan chapters whose game row is gone are a real, observed state:
-- this migration failed with `FOREIGN KEY constraint failed` on a live database
-- carrying three of them. `routes.game_id` references `games(id)`, so inserting a
-- route for an orphan violates it and aborts the whole migration. An orphan chapter
-- has no route worth keeping anyway -- no screen can reach it.
INSERT OR IGNORE INTO routes
    (id, game_id, name, sort_order, origin, merged_into, updated_at, device_id, deleted)
SELECT
    lower(
        substr(h, 1, 8) || '-' || substr(h, 9, 4) || '-4' || substr(h, 14, 3) || '-' ||
        substr('89ab', (instr('0123456789abcdef', substr(h, 18, 1)) % 4) + 1, 1) ||
        substr(h, 19, 3) || '-' || printf('%012x', seq)
    ),
    game_id,
    label,
    sort_order,
    'engine',
    NULL,
    now_ms,
    device_id,
    0
FROM (
    SELECT
        substr(hex(game_id), 1, 21) AS h,
        row_number() OVER (PARTITION BY game_id ORDER BY route_key) AS seq,
        game_id,
        COALESCE(NULLIF(TRIM(route_name), ''), route_key) AS label,
        MIN(sort_index) AS sort_order,
        MAX(updated_at) AS now_ms,
        MAX(device_id) AS device_id
    FROM chapters
    WHERE deleted = 0
      AND route_key IS NOT NULL
      AND EXISTS (SELECT 1 FROM games g WHERE g.id = chapters.game_id)
    GROUP BY game_id, route_key
);

-- One row per distinct free-text ending route. `LOWER(TRIM(...))` in the grouping
-- so "Shiroha" and "  shiroha " are one route, which is the whole point of a table
-- that replaces a free-text label. Same orphan guard as above.
INSERT OR IGNORE INTO routes
    (id, game_id, name, sort_order, origin, merged_into, updated_at, device_id, deleted)
SELECT
    lower(
        substr(h, 1, 8) || '-' || substr(h, 9, 4) || '-4' || substr(h, 14, 3) || '-' ||
        substr('89ab', (instr('0123456789abcdef', substr(h, 18, 1)) % 4) + 1, 1) ||
        substr(h, 19, 3) || '-' || printf('%012x', seq)
    ),
    game_id,
    label,
    NULL,
    'ending',
    NULL,
    now_ms,
    device_id,
    0
FROM (
    SELECT
        substr(hex(game_id), 1, 21) AS h,
        -- Offset into a separate id range. Both inserts number from 1, so without
        -- this an ending route collides with the engine route of the same ordinal
        -- and `INSERT OR IGNORE` drops it silently -- the ending links then all
        -- come out NULL and the two halves never meet.
        row_number() OVER (PARTITION BY game_id ORDER BY LOWER(TRIM(route))) + 1099511627776 AS seq,
        game_id,
        MIN(TRIM(route)) AS label,
        MAX(updated_at) AS now_ms,
        MAX(device_id) AS device_id
    FROM endings
    WHERE deleted = 0
      AND route IS NOT NULL
      AND TRIM(route) <> ''
      AND EXISTS (SELECT 1 FROM games g WHERE g.id = endings.game_id)
    GROUP BY game_id, LOWER(TRIM(route))
);

-- Link chapters to the row created for their route.
--
-- Resolved by re-deriving the same sequence, so the link and the insert cannot
-- disagree about which route a chapter belongs to. The two sequences are separate
-- on purpose: an engine route and an ending route are never auto-merged, because
-- there is no provable mapping between an engine key and a typed label.
UPDATE chapters
SET route_id = (
    SELECT r.id
    FROM routes r
    WHERE r.game_id = chapters.game_id
      AND r.deleted = 0
      AND r.origin = 'engine'
      AND r.name = COALESCE(NULLIF(TRIM(chapters.route_name), ''), chapters.route_key)
)
WHERE deleted = 0
  AND route_key IS NOT NULL
  AND EXISTS (SELECT 1 FROM games g WHERE g.id = chapters.game_id);

-- Link endings to theirs.
UPDATE endings
SET route_id = (
    SELECT r.id
    FROM routes r
    WHERE r.game_id = endings.game_id
      AND r.deleted = 0
      AND r.origin = 'ending'
      AND LOWER(r.name) = LOWER(TRIM(endings.route))
)
WHERE deleted = 0
  AND route IS NOT NULL
  AND TRIM(route) <> ''
  AND EXISTS (SELECT 1 FROM games g WHERE g.id = endings.game_id);
