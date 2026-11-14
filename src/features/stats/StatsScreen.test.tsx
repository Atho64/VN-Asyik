import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import { AppContext } from "@/app/AppProvider";
import { useAppStore } from "@/app/store";
import { createTranslator } from "@/i18n";
import { dayIndex } from "@/lib/time";

const mocks = vi.hoisted(() => ({
  statusBreakdown: vi.fn(),
  playtimeByDay: vi.fn(),
  finishedByDay: vi.fn(),
}));

vi.mock("@/lib/ipc/client", () => ({
  api: {
    statusBreakdown: mocks.statusBreakdown,
    playtimeByDay: mocks.playtimeByDay,
    finishedByDay: mocks.finishedByDay,
  },
}));

vi.mock("@/features/stats/OnThisDay", () => ({
  OnThisDay: () => <div data-testid="on-this-day" />,
}));

vi.mock("@/features/stats/TimeStats", () => ({
  TimeStats: () => <div data-testid="time-stats" />,
}));

vi.mock("@/features/stats/AllTimeWrapped", () => ({
  AllTimeWrapped: () => <div data-testid="all-time-wrapped" />,
}));

vi.mock("@/features/stats/YearReview", () => ({
  YearReview: () => <div data-testid="year-review" />,
}));

vi.mock("@/features/stats/TastePanel", () => ({
  TastePanel: () => <div data-testid="taste-panel" />,
}));

const { StatsScreen } = await import("./StatsScreen");

const t = createTranslator("en");

function renderStats() {
  return render(
    <AppContext.Provider value={{ t, locale: "en", ready: true, bootError: null }}>
      <StatsScreen />
    </AppContext.Provider>,
  );
}

describe("StatsScreen", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.statusBreakdown.mockResolvedValue({
      playing: 2,
      wishlist: 1,
      completed: 3,
      on_hold: 0,
      dropped: 0,
    });
    mocks.playtimeByDay.mockResolvedValue([[20_000, 3_600_000]]);
    mocks.finishedByDay.mockResolvedValue([]);
    useAppStore.setState({
      libraryLoaded: true,
      cards: [
        {
          game: {
            meta: { id: "g1", updated_at: 0, device_id_device: "d1", deleted: false },
            title: "Steins;Gate",
            title_original: null,
            vndb_id: "v2002",
            status: "completed",
            rating: 90,
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
          },
          tracked_ms: 7_200_000,
          afk_ms: 0,
          total_ms: 7_200_000,
        },
      ],
    });
  });

  it("organizes sections into Overview, Activity, Recap, and Taste tabs without stacking duplicates", async () => {
    const user = userEvent.setup();
    renderStats();

    expect(await screen.findByTestId("on-this-day")).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Overview" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Most played" })).toBeInTheDocument();
    expect(screen.queryByTestId("time-stats")).toBeNull();
    expect(screen.queryByTestId("year-review")).toBeNull();
    expect(screen.queryByTestId("all-time-wrapped")).toBeNull();
    expect(screen.queryByTestId("taste-panel")).toBeNull();

    const tabs = screen.getByRole("tablist", { name: "Stats views" });

    await user.click(within(tabs).getByRole("tab", { name: "Activity" }));
    expect(screen.getByTestId("time-stats")).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Play days, last 12 months" })).toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: "Overview" })).toBeNull();

    await user.click(within(tabs).getByRole("tab", { name: "Recap" }));
    expect(screen.getByTestId("year-review")).toBeInTheDocument();
    expect(screen.queryByTestId("all-time-wrapped")).toBeNull();

    const recapSwitch = screen.getByRole("tablist", { name: "Recap period" });
    await user.click(within(recapSwitch).getByRole("tab", { name: "All-time" }));
    expect(screen.getByTestId("all-time-wrapped")).toBeInTheDocument();
    expect(screen.queryByTestId("year-review")).toBeNull();

    await user.click(within(tabs).getByRole("tab", { name: "Taste" }));
    expect(screen.getByTestId("taste-panel")).toBeInTheDocument();
  });

  it("marks the days a VN was finished, even without recorded time", async () => {
    const user = userEvent.setup();
    // The component computes the local day index the same way, so the cells
    // are always inside the 53-week window.
    const today = dayIndex(Date.now() - new Date().getTimezoneOffset() * 60_000);
    mocks.playtimeByDay.mockResolvedValue([[today, 3_600_000]]);
    // Yesterday saw a finish but no reading: the mark is the event, not the
    // session it belongs to.
    mocks.finishedByDay.mockResolvedValue([
      [today, 1],
      [today - 1, 1],
    ]);
    const { container } = renderStats();

    await user.click(await screen.findByRole("tab", { name: "Activity" }));
    // The layout lives in the container: pin the attribute, not only text.
    const marked = container.querySelectorAll('.heatmap__cell[data-finished="true"]');
    expect(marked).toHaveLength(2);
    // The zero-time finished day still carries a mark on its level-0 surface,
    // and today's read day is the other marked one.
    const quiet = Array.from(marked).find(
      (cell) => cell.getAttribute("data-level") === "0",
    );
    expect(quiet).not.toBeNull();
    expect(
      Array.from(marked).some((cell) => cell.getAttribute("data-level") !== "0"),
    ).toBe(true);
  });
});
