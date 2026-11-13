import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { AppContext } from "@/app/AppProvider";
import { createTranslator } from "@/i18n";
import type { LibraryWrapped } from "@/lib/ipc/types";
import { AllTimeWrapped } from "./AllTimeWrapped";
import type * as ShareCardCanvasModule from "./shareCardCanvas";

const mocks = vi.hoisted(() => ({ libraryWrapped: vi.fn() }));
vi.mock("@/lib/ipc/client", () => ({ api: mocks }));
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

function mount() {
  render(
    <AppContext.Provider
      value={{ t: createTranslator("en"), locale: "en", ready: true, bootError: null }}
    >
      <AllTimeWrapped />
    </AppContext.Provider>,
  );
}

beforeEach(() => vi.clearAllMocks());

describe("all-time wrapped", () => {
  it("renders the library-wide figures", async () => {
    mocks.libraryWrapped.mockResolvedValueOnce(wrapped);
    mount();
    expect(await screen.findByText("All-time wrapped")).toBeInTheDocument();
    expect(screen.getByText("Since 2023 · 2 games · 4 sessions")).toBeInTheDocument();
    // The hero total and the busiest-month hint both read 1h 40m.
    expect(screen.getAllByText("1h 40m").length).toBeGreaterThan(0);
    // The title appears as the longest-session hint and in the top-games row.
    expect(screen.getAllByText("Long VN").length).toBeGreaterThan(0);
    expect(screen.getByText("Shiroha")).toBeInTheDocument();
  });

  it("opens the all-time share card", async () => {
    const user = userEvent.setup();
    mocks.libraryWrapped.mockResolvedValueOnce(wrapped);
    mount();
    await screen.findByText("All-time wrapped");
    await user.click(screen.getByRole("button", { name: "Share all-time" }));
    expect(await screen.findByRole("dialog")).toBeInTheDocument();
    expect(
      screen.getByRole("img", { name: "Share all-time wrapped" }),
    ).toBeInTheDocument();
  });

  it("reports a failure instead of an empty card", async () => {
    mocks.libraryWrapped.mockRejectedValueOnce(new Error("ipc"));
    mount();
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "The all-time recap could not be loaded.",
    );
  });
});
