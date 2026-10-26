-- Script chunk plan, computed once at import.
--
-- `plan_chunks` walks the whole stored body to split it into provider-sized
-- chunks. The summary loop used to run it again on every `script_compact` call,
-- so a 300k-line script was re-split once per batch of eight chunks just to find
-- the next eight. The body never changes after import and neither does the
-- budget, so the plan is deterministic: it is written here once.
--
-- `chunk_plan_json` is an array of `[start, end)` line ranges in story order.
-- `chunk_count` is repeated as a column so listing documents does not parse the
-- plan just to show "12 of 240 chunks".
--
-- Rows imported before this migration keep NULL and fall back to planning on
-- demand; the next compaction writes the plan back, so no re-import is needed.
ALTER TABLE script_documents ADD COLUMN chunk_plan_json TEXT;

ALTER TABLE script_documents ADD COLUMN chunk_count INTEGER;
