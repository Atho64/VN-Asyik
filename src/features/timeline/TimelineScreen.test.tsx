import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { StrictMode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { MemoryRouter } from "react-router";

import { AppContext } from "@/app/AppProvider";
import { useAppStore } from "@/app/store";
import { createTranslator } from "@/i18n";
import type { Game, TimelineEvent, TimelinePage } from "@/lib/ipc/types";
import { TimelineScreen } from "./TimelineScreen";

const mocks = vi.hoisted(() => ({ timelinePage: vi.fn(), search: vi.fn() }));
vi.mock("@/lib/ipc/client", () => ({ api: mocks }));
vi.mock("@/lib/ipc/reading", () => ({ readingApi: { search: mocks.search } }));

const game: Game = {
  meta: { id: "g1", updated_at: 1, device_id_device: "test", deleted: false },
  title: "Summer Pockets",
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
};

/** Two events on one local day (2026-11-08 10:00Z and 11:00Z). */
const base = Date.UTC(2026, 10, 8, 10, 0, 0);

const session: TimelineEvent = {
  kind: "session",
  ts: base,
  id: "s1",
  game,
  countedMs: 3_600_000,
  afkMs: 60_000,
  source: "desktop",
  note: "The fireworks scene",
  routeName: null,
  chapterName: null,
  endingName: null,
};

const journal: TimelineEvent = {
  kind: "journal",
  ts: base + 3_600_000,
  id: "j1",
  game,
  countedMs: null,
  afkMs: null,
  source: null,
  note: "Chapter 3 was heavy",
  routeName: "Mio",
  chapterName: "8月7日",
  endingName: null,
};

const page = (events: TimelineEvent[], hasMore: boolean): TimelinePage => ({
  events,
  hasMore,
});

function mount() {
  render(
    <AppContext.Provider
      value={{ t: createTranslator("en"), locale: "en", ready: true, bootError: null }}
    >
      <MemoryRouter>
        <TimelineScreen />
      </MemoryRouter>
    </AppContext.Provider>,
  );
}

beforeEach(() => {
  vi.resetAllMocks();
  useAppStore.setState({ timelineQuery: "" });
});

describe("timeline screen", () => {
  it("renders mixed events of one day under a shared header", async () => {
    mocks.timelinePage.mockResolvedValueOnce(page([session, journal], false));
    mount();
    expect(screen.getByLabelText("Laying out your journey")).toBeInTheDocument();
    expect((await screen.findAllByText("Summer Pockets")).length).toBe(2);
    expect(screen.getByText("Read for 1h")).toBeInTheDocument();
    expect(screen.getByText("Chapter 3 was heavy")).toBeInTheDocument();
    expect(screen.getByText("Mio · 8月7日")).toBeInTheDocument();
    // One local day, one header, two rows.
    expect(screen.getAllByRole("heading", { level: 2 })).toHaveLength(1);
    const links = screen.getAllByRole("link");
    expect(links).toHaveLength(2);
    expect(links[0]).toHaveAttribute("href", "/game/g1");
    // Away time is the row's aside.
    expect(screen.getByText(/Away/)).toBeInTheDocument();
    expect(mocks.timelinePage).toHaveBeenCalledTimes(1);
  });

  it("loads older events onto the same feed using the last row as the cursor", async () => {
    const user = userEvent.setup();
    mocks.timelinePage.mockResolvedValueOnce(page([session], true));
    mount();
    await screen.findByText("Summer Pockets");

    const olderSession: TimelineEvent = {
      ...session,
      id: "s0",
      ts: base - 86_400_000,
      note: "An earlier night",
    };
    mocks.timelinePage.mockResolvedValueOnce(page([olderSession], false));
    await user.click(screen.getByRole("button", { name: "Older events" }));

    await waitFor(() => {
      expect(screen.getByText("An earlier night")).toBeInTheDocument();
    });
    expect(mocks.timelinePage).toHaveBeenLastCalledWith({ ts: base, id: "s1" });
    // A second day header appeared for the older event.
    expect(screen.getAllByRole("heading", { level: 2 })).toHaveLength(2);
    // The feed has no older page left, so the button is gone.
    expect(
      screen.queryByRole("button", { name: "Older events" }),
    ).not.toBeInTheDocument();
  });

  it("shows the empty state when nothing has been recorded yet", async () => {
    mocks.timelinePage.mockResolvedValueOnce(page([], false));
    mount();
    expect(
      await screen.findByText("The journey starts with a first session"),
    ).toBeInTheDocument();
    expect(screen.queryByRole("link")).not.toBeInTheDocument();
  });

  it("recovers with retry when the first page fails", async () => {
    const user = userEvent.setup();
    mocks.timelinePage.mockRejectedValueOnce(new Error("ipc"));
    mount();
    expect(await screen.findByText("The timeline could not be loaded")).toBeInTheDocument();
    mocks.timelinePage.mockResolvedValueOnce(page([journal], false));
    await user.click(screen.getByRole("button", { name: "Try again" }));
    expect(await screen.findByText("Chapter 3 was heavy")).toBeInTheDocument();
  });

  it("keeps the loaded feed and reports the failure when an older page fails", async () => {
    const user = userEvent.setup();
    mocks.timelinePage.mockResolvedValueOnce(page([session], true));
    mount();
    await screen.findByText("Summer Pockets");
    mocks.timelinePage.mockRejectedValueOnce(new Error("ipc"));
    await user.click(screen.getByRole("button", { name: "Older events" }));
    expect(await screen.findByText("The timeline could not be loaded")).toBeInTheDocument();
    expect(screen.getByText("Summer Pockets")).toBeInTheDocument();
    // Retry walks the same cursor again.
    mocks.timelinePage.mockResolvedValueOnce(page([], false));
    await user.click(screen.getByRole("button", { name: "Try again" }));
    await waitFor(() => {
      expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    });
    expect(mocks.timelinePage).toHaveBeenLastCalledWith({ ts: base, id: "s1" });
  });

  it("searches the journal from the timeline and returns to the feed when cleared", async () => {
    const user = userEvent.setup();
    mocks.timelinePage.mockResolvedValue(page([session], false));
    mount();
    await screen.findByText("Read for 1h");

    const otherGame: Game = { ...game, meta: { ...game.meta, id: "g2" }, title: "Clannad" };
    mocks.search.mockResolvedValue({
      results: [
        {
          entry: {
            meta: { id: "j9", updated_at: 1, device_id_device: "test", deleted: false },
            game_id: "g2",
            session_id: null,
            created_at: base,
            played_at: null,
            note: "the fireworks after the festival",
            save_slot: "",
            route_name: "Nagisa",
            chapter_name: "",
            photo_image_id: "",
          },
          game: otherGame,
        },
      ],
      hasMore: false,
    });

    const box = screen.getByLabelText("Search journal");
    await user.type(box, "fireworks");
    // The debounced search lands and replaces the feed.
    expect(await screen.findByText("1 match", {}, { timeout: 2000 })).toBeInTheDocument();
    expect(screen.getByText("Clannad")).toBeInTheDocument();
    expect(screen.queryByText("Read for 1h")).not.toBeInTheDocument();
    // The query survives in the store, so a navigation back keeps the search.
    expect(useAppStore.getState().timelineQuery).toBe("fireworks");

    await user.clear(box);
    expect(await screen.findByText("Read for 1h")).toBeInTheDocument();
    expect(screen.queryByText("1 match")).not.toBeInTheDocument();
  });

  it("loads the feed under StrictMode's double mount instead of stalling", async () => {
    // The app renders in StrictMode. A ref guard that marks the feed "requested"
    // on the first pass would cancel that fetch in cleanup and skip the second,
    // leaving the page stuck on the loading skeleton.
    mocks.timelinePage.mockResolvedValue(page([session], false));
    render(
      <StrictMode>
        <AppContext.Provider
          value={{ t: createTranslator("en"), locale: "en", ready: true, bootError: null }}
        >
          <MemoryRouter>
            <TimelineScreen />
          </MemoryRouter>
        </AppContext.Provider>
      </StrictMode>,
    );
    expect(await screen.findByText("Read for 1h")).toBeInTheDocument();
    expect(screen.queryByLabelText("Laying out your journey")).not.toBeInTheDocument();
  });
});
