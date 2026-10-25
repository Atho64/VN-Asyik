# VN Asyik

A visual novel library and playtime tracker for Windows and Android. It reads
from VNDB, keeps everything in a local SQLite database, and works with no
network. Rust and Tauri on the backend, React on the front.

![VN Asyik](src/assets/vn-asyik.svg)

## What it does

**Library.** Search VNDB and import a game with its cover, tags, ratings and
screenshot gallery. Browse as a grid, list or shelf; all three are virtualized.
Explicit covers stay concealed behind a neutral surface until you ask to see
one, and changing that setting conceals them again.

**Playtime.** Start a game from the app and it records how long you actually
read. Time counts only while the game is in front of you; breaks longer than a
configurable threshold are filed as away time instead of playtime, and stored
separately so you can see both. Sessions are written every 20 seconds, so a
crash or a forced reboot costs you seconds, not an evening.

**Chapters and endings.** Routes are yours: you name them, and endings hang off
them. Chapter detection reads engine state where a verified adapter exists and
falls back to game-level attribution when it does not, which is the honest
answer for a game whose build nobody has analyzed.

**Stats.** Playtime by game, route and month, from the sessions themselves.

**Discord Rich Presence.** Optional, off by default. Shows what you are reading,
rotating through whatever fields you configure. Explicit cover art is blurred
before it is ever sent anywhere, if you opt into an image host at all.

**Android.** Playtime comes from `UsageStats` foreground events, which needs the
Usage Access permission. Android cannot see inside another app's windows, so
chapters there are set by hand and the app says so rather than guessing.

## Companion chat

The chat is not a general assistant bolted onto a library. It can read two
things and cite them:

- **Live text.** Text LunaTranslator extracted from the game you are playing
  right now.
- **Imported scripts.** Drop in a script you extracted yourself — plain text, a
  name/message JSON array, or the Asyik JSON format — and the app compacts it
  into a story digest it can answer from.

Compaction is a map-reduce over the script split into byte-budgeted chunks. Up
to 10 agents summarize contiguous segments in parallel, each carrying the tail
of its own previous outline for continuity, then their segment digests merge
into one. Progress is saved per chunk, so closing the app mid-run and coming
back resumes where it stopped instead of starting over.

Digests are withheld unless the chat's spoiler level is set to _All_. Live text
is not, because you already read it.

## What it will not do

Some of this is a platform limit and some is a decision, but either way the app
tells you rather than pretending.

- **No chapter detection on Android.** The OS does not let one app inspect
  another's window. Chapter and route are manual there.
- **The app never writes to a running game.** It does not patch files, re-inject
  translated text, or touch DRM.
- **No telemetry.** No analytics, no crash reporting, no phone-home. The only
  network calls are the ones you turn on: VNDB, Discord, your own chat provider,
  and Tavily if you ask for a recommendation that needs the open web.

## Install

Builds are produced locally; there is no signed public release yet.

```
pnpm install --frozen-lockfile
pnpm tauri build
```

Output lands in `src-tauri/target/release/bundle/`. Android needs JDK 21, the
Android SDK and NDK, and the Android Rust target:

```
pnpm tauri android build --apk --target aarch64
```

The application ID `app.vntracker.desktop` is deliberately not renamed: changing
it would orphan an existing library and its stored credentials. The product name
you see everywhere is VN Asyik.

## Development

```
pnpm install --frozen-lockfile
pnpm tauri dev
```

```
pnpm lint && pnpm typecheck && pnpm test
cargo fmt --check && cargo clippy --all-targets -- -D warnings
cargo test --workspace
```

The live tracking tests touch real windows and real idle time, so they are
ignored by default:

```
cargo test --test tracking_live -- --ignored --nocapture
```

Android compile check, which catches code that leaks out from behind a
`#[cfg]` gate:

```
cargo check --target aarch64-linux-android -p vn-tracker
```

## Reading the code

`crates/vn-core` is the place to start: it holds the pure domain logic — the
session state machine, sync merge, chapter mapping, script compaction — with no
Tauri and no OS calls, so it tests with plain `cargo test`. `src-tauri/src` wraps
that in commands, storage and platform adapters. `src/` is the React frontend and
talks to the backend only through `src/lib/ipc`.

Comments in the code explain _why_ a rule exists where the rule is not obvious,
so the reasoning travels with the code rather than living in a separate
document.

## Prior art and attribution

Each entry below says what it actually gave this project. _Ported_ means code or
constants were copied and carry provenance comments in the source. _Inspiration_
means the project was read to understand a problem; nothing was copied, and no
obligation attaches to an idea.

**[VaternID/EwE](https://github.com/VaternID/EwE) — GPL-3.0 — ported.**
The Discord presence card is modelled on EwE. `crates/vn-core/src/presence.rs`
records what was taken: the per-field text transform pipeline (remove, replace,
prefix/suffix, trim, in that order) and the independent rotation cursor per
field. What was deliberately not taken is recorded too — EwE's custom Python
variable scripts, which execute arbitrary code from a directory beside the
executable, and its upload of cover art without a separate opt-in.

**[dead8309/Kizzy](https://github.com/dead8309/Kizzy) — GPL-3.0 — inspiration.**
The reference for talking to Discord on Android, where the desktop IPC socket
does not exist. The Android presence path connects to the Discord Gateway
directly instead, which is the approach Kizzy demonstrated. No code is shared.

**[Yukin0a/KireiBox](https://github.com/Yukin0a/KireiBox) — GPL-3.0 — inspiration.**
A reference for how Android VN launchers approach externally installed engines.
Its README draws the line between bundled engine runners and launching a
separately installed app; VN Asyik takes the second path and bundles nothing.

**[LunaTranslator](https://github.com/HIllya51/LunaTranslator) — GPL-3.0 — integrated.**
Connected through its documented network service: the websocket endpoint streams
extracted text as plain frames, which the app consumes. No code is copied; the
integration is a client for a documented API.

**LunaBox — AGPL-3.0 — inspiration only.**
Read as prior art for what an AI feature over a VN library could present, and for
which failure modes matter (spoilers, invented conclusions). Ideas and functional
behaviour are not copyrightable, so this created no obligation either way.

## License

GPL-3.0-only. See [LICENSE](LICENSE).
