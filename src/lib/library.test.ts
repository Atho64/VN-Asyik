import { describe, expect, it } from "vitest";

import {
  classifyTitle,
  emptyFilter,
  EXPLICIT_FLAG_LEVEL,
  filterCards,
  flagLevelFromVote,
  foldTitle,
  formatRating,
  isExplicit,
  isExplicitAtLevel,
  isFilterActive,
  matchesQuery,
  needsReveal,
  normalizeTitle,
  rankGames,
  sortCards,
  statusOrder,
} from "./library";
import type { Game, GameCard } from "./ipc/types";

function makeGame(overrides: Partial<Game> = {}): Game {
  return {
    meta: {
      id: "0192f0aa-0000-7000-8000-000000000001",
      updated_at: 1_000,
      device_id_device: "dev",
      deleted: false,
    },
    title: "Symphonic Rain",
    title_original: null,
    vndb_id: null,
    status: "playing",
    rating: null,
    notes: null,
    length_minutes: null,
    manual_playtime_ms: 0,
    accent_color: null,
    cover_url: null,
    cover_sexual: null,
    cover_violence: null,
    custom_cover_path: null,
    custom_cover_source_vndb: false,
    started_at: null,
    finished_at: null,
    ...overrides,
  };
}

function makeCard(overrides: Partial<Game>, totalMs = 0): GameCard {
  return {
    game: makeGame(overrides),
    tracked_ms: totalMs,
    afk_ms: 0,
    total_ms: totalMs,
  };
}

describe("normalizeTitle", () => {
  it("lowercases and collapses whitespace", () => {
    expect(normalizeTitle("  Little   Busters!  ")).toBe("little busters!");
  });

  it("keeps Japanese text intact", () => {
    expect(normalizeTitle("うみねこのなく頃に")).toBe("うみねこのなく頃に");
  });
});

describe("matchesQuery", () => {
  it("matches an empty query", () => {
    expect(matchesQuery(makeGame(), "")).toBe(true);
    expect(matchesQuery(makeGame(), "   ")).toBe(true);
  });

  it("matches title case-insensitively", () => {
    expect(matchesQuery(makeGame(), "SYMPHONIC")).toBe(true);
    expect(matchesQuery(makeGame(), "rain")).toBe(true);
    expect(matchesQuery(makeGame(), "clannad")).toBe(false);
  });

  it("matches the original title", () => {
    const game = makeGame({ title_original: "シンフォニックレイン" });
    expect(matchesQuery(game, "レイン")).toBe(true);
  });

  it("handles a null original title", () => {
    expect(matchesQuery(makeGame({ title_original: null }), "zzz")).toBe(false);
  });
});

describe("filterCards", () => {
  it("excludes soft-deleted rows", () => {
    const dead = makeGame();
    dead.meta = { ...dead.meta, deleted: true };
    expect(filterCards([{ game: dead, total_ms: 0 }], emptyFilter)).toHaveLength(0);
  });

  it("filters by one or more statuses", () => {
    const games = [
      makeGame({ status: "playing" }),
      makeGame({ status: "completed" }),
      makeGame({ status: "wishlist" }),
    ];
    const cards = games.map((game) => ({ game, total_ms: 0 }));
    expect(filterCards(cards, { ...emptyFilter, statuses: ["playing"] })).toHaveLength(1);
    expect(
      filterCards(cards, { ...emptyFilter, statuses: ["playing", "completed"] }),
    ).toHaveLength(2);
  });

  it("filters to rated games only", () => {
    const cards = [makeGame({ rating: 8 }), makeGame({ rating: null })].map((game) => ({
      game,
      total_ms: 0,
    }));
    expect(filterCards(cards, { ...emptyFilter, ratedOnly: true })).toHaveLength(1);
  });

  it("combines query, status and rating", () => {
    const games = [
      makeGame({ title: "Aria", status: "playing", rating: 9 }),
      makeGame({ title: "Aria", status: "completed", rating: 9 }),
      makeGame({ title: "Other", status: "playing", rating: 9 }),
    ];
    const filtered = filterCards(
      games.map((game) => ({ game, total_ms: 0 })),
      { query: "aria", statuses: ["playing"], ratedOnly: true },
    );
    expect(filtered).toHaveLength(1);
    expect(filtered[0]?.game.status).toBe("playing");
  });
});

describe("formatRating", () => {
  it("reads a decivote as the number the user thinks in", () => {
    // Stored in tenths, because VNDB's vote is 10..=100 on the same scale.
    expect(formatRating(10)).toBe("1.0");
    expect(formatRating(75)).toBe("7.5");
    expect(formatRating(73)).toBe("7.3");
    expect(formatRating(100)).toBe("10.0");
  });

  it("keeps the one decimal even when it is zero", () => {
    // "8" beside a 7.5 would suggest the two are on different scales.
    expect(formatRating(80)).toBe("8.0");
  });
});

describe("isFilterActive", () => {
  it("is inactive for the empty filter", () => {
    expect(isFilterActive(emptyFilter)).toBe(false);
  });

  it("ignores a whitespace-only query", () => {
    expect(isFilterActive({ ...emptyFilter, query: "   " })).toBe(false);
  });

  it("activates on any real criterion", () => {
    expect(isFilterActive({ ...emptyFilter, query: "a" })).toBe(true);
    expect(isFilterActive({ ...emptyFilter, statuses: ["playing"] })).toBe(true);
    expect(isFilterActive({ ...emptyFilter, ratedOnly: true })).toBe(true);
  });
});

describe("sortCards", () => {
  const entries = [
    makeCard(
      { title: "Charlie", rating: 5, updated_at: 300, started_at: 30 } as never,
      300,
    ),
    makeCard(
      { title: "alpha", rating: 9, updated_at: 100, started_at: 10 } as never,
      100,
    ),
    makeCard(
      { title: "Bravo", rating: null, updated_at: 200, started_at: null } as never,
      200,
    ),
  ];

  it("sorts by title case-insensitively", () => {
    const sorted = sortCards(entries, "title", false);
    expect(sorted.map((entry) => entry.game.title)).toEqual([
      "alpha",
      "Bravo",
      "Charlie",
    ]);
  });

  it("sorts by rating with unrated lowest", () => {
    const sorted = sortCards(entries, "rating", true);
    expect(sorted[0]?.game.rating).toBe(9);
    expect(sorted.at(-1)?.game.rating).toBeNull();
  });

  it("sorts by playtime", () => {
    const sorted = sortCards(entries, "playtime", true);
    expect(sorted[0]?.total_ms).toBe(300);
  });

  it("treats a missing start date as zero", () => {
    const sorted = sortCards(entries, "started", false);
    expect(sorted[0]?.game.started_at).toBeNull();
  });

  it("does not mutate the input array", () => {
    const copy = [...entries];
    sortCards(entries, "title", true);
    expect(entries).toEqual(copy);
  });
});

describe("explicit art policy", () => {
  it("treats sexual or violent ratings of 2+ as explicit", () => {
    expect(isExplicit(makeGame({ cover_sexual: 2 }))).toBe(true);
    expect(isExplicit(makeGame({ cover_violence: 3 }))).toBe(true);
    // A single explicit vote level counts. This used to expect false, which let
    // every partially-rated cover through the filter untouched.
    expect(isExplicit(makeGame({ cover_sexual: 1 }))).toBe(true);
    expect(isExplicit(makeGame({ cover_violence: 1 }))).toBe(true);
    expect(isExplicit(makeGame())).toBe(false);
  });

  it("conceals a level-1 cover and a level-1 screenshot together", () => {
    // The divergence this change removed: the library compared `>= 2` while the
    // screenshot gallery compared the rounded level `>= 1`, so one scene could be
    // blurred as a screenshot and shown in full as a cover. Both now resolve through
    // `isExplicitAtLevel`, and the Rust side pins the same constant.
    expect(EXPLICIT_FLAG_LEVEL).toBe(1);
    for (const level of [0, 1, 2, 3]) {
      const concealed = level >= EXPLICIT_FLAG_LEVEL;
      expect(isExplicitAtLevel(level)).toBe(concealed);
    }
    expect(isExplicitAtLevel(null)).toBe(false);
    expect(isExplicitAtLevel(undefined)).toBe(false);
  });

  it("rounds a VNDB vote onto the stored level exactly as Rust does", () => {
    // Mirrors `flag_level` in `vndb::dto`: (hundredths + 50) / 100, rounded down.
    expect(flagLevelFromVote(0)).toBe(0);
    expect(flagLevelFromVote(49)).toBe(0);
    // Sanoba Witch: 0.93 sexual. Flooring left the filter inert for any cover short
    // of a unanimous vote, which is the bug that made 1.5 round down to 1.
    expect(flagLevelFromVote(93)).toBe(1);
    expect(flagLevelFromVote(150)).toBe(2);
    expect(flagLevelFromVote(200)).toBe(2);
    // Out-of-range and missing values are clamped, not trusted.
    expect(flagLevelFromVote(-40)).toBe(0);
    expect(flagLevelFromVote(999)).toBe(2);
    expect(flagLevelFromVote(null)).toBe(0);
    expect(flagLevelFromVote(Number.NaN)).toBe(0);
  });

  it("reveals only when explicit art actually exists", () => {
    const explicitNoArt = makeGame({ cover_sexual: 3 });
    expect(needsReveal(explicitNoArt, "blur")).toBe(false);

    const withArt = makeGame({ cover_sexual: 3, cover_url: "https://x/y.jpg" });
    expect(needsReveal(withArt, "blur")).toBe(true);
    expect(needsReveal(withArt, "hide")).toBe(true);
    expect(needsReveal(withArt, "show")).toBe(false);
  });
});

describe("statusOrder", () => {
  it("lists every status exactly once", () => {
    expect(new Set(statusOrder).size).toBe(statusOrder.length);
    expect(statusOrder).toHaveLength(5);
  });
});

describe("title search", () => {
  const game = (title: string, title_original: string | null = null) =>
    ({ title, title_original }) as unknown as Game;

  it("folds width, case, accents and punctuation", () => {
    expect(foldTitle("CLANNAD")).toBe("clannad");
    expect(foldTitle("ＣＬＡＮＮＡＤ")).toBe("clannad");
    expect(foldTitle("9-nine-")).toBe("9 nine");
    expect(foldTitle("Clannad -Hikari Mimamoru Sakamichi de-")).toBe(
      "clannad hikari mimamoru sakamichi de",
    );
    expect(foldTitle("Café")).toBe("cafe");
    // Script is preserved rather than stripped as punctuation, and voiced kana
    // keep their dakuten: NFD would split ド into ト + a combining mark, and
    // dropping that mark would silently rewrite the title.
    expect(foldTitle("クラナド")).toBe("クラナド");
    expect(foldTitle("ド")).toBe("ド");
    expect(foldTitle("ドラマ")).toBe("ドラマ");
  });

  it("matches the same cases as the Rust ranker", () => {
    expect(classifyTitle("clannad", "CLANNAD")).toBe("exact");
    expect(classifyTitle("clannad", "CLANNAD -Hikari Mimamoru Sakamichi de-")).toBe(
      "prefix",
    );
    expect(classifyTitle("clannad", "QUIZ de CLANNAD")).toBe("contains");
    expect(classifyTitle("naku koro umineko", "Umineko no Naku Koro ni")).toBe("words");
    expect(classifyTitle("zzz", "CLANNAD")).toBe("none");
    expect(classifyTitle("クラナド", "CLANNAD", "CLANNAD -クラナド-")).toBe("contains");
  });

  it("finds a title through its original-script name", () => {
    expect(matchesQuery(game("CLANNAD", "CLANNAD -クラナド-"), "クラナド")).toBe(true);
  });

  it("ignores punctuation differences in the query", () => {
    expect(matchesQuery(game("9-nine- Shinshou"), "9 nine shinshou")).toBe(true);
    expect(matchesQuery(game("D.C.4 ~Da Capo 4~"), "dc4")).toBe(false);
    expect(matchesQuery(game("D.C.4 ~Da Capo 4~"), "da capo 4")).toBe(true);
  });

  it("ranks exact and prefix matches above loose ones", () => {
    const games = [
      game("QUIZ de CLANNAD"),
      game("CLANNAD -Hikari Mimamoru Sakamichi de-"),
      game("CLANNAD"),
    ];
    expect(rankGames(games, "clannad").map((g) => g.title)).toEqual([
      "CLANNAD",
      "CLANNAD -Hikari Mimamoru Sakamichi de-",
      "QUIZ de CLANNAD",
    ]);
  });

  it("does not treat a partly-matching phrase as a hit", () => {
    // Regression: "game 4990" used to match every "Game 499x" title.
    expect(classifyTitle("game 4990", "Game 499")).toBe("none");
    expect(classifyTitle("game 4990", "Game 4991")).toBe("none");
    expect(classifyTitle("game 4990", "Game 4990")).toBe("exact");
    // Single-word prefixes still work.
    expect(classifyTitle("clann", "CLANNAD")).toBe("prefix");
    expect(classifyTitle("hikari", "CLANNAD -Hikari-")).toBe("contains");
    expect(classifyTitle("koro", "Umineko no Naku Koro ni")).toBe("contains");
  });

  it("drops games that do not match at all", () => {
    const games = [game("CLANNAD"), game("Rewrite")];
    expect(rankGames(games, "clannad").map((g) => g.title)).toEqual(["CLANNAD"]);
  });

  it("returns the input untouched for an empty query", () => {
    const games = [game("B"), game("A")];
    expect(rankGames(games, "   ")).toBe(games);
  });
});
