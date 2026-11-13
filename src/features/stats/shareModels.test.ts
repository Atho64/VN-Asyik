import { describe, expect, it } from "vitest";

import { createTranslator } from "@/i18n";
import type { LibraryWrapped, YearReview } from "@/lib/ipc/types";
import { MS_PER_DAY } from "@/lib/time";
import { allTimeShareModel, yearShareModel } from "./shareModels";

const review: YearReview = {
  year: 2024,
  totalMs: 7_200_000,
  afkMs: 0,
  sessionCount: 2,
  activeDays: 1,
  endingsReached: 1,
  journalEntries: 3,
  gamesStarted: 1,
  gamesFinished: 1,
  avgRatingDecivotes: 85,
  longestSession: { gameId: "g1", title: "Autumn VN", countedMs: 2_400_000 },
  firstGame: { gameId: "g1", title: "Autumn VN", startedAt: 1 },
  topGames: [{ gameId: "g1", title: "Autumn VN", countedMs: 3_600_000 }],
  busiestMonth: { month: 5, countedMs: 3_600_000 },
  busiestDay: {
    day: Math.floor(Date.UTC(2024, 4, 10) / MS_PER_DAY),
    countedMs: 3_600_000,
  },
  averageSessionMs: 3_600_000,
  longestStreakDays: 3,
  topRoute: { name: "Shiroha", countedMs: 2_400_000 },
  gamesContinued: 1,
  weekdayMs: [0, 0, 0, 0, 0, 3_600_000, 0],
  hourMs: Array.from({ length: 24 }, (_, index) =>
    index === 20 ? 3_600_000 : 0,
  ),
};

const wrapped: LibraryWrapped = {
  totalMs: 6_000_000,
  afkMs: 0,
  sessionCount: 4,
  activeDays: 4,
  gamesTracked: 2,
  gamesFinished: 1,
  gamesInLibrary: 3,
  endingsReached: 1,
  journalEntries: 2,
  averageRatingDecivotes: 85,
  averageSessionMs: 1_500_000,
  longestSession: { gameId: "g1", title: "Long VN", countedMs: 2_400_000 },
  longestStreakDays: 3,
  firstGame: { gameId: "g1", title: "Long VN", startedAt: 1 },
  firstYear: 2023,
  topGames: [{ gameId: "g1", title: "Long VN", countedMs: 3_600_000 }],
  topRoute: { name: "Shiroha", countedMs: 3_600_000 },
  busiestMonth: { month: 6, countedMs: 6_000_000 },
  busiestDay: { day: 20_000, countedMs: 2_400_000 },
  mostReadYear: { year: 2023, countedMs: 3_600_000 },
  weekdayMs: [0, 0, 0, 0, 0, 6_000_000, 0],
  hourMs: Array.from({ length: 24 }, (_, index) =>
    index === 12 ? 6_000_000 : 0,
  ),
};

const t = createTranslator("en");

describe("year share model", () => {
  it("carries the year, figures, facts and file stem", () => {
    const model = yearShareModel(review, t, "en", "#8b7cf6", null);
    expect(model.badge).toBe("2024");
    expect(model.eyebrow).toBe("YEAR IN REVIEW");
    expect(model.filename).toBe("vn-asyik-2024-wrapped");
    expect(model.subline).toBe("1 day · 2 sessions");
    expect(model.figures).toEqual([
      { value: "1", label: "Endings reached" },
      { value: "1", label: "Games finished" },
      { value: "3 days", label: "Longest streak" },
    ]);
    expect(model.facts).toEqual([
      { label: "Longest session", value: "40m" },
      { label: "Busiest month", value: "May" },
      { label: "Longest route", value: "Shiroha" },
      { label: "Busiest day", value: "10 May" },
    ]);
    expect(model.hourPeak).toBe("Most around 20:00 · 1h");
  });

  it("falls back to the no-route fact", () => {
    const model = yearShareModel(
      { ...review, topRoute: null },
      t,
      "en",
      "#8b7cf6",
      null,
    );
    expect(model.facts[2]).toEqual({
      label: "Longest route",
      value: "No route recorded",
    });
  });
});

describe("all-time share model", () => {
  it("carries the all-time badge, counts and file stem", () => {
    const model = allTimeShareModel(wrapped, t, "en", "#8b7cf6", null);
    expect(model.badge).toBe("ALL TIME");
    expect(model.eyebrow).toBe("ALL-TIME WRAPPED");
    expect(model.filename).toBe("vn-asyik-all-time-wrapped");
    expect(model.subline).toBe("Since 2023 · 2 games · 4 sessions");
    expect(model.figures).toEqual([
      { value: "4", label: "Days read" },
      { value: "1", label: "Endings reached" },
      { value: "1", label: "Games finished" },
    ]);
    expect(model.facts).toEqual([
      { label: "Longest session", value: "40m" },
      { label: "Longest streak", value: "3 days" },
      { label: "Longest route", value: "Shiroha" },
      { label: "Most read year", value: "2023" },
    ]);
    expect(model.hourPeak).toBe("Most around 12:00 · 1h 40m");
  });

  it("uses the beginning when the first year is unknown", () => {
    const model = allTimeShareModel(
      { ...wrapped, firstYear: null },
      t,
      "en",
      "#8b7cf6",
      null,
    );
    expect(model.subline).toContain("Since the beginning");
  });
});
