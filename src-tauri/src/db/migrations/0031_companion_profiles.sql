-- Device-local identities. Neither this table nor session snapshots are synced/exported.
CREATE TABLE companion_profiles (
    id TEXT PRIMARY KEY NOT NULL,
    version INTEGER NOT NULL CHECK (version BETWEEN 1 AND 2147483647),
    profile_json TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
