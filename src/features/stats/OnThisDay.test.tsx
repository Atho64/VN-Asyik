import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { MemoryRouter } from "react-router";

import { AppContext } from "@/app/AppProvider";
import { createTranslator } from "@/i18n";
import type { Game, MemoryEvent } from "@/lib/ipc/types";
import { OnThisDay } from "./OnThisDay";

const mocks = vi.hoisted(() => ({ onThisDay: vi.fn() }));
vi.mock("@/lib/ipc/client", () => ({ api: mocks }));

const game: Game = {
  meta: { id: "g1", updated_at: 1, device_id_device: "test", deleted: false },
  title: "Symphonic Rain",
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

const memory: MemoryEvent = {
  kind: "journal",
  ts: Date.UTC(2025, 10, 8, 21, 14),
  id: "j1",
  game,
  note: "The rain scene, again",
  routeName: "Fal",
  chapterName: "Chapter 3",
  endingName: null,
  yearsAgo: 1,
};

function mount() {
  render(
    <MemoryRouter>
      <AppContext.Provider
        value={{ t: createTranslator("en"), locale: "en", ready: true, bootError: null }}
      >
        <OnThisDay />
      </AppContext.Provider>
    </MemoryRouter>,
  );
}

beforeEach(() => {
  vi.resetAllMocks();
});

describe("on this day", () => {
  it("resurfaces a checkpoint with the years-ago label and its game", async () => {
    mocks.onThisDay.mockResolvedValue([memory]);
    mount();
    expect(await screen.findByText("1 year ago")).toBeInTheDocument();
    expect(screen.getByText("Symphonic Rain")).toBeInTheDocument();
    expect(screen.getByText("The rain scene, again")).toBeInTheDocument();
    expect(screen.getByText("Fal · Chapter 3")).toBeInTheDocument();
    expect(screen.getByRole("link")).toHaveAttribute("href", "/game/g1");
  });

  it("names the ending instead of a note for an ending memory", async () => {
    mocks.onThisDay.mockResolvedValue([
      { ...memory, kind: "ending", note: null, endingName: "True End", yearsAgo: 2 },
    ]);
    mount();
    expect(await screen.findByText("2 years ago")).toBeInTheDocument();
    expect(screen.getByText("Reached the ending: True End")).toBeInTheDocument();
  });

  it("renders nothing on a day with no anniversary", async () => {
    // Not an empty state: a memory with no data is simply not a memory, and a
    // permanent "nothing happened today" panel would be furniture on the page.
    mocks.onThisDay.mockResolvedValue([]);
    const { container } = render(
      <MemoryRouter>
        <AppContext.Provider
          value={{ t: createTranslator("en"), locale: "en", ready: true, bootError: null }}
        >
          <OnThisDay />
        </AppContext.Provider>
      </MemoryRouter>,
    );
    await Promise.resolve();
    expect(container).toBeEmptyDOMElement();
  });

  it("stays out of the way when the query fails", async () => {
    mocks.onThisDay.mockRejectedValue(new Error("offline"));
    const { container } = render(
      <MemoryRouter>
        <AppContext.Provider
          value={{ t: createTranslator("en"), locale: "en", ready: true, bootError: null }}
        >
          <OnThisDay />
        </AppContext.Provider>
      </MemoryRouter>,
    );
    await Promise.resolve();
    expect(container).toBeEmptyDOMElement();
  });
});
