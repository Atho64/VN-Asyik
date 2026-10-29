import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { MemoryRouter } from "react-router";

import { LibraryScreen } from "./LibraryScreen";
import { useAppStore } from "@/app/store";
import { createTranslator } from "@/i18n";
import { initialLibraryBrowse } from "@/lib/library";
import type { GameCard, GameStatus } from "@/lib/ipc/types";

// The collection itself is covered by VirtualLibrary.test.tsx. Here it is a plain
// list so the assertions are about which cards the screen decided to pass down.
vi.mock("./VirtualLibrary", () => ({
  VirtualLibrary: ({ cards, view }: { cards: GameCard[]; view: string }) => (
    <ul data-testid="collection" data-view={view}>
      {cards.map((card) => (
        <li key={card.game.meta.id}>{card.game.title}</li>
      ))}
    </ul>
  ),
}));
vi.mock("@/app/useI18n", () => ({
  useI18n: () => ({ t: createTranslator("en"), locale: "en" }),
}));
vi.mock("./AddGameSheet", () => ({ AddGameSheet: () => null }));
vi.mock("@/lib/ipc/client", () => ({
  api: { listGames: vi.fn() },
  IpcError: class extends Error {},
}));

function card(
  id: string,
  title: string,
  status: GameStatus,
  rating: number | null = null,
): GameCard {
  return {
    game: {
      meta: { id, updated_at: 0, device_id_device: "test", deleted: false },
      title,
      title_original: null,
      vndb_id: null,
      status,
      rating,
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
    tracked_ms: 0,
    afk_ms: 0,
    total_ms: 0,
  };
}

const cards: GameCard[] = [
  card("g1", "Fate/stay night", "playing", 90),
  card("g2", "Steins;Gate", "completed", 95),
  card("g3", "Tsukihime", "wishlist", null),
];

function renderScreen() {
  return render(
    <MemoryRouter>
      <main className="app__main">
        <LibraryScreen />
      </main>
    </MemoryRouter>,
  );
}

function titles(): string[] {
  return [...screen.getByTestId("collection").querySelectorAll("li")].map(
    (item) => item.textContent ?? "",
  );
}

beforeEach(() => {
  useAppStore.setState({
    cards,
    libraryLoaded: true,
    libraryError: null,
    libraryBrowse: initialLibraryBrowse,
  });
});

describe("LibraryScreen browsing state", () => {
  it("keeps the search text and its results after leaving the screen and coming back", () => {
    const first = renderScreen();
    fireEvent.change(screen.getByRole("searchbox"), { target: { value: "steins" } });
    expect(titles()).toEqual(["Steins;Gate"]);

    // The router unmounts the screen when another menu entry is opened.
    first.unmount();
    renderScreen();

    expect(screen.getByRole("searchbox")).toHaveValue("steins");
    expect(titles()).toEqual(["Steins;Gate"]);
    expect(screen.getByText("1 game")).toBeInTheDocument();
  });

  it("keeps the status chips, the view and the sort after leaving the screen", () => {
    const first = renderScreen();
    fireEvent.click(screen.getByRole("button", { name: "Wishlist" }));
    fireEvent.click(screen.getByRole("tab", { name: "List" }));
    fireEvent.change(screen.getByRole("combobox"), { target: { value: "title" } });
    expect(titles()).toEqual(["Tsukihime"]);
    first.unmount();

    renderScreen();

    expect(screen.getByRole("button", { name: "Wishlist" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    expect(screen.getByRole("tab", { name: "List" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(screen.getByRole("combobox")).toHaveValue("title");
    expect(titles()).toEqual(["Tsukihime"]);
  });

  it("clear filters empties the search and the chips but keeps the chosen view", () => {
    renderScreen();
    fireEvent.click(screen.getByRole("tab", { name: "Shelf" }));
    fireEvent.change(screen.getByRole("searchbox"), { target: { value: "fate" } });
    fireEvent.click(screen.getByRole("button", { name: "Wishlist" }));
    expect(screen.getByText("No games match this filter")).toBeInTheDocument();

    // Both the empty state and the strip under the collection offer this.
    fireEvent.click(screen.getAllByRole("button", { name: "Clear filters" })[0]!);

    expect(screen.getByRole("searchbox")).toHaveValue("");
    expect(screen.getByRole("button", { name: "Wishlist" })).toHaveAttribute(
      "aria-pressed",
      "false",
    );
    expect(screen.getByTestId("collection")).toHaveAttribute("data-view", "shelf");
    expect(titles()).toHaveLength(3);
  });
});
