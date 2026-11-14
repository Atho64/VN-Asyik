import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter, Route, Routes, useLocation, useNavigate } from "react-router";
import { useEffect, useState } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { createTranslator } from "@/i18n";
import { AppContext } from "./AppProvider";
import { useAppStore } from "./store";

const mocks = vi.hoisted(() => ({
  windowSetFullscreen: vi.fn().mockResolvedValue(undefined),
}));

vi.mock("@/lib/ipc/client", () => ({
  api: {
    windowSetFullscreen: mocks.windowSetFullscreen,
  },
  IpcError: class extends Error {},
}));

vi.mock("@/components/Backdrop", () => ({ Backdrop: () => null, focusArt: vi.fn() }));
vi.mock("@/features/tracking/NowPlayingBar", () => ({ NowPlayingBar: () => null }));
vi.mock("@/features/companion/CompanionBar", () => ({ CompanionBar: () => null }));
vi.mock("@/features/library/AddGameSheet", () => ({ AddGameSheet: () => null }));
vi.mock("@/lib/ipc/companionWindow", () => ({
  companionWindowApi: {
    hide: vi.fn().mockResolvedValue(undefined),
    listenMain: vi.fn().mockResolvedValue(() => undefined),
  },
}));
// The real Big Picture screen polls gamepads and measures covers; the shell's
// behaviour under test is only the route it lands on and the fullscreen drop.
vi.mock("@/features/bigpicture/BigPictureScreen", () => ({
  BigPictureScreen: () => null,
}));

const { AppShell } = await import("./AppShell");

const t = createTranslator("en");

// The shell navigates through the router context; this probe reports where
// the router actually is so the assertion reads the outcome, not the call.
function LocationProbe({ onLocation }: { onLocation: (path: string) => void }) {
  const { pathname } = useLocation();
  useEffect(() => {
    onLocation(pathname);
  }, [pathname, onLocation]);
  return null;
}

/** The mocked Big Picture screen, with a way to leave it. */
function BigPictureRoute() {
  const navigate = useNavigate();
  return (
    <button type="button" onClick={() => navigate("/")}>
      exit-big-picture
    </button>
  );
}

const probePath = (): string =>
  screen.getByTestId("probe-path").getAttribute("data-path") ?? "";

function renderAt(path: string) {
  // The shell navigates through the router context; the harness holds the
  // observed path as React state so assertions read the outcome.
  function Harness() {
    const [observed, setObserved] = useState(path);
    return (
      <>
        <div data-testid="probe-path" data-path={observed} />
        <MemoryRouter initialEntries={[path]}>
          <Routes>
            <Route
              element={
                <>
                  <LocationProbe onLocation={setObserved} />
                  <AppShell />
                </>
              }
            >
              <Route path="/" element={null} />
              <Route path="/big-picture" element={<BigPictureRoute />} />
              <Route path="*" element={null} />
            </Route>
          </Routes>
        </MemoryRouter>
      </>
    );
  }
  return render(
    <AppContext.Provider value={{ t, locale: "en", ready: true, bootError: null }}>
      <Harness />
    </AppContext.Provider>,
  );
}

describe("app shell startup behaviour", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useAppStore.setState({ startupPrefs: null });
  });

  it("opens Big Picture once after boot when the launch mode says so", async () => {
    // The mode applies to every launch, however the app was started; the
    // navigation happens once, after the prefs hydrate.
    useAppStore.setState({
      startupPrefs: {
        prefs: { startup_mode: "big_picture", close_to_tray: true },
        supported: true,
        autostart: null,
      },
    });
    renderAt("/");
    await waitFor(() => {
      expect(probePath()).toBe("/big-picture");
    });
    expect(screen.getByText("exit-big-picture")).toBeInTheDocument();
    expect(mocks.windowSetFullscreen).not.toHaveBeenCalled();
  });

  it("does not navigate when the launch mode is manual", async () => {
    useAppStore.setState({
      startupPrefs: {
        prefs: { startup_mode: "manual", close_to_tray: true },
        supported: true,
        autostart: null,
      },
    });
    renderAt("/");
    // The nav renders whatever the shell mounts; only the path matters here.
    await waitFor(() => {
      expect(screen.getAllByText("Library").length).toBeGreaterThan(0);
    });
    expect(probePath()).toBe("/");
  });

  it("drops the launch fullscreen when Big Picture is left", async () => {
    renderAt("/big-picture");
    await waitFor(() => {
      expect(screen.getByText("exit-big-picture")).toBeInTheDocument();
    });
    expect(mocks.windowSetFullscreen).not.toHaveBeenCalled();
    fireEvent.click(screen.getByText("exit-big-picture"));
    await waitFor(() => {
      expect(mocks.windowSetFullscreen).toHaveBeenCalledWith(false);
    });
    expect(probePath()).toBe("/");
  });
});
