import { beforeEach, describe, expect, it, vi } from "vitest";
import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { AppContext } from "@/app/AppProvider";
import { useAppStore } from "@/app/store";
import { createTranslator } from "@/i18n";

import { ForYouPanel } from "./ForYouPanel";

vi.mock("@/lib/ipc/client", () => ({
  api: {
    forYouProfile: vi.fn(),
    forYouSearch: vi.fn(),
    openExternal: vi.fn(),
  },
  IpcError: class extends Error {},
}));

const { api } = await import("@/lib/ipc/client");

const readyProfile = {
  considered: 23,
  ready: true,
  tags: [
    { id: "g505", name: "Cherry Blossom", count: 5, examples: [] },
    { id: "g902", name: "Island", count: 4, examples: [] },
  ],
  developers: [{ id: "p8", name: "KID", count: 3, examples: [] }],
  staff: [{ id: "s545", name: "Nakazawa Takumi", count: 3, examples: [] }],
  voices: [{ id: "s2001", name: "Suzumura Kenichi", count: 2, examples: [] }],
  topGames: ["Ever17", "CLANNAD"],
};

const plainCard = {
  vnId: "v17",
  title: "Ever17",
  alttitle: null,
  released: "2002-08-29",
  imageUrl: "https://t.vndb.org/cv/71/17871.jpg",
  sexual: 0,
  violence: 40,
  length: 4,
  lengthMinutes: 1987,
  rating: 80,
  votecount: 8800,
  devstatus: 0,
  developers: ["KID"],
  score: 7,
  because: "CLANNAD",
  themes: ["Cherry Blossom"],
  creators: ["KID"],
  voices: [],
};

function searchPage(items = [plainCard], extra = {}) {
  return { items, more: false, fromCache: false, note: null, ...extra };
}

beforeEach(() => {
  vi.clearAllMocks();
  useAppStore.setState({
    prefs: {
      theme: "dark",
      language: "en",
      nsfw: "blur",
      reduce_effects: false,
      beta_features: false,
    },
    toasts: [],
  });
  vi.mocked(api.forYouProfile).mockResolvedValue(readyProfile);
  vi.mocked(api.forYouSearch).mockResolvedValue(searchPage());
});

function renderPanel() {
  return render(
    <AppContext.Provider value={{ t: createTranslator("en"), locale: "en", ready: true, bootError: null }}>
      <ForYouPanel />
    </AppContext.Provider>,
  );
}

describe("for you panel", () => {
  it("asks for more games while the library is too small", async () => {
    vi.mocked(api.forYouProfile).mockResolvedValue({ ...readyProfile, considered: 1, ready: false });
    renderPanel();
    expect(await screen.findByText("Not enough to go on")).toBeInTheDocument();
    expect(api.forYouSearch).not.toHaveBeenCalled();
    expect(screen.queryByRole("tablist")).toBeNull();
  });

  it("shows the seeds and queries VNDB for the combined tab by default", async () => {
    renderPanel();
    expect(await screen.findByText("For you")).toBeInTheDocument();
    expect(screen.getByText("Based on the 23 VNs in your library, rated games counting most")).toBeInTheDocument();
    expect(screen.getByText("Cherry Blossom")).toBeInTheDocument();
    // One query, for the default source, with no filters attached.
    expect(api.forYouSearch).toHaveBeenCalledTimes(1);
    expect(api.forYouSearch).toHaveBeenCalledWith({ source: "combined", page: 1, sort: "taste" });
  });

  it("renders the result card with its explanations", async () => {
    renderPanel();
    const list = await screen.findByRole("list");
    // The layout lives in the container: pin it, not only the text.
    expect(list.className).toBe("foryou__list");
    const card = within(list).getByRole("listitem");
    expect(card.className).toBe("foryou-card");
    expect(card.querySelector(".foryou-card__art")).not.toBeNull();
    expect(within(card).getByText("8.0 · 2002 · KID · ~33h")).toBeInTheDocument();
    expect(within(card).getByText("Because you liked CLANNAD")).toBeInTheDocument();
    expect(within(card).getByText("Creators you like: KID")).toBeInTheDocument();
    expect(within(card).getByText("Themes you like: Cherry Blossom")).toBeInTheDocument();
  });

  it("switches the source tab and re-queries", async () => {
    renderPanel();
    await screen.findByRole("tablist");
    await userEvent.click(screen.getByRole("tab", { name: "Voices" }));
    await vi.waitFor(() => {
      expect(api.forYouSearch).toHaveBeenLastCalledWith(
        expect.objectContaining({ source: "voices" }),
      );
    });
  });

  it("hides an explicit cover under blur and shows it again under show", async () => {
    const explicit = { ...plainCard, vnId: "v99", title: "Explicit One", sexual: 150 };
    vi.mocked(api.forYouSearch).mockResolvedValue(searchPage([explicit, plainCard]));
    const { container } = renderPanel();
    await screen.findByRole("list");
    // A cover is a decorative image (alt=""), so query the DOM, not roles.
    // One image paints: only the safe cover.
    expect(container.querySelectorAll("img")).toHaveLength(1);
    act(() => {
      useAppStore.setState({
        prefs: { ...useAppStore.getState().prefs, nsfw: "show" },
      });
    });
    await vi.waitFor(() => {
      expect(container.querySelectorAll("img")).toHaveLength(2);
    });
  });

  it("opens the entry on VNDB through the external scheme", async () => {
    renderPanel();
    const card = await screen.findByRole("listitem");
    await userEvent.click(within(card).getByRole("button", { name: "Open Ever17 on VNDB" }));
    expect(api.openExternal).toHaveBeenCalledWith("https://vndb.org/v17");
  });

  it("translates an empty-source note instead of failing", async () => {
    vi.mocked(api.forYouSearch).mockResolvedValue(searchPage([], { note: "source" }));
    renderPanel();
    expect(
      await screen.findByText("No seeds for this tab yet. Refreshing your games' VNDB details fills it in."),
    ).toBeInTheDocument();
    expect(screen.queryByRole("list")).toBeNull();
  });

  it("refreshes both the profile and current search results", async () => {
    renderPanel();
    await screen.findByRole("list");
    expect(api.forYouSearch).toHaveBeenCalledTimes(1);
    await userEvent.click(screen.getByRole("button", { name: "Refresh" }));
    await vi.waitFor(() => {
      expect(api.forYouProfile).toHaveBeenCalledTimes(2);
      expect(api.forYouSearch).toHaveBeenCalledTimes(2);
    });
  });
});
