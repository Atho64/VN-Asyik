import { useContext } from "react";
import { act, fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { useAppStore } from "./store";
import { Backdrop, focusArt } from "@/components/Backdrop";

const mocks = vi.hoisted(() => ({
  platform: vi.fn(),
  session: vi.fn(),
  listGames: vi.fn(),
}));

vi.mock("@/lib/ipc/client", () => ({
  api: {
    // Appearance and the update check both load on boot; both are optional, but
    // the methods have to exist.
    appearanceGet: vi.fn().mockResolvedValue({
      accent: null,
      backgroundName: null,
      background: null,
      backgroundDim: 0.45,
    }),
    // The provider asks GitHub for a newer build on every boot; a rejected
    // promise is the documented silent path, but the method must exist.
    updateCheck: vi.fn().mockResolvedValue({
      current: "0.1.6",
      latest: null,
      available: false,
      url: null,
      asset: null,
      checkedAt: null,
      error: null,
      canInstall: false,
    }),
    getPrefs: vi.fn().mockResolvedValue({
      theme: "dark",
      language: "en",
      nsfw: "blur",
      reduce_effects: false,
      beta_features: false,
    }),
    uiPrefsGet: vi.fn().mockResolvedValue({
      sidebar_compact: false,
      library_remember_browse: false,
      library_default_sort: "recent",
      library_saved_browse: null,
    }),
    uiPrefsSet: vi.fn().mockImplementation(async (patch: Record<string, unknown>) => ({
      sidebar_compact: false,
      library_remember_browse: true,
      library_default_sort: "recent",
      library_saved_browse: null,
      ...patch,
    })),
    platformInfo: mocks.platform,
    trackingAndroidSession: mocks.session,
    listGames: mocks.listGames,
    // The provider relabels the tray once it knows the locale; the mock only has
    // to exist, since a tray is a desktop surface and this suite never sees one.
    traySetLabels: vi.fn().mockResolvedValue(undefined),
  },
  IpcError: class extends Error {},
}));

const { AppProvider, AppContext } = await import("./AppProvider");
const android = {
  os: "android",
  kind: "mobile",
  first_class: true,
  process_tracking: "supported",
  chapter_detection: "unavailable",
  discord_presence: "unavailable",
  cloud_sync: "unavailable",
};

beforeEach(() => {
  vi.clearAllMocks();
  window.localStorage.clear();
  mocks.platform.mockResolvedValue(android);
  mocks.session.mockResolvedValue({ reason: null });
  mocks.listGames.mockResolvedValue([]);
  useAppStore.setState({
    platform: null,
    cards: [],
    libraryLoaded: false,
    libraryError: null,
    appearance: null,
    uiPrefs: null,
    uiPrefsLoaded: false,
    sidebarCompact: false,
    libraryBrowse: {
      query: "",
      statuses: [],
      ratedOnly: false,
      view: "grid",
      sort: "recent",
    },
    prefs: {
      theme: "dark",
      language: "en",
      nsfw: "blur",
      reduce_effects: false,
      beta_features: false,
    },
  });
});

function BootState() {
  const state = useContext(AppContext)!;
  return <p>{state.bootError ?? (state.ready ? "Library ready" : "Loading library")}</p>;
}

function renderApp() {
  render(
    <AppProvider>
      <BootState />
    </AppProvider>,
  );
}

describe("Android boot playtime recovery", () => {
  it("waits for persisted usage recovery before reading library totals", async () => {
    let finishRecovery!: (value: { reason: null }) => void;
    mocks.session.mockReturnValue(
      new Promise((resolve) => {
        finishRecovery = resolve;
      }),
    );
    await act(async () => renderApp());
    expect(mocks.session).toHaveBeenCalledOnce();
    expect(mocks.listGames).not.toHaveBeenCalled();
    expect(screen.getByText("Loading library")).toBeInTheDocument();
    await act(async () => finishRecovery({ reason: null }));
    expect(screen.getByText("Library ready")).toBeInTheDocument();
    expect(mocks.listGames).toHaveBeenCalledOnce();
  });

  it("still opens saved library data when recovery fails", async () => {
    mocks.session.mockRejectedValue(new Error("UsageStats unavailable"));
    renderApp();
    expect(await screen.findByText("Library ready")).toBeInTheDocument();
    expect(mocks.listGames).toHaveBeenCalledOnce();
    expect(useAppStore.getState().libraryError).toBeNull();
  });

  it("does not request Android recovery on desktop", async () => {
    mocks.platform.mockResolvedValue({ ...android, os: "windows", kind: "desktop" });
    renderApp();
    expect(await screen.findByText("Library ready")).toBeInTheDocument();
    expect(mocks.session).not.toHaveBeenCalled();
  });
});

describe("content policy revokes artwork", () => {
  const revealedArt = "data:image/png;base64,revealed-explicit-fixture";
  function PreviouslyFocusedCard() {
    return <button onClick={() => focusArt(revealedArt)}>Reveal artwork</button>;
  }

  it.each(["show", "blur"] as const)(
    "clears a %s-mode reveal after the focused card leaves the virtual range",
    async (initialPolicy) => {
      mocks.platform.mockResolvedValue({ ...android, os: "windows", kind: "desktop" });
      const result = render(
        <AppProvider>
          <BootState />
          <Backdrop />
          <PreviouslyFocusedCard />
        </AppProvider>,
      );
      await screen.findByText("Library ready");
      act(() =>
        useAppStore
          .getState()
          .setPrefs({ ...useAppStore.getState().prefs, nsfw: initialPolicy }),
      );
      fireEvent.click(screen.getByRole("button", { name: "Reveal artwork" }));
      expect(
        result.container
          .querySelector(".backdrop__layer[data-visible='true']")
          ?.getAttribute("style"),
      ).toContain(revealedArt);
      result.rerender(
        <AppProvider>
          <BootState />
          <Backdrop />
        </AppProvider>,
      );
      expect(
        screen.queryByRole("button", { name: "Reveal artwork" }),
      ).not.toBeInTheDocument();
      expect(result.container.querySelector(".backdrop__layer")).toBeInTheDocument();
      act(() =>
        useAppStore
          .getState()
          .setPrefs({ ...useAppStore.getState().prefs, nsfw: "hide" }),
      );
      expect(result.container.querySelector(".backdrop__layer")).not.toBeInTheDocument();
      expect(result.container.querySelector(".backdrop__fallback")).toBeInTheDocument();
    },
  );
});

describe("custom accent", () => {
  const appearance = {
    accent: "#147363",
    background: null,
    backgroundName: null,
    backgroundDim: 0.45,
    theme: null,
    gameTint: true,
  };

  it("applies a theme-adapted accent with readable label text", async () => {
    renderApp();
    await screen.findByText("Library ready");
    // Set after boot: the boot pass loads the stored appearance (null here) and
    // would otherwise overwrite a value planted before mount.
    act(() => useAppStore.getState().setAppearance(appearance));
    const root = document.documentElement;
    // A dark accent on the dark theme is lightened until it reads, and the label
    // colour is set on both tokens that paint text over an accent fill.
    expect(root.style.getPropertyValue("--accent")).not.toBe("");
    expect(root.style.getPropertyValue("--accent")).not.toBe("#147363");
    expect(root.style.getPropertyValue("--on-accent")).not.toBe("");
    expect(root.style.getPropertyValue("--accent-contrast")).not.toBe("");
  });

  it("removes every override when the accent is cleared", async () => {
    renderApp();
    await screen.findByText("Library ready");
    act(() => useAppStore.getState().setAppearance(appearance));
    act(() => useAppStore.getState().setAppearance({ ...appearance, accent: null }));
    const root = document.documentElement;
    for (const name of ["--accent", "--accent-soft", "--accent-strong", "--on-accent", "--accent-contrast"]) {
      expect(root.style.getPropertyValue(name)).toBe("");
    }
  });
});

describe("UI startup preferences hydration", () => {
  it("hydrates compact rail and saved library browse filters on boot", async () => {
    const { api } = await import("@/lib/ipc/client");
    vi.mocked(api.uiPrefsGet).mockResolvedValueOnce({
      sidebar_compact: true,
      library_remember_browse: true,
      library_default_sort: "recent",
      library_saved_browse: {
        statuses: ["playing", "completed"],
        rated_only: true,
        view: "list",
        sort: "rating",
      },
    });

    renderApp();
    await screen.findByText("Library ready");

    expect(useAppStore.getState().sidebarCompact).toBe(true);
    expect(useAppStore.getState().libraryBrowse).toEqual({
      query: "",
      statuses: ["playing", "completed"],
      ratedOnly: true,
      view: "list",
      sort: "rating",
    });
  });

  it("uses library_default_sort when remember-on-launch is off", async () => {
    const { api } = await import("@/lib/ipc/client");
    vi.mocked(api.uiPrefsGet).mockResolvedValueOnce({
      sidebar_compact: false,
      library_remember_browse: false,
      library_default_sort: "title",
      library_saved_browse: {
        statuses: ["wishlist"],
        rated_only: true,
        view: "shelf",
        sort: "playtime",
      },
    });

    renderApp();
    await screen.findByText("Library ready");

    expect(useAppStore.getState().sidebarCompact).toBe(false);
    expect(useAppStore.getState().libraryBrowse).toEqual({
      query: "",
      statuses: [],
      ratedOnly: false,
      view: "grid",
      sort: "title",
    });
  });
});
