import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { AppContext } from "@/app/AppProvider";
import { createTranslator } from "@/i18n";
import { MS_PER_DAY } from "@/lib/time";
import type * as ShareCardCanvasModule from "./shareCardCanvas";
import { YearReview, yearsWithDays } from "./YearReview";

const mocks = vi.hoisted(() => ({
  yearReview: vi.fn(),
  saveShareCard: vi.fn(),
  shareShareCard: vi.fn(),
}));
vi.mock("@/lib/ipc/client", () => ({ api: mocks }));
// jsdom has no canvas implementation; the card's drawing is covered by the pure
// helper tests, while this suite only checks the dialog wiring.
vi.mock("./shareCardCanvas", async (importOriginal) => {
  const actual = await importOriginal<typeof ShareCardCanvasModule>();
  return {
    ...actual,
    drawShareCard: vi.fn().mockResolvedValue(undefined),
    shareCardDataUrl: vi.fn(() => "data:image/png;base64,AAAA"),
    shareCardFilename: vi.fn(
      (model: { filename: string }, format: string) =>
        format === "story"
          ? `${model.filename}-story.png`
          : `${model.filename}.png`,
    ),
  };
});

describe("yearsWithDays", () => {
  it("collects the years of recorded days plus the current one, newest first", () => {
    const day2024 = Math.floor(Date.UTC(2024, 4, 10) / MS_PER_DAY);
    const day2023 = Math.floor(Date.UTC(2023, 6, 1) / MS_PER_DAY);
    expect(yearsWithDays([{ day: day2024 }, { day: day2023 }], 2026)).toEqual([
      2026, 2024, 2023,
    ]);
  });

  it("offers the current year even with no recorded days", () => {
    expect(yearsWithDays([], 2026)).toEqual([2026]);
  });

  it("does not duplicate a current year that already has days", () => {
    const day2026 = Math.floor(Date.UTC(2026, 0, 15) / MS_PER_DAY);
    expect(yearsWithDays([{ day: day2026 }], 2026)).toEqual([2026]);
  });
});

const review2024 = {
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

function mount(byDay: Array<{ day: number }>) {
  render(
    <AppContext.Provider
      value={{ t: createTranslator("en"), locale: "en", ready: true, bootError: null }}
    >
      <YearReview byDay={byDay} />
    </AppContext.Provider>,
  );
}

// Clear calls, not implementations: the share-card mock's resolved promise must
// survive between tests, or the dialog's draw effect calls `.catch` on undefined.
beforeEach(() => vi.clearAllMocks());

describe("year in review", () => {
  it("renders the figures of the selected year", async () => {
    // The card opens on the current year; the fixture answers for whatever
    // year is asked, and the switch test pins the exact chip → year contract.
    mocks.yearReview.mockResolvedValueOnce(review2024);
    mount([{ day: Math.floor(Date.UTC(2024, 4, 10) / MS_PER_DAY) }]);
    expect(await screen.findByText("Reading time")).toBeInTheDocument();
    expect(screen.getByText("2h")).toBeInTheDocument();
    expect(screen.getByText("Days read")).toBeInTheDocument();
    // The game title appears as the longest-session hint, the first-game line
    // and the top-games row.
    expect(screen.getAllByText("Autumn VN").length).toBeGreaterThan(0);
    expect(screen.getByText("8.5")).toBeInTheDocument();
    expect(mocks.yearReview).toHaveBeenCalledWith(expect.any(Number));
  });

  it("switches years through the chips and refetches", async () => {
    const user = userEvent.setup();
    const day2024 = Math.floor(Date.UTC(2024, 4, 10) / MS_PER_DAY);
    const day2023 = Math.floor(Date.UTC(2023, 6, 1) / MS_PER_DAY);
    mocks.yearReview.mockResolvedValue(review2024);
    mount([{ day: day2024 }, { day: day2023 }]);
    await screen.findByText("Reading time");
    await user.click(screen.getByRole("tab", { name: "2023" }));
    await waitFor(() => {
      expect(mocks.yearReview).toHaveBeenLastCalledWith(2023);
    });
  });

  it("reports the failure as an alert instead of an empty card", async () => {
    mocks.yearReview.mockRejectedValueOnce(new Error("ipc"));
    mount([]);
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "The year could not be loaded",
    );
    expect(screen.queryByText("Reading time")).not.toBeInTheDocument();
  });

  it("says the year is empty when nothing was recorded", async () => {
    mocks.yearReview.mockResolvedValueOnce({
      ...review2024,
      totalMs: 0,
      sessionCount: 0,
      activeDays: 0,
      endingsReached: 0,
      journalEntries: 0,
      gamesStarted: 0,
      gamesFinished: 0,
      avgRatingDecivotes: null,
      longestSession: null,
      firstGame: null,
      topGames: [],
      busiestMonth: null,
      busiestDay: null,
      averageSessionMs: null,
      longestStreakDays: 0,
      topRoute: null,
      gamesContinued: 0,
      weekdayMs: [0, 0, 0, 0, 0, 0, 0],
      hourMs: Array.from({ length: 24 }, () => 0),
    });
    mount([]);
    expect(await screen.findByText("Nothing recorded in 2024 yet.")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Share" })).not.toBeInTheDocument();
  });

  it("opens the share card for a year with recorded time", async () => {
    const user = userEvent.setup();
    mocks.yearReview.mockResolvedValueOnce(review2024);
    mount([{ day: Math.floor(Date.UTC(2024, 4, 10) / MS_PER_DAY) }]);
    await screen.findByText("Reading time");
    await user.click(screen.getByRole("button", { name: "Share" }));
    expect(await screen.findByRole("dialog")).toBeInTheDocument();
    expect(
      screen.getByRole("img", { name: "Share 2024 in review" }),
    ).toBeInTheDocument();
  });

  it("saves the card through the PNG command", async () => {
    const user = userEvent.setup();
    mocks.yearReview.mockResolvedValueOnce(review2024);
    mocks.saveShareCard.mockResolvedValueOnce(true);
    mount([{ day: Math.floor(Date.UTC(2024, 4, 10) / MS_PER_DAY) }]);
    await screen.findByText("Reading time");
    await user.click(screen.getByRole("button", { name: "Share" }));
    await user.click(await screen.findByRole("button", { name: "Save image" }));
    await waitFor(() => {
      expect(mocks.saveShareCard).toHaveBeenCalledWith(
        "data:image/png;base64,AAAA",
        "vn-asyik-2024-wrapped.png",
      );
    });
    expect(await screen.findByRole("status")).toHaveTextContent("Image saved.");
  });

  it("saves the story format with its own file name", async () => {
    const user = userEvent.setup();
    mocks.yearReview.mockResolvedValueOnce(review2024);
    mocks.saveShareCard.mockResolvedValueOnce(true);
    mount([{ day: Math.floor(Date.UTC(2024, 4, 10) / MS_PER_DAY) }]);
    await screen.findByText("Reading time");
    await user.click(screen.getByRole("button", { name: "Share" }));
    await user.click(await screen.findByRole("tab", { name: "Story (9:16)" }));
    await user.click(await screen.findByRole("button", { name: "Save image" }));
    await waitFor(() => {
      expect(mocks.saveShareCard).toHaveBeenCalledWith(
        "data:image/png;base64,AAAA",
        "vn-asyik-2024-wrapped-story.png",
      );
    });
  });
});
