import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { AppContext } from "@/app/AppProvider";
import { useAppStore } from "@/app/store";
import { createTranslator } from "@/i18n";
import type { CaptureStatus } from "@/lib/ipc/types";

const mocks = vi.hoisted(() => ({ status: vi.fn(), capture: vi.fn() }));
vi.mock("@/lib/ipc/client", () => ({
  api: { screenshotStatus: mocks.status, screenshotCapture: mocks.capture },
  IpcError: class extends Error {
    constructor(
      public readonly code: string,
      message: string,
    ) {
      super(message);
    }
  },
}));

const { ScreenshotButton } = await import("./ScreenshotButton");
const { IpcError } = await import("@/lib/ipc/client");
const t = createTranslator("en");
const ready: CaptureStatus = { supported: true, gameId: "game-one", reason: null };

function content(gameId = "game-one", onCaptured = vi.fn()) {
  return (
    <AppContext.Provider value={{ t, locale: "en", ready: true, bootError: null }}>
      <ScreenshotButton gameId={gameId} onCaptured={onCaptured} />
    </AppContext.Provider>
  );
}

beforeEach(() => {
  vi.resetAllMocks();
  mocks.status.mockResolvedValue(ready);
  mocks.capture.mockResolvedValue({ id: "shot-one", gameId: "game-one" });
  useAppStore.setState({
    toasts: [],
    platform: {
      os: "android",
      kind: "mobile",
      first_class: true,
      process_tracking: "supported",
      chapter_detection: "unavailable",
      discord_presence: "supported",
      cloud_sync: "unavailable",
    },
  });
});

describe("Android screenshot capture control", () => {
  it("scopes capture to the displayed game and refreshes its gallery on success", async () => {
    const captured = vi.fn();
    render(content("game-one", captured));
    const button = await screen.findByRole("button", { name: t("screenshot.capture") });
    await waitFor(() => expect(button).toBeEnabled());
    fireEvent.click(button);
    await waitFor(() => expect(captured).toHaveBeenCalledOnce());
    expect(mocks.capture).toHaveBeenCalledWith("game-one");
    expect(useAppStore.getState().toasts.at(-1)?.message).toBe(t("screenshot.captured"));
  });

  it("disables duplicate requests while waiting and treats Cancel as a quiet completion", async () => {
    let rejectCapture: ((error: Error) => void) | undefined;
    mocks.capture.mockReturnValue(
      new Promise((_resolve, reject) => {
        rejectCapture = reject;
      }),
    );
    const captured = vi.fn();
    render(content("game-one", captured));
    const button = await screen.findByRole("button", { name: t("screenshot.capture") });
    await waitFor(() => expect(button).toBeEnabled());
    fireEvent.click(button);
    expect(button).toBeDisabled();
    expect(screen.getByRole("status")).toHaveTextContent(t("screenshot.androidWaiting"));
    fireEvent.click(button);
    expect(mocks.capture).toHaveBeenCalledOnce();
    if (!rejectCapture) throw new Error("Missing pending capture");
    await act(async () =>
      rejectCapture?.(new IpcError("screenshot_cancelled", "Cancelled")),
    );
    expect(button).toBeEnabled();
    expect(captured).not.toHaveBeenCalled();
    expect(useAppStore.getState().toasts).toEqual([]);
    expect(screen.queryByRole("status")).toBeNull();
  });

  it("refreshes availability on focus after a game is selected", async () => {
    mocks.status.mockResolvedValueOnce({
      supported: true,
      gameId: null,
      reason: "screenshot_no_game",
    });
    render(content());
    await waitFor(() => expect(mocks.status).toHaveResolved());
    expect(screen.queryByRole("button", { name: t("screenshot.capture") })).toBeNull();
    fireEvent.focus(window);
    expect(
      await screen.findByRole("button", { name: t("screenshot.capture") }),
    ).toBeEnabled();
    expect(mocks.status).toHaveBeenCalledTimes(2);
  });

  it("discards an older availability reply after a foreground refresh", async () => {
    let finishFirst: ((status: CaptureStatus) => void) | undefined;
    mocks.status.mockReturnValueOnce(
      new Promise<CaptureStatus>((resolve) => {
        finishFirst = resolve;
      }),
    );
    render(content());
    fireEvent.focus(window);
    const button = await screen.findByRole("button", { name: t("screenshot.capture") });
    expect(button).toBeEnabled();
    if (!finishFirst) throw new Error("Missing initial request");
    await act(async () =>
      finishFirst?.({ supported: true, gameId: null, reason: "screenshot_no_game" }),
    );
    expect(screen.getByRole("button", { name: t("screenshot.capture") })).toBeEnabled();
  });

  it("does not expose another game's stale status after the displayed game changes", async () => {
    const view = render(content());
    expect(
      await screen.findByRole("button", { name: t("screenshot.capture") }),
    ).toBeEnabled();
    mocks.status.mockResolvedValue({ ...ready, gameId: "game-two" });
    view.rerender(content("game-two"));
    const next = await screen.findByRole("button", { name: t("screenshot.capture") });
    expect(next).toBeEnabled();
    expect(mocks.status).toHaveBeenCalledTimes(2);
    fireEvent.click(next);
    await waitFor(() => expect(mocks.capture).toHaveBeenCalledWith("game-two"));
    await waitFor(() => expect(mocks.status).toHaveBeenCalledTimes(3));
  });

  it("refreshes when visible again and skips foreground events while hidden", async () => {
    const visibility = vi.spyOn(document, "visibilityState", "get");
    try {
      visibility.mockReturnValue("visible");
      render(content());
      expect(
        await screen.findByRole("button", { name: t("screenshot.capture") }),
      ).toBeEnabled();
      visibility.mockReturnValue("hidden");
      fireEvent(document, new Event("visibilitychange"));
      fireEvent.focus(window);
      expect(mocks.status).toHaveBeenCalledTimes(1);
      visibility.mockReturnValue("visible");
      fireEvent(document, new Event("visibilitychange"));
      await waitFor(() => expect(mocks.status).toHaveBeenCalledTimes(2));
    } finally {
      visibility.mockRestore();
    }
  });
});
