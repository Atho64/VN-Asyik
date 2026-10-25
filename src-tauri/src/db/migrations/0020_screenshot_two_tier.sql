-- 0020_screenshot_two_tier.sql
--
-- Store two sizes of every screenshot: a grid tile and a lightbox image.
--
-- The first version cached one 512 px file and painted it in both places. A grid tile
-- is ~320 px so that was already marginal, and the lightbox renders at ~1300 px on a
-- desktop, which upscaled a 512 px source by 2.5x. The result was reported, accurately,
-- as "very low pixel". Raising the single size to 768 helped the lightbox and did
-- nothing for the tile, and still threw away most of a 1280x720 original.
--
-- Two sizes solve it without a 5 MB gallery: the tile stays small enough that 30 of
-- them travel in one IPC response, and the lightbox gets the source resolution.
--
-- Blurred variants exist at both sizes for the same reason. A single small blur looks
-- acceptable as a tile and like a mosaic when the lightbox paints it.
--
-- Existing rows get NULL for the new paths; `vndb_screenshots` treats a NULL path as
-- "not cached at this size" and regenerates from the original, which is still on
-- VNDB. No data is lost: these are cached derivatives, not user content.
ALTER TABLE vndb_screenshots
    ADD COLUMN full_path TEXT;

ALTER TABLE vndb_screenshots
    ADD COLUMN blur_full_path TEXT;
