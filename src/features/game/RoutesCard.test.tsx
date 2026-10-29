import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { AppContext } from "@/app/AppProvider";
import { createTranslator } from "@/i18n";
import type {
  ChapterCapabilityView,
  ChapterView,
  Ending,
  ManualTimerView,
  RouteView,
} from "@/lib/ipc/types";

const mocks = vi.hoisted(() => ({
  gameRoutes: vi.fn(),
  trackingManualStatus: vi.fn(),
  trackingChapterCapability: vi.fn(),
  createRoute: vi.fn(),
  renameRoute: vi.fn(),
  mergeRoutes: vi.fn(),
  setChapterRoute: vi.fn(),
  setEndingRoute: vi.fn(),
  trackingManualStart: vi.fn(),
  trackingManualStartRoute: vi.fn(),
  trackingManualStop: vi.fn(),
  trackingRenameChapter: vi.fn(),
  trackingCreateChapter: vi.fn(),
  trackingManualPause: vi.fn(),
  trackingManualResume: vi.fn(),
  createEnding: vi.fn(),
  listEndings: vi.fn(),
  updateEnding: vi.fn(),
  deleteEnding: vi.fn(),
}));

vi.mock("@/lib/ipc/client", () => ({
  IpcError: class extends Error {},
  api: mocks,
}));

const { RoutesCard } = await import("./RoutesCard");
const t = createTranslator("en");

function chapter(overrides: Partial<ChapterView> = {}): ChapterView {
  return {
    id: "c1",
    rawName: "Prologue",
    displayName: "Prologue",
    renamed: false,
    manual: false,
    order: 0,
    routeKey: null,
    routeName: null,
    routeLabel: "",
    reached: false,
    playtimeMs: 0,
    routeId: null,
    ...overrides,
  };
}

function ending(overrides: Partial<Ending> = {}): Ending {
  return {
    meta: {
      id: "e1",
      updated_at: 1,
      device_id_device: "d1",
      deleted: false,
    },
    game_id: "g1",
    name: "Good End",
    route: null,
    route_id: null,
    achieved: false,
    achieved_at: null,
    ...overrides,
  };
}

function route(overrides: Partial<RouteView> = {}): RouteView {
  return {
    id: "r1",
    name: "Shiroha",
    origin: "engine",
    chapters: [chapter()],
    endings: [],
    chaptersReached: 0,
    chaptersTotal: 1,
    endingsReached: 0,
    endingsTotal: 0,
    playtimeMs: 0,
    mergeCandidates: [],
    ...overrides,
  };
}

const capability: ChapterCapabilityView = {
  platform: "windows",
  sources: ["windowTitle"],
  automatic: true,
  engine: "siglus",
  unavailableReason: null,
};

const idleTimer: ManualTimerView = {
  active: false,
  chapterId: null,
  routeId: null,
  chapterName: null,
  elapsedMs: 0,
  activeMs: 0,
  afkMs: 0,
  running: false,
};

function renderCard() {
  return render(
    <AppContext.Provider value={{ t, locale: "en", ready: true, bootError: null }}>
      <RoutesCard gameId="g1" />
    </AppContext.Provider>,
  );
}

beforeEach(() => {
  for (const mock of Object.values(mocks)) mock.mockReset();
  mocks.trackingManualStatus.mockResolvedValue(idleTimer);
  mocks.trackingChapterCapability.mockResolvedValue(capability);
  mocks.listEndings.mockResolvedValue([]);
});

describe("Routes card", () => {
  it("puts a route's chapters and endings under one heading", async () => {
    // The whole point of the merge: one story, one section. Before this a chapter
    // list and an ending list were separate columns grouped two different ways.
    mocks.gameRoutes.mockResolvedValue([
      route({
        chapters: [chapter({ id: "c1", displayName: "Prologue" })],
        endings: [ending({ meta: { ...ending().meta, id: "e1" }, name: "Good End" })],
        endingsTotal: 1,
      }),
    ]);

    renderCard();

    expect(await screen.findByText("Shiroha")).toBeTruthy();
    expect(screen.getByText("Prologue")).toBeTruthy();
    expect(screen.getByText("Good End")).toBeTruthy();
  });

  it("adds a chapter from inside a route and links it to that route", async () => {
    // A route with no chapter has nothing to time, so the card looked like a dead
    // end: a disabled Start button and no way to give the route any content.
    mocks.gameRoutes.mockResolvedValue([route({ id: "r1", chapters: [], chaptersTotal: 0 })]);
    mocks.trackingCreateChapter.mockResolvedValue("c_new");

    renderCard();

    await userEvent.click(await screen.findByText("Add chapter"));
    await userEvent.type(screen.getByPlaceholderText("Chapter name"), "Prologue");
    await userEvent.click(screen.getByRole("button", { name: "Add" }));

    await waitFor(() => {
      // The route id is what puts the chapter on this route; without it the chapter
      // lands in the unlinked bucket and the route stays empty.
      expect(mocks.trackingCreateChapter).toHaveBeenCalledWith(
        "g1",
        "Prologue",
        null,
        "r1",
      );
    });
  });

  it("offers Add chapter in the unlinked bucket too", async () => {
    // A library with no routes yet needs a way to create the first chapter.
    mocks.gameRoutes.mockResolvedValue([
      route({ id: null, name: "", origin: "manual", chapters: [], chaptersTotal: 0 }),
    ]);

    renderCard();

    expect(await screen.findByText("Add chapter")).toBeTruthy();
  });

  it("shows chapters and endings that belong to no route at all", async () => {
    // A route-less row that vanishes when a route exists is the bug that would make
    // the card unusable on a fresh library with detection off.
    mocks.gameRoutes.mockResolvedValue([
      route({ id: null, name: "", origin: "manual", chapters: [chapter()], endings: [] }),
    ]);

    renderCard();

    expect(await screen.findByText("No route")).toBeTruthy();
    expect(screen.getByText("Prologue")).toBeTruthy();
  });

  it("explains where a duplicated route came from", async () => {
    // The backfill deliberately creates both halves unlinked rather than guessing a
    // mapping, so the two rows need to be distinguishable before a merge.
    mocks.gameRoutes.mockResolvedValue([
      route({ id: "r1", name: "Shiroha", origin: "engine" }),
      route({ id: "r2", name: "Shiroha", origin: "ending", chapters: [] }),
    ]);

    renderCard();

    expect(await screen.findByText("From chapters")).toBeTruthy();
    expect(screen.getByText("From endings")).toBeTruthy();
  });

  it("offers a merge only when there is another route to merge into", async () => {
    mocks.gameRoutes.mockResolvedValue([
      route({ id: "r1", mergeCandidates: ["r2"] }),
      route({ id: "r2", name: "Mio", chapters: [], mergeCandidates: ["r1"] }),
    ]);

    renderCard();

    await screen.findByText("Shiroha");
    // Each button names the route it merges into, so the user picks the survivor
    // instead of accepting whichever row happened to come first.
    expect(screen.getByText("Same route as Mio")).toBeTruthy();
    expect(screen.getByText("Same route as Shiroha")).toBeTruthy();
  });

  it("wraps the merge buttons in their own container, not the library row", async () => {
    // Regression: the buttons sat in `row row--wrap`, and `.row` is the library's
    // list-row grid (`44px 1fr auto` — a cover thumbnail, the text, the rating). A
    // bare button dropped into that grid took the 44px art cell and was clipped, so
    // on a phone the label rendered as "…e route as Mio" hanging off the left edge.
    // No test caught it because every assertion was about text, never about which
    // element the text was in.
    mocks.gameRoutes.mockResolvedValue([
      route({ id: "r1", mergeCandidates: ["r2"] }),
      route({ id: "r2", name: "Mio", chapters: [], mergeCandidates: ["r1"] }),
    ]);

    const { container } = renderCard();

    const merge = (await screen.findByText("Same route as Mio")).closest(
      ".chapter-group__merge",
    );
    expect(merge).not.toBeNull();
    // The library row class must not appear anywhere in the routes card.
    expect(container.querySelector(".row")).toBeNull();
  });

  it("merges a route through the command, not by editing rows", async () => {
    mocks.gameRoutes.mockResolvedValue([
      route({ id: "r1", mergeCandidates: ["r2"] }),
      route({ id: "r2", name: "Mio", chapters: [], mergeCandidates: ["r1"] }),
    ]);
    mocks.mergeRoutes.mockResolvedValue(null);

    renderCard();

    // Shiroha (r1) merges into Mio (r2), which is the target the label names.
    await userEvent.click(await screen.findByText("Same route as Mio"));

    await waitFor(() => {
      expect(mocks.mergeRoutes).toHaveBeenCalledWith("r1", "r2");
    });
  });

  it("counts reached chapters and endings on the same line", async () => {
    mocks.gameRoutes.mockResolvedValue([
      route({
        chapters: [chapter({ reached: true }), chapter({ id: "c2" })],
        chaptersReached: 1,
        chaptersTotal: 2,
        endingsTotal: 1,
        endingsReached: 1,
      }),
    ]);

    renderCard();

    expect(await screen.findByText(/1\/2 chapters/)).toBeTruthy();
    expect(screen.getByText(/1 of 1/)).toBeTruthy();
  });

  it("starts the timer on the selected chapter", async () => {
    mocks.gameRoutes.mockResolvedValue([route()]);
    mocks.trackingManualStart.mockResolvedValue({
      active: true,
      chapterId: "c1",
      routeId: null,
      chapterName: "Prologue",
      elapsedMs: 0,
      running: true,
    });

    renderCard();

    await userEvent.click(await screen.findByText("Prologue"));
    // The button names its target: "Start" beside a list does not say which chapter
    // it starts, and picking the wrong row records time against the wrong chapter.
    await userEvent.click(screen.getByRole("button", { name: /Start: Prologue/ }));

    await waitFor(() => {
      expect(mocks.trackingManualStart).toHaveBeenCalledWith("c1", "g1");
    });
  });

  it("times a route with no chapter named", async () => {
    // "Record an hour on the Shiroha route" used to require inventing a chapter to
    // blame it on, which polluted the chapter list with rows that only held time.
    mocks.gameRoutes.mockResolvedValue([route({ id: "r1", chapters: [], chaptersTotal: 0 })]);
    mocks.trackingManualStartRoute.mockResolvedValue({
      active: true,
      chapterId: null,
      routeId: "r1",
      chapterName: "Shiroha",
      elapsedMs: 0,
      running: true,
    });

    renderCard();

    await userEvent.click(await screen.findByText("Time this route"));

    await waitFor(() => {
      expect(mocks.trackingManualStartRoute).toHaveBeenCalledWith("r1", "g1");
    });
    // It must not fall through to the chapter path, which needs a chapter id.
    expect(mocks.trackingManualStart).not.toHaveBeenCalled();
  });

  it("shows Stop on the route that is being timed", async () => {
    mocks.gameRoutes.mockResolvedValue([route({ id: "r1" })]);
    mocks.trackingManualStatus.mockResolvedValue({
      active: true,
      chapterId: null,
      routeId: "r1",
      chapterName: "Shiroha",
      elapsedMs: 5_000,
      running: true,
    });

    renderCard();

    expect(await screen.findByText("Timing Shiroha")).toBeTruthy();
    // Two stops on purpose: one on the route header, one in the timer strip, so the
    // control is where the user is looking whichever they reach for.
    expect(screen.getAllByRole("button", { name: "Stop and save" }).length).toBeGreaterThan(0);
    // And the route must not offer Start while it is the thing being timed.
    expect(screen.queryByText("Time this route")).toBeNull();
  });

  it("does not offer a route timer in the unlinked bucket", async () => {
    // There is no route row to attribute the time to, so a session with neither
    // chapter nor route would be time nobody can point at.
    mocks.gameRoutes.mockResolvedValue([
      route({ id: null, name: "", origin: "manual", chapters: [chapter()] }),
    ]);

    renderCard();

    await screen.findByText("Prologue");
    expect(screen.queryByText("Time this route")).toBeNull();
  });

  it("says detection is unavailable rather than promising it", async () => {
    // Three outcomes, not two: the old copy collapsed "platform cannot read" and
    // "no verified profile for this build" into a promise that never came true.
    mocks.trackingChapterCapability.mockResolvedValue({
      ...capability,
      automatic: false,
      unavailableReason: "engineHasNoVerifiedSource",
    });
    mocks.gameRoutes.mockResolvedValue([]);

    renderCard();

    expect(await screen.findByText(/kiri|siglus|no verified/i)).toBeTruthy();
  });

  it("surfaces a failed load instead of showing an empty state", async () => {
    mocks.gameRoutes.mockRejectedValue(new Error("database is locked"));

    renderCard();

    expect(await screen.findByText(/could not be loaded/i)).toBeTruthy();
    expect(screen.getByText("database is locked")).toBeTruthy();
  });
});
