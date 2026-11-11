import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { MemoryRouter } from "react-router";

import { AppContext } from "@/app/AppProvider";
import { createTranslator } from "@/i18n";
import type { Game } from "@/lib/ipc/types";
import type { JournalSearchResult, ReadingEntry } from "@/lib/ipc/reading";
import { JournalSearch } from "./JournalSearch";

const mocks = vi.hoisted(() => ({ search: vi.fn() }));
vi.mock("@/lib/ipc/reading", () => ({ readingApi: mocks }));

const game: Game = {
  meta: { id: "g1", updated_at: 1, device_id_device: "test", deleted: false },
  title: "Summer Pockets",
  title_original: null,
  vndb_id: null,
  status: "playing",
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
};

function entry(overrides: Partial<ReadingEntry> = {}): ReadingEntry {
  return {
    meta: { id: "j1", updated_at: 1, device_id_device: "test", deleted: false },
    game_id: "g1",
    session_id: null,
    created_at: Date.UTC(2026, 10, 8, 21, 14),
    played_at: null,
    note: "The fireworks scene by the pier",
    save_slot: "",
    route_name: "Mio",
    chapter_name: "8月7日",
    photo_image_id: "",
    ...overrides,
  };
}

function result(overrides: Partial<ReadingEntry> = {}): JournalSearchResult {
  return { entry: entry(overrides), game };
}

const page = (results: JournalSearchResult[], hasMore: boolean) => ({ results, hasMore });

function mount(query = "fireworks") {
  render(
    <AppContext.Provider
      value={{ t: createTranslator("en"), locale: "en", ready: true, bootError: null }}
    >
      <MemoryRouter>
        <JournalSearch query={query} />
      </MemoryRouter>
    </AppContext.Provider>,
  );
}

beforeEach(() => vi.resetAllMocks());

describe("journal search", () => {
  it("renders matching checkpoints with the query highlighted and a link to the game", async () => {
    mocks.search.mockResolvedValueOnce(page([result()], false));
    mount();
    expect(await screen.findByText("Summer Pockets")).toBeInTheDocument();
    // The matched word is wrapped in a mark, the rest of the note stays plain.
    const hit = screen.getByText("fireworks");
    expect(hit.tagName).toBe("MARK");
    // The highlight splits the note across nodes, so read the whole line back.
    expect(hit.closest(".jsearch__note")?.textContent).toBe(
      "The fireworks scene by the pier",
    );
    expect(screen.getByText(/Mio · 8月7日/)).toBeInTheDocument();
    expect(screen.getByRole("link")).toHaveAttribute("href", "/game/g1");
    expect(screen.getByText("1 match")).toBeInTheDocument();
    expect(mocks.search).toHaveBeenCalledWith("fireworks", undefined, undefined);
  });

  it("shows the empty state when nothing matches", async () => {
    mocks.search.mockResolvedValueOnce(page([], false));
    mount("nonexistent");
    expect(await screen.findByText("No matching checkpoints")).toBeInTheDocument();
    expect(screen.queryByRole("link")).not.toBeInTheDocument();
  });

  it("labels a photo-only checkpoint instead of showing a blank note", async () => {
    mocks.search.mockResolvedValueOnce(
      page([result({ note: "", photo_image_id: "sf83040" })], false),
    );
    mount();
    expect(await screen.findByText("Journal checkpoint")).toBeInTheDocument();
  });

  it("loads older matches onto the same list using the last row as the cursor", async () => {
    const user = userEvent.setup();
    const first = result({ meta: { id: "j2", updated_at: 2, device_id_device: "t", deleted: false }, created_at: 2_000 });
    mocks.search.mockResolvedValueOnce(page([first], true));
    mount();
    await screen.findByText("Summer Pockets");

    const older = result({ meta: { id: "j1", updated_at: 1, device_id_device: "t", deleted: false }, created_at: 1_000, note: "An earlier fireworks night" });
    mocks.search.mockResolvedValueOnce(page([older], false));
    await user.click(screen.getByRole("button", { name: "Older events" }));

    // Both notes contain the query, so a second highlighted mark means the older
    // page landed (the text itself is split across nodes by the highlight).
    await waitFor(() => expect(screen.getAllByText("fireworks")).toHaveLength(2));
    expect(mocks.search).toHaveBeenLastCalledWith("fireworks", undefined, { ts: 2_000, id: "j2" });
    expect(screen.queryByRole("button", { name: "Older events" })).not.toBeInTheDocument();
    expect(screen.getByText("2 matches")).toBeInTheDocument();
  });

  it("recovers with retry when the search fails", async () => {
    const user = userEvent.setup();
    mocks.search.mockRejectedValueOnce(new Error("ipc"));
    mount();
    expect(await screen.findByText("The timeline could not be loaded")).toBeInTheDocument();
    mocks.search.mockResolvedValueOnce(page([result()], false));
    await user.click(screen.getByRole("button", { name: "Try again" }));
    expect(await screen.findByText("Summer Pockets")).toBeInTheDocument();
  });

  it("escapes regex characters in the query so a literal search still highlights", async () => {
    mocks.search.mockResolvedValueOnce(
      page([result({ note: "100% completed the route" })], false),
    );
    mount("100%");
    const hit = await screen.findByText("100%");
    expect(hit.tagName).toBe("MARK");
  });
});