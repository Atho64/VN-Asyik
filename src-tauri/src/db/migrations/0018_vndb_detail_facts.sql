-- 0018_vndb_detail_facts.sql
--
-- Keep the rest of what VNDB tells us about an entry, so the detail screen can
-- show the facts a VN page is actually read for instead of a title, a score and a
-- paragraph.
--
-- `DETAIL_FIELDS` has requested `languages`, `platforms`, `devstatus`,
-- `length_votes` and `average` since the first import and `store_detail` threw all
-- five away, exactly as it once threw away `titles` and `aliases`. The DTO has
-- decoded them the whole time, so nothing here changes the API surface: it stops
-- discarding data that was already paid for in the same response.
--
-- `developers_json` and `extlinks_json` are new requests, both small and stable.
-- Developers are who made the thing (VNDB lists them at the top of the page), and
-- extlinks are the official site, the Wikipedia article and the store page the
-- user opens from here. `relations` is deliberately *not* stored: related VNs need
-- their own cover art and metadata, which is a browsing surface rather than a
-- field, and stashing titles without their art would produce dead links.
--
-- One JSON blob per list, matching the DTO shapes, for the same reason `titles_json`
-- and `aliases_json` are blobs: nothing joins on an individual developer or link,
-- the whole set is read and rendered at once, and `vndb_details` is a cache rather
-- than user-authored data, so it needs no sync rules of its own.
--
--   languages_json  ["en","ja","zh-Hans"]
--   platforms_json  ["win","psp","ps2"]
--   developers_json [{"id":"p8","name":"KID","original":"キッド","type":"co"}]
--   extlinks_json   [{"url":"https://…","label":"Wikipedia (en)","name":"enwiki"}]
--
-- `devstatus` is VNDB's integer (0 finished, 1 in development, 2 cancelled),
-- `length_votes` the number of playtime votes behind `length_minutes`, and
-- `average` the raw vote average before the Bayesian correction. All three are
-- nullable: a VN nobody has voted on has no rating at all, and writing 0 there
-- would claim it was rated zero.
--
-- Existing rows get the empty value, which the UI reads as "not known yet". The
-- next metadata refresh fills them in; nothing breaks while they are empty.
ALTER TABLE vndb_details
    ADD COLUMN languages_json TEXT NOT NULL DEFAULT '[]';

ALTER TABLE vndb_details
    ADD COLUMN platforms_json TEXT NOT NULL DEFAULT '[]';

ALTER TABLE vndb_details
    ADD COLUMN devstatus INTEGER;

ALTER TABLE vndb_details
    ADD COLUMN length_votes INTEGER;

ALTER TABLE vndb_details
    ADD COLUMN average REAL;

ALTER TABLE vndb_details
    ADD COLUMN developers_json TEXT NOT NULL DEFAULT '[]';

ALTER TABLE vndb_details
    ADD COLUMN extlinks_json TEXT NOT NULL DEFAULT '[]';
