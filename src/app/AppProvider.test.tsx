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
    getPrefs: vi.fn().mockResolvedValue({
      theme: "dark",
      language: "en",
      nsfw: "blur",
      reduce_effects: false,
    }),
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
  mocks.platform.mockResolvedValue(android);
  mocks.session.mockResolvedValue({ reason: null });
  mocks.listGames.mockResolvedValue([]);
  useAppStore.setState({
    platform: null,
    cards: [],
    libraryLoaded: false,
    libraryError: null,
    prefs: { theme: "dark", language: "en", nsfw: "blur", reduce_effects: false },
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
