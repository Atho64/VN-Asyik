-- 0017_vndb_aliases.sql
--
-- Keep the short names VNDB publishes beside its titles.
--
-- `aliases` has been in `DETAIL_FIELDS` and in the DTO since the beginning and was
-- dropped by `store_detail` with the titles array. It is where the names people
-- actually use live: v24 stores "When They Cry 3" and "When The Seagulls Cry", v4
-- stores "クラナド" and "团子大家族". None of those are titles, so a list built from
-- `titles` alone never shows them.
--
-- A separate column from `titles_json` rather than merged into one array: an alias
-- has no language, no `official` flag and no `latin` form, so folding it into a
-- `TitleVariant` would mean inventing values for three fields and then having the
-- UI guess which entries were real titles.
--
-- Existing rows get `[]`. The next metadata fetch fills them in.
ALTER TABLE vndb_details
    ADD COLUMN aliases_json TEXT NOT NULL DEFAULT '[]';
