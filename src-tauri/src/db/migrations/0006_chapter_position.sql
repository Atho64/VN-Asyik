-- 0006_chapter_position.sql
--
-- Chapters gain the position marker the engine reported, and a manual-playtime
-- column so a chapter's time can be adjusted by hand.
--
-- Migrations 0001-0005 are applied and must not be edited.
--
-- Why manual time exists: engine detection cannot work everywhere. KiriKiri keeps
-- no scene name in the window title and its script engine is statically linked, so
-- no verified offset exists. Rather than show nothing, the user can start and stop
-- a chapter themselves, and the result is a real play session like any other.

-- The position marker as the engine wrote it, e.g. `7月26日` from a window title.
-- Kept separate from the display name so the UI can format and sort on it.
ALTER TABLE chapters ADD COLUMN position TEXT;

-- A chapter the user created by hand rather than one detection found. Manual
-- chapters survive detection and are never deduplicated against engine scenes.
ALTER TABLE chapters ADD COLUMN manual INTEGER NOT NULL DEFAULT 0;
