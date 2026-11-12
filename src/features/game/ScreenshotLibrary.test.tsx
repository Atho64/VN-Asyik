import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";

import { AppContext } from "@/app/AppProvider";
import { useAppStore } from "@/app/store";
import { createTranslator } from "@/i18n";
import { api } from "@/lib/ipc/client";
import type { ScreenshotCard } from "@/lib/ipc/types";

vi.mock("@/lib/ipc/client", () => ({
  api: {
    screenshotList: vi.fn(),
    screenshotImage: vi.fn(),
    screenshotDelete: vi.fn(),
    screenshotStatus: vi.fn(),
    screenshotCapture: vi.fn(),
  },
  IpcError: class extends Error {},
}));

const { ScreenshotLibrary } = await import("./ScreenshotLibrary");

const shot: ScreenshotCard = {
  id: "shot-1",
  gameId: "game-1",
  width: 1280,
  height: 720,
  capturedAt: 1_700_000_000_000,
  source: "windows_window",
  caption: "",
};

function renderLibrary(): void {
  render(
    <AppContext.Provider
      value={{ t: createTranslator("en"), locale: "en", ready: true, bootError: null }}
    >
      <ScreenshotLibrary gameId="game-1" />
    </AppContext.Provider>,
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  useAppStore.setState({ toasts: [] });
  vi.mocked(api.screenshotStatus).mockResolvedValue({
    supported: true,
    gameId: "game-1",
    reason: null,
  });
  vi.mocked(api.screenshotImage).mockResolvedValue("data:image/jpeg;base64,AA==");
});

describe("the per-game screenshot library", () => {
  it("says what to do when nothing has been captured", async () => {
    vi.mocked(api.screenshotList).mockResolvedValue([]);
    renderLibrary();
    expect(await screen.findByText("Your screenshots")).toBeInTheDocument();
    expect(screen.getByText(/No screenshots yet/)).toBeInTheDocument();
  });

  it("lists a capture with its date and opens it", async () => {
    vi.mocked(api.screenshotList).mockResolvedValue([shot]);
    renderLibrary();

    const tile = await screen.findByRole("button", { name: /Open the screenshot/ });
    await waitFor(() => expect(api.screenshotImage).toHaveBeenCalledWith("shot-1"));
    expect(tile).toBeInTheDocument();
    expect(screen.getByText("1 screenshot")).toBeInTheDocument();
  });

  it("asks for the list of the game it was given", async () => {
    vi.mocked(api.screenshotList).mockResolvedValue([]);
    renderLibrary();
    await waitFor(() => expect(api.screenshotList).toHaveBeenCalledWith("game-1"));
  });
});
