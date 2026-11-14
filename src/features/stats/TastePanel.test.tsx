import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, within } from "@testing-library/react";
import { AppContext } from "@/app/AppProvider";
import { createTranslator } from "@/i18n";

import { TastePanel } from "./TastePanel";

vi.mock("@/lib/ipc/client", () => ({
  api: {
    aiTasteProfile: vi.fn(),
  },
  // The real signature, so a rejection carries its message like the wire does.
  IpcError: class extends Error {
    code: string;
    constructor(code: string, message: string) {
      super(message);
      this.code = code;
    }
  },
}));

const { api, IpcError } = await import("@/lib/ipc/client");

const seeded = {
  profile: {
    considered: 23,
    undetermined: ["Bare Game"],
    themes: [{ name: "Cherry Blossom", count: 5, examples: ["Ever17"], share: 0.2 }],
  },
  seeds: {
    ready: true,
    considered: 23,
    tags: [
      { id: "g505", name: "Cherry Blossom", count: 5, examples: ["Ever17", "CLANNAD"] },
      { id: "g902", name: "Island", count: 4, examples: ["Ever17"] },
    ],
    developers: [{ id: "p8", name: "KID", count: 3, examples: ["Ever17"] }],
    staff: [{ id: "s545", name: "Nakazawa Takumi", count: 2, examples: ["Ever17"] }],
    voices: [{ id: "s2001", name: "Suzumura Kenichi", count: 2, examples: ["Ever17"] }],
    topGames: ["Ever17", "CLANNAD"],
  },
};

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(api.aiTasteProfile).mockResolvedValue(seeded);
});

function renderPanel() {
  return render(
    <AppContext.Provider value={{ t: createTranslator("en"), locale: "en", ready: true, bootError: null }}>
      <TastePanel />
    </AppContext.Provider>,
  );
}

describe("taste panel", () => {
  it("renders every seed category with its counts and anchor games", async () => {
    renderPanel();
    expect(await screen.findByText("Based on 23 games")).toBeInTheDocument();
    expect(screen.getByText("Driven most by")).toBeInTheDocument();
    expect(screen.getByText("Ever17")).toBeInTheDocument();
    expect(screen.getByText("Themes")).toBeInTheDocument();
    expect(screen.getByText("Studios")).toBeInTheDocument();
    expect(screen.getByText("Writers & artists")).toBeInTheDocument();
    expect(screen.getByText("Voice actors")).toBeInTheDocument();
    expect(screen.getByText("Cherry Blossom · 5")).toBeInTheDocument();
    expect(screen.getByText("KID · 3")).toBeInTheDocument();
    expect(screen.getByText("Nakazawa Takumi · 2")).toBeInTheDocument();
    expect(screen.getByText("Suzumura Kenichi · 2")).toBeInTheDocument();
  });

  it("keeps the evidence in the containers, not just the text", async () => {
    // The layout lives in the container: pin the classes, not only the words.
    const { container } = renderPanel();
    await screen.findByText("Driven most by");
    const sections = container.querySelectorAll(".taste-section");
    expect(sections.length).toBeGreaterThanOrEqual(5);
    const chipLists = container.querySelectorAll(".taste-tags");
    expect(chipLists.length).toBeGreaterThanOrEqual(5);
    const chips = container.querySelectorAll(".taste-tags li");
    expect(chips.length).toBeGreaterThanOrEqual(7);
  });

  it("names the games behind a count on hover", async () => {
    renderPanel();
    const chip = await screen.findByText("Cherry Blossom · 5");
    expect(chip).toHaveAttribute("title", "Ever17, CLANNAD");
  });

  it("falls back to the plain tag count when nothing is rated", async () => {
    // Seeds want ratings; an unrated library must not lose its theme list.
    vi.mocked(api.aiTasteProfile).mockResolvedValue({
      profile: {
        considered: 4,
        undetermined: [],
        themes: [
          { name: "Nakige", count: 2, examples: ["A", "B"], share: 0.5 },
        ],
      },
      seeds: {
        ready: false,
        considered: 0,
        tags: [],
        developers: [],
        staff: [],
        voices: [],
        topGames: [],
      },
    });
    renderPanel();
    expect(await screen.findByText("Nakige · 2")).toBeInTheDocument();
    expect(screen.getByText("Themes")).toBeInTheDocument();
    // No anchors or creators exist, so those sections stay hidden.
    expect(screen.queryByText("Driven most by")).toBeNull();
    expect(screen.queryByText("Studios")).toBeNull();
  });

  it("asks for VNDB details when a ready library has no creator seeds", async () => {
    vi.mocked(api.aiTasteProfile).mockResolvedValue({
      profile: { considered: 5, undetermined: [], themes: [] },
      seeds: {
        ready: true,
        considered: 5,
        tags: [{ id: "g1", name: "Nakige", count: 3, examples: ["A", "B"] }],
        developers: [],
        staff: [],
        voices: [],
        topGames: ["A", "B", "C"],
      },
    });
    renderPanel();
    expect(
      await screen.findByText(
        "Refreshing your games' VNDB details from their pages fills this in.",
      ),
    ).toBeInTheDocument();
  });

  it("still names the games the profile could not describe", async () => {
    renderPanel();
    const notice = await screen.findByRole("status");
    expect(
      within(notice).getByText(
        "1 games have no tags, so they are not part of this: Bare Game.",
      ),
    ).toBeInTheDocument();
  });

  it("shows the empty state when there is nothing at all", async () => {
    vi.mocked(api.aiTasteProfile).mockResolvedValue({
      profile: { considered: 0, undetermined: [], themes: [] },
      seeds: {
        ready: false,
        considered: 0,
        tags: [],
        developers: [],
        staff: [],
        voices: [],
        topGames: [],
      },
    });
    renderPanel();
    expect(await screen.findByText("Not enough overlap yet")).toBeInTheDocument();
    expect(screen.queryByText("Themes")).toBeNull();
  });

  it("shows the backend error instead of a blank card", async () => {
    vi.mocked(api.aiTasteProfile).mockRejectedValue(new IpcError("db", "db down"));
    renderPanel();
    expect(await screen.findByRole("alert")).toHaveTextContent("db down");
  });
});
