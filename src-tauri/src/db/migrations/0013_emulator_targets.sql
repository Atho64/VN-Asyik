-- 0013_emulator_targets.sql
--
-- Emulators and VN runners host many games in one binary or package, so an
-- executable row needs to say *which game inside the host* it stands for.
--
-- Additive only: four columns on `game_executables`, no table rebuild. A rebuild
-- would need `-- no-transaction` and `PRAGMA foreign_keys = OFF` (AGENTS.md §5),
-- and `0011` showed what a careless one costs. `ADD COLUMN` with a constant
-- default is safe in SQLite and leaves every existing row meaning exactly what it
-- meant before: `runner_kind = 'native'`, one path, one game.
--
-- Migrations 0001-0012 are applied and must not be edited.

-- What runs the game. The stored strings are `vn_core::emulator::RunnerKind`.
ALTER TABLE game_executables ADD COLUMN runner_kind TEXT NOT NULL DEFAULT 'native';

-- What the host is told to open: a ROM or ISO path, a title id (PCSG00123), or a
-- game folder inside Winlator. NULL for native games.
ALTER TABLE game_executables ADD COLUMN launch_target TEXT;

-- Text to look for in the host's window caption to recognise this game while
-- several share one emulator. NULL falls back to a name derived from
-- `launch_target`; with neither, the game is never attributed by caption.
ALTER TABLE game_executables ADD COLUMN title_match TEXT;

-- Optional JSON for host-specific options (a Winlator container id, a locale).
ALTER TABLE game_executables ADD COLUMN extra_args TEXT;

-- Shared hosts are looked up by (executable, target); this keeps that lookup off
-- a full scan once a library holds hundreds of emulated games.
CREATE INDEX idx_exec_runner ON game_executables(runner_kind, launch_target) WHERE deleted = 0;
