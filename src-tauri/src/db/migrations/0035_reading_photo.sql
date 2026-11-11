-- A journal checkpoint can carry one pinned screenshot, so the journal reads as
-- a photo diary. Only the VNDB image id is stored: the bytes already live in the
-- disposable `vndb_screenshots` cache, which is rebuilt from VNDB on demand, and
-- duplicating them here would make the row large and the sync payload heavy.
--
-- Empty string rather than NULL keeps the column additive for every reader that
-- already selects `e.*`; a row written before this migration means "no photo".
ALTER TABLE reading_entries ADD COLUMN photo_image_id TEXT NOT NULL DEFAULT '';
