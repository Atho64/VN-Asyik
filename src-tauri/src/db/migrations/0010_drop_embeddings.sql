-- 0010_drop_embeddings.sql
--
-- The embedding stack was removed: similarity and free-text library search
-- moved to Tavily-backed recommendations, so nothing reads these vectors.
--
-- 0009 is kept rather than deleted because it is already applied in existing
-- databases, and sqlx refuses to run when a previously applied migration
-- disappears from the set. Dropping the table here is the honest way to retire
-- it: installations that have never seen 0009 still apply it and then drop it,
-- and installations that already ran it just drop it.
DROP INDEX IF EXISTS embeddings_model;
DROP TABLE IF EXISTS embeddings;
