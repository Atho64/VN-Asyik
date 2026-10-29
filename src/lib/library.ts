/**
 * Library query logic, mirroring `vn_core::library` so the same rules hold when
 * the UI filters client-side (instant search) instead of re-querying.
 */

import type { Game, GameStatus } from "@/lib/ipc/types";

export type SortKey = "recent" | "title" | "rating" | "playtime" | "started";

/** Which of the three library surfaces is showing. */
export type LibraryView = "grid" | "list" | "shelf";

export interface LibraryFilter {
  query: string;
  statuses: GameStatus[];
  ratedOnly: boolean;
}

export const emptyFilter: LibraryFilter = {
  query: "",
  statuses: [],
  ratedOnly: false,
};

/**
 * Everything that decides what the library shows: the search text, the status
 * chips, the rated-only toggle, the view and the sort.
 *
 * It lives in the app store, not in `LibraryScreen`, because the router unmounts
 * the screen on every navigation. Search "fate", open a game or another menu,
 * come back, and the query (with the results it produced) was gone — the state
 * died with the component, exactly as the VNDB detail cache once did.
 */
export interface LibraryBrowse extends LibraryFilter {
  view: LibraryView;
  sort: SortKey;
}

/** The library as it opens: no filter, grid, newest first. */
export const initialLibraryBrowse: LibraryBrowse = {
  query: "",
  statuses: [],
  ratedOnly: false,
  view: "grid",
  sort: "recent",
};

export const statusOrder: GameStatus[] = [
  "playing",
  "wishlist",
  "completed",
  "on_hold",
  "dropped",
];

/** Lowercase, trim, collapse whitespace. Matches the Rust implementation. */
export function normalizeTitle(text: string): string {
  return text.trim().toLowerCase().replace(/\s+/g, " ");
}

const CJK = /[぀-ヿ㐀-䶿一-鿿豈-﫿가-힯]/;

/**
 * Fold a title for comparison, mirroring `vn_core::search::fold`.
 *
 * Visual novel titles vary in ways a plain lowercase comparison misses:
 * full-width Latin (`ＣＬＡＮＮＡＤ`), diacritics, and decorative punctuation
 * (`9-nine-` vs `9 nine`). Everything non-alphanumeric becomes a single space,
 * so `CLANNAD -Hikari-` still matches `clannad hikari`. Kana and CJK ideographs
 * are kept because they are meaningful, not decoration.
 */
export function foldTitle(text: string): string {
  let out = "";
  let lastWasSpace = true;
  for (const raw of text) {
    // Full-width ASCII maps onto its half-width form first.
    const code = raw.charCodeAt(0);
    const ch = (
      code >= 0xff01 && code <= 0xff5e ? String.fromCharCode(code - 0xfee0) : raw
    ).toLowerCase();
    // Kana and CJK must never be decomposed: NFD splits ド into ト + dakuten, so
    // stripping marks would silently rewrite the title. Script passes through
    // whole, before any normalisation.
    if (CJK.test(ch)) {
      out += ch;
      lastWasSpace = false;
      continue;
    }
    for (const part of ch.normalize("NFD")) {
      // Strip combining accents; the base letter carries the meaning.
      if (/\p{M}/u.test(part)) continue;
      if (/[a-z0-9]/.test(part)) {
        out += part;
        lastWasSpace = false;
      } else if (!lastWasSpace) {
        out += " ";
        lastWasSpace = true;
      }
    }
  }
  return out.replace(/\s+$/, "");
}

/** How well a title answers a query; higher is better. Mirrors the Rust enum. */
export type SearchMatch = "none" | "words" | "contains" | "prefix" | "exact";

const MATCH_SCORE: Record<SearchMatch, number> = {
  none: 0,
  words: 2,
  contains: 3,
  prefix: 4,
  exact: 5,
};

/** True when the query matches anything at all. */
export function matchesQuery(game: Game, query: string): boolean {
  return classifyTitle(query, game.title, game.title_original) !== "none";
}

/**
 * Classify a query against a title, treating the original-script title as
 * equally valid. Mirrors `vn_core::search::classify`.
 */
export function classifyTitle(
  query: string,
  title: string,
  altTitle?: string | null,
): SearchMatch {
  const needle = foldTitle(query);
  if (needle.length === 0) return "words";
  let best: SearchMatch = "none";
  for (const candidate of [title, altTitle]) {
    if (!candidate) continue;
    const grade = classifyOne(needle, foldTitle(candidate));
    if (MATCH_SCORE[grade] > MATCH_SCORE[best]) best = grade;
    if (best === "exact") break;
  }
  return best;
}

function classifyOne(needle: string, hay: string): SearchMatch {
  if (hay.length === 0 || needle.length === 0) return "none";
  if (hay === needle) return "exact";
  if (hay.startsWith(needle)) return "prefix";
  const words = needle.split(" ").filter(Boolean);
  if (words.length > 1) {
    const hayWords = hay.split(" ").filter(Boolean);
    const matched = words.filter((word) =>
      hayWords.some((hw) => hw.startsWith(word)),
    ).length;
    if (matched === words.length) return "words";
    // A partial word match is not a hit: typing "game 4990" must not return
    // every title containing "game". Only a contiguous run counts.
    return hay.includes(needle) ? "contains" : "none";
  }
  if (hay.includes(needle)) return "contains";
  return "none";
}

/**
 * Rank games for a query, best match first, then by title.
 *
 * Used where order matters (the command palette). The library grid keeps its
 * own sort key, so this is only applied when the user is actively searching.
 */
export function rankGames(games: Game[], query: string): Game[] {
  const needle = query.trim();
  if (needle.length === 0) return games;
  return games
    .map((game) => ({
      game,
      score: MATCH_SCORE[classifyTitle(needle, game.title, game.title_original)],
    }))
    .filter((entry) => entry.score > 0)
    .sort((a, b) => b.score - a.score || a.game.title.localeCompare(b.game.title))
    .map((entry) => entry.game);
}

/// Cards are what the library renders, so the filter works on them directly.
export function filterCards<T extends { game: Game }>(
  cards: T[],
  filter: LibraryFilter,
): T[] {
  return cards.filter(({ game }) => {
    if (game.meta.deleted) return false;
    if (!matchesQuery(game, filter.query)) return false;
    if (filter.statuses.length > 0 && !filter.statuses.includes(game.status)) {
      return false;
    }
    if (filter.ratedOnly && game.rating === null) return false;
    return true;
  });
}

/// Sort entries by key, descending by default because "recent" and "rating" both
/// read best newest/highest first.
export function sortCards<T extends { game: Game; total_ms: number }>(
  entries: T[],
  key: SortKey,
  descending = true,
): T[] {
  const sorted = [...entries].sort((a, b) => {
    switch (key) {
      case "title":
        return normalizeTitle(a.game.title).localeCompare(normalizeTitle(b.game.title));
      case "rating":
        return (a.game.rating ?? 0) - (b.game.rating ?? 0);
      case "playtime":
        return a.total_ms - b.total_ms;
      case "started":
        return (a.game.started_at ?? 0) - (b.game.started_at ?? 0);
      default:
        return a.game.meta.updated_at - b.game.meta.updated_at;
    }
  });
  return descending ? sorted.reverse() : sorted;
}

export function isFilterActive(filter: LibraryFilter): boolean {
  return filter.query.trim().length > 0 || filter.statuses.length > 0 || filter.ratedOnly;
}

export function sortGames<T extends { game: Game; total_ms: number }>(
  entries: T[],
  key: SortKey,
  descending: boolean,
): T[] {
  const sorted = [...entries].sort((a, b) => {
    switch (key) {
      case "recent":
        return a.game.meta.updated_at - b.game.meta.updated_at;
      case "title":
        return normalizeTitle(a.game.title).localeCompare(normalizeTitle(b.game.title));
      case "rating":
        return (a.game.rating ?? 0) - (b.game.rating ?? 0);
      case "playtime":
        return a.total_ms - b.total_ms;
      case "started":
        return (a.game.started_at ?? 0) - (b.game.started_at ?? 0);
      default:
        return 0;
    }
  });
  return descending ? sorted.reverse() : sorted;
}

/** Ratings are stored in decivotes: 10 is 1.0, 75 is 7.5, 100 is 10.0. */
export const RATING_MIN = 10;
export const RATING_MAX = 100;

/** Format a stored decivote as the number the user thinks in, e.g. "7.5". */
export function formatRating(decivotes: number): string {
  return (decivotes / 10).toFixed(1);
}

/**
 * Explicit art policy check. `blur` and `hide` both treat explicit art as
 * hidden; the difference is only how it is presented.
 *
 * Level 1 is "the community voted this explicit at all", which is the point a
 * cover should stop being shown in full. Requiring 2 meant only a unanimous
 * vote counted, so the filter did nothing for the ordinary case: Sanoba Witch
 * is rated 0.93 and was treated as clear.
 *
 * The same threshold applies to a screenshot, which is rated per image: a tile
 * and a cover at level 1 are concealed together. See
 * `EXPLICIT_FLAG_LEVEL` in the backend (`vn_core::model`), which this mirrors —
 * that duplication is deliberate because the browser cannot read the Rust
 * constant, and `library.test.ts` pins the two to the same value.
 */
export const EXPLICIT_FLAG_LEVEL = 1;

/** Whether one stored flag level counts as explicit. `null` is never explicit. */
export function isExplicitAtLevel(level: number | null | undefined): boolean {
  return level !== null && level !== undefined && level >= EXPLICIT_FLAG_LEVEL;
}

/**
 * Round a VNDB average vote in hundredths (0..200) onto the 0..2 flag level the
 * database stores.
 *
 * Rounds, not floors, and mirrors `flag_level` in `vn-core`/`vndb::dto` exactly. This
 * is the browser half of a pair that must not disagree: the backend decides what to
 * conceal for a cached cover and a cached screenshot, and the search sheet decides for
 * a hit that has not been imported yet. A `>= 100` test here would call 0.99 explicit
 * in Rust and clear in the sheet, which is the mismatch this exists to prevent.
 */
export function flagLevelFromVote(hundredths: number | null | undefined): number {
  if (hundredths === null || hundredths === undefined || !Number.isFinite(hundredths)) {
    return 0;
  }
  return Math.floor((Math.min(Math.max(hundredths, 0), 200) + 50) / 100);
}

export function isExplicit(game: Game): boolean {
  // Level 1 is suggestive rather than outright explicit, and it is still
  // concealed: the setting exists so a reader is not shown adult art without
  // asking, and a suggestive cover is that.
  return isExplicitAtLevel(game.cover_sexual) || isExplicitAtLevel(game.cover_violence);
}

export function needsReveal(game: Game, mode: string): boolean {
  if (mode === "show") return false;
  return isExplicit(game) && Boolean(game.cover_url);
}
