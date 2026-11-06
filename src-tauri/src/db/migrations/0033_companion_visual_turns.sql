-- Local provenance only. No pixels, window titles, handles, paths or hashes.
CREATE TABLE companion_visual_turns (
    message_id TEXT PRIMARY KEY NOT NULL REFERENCES chat_messages(id) ON DELETE CASCADE,
    session_id TEXT NOT NULL REFERENCES companion_sessions(id) ON DELETE CASCADE,
    playthrough_id TEXT NOT NULL,
    origin TEXT NOT NULL CHECK(origin IN ('manual_image','automatic_reaction')),
    generation INTEGER NOT NULL CHECK(generation>=0),
    sequence INTEGER NOT NULL CHECK(sequence>=0),
    captured_at INTEGER NOT NULL,
    width INTEGER NOT NULL CHECK(width BETWEEN 1 AND 1280),
    height INTEGER NOT NULL CHECK(height BETWEEN 1 AND 1280),
    crop_json TEXT NOT NULL CHECK(length(crop_json)<=160),
    knowledge TEXT NOT NULL CHECK(knowledge IN ('observed','all')),
    provider TEXT NOT NULL CHECK(provider IN ('openai','anthropic')),
    model TEXT NOT NULL CHECK(length(model)<=120),
    status TEXT NOT NULL CHECK(status IN ('running','complete','silent','cancelled','error')),
    description TEXT NOT NULL CHECK(length(description)<=240)
);
CREATE INDEX idx_companion_visual_session ON companion_visual_turns(session_id);
