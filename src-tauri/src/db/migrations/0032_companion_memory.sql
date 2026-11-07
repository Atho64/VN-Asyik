-- Local-only memory. Never included in sync, library exports or backups.
CREATE TABLE companion_memory_state (
  id INTEGER PRIMARY KEY CHECK(id=1),
  revision INTEGER NOT NULL DEFAULT 0 CHECK(revision>=0),
  policy_version INTEGER NOT NULL DEFAULT 1 CHECK(policy_version=1),
  retrieval_enabled INTEGER NOT NULL DEFAULT 0 CHECK(retrieval_enabled IN (0,1)),
  auto_summary INTEGER NOT NULL DEFAULT 0 CHECK(auto_summary IN (0,1)),
  gap TEXT CHECK(gap IN ('disabled','capacity','input_limit','no_evidence'))
);
INSERT INTO companion_memory_state(id) VALUES(1);

CREATE TABLE companion_memories (
  id TEXT PRIMARY KEY,
  version INTEGER NOT NULL CHECK(version>0),
  class TEXT NOT NULL CHECK(class IN ('preference','continuity','episode','observation','summary')),
  scope TEXT NOT NULL CHECK(scope IN ('shared','companion','reading')),
  companion_id TEXT, game_id TEXT, playthrough_id TEXT,
  knowledge TEXT NOT NULL CHECK(knowledge IN ('metadata','observed','all')),
  body TEXT NOT NULL CHECK(length(CAST(body AS BLOB))<=2048),
  evidence TEXT NOT NULL CHECK(evidence IN ('reader','passage','inference')),
  approved INTEGER NOT NULL DEFAULT 1 CHECK(approved IN (0,1)),
  pinned INTEGER NOT NULL DEFAULT 0 CHECK(pinned IN (0,1)),
  origin_session TEXT,
  idempotency_key TEXT UNIQUE,
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, expires_at INTEGER,
  deleted INTEGER NOT NULL DEFAULT 0 CHECK(deleted IN (0,1)),
  revoked_revision INTEGER,
  superseded_by TEXT,
  CHECK(deleted=0 OR (body='' AND pinned=0)),
  CHECK((scope='shared' AND class='preference' AND evidence='reader' AND knowledge='metadata'
         AND companion_id IS NULL AND game_id IS NULL AND playthrough_id IS NULL AND origin_session IS NULL)
     OR (scope='companion' AND class IN ('preference','continuity') AND evidence='reader' AND knowledge='metadata'
         AND companion_id IS NOT NULL AND game_id IS NULL AND playthrough_id IS NULL AND origin_session IS NULL)
     OR (scope='reading' AND class IN ('episode','observation','summary') AND knowledge IN ('observed','all')
         AND companion_id IS NULL AND game_id IS NOT NULL AND playthrough_id IS NOT NULL AND origin_session IS NOT NULL))
);
CREATE INDEX companion_memory_scope ON companion_memories(deleted,scope,companion_id,game_id,playthrough_id);
CREATE INDEX companion_memory_origin ON companion_memories(origin_session);

CREATE TABLE companion_memory_sources (
  id TEXT PRIMARY KEY,
  memory_id TEXT NOT NULL REFERENCES companion_memories(id) ON DELETE CASCADE,
  memory_version INTEGER NOT NULL CHECK(memory_version>0),
  kind TEXT NOT NULL CHECK(kind IN ('reader','user','assistant','summary','luna')),
  origin_session TEXT, thread_id TEXT, message_id TEXT,
  from_position INTEGER, through_position INTEGER,
  generation INTEGER, from_sequence INTEGER, through_sequence INTEGER,
  knowledge TEXT NOT NULL CHECK(knowledge IN ('metadata','observed','all')),
  evidence TEXT NOT NULL CHECK(length(CAST(evidence AS BLOB))<=4096),
  revoked INTEGER NOT NULL DEFAULT 0 CHECK(revoked IN (0,1)),
  CHECK(revoked=0 OR evidence='')
);
CREATE INDEX companion_memory_source_owner ON companion_memory_sources(memory_id,memory_version);

CREATE TABLE companion_memory_jobs (
  id TEXT PRIMARY KEY,
  input_key TEXT NOT NULL UNIQUE,
  origin_session TEXT NOT NULL, thread_id TEXT NOT NULL,
  context_json TEXT NOT NULL CHECK(length(CAST(context_json AS BLOB))<=2048),
  memory_revision INTEGER NOT NULL CHECK(memory_revision>=0),
  from_position INTEGER NOT NULL, through_position INTEGER NOT NULL,
  input TEXT NOT NULL CHECK(length(CAST(input AS BLOB))<=65536),
  output TEXT NOT NULL DEFAULT '' CHECK(length(CAST(output AS BLOB))<=16384),
  config_json TEXT NOT NULL CHECK(length(CAST(config_json AS BLOB))<=4096),
  prompt TEXT NOT NULL CHECK(length(CAST(prompt AS BLOB))<=16384),
  prompt_version INTEGER NOT NULL CHECK(prompt_version=1),
  policy_version INTEGER NOT NULL CHECK(policy_version=1),
  profile_id TEXT NOT NULL, profile_version INTEGER NOT NULL,
  state TEXT NOT NULL CHECK(state IN ('pending','running','paused','review','approved','cancelled','expired')),
  attempts INTEGER NOT NULL DEFAULT 0 CHECK(attempts BETWEEN 0 AND 20),
  automatic INTEGER NOT NULL CHECK(automatic IN (0,1)),
  error TEXT,
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, expires_at INTEGER NOT NULL,
  CHECK(state NOT IN ('cancelled','expired','approved') OR (input='' AND output='' AND prompt=''))
);
CREATE INDEX companion_memory_job_origin ON companion_memory_jobs(origin_session,state);
CREATE UNIQUE INDEX companion_memory_one_worker ON companion_memory_jobs(state) WHERE state='running';

CREATE TABLE companion_replay_barriers (
  thread_id TEXT PRIMARY KEY REFERENCES chat_threads(id) ON DELETE CASCADE,
  excluded_through INTEGER NOT NULL,
  revision INTEGER NOT NULL CHECK(revision>=0)
);
