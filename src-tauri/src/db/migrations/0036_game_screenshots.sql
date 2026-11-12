-- Screenshots the reader captured themselves, as opposed to the VNDB gallery,
-- which is a disposable cache of somebody else's images.
--
-- Local-only in sync: the bytes live beside the database and are far too large
-- for the row payload, and a path is meaningless on another device. The row is
-- carried by the library export (bodies inline) so a backup still restores the
-- pictures, but folder sync never sees it.
--
-- `relative_path` is `screenshots/<game_id>/<id>.jpg`, relative to the app data
-- directory. Absolute paths would break the moment the library file moves.
CREATE TABLE game_screenshots (
    id TEXT PRIMARY KEY NOT NULL,
    game_id TEXT NOT NULL REFERENCES games(id) ON DELETE CASCADE,
    relative_path TEXT NOT NULL,
    width INTEGER NOT NULL,
    height INTEGER NOT NULL,
    captured_at INTEGER NOT NULL,
    -- `windows_window` or `android_screen`. Recorded because the two are not
    -- equivalent: one is the game, the other is whatever was on the display.
    source TEXT NOT NULL,
    caption TEXT NOT NULL DEFAULT '',
    deleted INTEGER NOT NULL DEFAULT 0 CHECK (deleted IN (0, 1))
);

-- Newest first, per game: the only order the library ever asks for.
CREATE INDEX idx_game_screenshots_game
    ON game_screenshots (game_id, captured_at DESC, id DESC);
