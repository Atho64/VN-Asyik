-- 0009_embeddings.sql
--
-- Vectors for similarity search.
--
-- Migrations 0001-0008 are applied and must not be edited.
--
-- One row per embedded record per model. Three things about this schema are
-- deliberate and load-bearing:
--
--   * `model_id` is part of the key. Cosine distance between vectors from
--     different models is meaningless, and mixing them would produce
--     plausible-looking nonsense, which is the worst failure mode available here.
--     Keeping both sets lets a user try a second model without losing the first.
--   * `source_hash` records what text produced the vector. Re-embedding unchanged
--     text on every launch would burn CPU or API credit for nothing, so indexing
--     is incremental by comparing hashes.
--   * The vector is a BLOB of little-endian f32, not a separate vector database.
--     A personal library is thousands of rows; brute-force cosine over 5,000
--     vectors of 384 floats is milliseconds. A vector engine would add a native
--     dependency and a second source of truth for no gain at this scale.
CREATE TABLE embeddings (
    -- 'game' today; chapter and note text may follow.
    record_kind TEXT    NOT NULL,
    record_id   TEXT    NOT NULL,
    -- Which embedder produced this vector, e.g. 'hash-64' or 'bge-small-en'.
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
