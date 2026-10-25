import { describe, expect, it, vi, beforeEach, afterEach } from "vitest";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router";

import { AppContext } from "@/app/AppProvider";
import { createTranslator } from "@/i18n";
import type { UsageAccessView } from "@/lib/ipc/types";

const t = createTranslator("en");

/**
 * The Android usage-access card.
 *
 * Android is the one platform where playtime silently does nothing until the user
 * grants a permission that has no runtime dialog. The card is the only thing that
 * explains that, and the desktop build cannot show it, so it is covered here by
 * driving the state the platform would report.
 */

// Hoisted because `vi.mock` is lifted above the imports, so anything it
// references has to exist before the module body runs.
const mocks = vi.hoisted(() => ({
  openUsageSettings: vi.fn().mockResolvedValue(undefined),
  androidSession: vi.fn(),
  androidStop: vi.fn(),
  available: false,
  usage: {
    applicable: false,
    granted: false,
    settingsAction: null,
    reason: null,
  } as UsageAccessView,
}));
const openUsageSettings = mocks.openUsageSettings;

vi.mock("@/lib/ipc/client", () => ({
  api: {
    trackingCapability: vi.fn(async () => ({
      available: mocks.available,
      reason: mocks.available ? null : "unsupported on Android",
      enabled: true,
      poll_interval_ms: 2000,
    })),
    trackingListExecutables: vi.fn().mockResolvedValue([]),
    trackingListScanFolders: vi.fn().mockResolvedValue([]),
    trackingUsageAccess: vi.fn(async () => mocks.usage),
    trackingOpenUsageSettings: mocks.openUsageSettings,
    trackingCandidates: vi.fn().mockResolvedValue([]),
    trackingAndroidSession: mocks.androidSession,
    trackingAndroidStop: mocks.androidStop,
    listGames: vi.fn().mockResolvedValue([]),
  },
  IpcError: class IpcError extends Error {},
}));

const { TrackingScreen } = await import("./TrackingScreen");

function renderScreen(usage: UsageAccessView, available = false) {
  mocks.usage = usage;
  mocks.available = available;
  render(
    <AppContext.Provider value={{ t, locale: "en", ready: true, bootError: null }}>
      <MemoryRouter>
        <TrackingScreen />
      </MemoryRouter>
    </AppContext.Provider>,
  );
}

const NOT_GRANTED: UsageAccessView = {
  applicable: true,
  granted: false,
  settingsAction: "android.settings.USAGE_ACCESS_SETTINGS",
  reason: null,
};

const SESSION = {
  active: true,
  gameId: "selected-game",
  package: "org.example.runner",
  startedAtMs: 1_000,
  countedMs: 60_000,
  lastImportedAtMs: 61_000,
  reason: null,
};

beforeEach(() => {
  vi.clearAllMocks();
  mocks.androidSession.mockResolvedValue(SESSION);
  mocks.androidStop.mockResolvedValue({ ...SESSION, active: false });
});

afterEach(() => vi.useRealTimers());

describe("Android usage access card", () => {
  it("offers a way to grant access when it is missing", async () => {
    // This is the state that matters: without the card, Android records nothing
    // and the screen looks merely empty.
    renderScreen(NOT_GRANTED);
    expect(await screen.findByText("Access is not granted yet")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Open Settings" })).toBeInTheDocument();
  });

  it("opens the settings screen when the button is pressed", async () => {
    renderScreen(NOT_GRANTED);
    const button = await screen.findByRole("button", { name: "Open Settings" });
    await userEvent.click(button);
    expect(openUsageSettings).toHaveBeenCalledOnce();
  });

  it("shows a granted state without a dead button", async () => {
    // A button that opens a screen the user already enabled is a dead control;
    // the granted state says so instead.
    renderScreen({ ...NOT_GRANTED, granted: true });
    expect(await screen.findByText("Access granted")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Open Settings" })).toBeNull();
  });

  it("renders no card at all where usage access does not apply", async () => {
    // Desktop and macOS must not see an Android-only permission prompt.
    renderScreen(
      {
        applicable: false,
        granted: false,
        settingsAction: null,
        reason: null,
      },
      true,
    );
    await waitFor(() => {
      expect(screen.queryByText("Access is not granted yet")).toBeNull();
    });
    expect(screen.queryByRole("button", { name: "Open Settings" })).toBeNull();
  });
});

describe("Android session recovery status", () => {
  it("surfaces a UsageStats failure returned in an otherwise successful response", async () => {
    mocks.androidSession.mockResolvedValue({
      ...SESSION,
      reason: "Usage access permission was revoked",
    });
    renderScreen({ ...NOT_GRANTED, granted: true });
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Usage access permission was revoked",
    );
    expect(screen.queryByText("Session selected")).toBeNull();
  });

  it("clears a previous query error after a successful status poll", async () => {
    vi.useFakeTimers();
    mocks.androidSession.mockResolvedValueOnce({ ...SESSION, reason: "Query failed" });
    await act(async () => renderScreen({ ...NOT_GRANTED, granted: true }));
    expect(screen.getByRole("alert")).toHaveTextContent("Query failed");
    await act(async () => vi.advanceTimersByTimeAsync(2_000));
    expect(screen.queryByRole("alert")).toBeNull();
    expect(screen.getByText("Session selected")).toBeInTheDocument();
  });

  it("clears a recovery error when stopping the session succeeds", async () => {
    mocks.androidSession.mockResolvedValue({ ...SESSION, reason: "Query failed" });
    renderScreen({ ...NOT_GRANTED, granted: true });
    expect(await screen.findByRole("alert")).toHaveTextContent("Query failed");
    await userEvent.click(screen.getByRole("button", { name: "Stop" }));
    expect(await screen.findByText("Session saved")).toBeInTheDocument();
    expect(screen.queryByRole("alert")).toBeNull();
  });
});
