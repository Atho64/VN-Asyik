-- 0008_sync.sql
--
-- State for folder-based sync.
--
-- Migrations 0001-0007 are applied and must not be edited.
--
-- The data model already carries `device_id`, `updated_at` and `deleted` on every
-- synced table, so merging needs no new columns. What sync does need is:
--
--   * a record of what was last exchanged, so the UI can say when sync last ran
--     and between which devices;
--   * a log of conflicts, because last-write-wins silently discards a revision and
--     the user should be able to see that it happened;
--   * per-table change counters, so a push does not have to serialise the whole
--     library every time.

-- One row per completed sync exchange.
CREATE TABLE sync_runs (
    id            TEXT PRIMARY KEY,
    -- Which device this run talked to, or the folder's own tag when unknown.
    peer_device   TEXT,
    -- What was pushed and pulled, as counts, for the log line.
    pushed        INTEGER NOT NULL DEFAULT 0,
    pulled        INTEGER NOT NULL DEFAULT 0,
    -- Conflicting records resolved during this run.
    conflicts     INTEGER NOT NULL DEFAULT 0,
    started_at    INTEGER NOT NULL,
    finished_at   INTEGER NOT NULL,
    -- 'ok' or an error code, so a failed run is visible rather than absent.
    outcome       TEXT NOT NULL,
    message       TEXT
);

CREATE INDEX sync_runs_finished ON sync_runs (finished_at DESC);

-- One row per record where last-write-wins discarded a revision.
--
-- Append-only: a conflict is a historical fact about a sync, not state to update.
CREATE TABLE sync_conflicts (
    id                      TEXT PRIMARY KEY,
    run_id                  TEXT NOT NULL REFERENCES sync_runs (id) ON DELETE CASCADE,
    -- 'games', 'chapters', 'sessions', 'endings'.
    table_name              TEXT NOT NULL,
    -- The record that had two revisions.
    record_id               TEXT NOT NULL,
    -- 'local' or 'remote': which copy was kept.
    kept                    TEXT NOT NULL,
    -- Why the winner won: 'newer', 'tie_break', 'delete_wins'.
    reason                  TEXT NOT NULL,
    -- When the discarded copy was last modified, so the log shows what was lost.
    discarded_updated_at    INTEGER NOT NULL,
    -- Which device wrote the discarded copy.
    discarded_device        TEXT,
    created_at              INTEGER NOT NULL,
    CHECK (kept IN ('local','remote'))
);

CREATE INDEX sync_conflicts_run ON sync_conflicts (run_id);

-- Per-device bookkeeping reuses `sync_state` from 0001, which is already a
-- generic key/value table. Keys written by this module:
--
--   sync.last_folder      folder last used, for convenience in the UI
--   sync.last_sync_at     when a run last completed, for the status line
--   sync.generation       counter for this device's own payload file
--
-- No new table is needed; adding one would duplicate a table that already exists
-- for exactly this purpose.
