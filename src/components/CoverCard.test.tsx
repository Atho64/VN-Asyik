import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { MemoryRouter, useLocation } from "react-router";
import type { ReactNode } from "react";

import { AppContext } from "@/app/AppProvider";
import { ContextMenuProvider } from "@/app/ContextMenuProvider";
import { useAppStore } from "@/app/store";
import { createTranslator } from "@/i18n";
import type { GameCard, LocalArt, NsfwMode } from "@/lib/ipc/types";

const mocks = vi.hoisted(() => ({
  localArt: vi.fn(),
  customArt: vi.fn(),
  setAccent: vi.fn(),
  focusArt: vi.fn(),
}));
vi.mock("@/lib/ipc/client", () => ({
  api: {
    vndbLocalArt: mocks.localArt,
    gameCustomArt: mocks.customArt,
    setGameAccent: mocks.setAccent,
  },
}));
vi.mock("./Backdrop", () => ({ focusArt: mocks.focusArt }));
vi.mock("@/lib/usePalette", () => ({
  usePalette: () => ({ accent: "#65d8c5", accentSoft: "#65d8c52e", onAccent: "#101719" }),
}));

const { CoverCard, GameRow } = await import("./CoverCard");
const t = createTranslator("en");
const cachedCover = "data:image/jpeg;base64,Y292ZXI=";
const cachedBackdrop = "data:image/jpeg;base64,YmFja2Ryb3A=";
const remoteCover = "https://t.vndb.org/cv/00/100.jpg";
const cachedArt: LocalArt = {
  cover_data_url: cachedCover,
  backdrop_data_url: cachedBackdrop,
};

function card(overrides: Partial<GameCard["game"]> = {}): GameCard {
  return {
    game: {
      meta: { id: "game-one", updated_at: 1, device_id_device: "test", deleted: false },
      title: "A visual novel",
      title_original: null,
      vndb_id: "v1",
      status: "wishlist",
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

function Location() {
  return <output aria-label="Location">{useLocation().pathname}</output>;
}

function withProviders(ui: ReactNode) {
  return (
    <AppContext.Provider value={{ t, locale: "en", ready: true, bootError: null }}>
      <MemoryRouter>
        <ContextMenuProvider>
          {ui}
          <Location />
        </ContextMenuProvider>
      </MemoryRouter>
    </AppContext.Provider>
  );
}

function setPolicy(nsfw: NsfwMode) {
  useAppStore.setState({
    prefs: { theme: "dark", language: "en", nsfw, reduce_effects: false },
  });
}

beforeEach(() => {
  vi.clearAllMocks();
  mocks.localArt.mockResolvedValue(cachedArt);
  mocks.customArt.mockResolvedValue({ cover_data_url: null, backdrop_data_url: null });
  mocks.setAccent.mockResolvedValue(undefined);
  setPolicy("blur");
});

describe("a user-chosen cover", () => {
  const customCover = "data:image/jpeg;base64,Y3VzdG9t";
  const customBackdrop = "data:image/jpeg;base64,Y3VzdG9tYmFja2Ryb3A=";

  beforeEach(() => {
    mocks.customArt.mockResolvedValue({
      cover_data_url: customCover,
      backdrop_data_url: customBackdrop,
    });
  });

  it("wins over the cached VNDB cover", async () => {
    const { container } = render(
      withProviders(
        <CoverCard
          card={card({
            cover_sexual: 0,
            custom_cover_path: "covers/custom/game-one.jpg",
          })}
        />,
      ),
    );
    await waitFor(() => expect(mocks.customArt).toHaveBeenCalledWith("game-one"));
    // The user picked this deliberately, so a leftover VNDB cache entry must not
    // shadow it.
    await waitFor(() =>
      expect(container.querySelector("img")).toHaveAttribute("src", customCover),
    );
  });

  it("is the only artwork a game with no VNDB entry can show", async () => {
    const { container } = render(
      withProviders(
        <CoverCard
          card={card({
            vndb_id: null,
            cover_url: null,
            cover_sexual: null,
            cover_violence: null,
            custom_cover_path: "covers/custom/game-one.jpg",
          })}
        />,
      ),
    );
    // A manual game has no VN id, so the VNDB cache lookup never runs.
    expect(mocks.localArt).not.toHaveBeenCalled();
    await waitFor(() =>
      expect(container.querySelector("img")).toHaveAttribute("src", customCover),
    );
  });

  it("does not fetch art for a game that has no custom cover", async () => {
    const { container } = render(withProviders(<CoverCard card={card()} />));
    await waitFor(() => expect(mocks.localArt).toHaveBeenCalled());
    // A virtualized grid renders hundreds of cards; an unconditional IPC call
    // per card would be paid on every one of them.
    expect(mocks.customArt).not.toHaveBeenCalled();
    expect(container.querySelector("img")).toBeInTheDocument();
  });

  it("still honours the content policy, blurring with its own backdrop", async () => {
    setPolicy("blur");
    const { container } = render(
      withProviders(
        <CoverCard
          card={card({
            cover_sexual: 3,
            custom_cover_path: "covers/custom/game-one.jpg",
          })}
        />,
      ),
    );
    // The custom cover has its own generated backdrop, so Blur must use that one
    // rather than falling back to Hide for lack of a blurred source.
    await waitFor(() =>
      expect(container.querySelector("img")).toHaveAttribute("src", customBackdrop),
    );
    expect(container.querySelector("img")).toHaveClass(/cover__art--blurred/);
  });
});

describe.each([
  ["grid", CoverCard],
  ["list", GameRow],
] as const)("%s library art", (_view, Component) => {
  it("blurs explicit art under the Blur setting instead of replacing it", async () => {
    setPolicy("blur");
    const { container } = render(withProviders(<Component card={card()} />));
    await waitFor(() => expect(mocks.localArt).toHaveBeenCalledWith("v1"));
    // The blurred variant is the cached pre-blurred backdrop, never the sharp
    // cover: this setting is called "Blur until revealed" and used to behave
    // exactly like Hide, with no blur anywhere in the app.
    await waitFor(() =>
      expect(container.querySelector("img")).toHaveAttribute("src", cachedBackdrop),
    );
    expect(container.querySelector("img")).toHaveClass(/cover__art--blurred/);
    expect(screen.getByRole("button", { name: "Show once" })).toBeInTheDocument();
    const link = screen.getByRole("link");
    fireEvent.pointerEnter(link);
    // A blurred downscale is still a legible picture at backdrop scale.
    expect(mocks.focusArt).toHaveBeenLastCalledWith(null);
  });

  it("replaces explicit art with a neutral surface under Hide", async () => {
    setPolicy("hide");
    const { container } = render(withProviders(<Component card={card()} />));
    await waitFor(() => expect(mocks.localArt).toHaveBeenCalledWith("v1"));
    expect(container.querySelector("img")).toBeNull();
    const link = screen.getByRole("link");
    fireEvent.pointerEnter(link);
    expect(mocks.focusArt).toHaveBeenLastCalledWith(null);

    await userEvent.click(screen.getByRole("button", { name: "Show once" }));
    expect(container.querySelector("img")).toHaveAttribute("src", cachedCover);
    expect(screen.getByLabelText("Location")).toHaveTextContent("/");
    fireEvent.pointerEnter(link);
    expect(mocks.focusArt).toHaveBeenLastCalledWith(cachedBackdrop);
  });

  it("reveals the sharp cover and the backdrop from the blurred state", async () => {
    setPolicy("blur");
    const { container } = render(withProviders(<Component card={card()} />));
    await waitFor(() =>
      expect(container.querySelector("img")).toHaveAttribute("src", cachedBackdrop),
    );

    await userEvent.click(screen.getByRole("button", { name: "Show once" }));
    expect(container.querySelector("img")).toHaveAttribute("src", cachedCover);
    expect(container.querySelector("img")).not.toHaveClass(/cover__art--blurred/);
    expect(screen.getByLabelText("Location")).toHaveTextContent("/");
    fireEvent.pointerEnter(screen.getByRole("link"));
    expect(mocks.focusArt).toHaveBeenLastCalledWith(cachedBackdrop);
  });

  it("falls back to the neutral surface when Blur has no cached blur to show", async () => {
    setPolicy("blur");
    mocks.localArt.mockResolvedValueOnce({
      cover_data_url: cachedCover,
      backdrop_data_url: null,
    });
    const { container } = render(withProviders(<Component card={card()} />));
    await waitFor(() => expect(mocks.localArt).toHaveBeenCalledWith("v1"));
    // A synthetic CSS blur was replaced by the cached backdrop; without one there
    // is nothing to blur, and showing the sharp cover would break the setting.
    expect(container.querySelector("img")).toBeNull();
    expect(screen.getByRole("button", { name: "Show once" })).toBeInTheDocument();
  });

  it("shows explicit artwork directly only under Show all", async () => {
    setPolicy("show");
    const { container } = render(withProviders(<Component card={card()} />));
    await waitFor(() =>
      expect(container.querySelector("img")).toHaveAttribute("src", cachedCover),
    );
    expect(screen.queryByRole("button", { name: "Show once" })).toBeNull();
  });

  it("revokes a reveal on policy changes, including a round-trip", async () => {
    const { container } = render(withProviders(<Component card={card()} />));
    await userEvent.click(screen.getByRole("button", { name: "Show once" }));
    await waitFor(() =>
      expect(container.querySelector("img")).toHaveAttribute("src", cachedCover),
    );
    fireEvent.focus(screen.getByRole("link"));
    fireEvent.blur(screen.getByRole("link"));
    fireEvent.pointerLeave(screen.getByRole("link"));

    act(() => setPolicy("hide"));
    expect(container.querySelector("img")).toBeNull();
    expect(mocks.focusArt).toHaveBeenLastCalledWith(null);
    act(() => setPolicy("blur"));
    // Revoked means "back under the setting", which for Blur is the blurred
    // variant: the sharp cover is what must never come back unasked.
    expect(container.querySelector("img")).toHaveAttribute("src", cachedBackdrop);
    expect(container.querySelector("img")).toHaveClass(/cover__art--blurred/);
    expect(screen.getByRole("button", { name: "Show once" })).toBeInTheDocument();
  });

  it("uses cached artwork before the remote URL, and never flashes remote while loading cache", async () => {
    let resolve!: (art: LocalArt) => void;
    mocks.localArt.mockReturnValueOnce(
      new Promise<LocalArt>((done) => {
        resolve = done;
      }),
    );
    const { container } = render(
      withProviders(<Component card={card({ cover_sexual: 0 })} />),
    );
    expect(container.querySelector("img")).toBeNull();
    await act(async () => resolve(cachedArt));
    expect(container.querySelector("img")).toHaveAttribute("src", cachedCover);
  });

  it("falls back to remote then a neutral placeholder if neither image loads", async () => {
    const { container } = render(
      withProviders(<Component card={card({ cover_sexual: 0 })} />),
    );
    await waitFor(() =>
      expect(container.querySelector("img")).toHaveAttribute("src", cachedCover),
    );
    fireEvent.error(container.querySelector("img")!);
    expect(container.querySelector("img")).toHaveAttribute("src", remoteCover);
    fireEvent.error(container.querySelector("img")!);
    expect(container.querySelector("img")).toBeNull();
    expect(screen.getByRole("link")).toHaveTextContent("A visual novel");
  });

  it("handles cache failures and absent artwork", async () => {
    mocks.localArt.mockRejectedValueOnce(new Error("cache missing"));
    const { container, rerender } = render(
      withProviders(<Component card={card({ cover_sexual: 0 })} />),
    );
    await waitFor(() =>
      expect(container.querySelector("img")).toHaveAttribute("src", remoteCover),
    );
    rerender(
      withProviders(
        <Component card={card({ vndb_id: null, cover_url: null, cover_sexual: 0 })} />,
      ),
    );
    expect(container.querySelector("img")).toBeNull();
  });

  it("does not let a stale cache response supply a different game's artwork", async () => {
    let resolve!: (art: LocalArt) => void;
    mocks.localArt.mockReturnValueOnce(
      new Promise<LocalArt>((done) => {
        resolve = done;
      }),
    );
    mocks.localArt.mockResolvedValueOnce({
      cover_data_url: null,
      backdrop_data_url: null,
    });
    const { container, rerender } = render(
      withProviders(<Component card={card({ cover_sexual: 0 })} />),
    );
    const next = card({ vndb_id: "v2", cover_url: null, cover_sexual: 0 });
    rerender(withProviders(<Component card={next} />));
    await waitFor(() => expect(mocks.localArt).toHaveBeenCalledWith("v2"));
    await act(async () => resolve(cachedArt));
    expect(container.querySelector("img")).toBeNull();
  });

  it("conceals cached-only explicit artwork even without a remote URL", async () => {
    const { container } = render(
      withProviders(<Component card={card({ cover_url: null })} />),
    );
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Show once" })).toBeInTheDocument(),
    );
    expect(container.querySelector("img")).toHaveAttribute("src", cachedBackdrop);
    expect(container.querySelector("img")).toHaveClass(/cover__art--blurred/);
  });
});
