-- 0019_vndb_screenshots.sql
--
-- Cache a VN's screenshots so the detail screen can show the gallery offline.
--
-- Why a separate table from `vndb_images`: that one is keyed by `vn_id` and holds
-- exactly one row per VN, which is what a cover is. A VN has up to ~30
-- screenshots, each with its own flagging votes, and the content setting has to act
-- per image — one explicit scene in a gallery of harmless backgrounds must not
-- blur the whole set, and a harmless tile must not slip through because its
-- neighbours are fine. So the key here is the VNDB image id (`sf190`).
--
-- Two files per screenshot, mirroring the cover cache: a downscaled sharp copy and
-- a pre-blurred one. Both are generated in Rust (`vndb/screenshots.rs`) because a
-- CSS `filter: blur()` on a scrolling WebView surface is what drops frames on
-- low-end Android (DESIGN.md §2). `thumbnail` is not used: it is another remote
-- URL to fetch and cache, and the local 512px copy serves the same purpose offline.
--
-- `position` preserves VNDB's own ordering. It is not derived from rowid because
-- the gallery is re-ordered whenever the site's list changes, and a rowid-based
-- order would be insertion order, i.e. whichever the download happened to finish
-- first.
--
-- No foreign key to `games`: this is a cache of VNDB data, not user content, and a
-- VN can be cached before (or without) a game row existing — the add sheet previews
-- entries that are not in the library yet. Orphaned rows are pruned by age instead.
CREATE TABLE IF NOT EXISTS vndb_screenshots (
    image_id    TEXT PRIMARY KEY NOT NULL,
    vn_id       TEXT NOT NULL,
    url         TEXT NOT NULL,
    thumb_path  TEXT,
    blur_path   TEXT,
    width       INTEGER,
    height      INTEGER,
    sexual      INTEGER,
    violence    INTEGER,
    position    INTEGER NOT NULL DEFAULT 0,
    fetched_at  INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_vndb_screenshots_vn ON vndb_screenshots(vn_id, position);
