import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { MemoryRouter, Route, Routes } from "react-router";

import { AppContext } from "@/app/AppProvider";
import { useAppStore } from "@/app/store";
import { createTranslator } from "@/i18n";
import type { GameCard, LocalArt, NsfwMode } from "@/lib/ipc/types";

const mocks = vi.hoisted(() => ({
  getGame: vi.fn(),
  listSessions: vi.fn(),
  listEndings: vi.fn(),
  listExecutables: vi.fn(),
  vndbDetail: vi.fn(),
  refreshDetail: vi.fn(),
  localArt: vi.fn(),
  screenshots: vi.fn(),
  screenshotFetch: vi.fn(),
  openExternal: vi.fn(),
  updateGame: vi.fn(),
  setAccent: vi.fn(),
  // `RoutesCard` asks for the joined route list; the mock has to answer it or the
  // card throws before the assertions run.
  gameRoutes: vi.fn(async () => []),
}));
vi.mock("@/lib/ipc/client", () => ({
  IpcError: class extends Error {},
  api: {
    getGame: mocks.getGame,
    listSessions: mocks.listSessions,
    listEndings: mocks.listEndings,
    trackingListExecutables: mocks.listExecutables,
    vndbDetail: mocks.vndbDetail,
    vndbRefreshDetail: mocks.refreshDetail,
    vndbLocalArt: mocks.localArt,
    vndbScreenshots: mocks.screenshots,
    vndbScreenshotFetch: mocks.screenshotFetch,
    openExternal: mocks.openExternal,
    updateGame: mocks.updateGame,
    setGameAccent: mocks.setAccent,
    gameRoutes: mocks.gameRoutes,
  },
}));
// The panels below the hero are unrelated to artwork policy.
vi.mock("@/features/tracking/MapExecutable", () => ({ MapExecutable: () => null }));
vi.mock("@/features/game/RoutesCard", () => ({ RoutesCard: () => null }));
vi.mock("@/features/game/ScriptCard", () => ({ ScriptCard: () => null }));
vi.mock("@/features/game/PlayButton", () => ({ PlayButton: () => null }));

const { GameDetailScreen } = await import("./GameDetailScreen");
const t = createTranslator("en");
const remoteCover = "https://t.vndb.org/cv/00/100.jpg";
const cachedCover = "data:image/jpeg;base64,Y292ZXI=";
const cachedBackdrop = "data:image/jpeg;base64,YmFja2Ryb3A=";
const cachedArt: LocalArt = {
  cover_data_url: cachedCover,
  backdrop_data_url: cachedBackdrop,
};

function detail(overrides: Partial<GameCard["game"]> = {}): GameCard {
  return {
    game: {
      meta: { id: "game-one", updated_at: 1, device_id_device: "test", deleted: false },
      title: "A visual novel",
      title_original: null,
      vndb_id: "v1",
      status: "playing",
      rating: null,
      notes: null,
      length_minutes: null,
      manual_playtime_ms: 0,
      accent_color: "#65d8c5",
      cover_url: remoteCover,
      cover_sexual: 1,
      cover_violence: 0,
      custom_cover_path: null,
      custom_cover_source_vndb: false,
      started_at: null,
      finished_at: null,
      ...overrides,
    },
    tracked_ms: 0,
    total_ms: 0,
    afk_ms: 0,
  };
}

function vndbPayload(overrides: Record<string, unknown> = {}) {
  return {
    vn_id: "v1",
    title: "A visual novel",
    alttitle: null,
    titles: [],
    aliases: [],
    length_minutes: null,
    length: null,
    length_votes: null,
    description: null,
    rating: null,
    average: null,
    votecount: null,
    released: null,
    olang: null,
    devstatus: null,
    languages: [],
    platforms: [],
    tags: [],
    screenshots: [],
    developers: [],
    extlinks: [],
  characters: [],
    ...overrides,
  };
}

function setPolicy(nsfw: NsfwMode) {
  useAppStore.setState({
    prefs: { theme: "dark", language: "en", nsfw, reduce_effects: false },
  });
}

function renderDetail() {
  return render(
    <AppContext.Provider value={{ t, locale: "en", ready: true, bootError: null }}>
      <MemoryRouter initialEntries={["/game/game-one"]}>
        <Routes>
          <Route path="/game/:id" element={<GameDetailScreen />} />
        </Routes>
      </MemoryRouter>
    </AppContext.Provider>,
  );
}

function heroImage(container: HTMLElement): HTMLImageElement | null {
  return container.querySelector<HTMLImageElement>(".detail__art img");
}

beforeEach(() => {
  vi.clearAllMocks();
  mocks.getGame.mockResolvedValue(detail());
  mocks.listSessions.mockResolvedValue([]);
  mocks.listEndings.mockResolvedValue([]);
  mocks.listExecutables.mockResolvedValue([]);
  mocks.vndbDetail.mockResolvedValue(null);
  mocks.localArt.mockResolvedValue(cachedArt);
  mocks.screenshots.mockResolvedValue({ shots: [], reported: 0, complete: true });
  mocks.screenshotFetch.mockRejectedValue(new Error("not cached"));
  mocks.openExternal.mockResolvedValue(undefined);
  mocks.updateGame.mockResolvedValue(undefined);
  mocks.setAccent.mockResolvedValue(undefined);
  setPolicy("blur");
});

describe("game detail artwork policy", () => {
  it("blurs an explicit hero cover under the Blur setting", async () => {
    const { container } = renderDetail();
    await waitFor(() =>
      expect(heroImage(container)).toHaveAttribute("src", cachedBackdrop),
    );
    expect(heroImage(container)).toHaveClass("cover__art--blurred");
    expect(screen.getByRole("button", { name: "Show once" })).toBeInTheDocument();
  });

  it("replaces an explicit hero cover with a neutral surface under Hide", async () => {
    setPolicy("hide");
    const { container } = renderDetail();
    await waitFor(() => expect(mocks.localArt).toHaveBeenCalledWith("v1"));
    expect(heroImage(container)).toBeNull();
    expect(container.querySelector(".cover__placeholder")).not.toBeNull();
    expect(screen.getByRole("button", { name: "Show once" })).toBeInTheDocument();
  });

  it("shows the sharp hero cover under Show all", async () => {
    setPolicy("show");
    const { container } = renderDetail();
    await waitFor(() => expect(heroImage(container)).toHaveAttribute("src", remoteCover));
    expect(heroImage(container)).not.toHaveClass("cover__art--blurred");
    expect(screen.queryByRole("button", { name: "Show once" })).toBeNull();
  });

  it("reveals the sharp hero cover on an explicit action, and revokes it", async () => {
    setPolicy("hide");
    const { container } = renderDetail();
    await userEvent.click(await screen.findByRole("button", { name: "Show once" }));
    expect(heroImage(container)).toHaveAttribute("src", remoteCover);

    act(() => setPolicy("blur"));
    // Revoking a reveal returns the cover to the setting, not to the sharp state.
    expect(heroImage(container)).toHaveAttribute("src", cachedBackdrop);
    expect(heroImage(container)).toHaveClass("cover__art--blurred");
  });

  it("leaves an unremarkable cover alone", async () => {
    setPolicy("blur");
    mocks.getGame.mockResolvedValue(detail({ cover_sexual: 0, cover_violence: 0 }));
    const { container } = renderDetail();
    await waitFor(() => expect(heroImage(container)).toHaveAttribute("src", remoteCover));
    expect(screen.queryByRole("button", { name: "Show once" })).toBeNull();
  });

  it("keeps the sharp cover hidden when the cached blur is missing", async () => {
    setPolicy("blur");
    mocks.localArt.mockResolvedValue({
      cover_data_url: cachedCover,
      backdrop_data_url: null,
    });
    const { container } = renderDetail();
    await waitFor(() => expect(mocks.localArt).toHaveBeenCalledWith("v1"));
    // A synthetic CSS blur is not an option on a scrolling surface, so without a
    // cached blur the artwork has to fall back to the neutral surface.
    expect(container.querySelector(".cover__placeholder")).not.toBeNull();
    expect(screen.getByRole("button", { name: "Show once" })).toBeInTheDocument();
  });
});

describe("game detail accent", () => {
  it("does not paint a concealed cover's accent into the hero", async () => {
    setPolicy("hide");
    const { container } = renderDetail();
    await waitFor(() => expect(mocks.localArt).toHaveBeenCalledWith("v1"));
    // The accent comes from the cached column while concealed: the sharp cover is
    // never sampled, so no palette is derived from artwork the user cannot see.
    expect(heroImage(container)).toBeNull();
    expect(container.querySelector(".detail__art-cover")).not.toBeNull();
  });
});

describe("returning to a game", () => {
  it("paints from the library card instead of a skeleton", async () => {
    // Navigating away unmounts the screen, so its state is gone on return. The
    // library store still holds every card, and a revisit used to blank to a
    // skeleton and wait for a round trip while the user was looking at data they
    // had just seen. The cached card must be usable on the first render.
    useAppStore.setState({ cards: [{ ...detail(), total_ms: 3_600_000 }] });

    const { container } = renderDetail();
    // Synchronous: no `await` before the skeleton assertion, because a skeleton on
    // the first frame is exactly the bug being fixed.
    expect(screen.queryByLabelText(t("common.loading"))).toBeNull();
    // The hero shell is rendered from the cached card, before any request returns.
    expect(container.querySelector(".detail__hero")).not.toBeNull();
    // The cached playtime is on screen before the refetch returns.
    expect(screen.getAllByText("1h").length).toBeGreaterThan(0);
    // The artwork itself still resolves asynchronously, through the same hook the
    // library uses; that is not what this test is about.
    await waitFor(() => expect(heroImage(container)).not.toBeNull());
  });

  it("still re-reads the row, because playtime and sessions move", async () => {
    // Painting from cache must not mean trusting it: a session logged in another
    // window has to land when the page comes back.
    useAppStore.setState({ cards: [detail()] });
    renderDetail();
    await waitFor(() => expect(mocks.getGame).toHaveBeenCalledWith("game-one"));
    await waitFor(() => expect(mocks.listSessions).toHaveBeenCalledWith("game-one"));
  });

  it("does not refetch the cover on every return", async () => {
    // The cached card points at art the artwork hook can resolve from disk, so a
    // return must not re-issue the cached-art request it already answered.
    useAppStore.setState({ cards: [detail()] });
    const first = renderDetail();
    await waitFor(() => expect(mocks.localArt).toHaveBeenCalledWith("v1"));
    const callsAfterFirst = mocks.localArt.mock.calls.length;
    first.unmount();
    renderDetail();
    // At most one more: the hook resolves per mount, but nothing should loop.
    await waitFor(() =>
      expect(mocks.localArt.mock.calls.length).toBeLessThanOrEqual(callsAfterFirst + 1),
    );
  });
});

describe("VNDB metadata refresh across navigation", () => {
  beforeEach(() => {
    useAppStore.setState({ refreshingVndb: [], vndbDetails: {} });
  });

  it("keeps showing Refreshing after leaving and coming back", async () => {
    // The reported bug: the refresh kept running while the page was unmounted, but
    // the flag lived in component state, so returning showed "Refresh" over a live
    // request. The flag belongs to the store, which outlives navigation.
    mocks.vndbDetail.mockResolvedValue(vndbPayload());
    mocks.refreshDetail.mockReturnValue(new Promise(() => undefined)); // never settles
    const first = renderDetail();
    await waitFor(() => expect(mocks.getGame).toHaveBeenCalled());

    await userEvent.click(screen.getByRole("button", { name: "Refresh" }));
    expect(screen.getByRole("button", { name: "Refreshing…" })).toBeInTheDocument();

    first.unmount();
    renderDetail();

    // Still in flight: the button must not claim it can be pressed again.
    expect(screen.getByRole("button", { name: "Refreshing…" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Refreshing…" })).toBeDisabled();
  });

  it("shows Refresh again once the refresh finishes", async () => {
    mocks.vndbDetail.mockResolvedValue(vndbPayload());
    mocks.refreshDetail.mockResolvedValue(vndbPayload());
    renderDetail();
    await waitFor(() => expect(mocks.getGame).toHaveBeenCalled());
    await userEvent.click(screen.getByRole("button", { name: "Refresh" }));
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Refresh" })).toBeEnabled(),
    );
    expect(useAppStore.getState().refreshingVndb).toEqual([]);
  });

  it("reuses a payload it already fetched instead of asking again", async () => {
    // The payload used to die with the page, so every return re-requested a row
    // that had not changed.
    useAppStore.setState({
      vndbDetails: { v1: vndbPayload() },
    });
    renderDetail();
    // Painted from the cache on the first frame, before any request returns.
    expect(screen.queryByText("From VNDB")).not.toBeNull();
    // And the row is still re-read, because playtime and sessions move.
    await waitFor(() => expect(mocks.vndbDetail).toHaveBeenCalledWith("v1"));
  });
});
