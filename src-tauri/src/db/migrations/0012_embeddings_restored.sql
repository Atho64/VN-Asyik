-- 0012_embeddings_restored.sql
--
-- Vectors for similarity, restored.
--
-- 0010 dropped this table when the embedding stack was removed. The stack was
-- later put back, but the table was not, so `ai::index` queried a table that no
-- longer existed and nothing called it -- the feature was dead without anyone
-- noticing, because no command reached it.
--
-- The schema is 0009's, unchanged. Rows are not preserved because 0010 deleted
-- them; the index rebuilds from the library on first use, which for a personal
-- collection is seconds rather than the hours a full VNDB catalog would take.
--
-- Still deliberate, and still load-bearing:
--
--   * model_id is part of the key. Cosine distance across two different models
--     is meaningless, and mixing them would produce plausible nonsense.
--   * source_hash records what text produced the vector, so re-indexing skips
--     unchanged games instead of re-embedding the library on every launch.
--   * The vector is little-endian f32 in a BLOB. Brute-force cosine over a few
--     thousand vectors is milliseconds; a vector database would add a native
--     dependency and a second source of truth for no gain at this size.
CREATE TABLE embeddings (
    -- 'game' today; chapter and note text may follow.
    record_kind TEXT    NOT NULL,
    record_id   TEXT    NOT NULL,
    -- Which embedder produced this vector.
    model_id    TEXT    NOT NULL,
    -- Hash of the text that was embedded, to skip unchanged records.
    source_hash TEXT    NOT NULL,
    -- Component count, so a truncated blob is detectable on read.
    dims        INTEGER NOT NULL,
    -- dims * 4 bytes of little-endian f32.
    vector      BLOB    NOT NULL,
    created_at  INTEGER NOT NULL,
    PRIMARY KEY (record_kind, record_id, model_id),
    CHECK (dims > 0),
    CHECK (length(vector) = dims * 4)
);

CREATE INDEX embeddings_model ON embeddings (model_id, record_kind);
