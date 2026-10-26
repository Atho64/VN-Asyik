-- Walkthrough tables: stores parsed walkthroughs from Seiya-Saiga (or manual entries)
-- and their translations (AI / Google Translate).

CREATE TABLE IF NOT EXISTS game_walkthroughs (
    game_id TEXT PRIMARY KEY REFERENCES games(id) ON DELETE CASCADE,
    source_url TEXT NOT NULL,
    title TEXT NOT NULL,
    notes TEXT,
    routes_json TEXT NOT NULL,
    checked_items_json TEXT NOT NULL DEFAULT '[]',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS walkthrough_translations (
    game_id TEXT NOT NULL REFERENCES games(id) ON DELETE CASCADE,
    provider TEXT NOT NULL, -- 'ai' | 'google'
    target_lang TEXT NOT NULL, -- 'en' | 'id'
    routes_json TEXT NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (game_id, provider, target_lang)
);

CREATE INDEX IF NOT EXISTS idx_walkthrough_translations_game
    ON walkthrough_translations(game_id);
