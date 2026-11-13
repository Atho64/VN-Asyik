-- Production staff credits and the voice cast, per cached VNDB detail.
--
-- The "For you" recommendations derive taste from the people behind the games
-- the user rates highly: scenario writers and artists (roles come with each
-- credit), and voice actors shared across a library. Both lists are already
-- fetched by `DETAIL_FIELDS` (`staff.*` and `va.staff.*`) but were dropped at
-- store time; these columns keep them.
--
-- Plain JSON columns like `tags_json`/`developers_json`, not child tables:
-- cached VNDB data is display data with no sync rules, and the read path
-- decodes with `unwrap_or_default`. Rows written before this migration have
-- NULL here and decode to an empty list; a detail refresh backfills them.
ALTER TABLE vndb_details ADD COLUMN staff_json TEXT;
ALTER TABLE vndb_details ADD COLUMN va_json TEXT;
