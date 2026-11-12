import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter } from "react-router";

import { AppContext } from "@/app/AppProvider";
import { useAppStore } from "@/app/store";
import { createTranslator } from "@/i18n";
import { api } from "@/lib/ipc/client";
import type { ScreenshotFeedCard } from "@/lib/ipc/types";

vi.mock("@/lib/ipc/client", () => ({
  api: {
    screenshotListAll: vi.fn(),
    screenshotImage: vi.fn(),
    screenshotDelete: vi.fn(),
  },
  IpcError: class extends Error {},
}));

const { ScreenshotsPanel } = await import("./ScreenshotsPanel");

function shot(
  id: string,
  gameId: string,
  gameTitle: string,
  at: number,
): ScreenshotFeedCard {
  return {
    id,
    gameId,
    gameTitle,
    width: 1280,
    height: 720,
    capturedAt: at,
    source: "windows_window",
    caption: "",
  };
}

function renderScreen(): void {
  render(
    <MemoryRouter>
      <AppContext.Provider
        value={{ t: createTranslator("en"), locale: "en", ready: true, bootError: null }}
      >
        <ScreenshotsPanel />
      </AppContext.Provider>
    </MemoryRouter>,
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  useAppStore.setState({ toasts: [] });
  vi.mocked(api.screenshotImage).mockResolvedValue("data:image/jpeg;base64,AA==");
});

describe("the cross-game screenshot library", () => {
  it("groups captures under the game they came from", async () => {
    vi.mocked(api.screenshotListAll).mockResolvedValue([
      shot("a", "g1", "CLANNAD", 300),
      shot("b", "g2", "Sakura, Moyu", 200),
      shot("c", "g1", "CLANNAD", 100),
    ]);
    renderScreen();

    expect(await screen.findByRole("link", { name: "CLANNAD" })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Sakura, Moyu" })).toBeInTheDocument();
    // Two under CLANNAD, one under the other: the count is per group, not global.
    expect(screen.getByText("2 screenshots")).toBeInTheDocument();
    expect(screen.getByText("1 screenshot")).toBeInTheDocument();
    expect(screen.getByText("3 captures across your library")).toBeInTheDocument();
  });

  it("says how to get started when nothing has been captured", async () => {
    vi.mocked(api.screenshotListAll).mockResolvedValue([]);
    renderScreen();
    expect(await screen.findByText(/No screenshots yet/)).toBeInTheDocument();
  });

  it("links each group back to its game", async () => {
    vi.mocked(api.screenshotListAll).mockResolvedValue([shot("a", "g1", "CLANNAD", 1)]);
    renderScreen();
    const link = await screen.findByRole("link", { name: "CLANNAD" });
    expect(link).toHaveAttribute("href", "/game/g1");
  });

  it("fetches each tile's image once", async () => {
    vi.mocked(api.screenshotListAll).mockResolvedValue([shot("a", "g1", "CLANNAD", 1)]);
    renderScreen();
    await waitFor(() => expect(api.screenshotImage).toHaveBeenCalledWith("a"));
  });
});
