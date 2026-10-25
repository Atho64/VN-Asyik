-- 0005_chapter_routes.sql
--
-- Chapters gain a route and a user-editable display name.
--
-- Migrations 0001-0004 are applied and must not be edited.
--
-- Why the raw name is the identity: chapter detection re-reads the engine's own
-- scene name on every poll. If a rename replaced `name`, the next poll would not
-- match the stored row and would create a second chapter for the same scene, so
-- the raw name must stay the key that detection matches on. `display_name` is
-- where a rename lives, and it is what the user sees.

-- The engine's raw scene name, minus its route prefix, if that is already stored.
-- For rows created before this migration `name` already holds that value, so the
-- column starts equal and only diverges on rename.
ALTER TABLE chapters ADD COLUMN display_name TEXT;

-- Route grouping. `route_key` is a stable identifier (`route_20`), `route_name`
-- is the human label, and `route_order` sorts routes the way the game presents
-- them. All three are NULL for chapters whose engine naming reveals no route.
ALTER TABLE chapters ADD COLUMN route_key TEXT;
ALTER TABLE chapters ADD COLUMN route_name TEXT;
ALTER TABLE chapters ADD COLUMN route_order INTEGER;

-- Detection matches on (game, raw name), so this is the lookup path.
CREATE INDEX idx_chapter_game_name ON chapters(game_id, name) WHERE deleted = 0;
