-- 0026_script_agents.sql
--
-- Compaction becomes multi-agent (phase 19C): several summarizer agents work
-- parallel segments of a script, each with rolling context, and a final reduce
-- merges their segment digests. Progress storage follows the new shape:
--
-- - `agent_count`: how many agents the stored summaries are laid out for. A
--   different layout cannot resume (the summaries belong to other agents'
--   segments), so it is part of the resume key.
-- - `compactor_version`: which pipeline (prompts, context strategy) produced
--   the summaries. 0 marks phase 18/19B rows; those restart once on first run
--   of the multi-agent pipeline. A finished digest is untouched: the version
--   only gates resuming *progress*.
-- - `summaries_json` becomes one array per agent: `[[...], [...]]`. Flat
--   phase 18/19B arrays are wrapped into a single agent's array.
-- - `segment_digests_json`: one digest per agent once its segment is complete,
--   so retrying the final reduce does not redo every segment reduce.
--
-- Plain additive ALTERs and a JSON reshape; no table rebuild, so this file
-- runs inside the normal transaction.

ALTER TABLE script_compaction ADD COLUMN agent_count INTEGER NOT NULL DEFAULT 1;
ALTER TABLE script_compaction ADD COLUMN compactor_version INTEGER NOT NULL DEFAULT 0;
ALTER TABLE script_compaction ADD COLUMN segment_digests_json TEXT;

UPDATE script_compaction
   SET summaries_json = json_array(json(summaries_json))
 WHERE json_valid(summaries_json)
   AND json_type(summaries_json) = 'array';
