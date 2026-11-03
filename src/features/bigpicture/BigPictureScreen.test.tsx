import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes } from "react-router";

import { AppContext } from "@/app/AppProvider";
import { useAppStore } from "@/app/store";
import { createTranslator } from "@/i18n";
import type { GameCard } from "@/lib/ipc/types";

import { BigPictureScreen } from "./BigPictureScreen";
import { api } from "@/lib/ipc/client";

vi.mock("@/lib/ipc/client", () => ({
  api: {
    launchGame: vi.fn(),
    // Artwork resolution asks for a cached cover; null means "use the VNDB URL".
    artworkLocal: vi.fn(),
    gameCustomArt: vi.fn(),
  },
  IpcError: class IpcError extends Error {
    readonly code: string;
    constructor(code: string, message: string) {
      super(message);
      this.code = code;
    }
  },
}));

vi.mock("@/components/useArtwork", () => ({
  useArtwork: (game: { title: string }) => ({
    game,
    artUrl: null,
    imageUrl: null,
    backdropUrl: null,
    policy: "visible",
    palette: { accent: "#65d8c5", fallback: "#102125" },
    fallback: "#102125",
    publishAccent: () => undefined,
    imageFailed: () => undefined,
  }),
}));

const card = (id: string, title: string, status: GameCard["game"]["status"]): GameCard =>
  ({
    game: {
      meta: { id, updated_at: 0, device_id_device: "d", deleted: false },
      title,
      title_original: null,
      vndb_id: null,
      status,
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
    },
    tracked_ms: id === "a" ? 3_600_000 : 0,
    afk_ms: 0,
    total_ms: id === "a" ? 3_600_000 : 0,
  }) as GameCard;

beforeEach(() => {
  vi.clearAllMocks();
  useAppStore.setState({
    toasts: [],
    platform: null,
    cards: [
      card("c", "Clannad", "completed"),
      card("a", "anemoi", "playing"),
      card("b", "Little Busters!", "playing"),
    ],
  });
  vi.mocked(api.launchGame).mockResolvedValue({
    started: true,
    localized: false,
    command: "game.exe",
    explanation: "Started.",
  });
});

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

function renderScreen() {
  return render(
    <AppContext.Provider
      value={{ t: createTranslator("en"), locale: "en", ready: true, bootError: null }}
    >
      <MemoryRouter initialEntries={["/big-picture"]}>
        <Routes>
          <Route path="/big-picture" element={<BigPictureScreen />} />
          <Route path="/" element={<p>library</p>} />
          <Route path="/game/:id" element={<p>details</p>} />
        </Routes>
      </MemoryRouter>
    </AppContext.Provider>,
  );
}

describe("BigPictureScreen", () => {
  it("leads with what is being read, not with the alphabet", async () => {
    renderScreen();
    const options = screen.getAllByRole("option");
    // Playing first, then by time played; Clannad is completed so it is last.
    expect(options.map((option) => option.getAttribute("title"))).toEqual([
      "anemoi",
      "Little Busters!",
      "Clannad",
    ]);
    expect(screen.getByRole("heading", { name: "Big Picture" })).toBeInTheDocument();
    // The panel follows the focused cover.
    expect(screen.getByRole("heading", { name: "anemoi" })).toBeInTheDocument();
    // Compact duration, the same formatter the library uses.
    expect(screen.getByText(/1h played/)).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "anemoi" })).toHaveClass("bp__hero");
    expect(screen.getByRole("listbox")).toHaveClass("bp__grid");
    expect(options[0]?.querySelector(".bp-card__cover")).toBeInTheDocument();
  });

  it("moves the focus with the keyboard and the panel follows", async () => {
    renderScreen();
    fireEvent.keyDown(window, { key: "ArrowRight" });
    await waitFor(() =>
      expect(
        screen.getByRole("heading", { name: "Little Busters!" }),
      ).toBeInTheDocument(),
    );
    expect(screen.getByRole("option", { name: "Little Busters!" })).toHaveFocus();
    fireEvent.keyDown(window, { key: "ArrowLeft" });
    await waitFor(() =>
      expect(screen.getByRole("heading", { name: "anemoi" })).toBeInTheDocument(),
    );
    // A vertical move from the first row cannot leave the grid.
    fireEvent.keyDown(window, { key: "ArrowUp" });
    expect(screen.getByRole("heading", { name: "anemoi" })).toBeInTheDocument();
  });

  it("launches the focused game with Enter", async () => {
    renderScreen();
    fireEvent.keyDown(window, { key: "ArrowRight" });
    fireEvent.keyDown(window, { key: "Enter" });
    await waitFor(() => expect(api.launchGame).toHaveBeenCalledWith("b"));
    await waitFor(() =>
      expect(useAppStore.getState().toasts.at(-1)?.message).toBe("Started."),
    );
  });

  it("reports a launch that did not start instead of pretending", async () => {
    vi.mocked(api.launchGame).mockResolvedValue({
      started: false,
      localized: false,
      command: "",
      explanation: "No executable is mapped to this game.",
    });
    renderScreen();
    fireEvent.click(screen.getByRole("button", { name: "Play" }));
    await waitFor(() =>
      expect(useAppStore.getState().toasts.at(-1)?.message).toBe(
        "No executable is mapped to this game.",
      ),
    );
    // A failed launch is information, not an error: the screen stays usable.
    expect(useAppStore.getState().toasts.at(-1)?.kind).toBe("info");
  });

  it("leaves with Escape", async () => {
    renderScreen();
    fireEvent.keyDown(window, { key: "Escape" });
    await waitFor(() => expect(screen.getByText("library")).toBeInTheDocument());
  });

  it("opens the game page with E, for everything Big Picture does not show", async () => {
    renderScreen();
    fireEvent.keyDown(window, { key: "e" });
    await waitFor(() => expect(screen.getByText("details")).toBeInTheDocument());
  });

  it("still works with an empty library", async () => {
    useAppStore.setState({ cards: [] });
    renderScreen();
    expect(screen.getByText(/No games yet/)).toBeInTheDocument();
    // Nothing to launch, and no crash from the missing focus target.
    fireEvent.keyDown(window, { key: "Enter" });
    expect(api.launchGame).not.toHaveBeenCalled();
  });

  it("selects a clicked cover before launching it explicitly", async () => {
    renderScreen();
    fireEvent.click(screen.getByRole("option", { name: "Clannad" }));
    expect(screen.getByRole("heading", { name: "Clannad" })).toBeInTheDocument();
    expect(api.launchGame).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Play" }));
    await waitFor(() => expect(api.launchGame).toHaveBeenCalledWith("c"));
  });

  it("lets Enter activate the focused Details control without launching", async () => {
    const user = userEvent.setup();
    renderScreen();
    screen.getByRole("button", { name: "Details" }).focus();
    await user.keyboard("{Enter}");
    expect(screen.getByText("details")).toBeInTheDocument();
    expect(api.launchGame).not.toHaveBeenCalled();
  });

  it("keeps a large collection bounded and brings keyboard focus onto each new page", () => {
    useAppStore.setState({
      cards: Array.from({ length: 203 }, (_, i) =>
        card(`id-${i}`, `Game ${String(i).padStart(3, "0")}`, "playing"),
      ),
    });
    renderScreen();
    expect(screen.getAllByRole("option")).toHaveLength(5);
    fireEvent.keyDown(window, { key: "PageDown" });
    expect(screen.getByRole("heading", { name: "Game 005" })).toBeInTheDocument();
    expect(screen.getByRole("option", { name: "Game 005" })).toHaveFocus();
    fireEvent.keyDown(window, { key: "End" });
    expect(screen.getAllByRole("option")).toHaveLength(3);
    expect(screen.getByRole("option", { name: "Game 202" })).toHaveFocus();
    expect(screen.getByRole("option", { name: "Game 202" })).toHaveAttribute(
      "aria-setsize",
      "203",
    );
    expect(screen.getByRole("button", { name: "Next games" })).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "Previous games" }));
    expect(screen.getByRole("heading", { name: "Game 195" })).toBeInTheDocument();
  });

  it("debounces gamepad buttons across renders, resets released directions, and ignores background input", async () => {
    let frame: FrameRequestCallback = () => undefined;
    vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
      frame = callback;
      return 1;
    });
    vi.stubGlobal("cancelAnimationFrame", vi.fn());
    const focused = vi.spyOn(document, "hasFocus").mockReturnValue(true);
    const pads = vi.fn<() => Gamepad[]>(() => []);
    vi.stubGlobal("navigator", { ...navigator, getGamepads: pads });
    renderScreen();
    const press = async (buttons: number[]) => {
      pads.mockReturnValue([
        {
          connected: true,
          axes: [],
          buttons: Array.from({ length: 16 }, (_, index) => ({
            pressed: buttons.includes(index),
          })),
        } as unknown as Gamepad,
      ]);
      await act(async () => frame(0));
    };
    await press([15]);
    expect(screen.getByRole("heading", { name: "Little Busters!" })).toBeInTheDocument();
    await press([15]);
    expect(screen.getByRole("heading", { name: "Little Busters!" })).toBeInTheDocument();
    await press([]);
    await press([15]);
    expect(screen.getByRole("heading", { name: "Clannad" })).toBeInTheDocument();
    await press([0]);
    await press([0]);
    expect(api.launchGame).toHaveBeenCalledTimes(1);
    await press([]);
    focused.mockReturnValue(false);
    await press([0]);
    expect(api.launchGame).toHaveBeenCalledTimes(1);
  });
});
