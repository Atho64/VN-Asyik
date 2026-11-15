import { beforeEach, describe, expect, it, vi } from "vitest";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { AppContext } from "@/app/AppProvider";
import { useAppStore } from "@/app/store";
import { createTranslator } from "@/i18n";
import { WalkthroughCard } from "./WalkthroughCard";
import { api } from "@/lib/ipc/client";
import type { WalkthroughView } from "@/lib/ipc/types";

vi.mock("@/lib/ipc/client", () => ({
  api: {
    walkthroughGet: vi.fn(),
    walkthroughSearchCandidate: vi.fn(),
    walkthroughFetch: vi.fn(),
    walkthroughTranslate: vi.fn(),
    walkthroughSaveManual: vi.fn(),
    walkthroughToggleItem: vi.fn(),
    walkthroughClearProgress: vi.fn(),
    walkthroughSearch: vi.fn(),
    openExternal: vi.fn(),
  },
  IpcError: class extends Error {},
}));

function mockWalkthrough(): WalkthroughView {
  return {
    gameId: "g1",
    title: "Rewrite+",
    sourceUrl: "https://galge.seiya-saiga.com/game/key/rewrite_plus.html",
    notes: "1周目は小鳥、ちはや攻略可能。",
    routes: [
      {
        id: "route_0",
        title: "共通・小鳥、ちはや",
        items: [
          {
            id: "s_1",
            kind: "save",
            text: "◆セーブ01から",
          },
          {
            id: "c_1",
            kind: "choice",
            text: "男の態度が気に食わないのでもう帰る",
          },
          {
            id: "e_1",
            kind: "ending",
            text: "共通ルートBAD回収終了",
          },
        ],
      },
    ],
    checkedItems: [],
    translations: {
      ai_en: [
        {
          id: "route_0",
          title: "Common - Kotori, Chihaya",
          items: [
            {
              id: "s_1",
              kind: "save",
              text: "From Save 01",
            },
            {
              id: "c_1",
              kind: "choice",
              text: "Go home because the guy's attitude is annoying",
            },
            {
              id: "e_1",
              kind: "ending",
              text: "Common Route BAD End",
            },
          ],
        },
      ],
    },
    characterGlossary: {
      "神戸 小鳥": "Kotori Kanbe",
      小鳥: "Kotori",
    },
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  useAppStore.setState({ toasts: [], platform: null });
  vi.mocked(api.walkthroughGet).mockResolvedValue(null);
  vi.mocked(api.walkthroughSearchCandidate).mockResolvedValue([]);
  vi.mocked(api.walkthroughSearch).mockResolvedValue([]);
  vi.mocked(api.walkthroughFetch).mockResolvedValue(mockWalkthrough());
  vi.mocked(api.walkthroughToggleItem).mockResolvedValue(["c_1"]);
  vi.mocked(api.walkthroughClearProgress).mockResolvedValue(undefined);
});

function renderCard() {
  return render(
    <AppContext.Provider
      value={{ t: createTranslator("en"), locale: "en", ready: true, bootError: null }}
    >
      <WalkthroughCard gameId="g1" vnId="v751" />
    </AppContext.Provider>,
  );
}

describe("WalkthroughCard", () => {
  it("edits text beside the original and saves a separate manual copy with checked choices", async () => {
    const original = mockWalkthrough();
    original.checkedItems = ["c_1"];
    vi.mocked(api.walkthroughGet).mockResolvedValue(original);
    const saved = structuredClone(original);
    saved.translations.manual_en = structuredClone(original.translations.ai_en!);
    saved.translations.manual_en[0]!.items[1]!.text =
      "Leave because his attitude annoys you";
    saved.checkedItems = [];
    vi.mocked(api.walkthroughSaveManual).mockResolvedValue(saved);
    renderCard();
    await userEvent.click(
      await screen.findByRole("button", { name: "Edit translation" }),
    );
    const form = screen.getByRole("form", { name: "Edit translation" });
    expect(form).toHaveClass("walkthrough__editor");
    expect(screen.getByText("男の態度が気に食わないのでもう帰る")).toBeInTheDocument();
    const choice = screen.getByRole("textbox", { name: "2. Choice translation" });
    await userEvent.clear(choice);
    await userEvent.type(choice, "Leave because his attitude annoys you");
    await userEvent.click(screen.getByRole("button", { name: "Save my translation" }));
    await screen.findByText("Leave because his attitude annoys you");
    expect(api.walkthroughSaveManual).toHaveBeenCalledWith(
      "g1",
      "en",
      original.sourceUrl,
      original.routes,
      [
        "Common - Kotori, Chihaya",
        "From Save 01",
        "Leave because his attitude annoys you",
        "Common Route BAD End",
      ],
      "v751",
    );
    expect(screen.getByRole("checkbox")).toBeChecked();
    expect(screen.getByRole("button", { name: "My translation" })).toBeInTheDocument();
    expect(screen.queryByRole("form", { name: "Edit translation" })).toBeNull();
  });

  it("keeps failed manual edits for retry and prevents blank translations", async () => {
    vi.mocked(api.walkthroughGet).mockResolvedValue(mockWalkthrough());
    vi.mocked(api.walkthroughSaveManual).mockRejectedValue(
      new Error("Source guide changed"),
    );
    renderCard();
    await userEvent.click(
      await screen.findByRole("button", { name: "Edit translation" }),
    );
    const title = screen.getByRole("textbox", { name: "Translated route title" });
    await userEvent.clear(title);
    expect(screen.getByRole("button", { name: "Save my translation" })).toBeDisabled();
    await userEvent.type(title, "Edited Kotori route");
    await userEvent.click(screen.getByRole("button", { name: "Save my translation" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Source guide changed");
    expect(title).toHaveValue("Edited Kotori route");
    await userEvent.keyboard("{Escape}");
    expect(screen.queryByRole("form", { name: "Edit translation" })).toBeNull();
    expect(screen.getByText("Common - Kotori, Chihaya")).toBeInTheDocument();
  });

  it("keeps drafts when changing editor routes and saves them in source order", async () => {
    const data = mockWalkthrough();
    data.routes.push({
      id: "route_1",
      title: "ルチア",
      items: [{ id: "c_2", kind: "choice", text: "ルチアを選ぶ" }],
    });
    data.translations.ai_en!.push({
      id: "route_1",
      title: "Lucia",
      items: [{ id: "c_2", kind: "choice", text: "Choose Lucia" }],
    });
    vi.mocked(api.walkthroughGet).mockResolvedValue(data);
    vi.mocked(api.walkthroughSaveManual).mockResolvedValue(data);
    renderCard();
    await userEvent.click(
      await screen.findByRole("button", { name: "Edit translation" }),
    );
    const title = screen.getByRole("textbox", { name: "Translated route title" });
    await userEvent.clear(title);
    await userEvent.type(title, "First route edited");
    const select = screen.getByRole("combobox", { name: "Routes" });
    await userEvent.selectOptions(select, "1");
    const second = screen.getByRole("textbox", { name: "1. Choice translation" });
    await userEvent.clear(second);
    await userEvent.type(second, "Pick Lucia");
    await userEvent.selectOptions(select, "0");
    expect(screen.getByRole("textbox", { name: "Translated route title" })).toHaveValue(
      "First route edited",
    );
    await userEvent.click(screen.getByRole("button", { name: "Save my translation" }));
    expect(api.walkthroughSaveManual).toHaveBeenCalledWith(
      "g1",
      "en",
      data.sourceUrl,
      data.routes,
      [
        "First route edited",
        "From Save 01",
        "Go home because the guy's attitude is annoying",
        "Common Route BAD End",
        "Lucia",
        "Pick Lucia",
      ],
      "v751",
    );
  });

  it("loads manual translations first and cancelling an edit writes nothing", async () => {
    const data = mockWalkthrough();
    data.translations.manual_en = structuredClone(data.translations.ai_en!);
    data.translations.manual_en[0]!.title = "My Kotori route";
    vi.mocked(api.walkthroughGet).mockResolvedValue(data);
    renderCard();
    await screen.findByText("My Kotori route");
    await userEvent.click(screen.getByRole("button", { name: "Edit translation" }));
    await userEvent.click(screen.getByRole("button", { name: "Cancel edits" }));
    expect(api.walkthroughSaveManual).not.toHaveBeenCalled();
    expect(screen.getByText("My Kotori route")).toBeInTheDocument();
  });

  it("discards a late manual save reply after changing games", async () => {
    vi.mocked(api.walkthroughGet).mockResolvedValue(mockWalkthrough());
    let resolve!: (view: WalkthroughView) => void;
    vi.mocked(api.walkthroughSaveManual).mockReturnValue(
      new Promise((done) => {
        resolve = done;
      }),
    );
    const rendered = renderCard();
    await userEvent.click(
      await screen.findByRole("button", { name: "Edit translation" }),
    );
    await userEvent.click(screen.getByRole("button", { name: "Save my translation" }));
    const other = mockWalkthrough();
    other.gameId = "g2";
    other.translations = {};
    other.routes[0]!.title = "Another game";
    vi.mocked(api.walkthroughGet).mockResolvedValue(other);
    rendered.rerender(
      <AppContext.Provider
        value={{ t: createTranslator("en"), locale: "en", ready: true, bootError: null }}
      >
        <WalkthroughCard gameId="g2" vnId="v2" />
      </AppContext.Provider>,
    );
    await screen.findByText("Another game");
    await act(async () => resolve(mockWalkthrough()));
    expect(screen.getByText("Another game")).toBeInTheDocument();
    expect(useAppStore.getState().toasts).toHaveLength(0);
  });

  it.each(["ai", "google"] as const)(
    "regenerates a cached %s translation and keeps current checked progress",
    async (provider) => {
      const original = mockWalkthrough();
      original.translations.google_en = structuredClone(original.translations.ai_en!);
      vi.mocked(api.walkthroughGet).mockResolvedValue(original);
      let resolve!: (view: WalkthroughView) => void;
      vi.mocked(api.walkthroughTranslate).mockReturnValue(
        new Promise((done) => {
          resolve = done;
        }),
      );
      renderCard();
      await screen.findByText("Common - Kotori, Chihaya");
      if (provider === "google")
        await userEvent.click(screen.getByRole("button", { name: "Google Translate" }));
      expect(api.walkthroughTranslate).not.toHaveBeenCalled();
      const refresh = screen.getByRole("button", {
        name:
          provider === "ai" ? "Refresh model translation" : "Refresh Google translation",
      });
      expect(refresh.closest(".walkthrough__translation-actions")).toBeInTheDocument();
      await userEvent.click(refresh);
      expect(refresh).toBeDisabled();
      expect(api.walkthroughTranslate).toHaveBeenCalledWith("g1", provider, "en", "v751");
      await userEvent.click(screen.getByRole("checkbox"));
      await waitFor(() => expect(api.walkthroughToggleItem).toHaveBeenCalled());
      const refreshed = structuredClone(original);
      refreshed.translations[`${provider}_en`]![0]!.title = "Refreshed Kotori route";
      await act(async () => resolve(refreshed));
      expect(await screen.findByText("Refreshed Kotori route")).toBeInTheDocument();
      expect(screen.getByRole("checkbox")).toBeChecked();
      await userEvent.click(screen.getByRole("button", { name: "Japanese (original)" }));
      expect(screen.getByText("共通・小鳥、ちはや")).toBeInTheDocument();
    },
  );

  it("keeps the previous translation and progress after a failed refresh", async () => {
    const original = mockWalkthrough();
    original.checkedItems = ["c_1"];
    vi.mocked(api.walkthroughGet).mockResolvedValue(original);
    vi.mocked(api.walkthroughTranslate).mockRejectedValue(
      new Error("Translation unavailable"),
    );
    renderCard();
    await userEvent.click(
      await screen.findByRole("button", { name: "Refresh model translation" }),
    );
    expect(screen.getByText("Common - Kotori, Chihaya")).toBeInTheDocument();
    expect(screen.getByRole("checkbox")).toBeChecked();
    await waitFor(() =>
      expect(useAppStore.getState().toasts.at(-1)?.message).toContain(
        "Translation unavailable",
      ),
    );
    expect(
      screen.getByRole("button", { name: "Refresh model translation" }),
    ).toBeEnabled();
  });

  it("discards translation replies after changing games", async () => {
    vi.mocked(api.walkthroughGet).mockResolvedValue(mockWalkthrough());
    let resolve!: (view: WalkthroughView) => void;
    vi.mocked(api.walkthroughTranslate).mockReturnValue(
      new Promise((done) => {
        resolve = done;
      }),
    );
    const rendered = renderCard();
    await userEvent.click(
      await screen.findByRole("button", { name: "Refresh model translation" }),
    );
    const other = mockWalkthrough();
    other.gameId = "g2";
    other.translations = {};
    other.routes[0]!.title = "別のゲーム";
    vi.mocked(api.walkthroughGet).mockResolvedValue(other);
    rendered.rerender(
      <AppContext.Provider
        value={{ t: createTranslator("en"), locale: "en", ready: true, bootError: null }}
      >
        <WalkthroughCard gameId="g2" vnId="v2" />
      </AppContext.Provider>,
    );
    await screen.findByText("別のゲーム");
    await act(async () => resolve(mockWalkthrough()));
    expect(screen.getByText("別のゲーム")).toBeInTheDocument();
    expect(screen.queryByText("Common - Kotori, Chihaya")).toBeNull();
    expect(useAppStore.getState().toasts).toHaveLength(0);
  });

  it("shows empty state and suggests candidate if found", async () => {
    vi.mocked(api.walkthroughSearchCandidate).mockResolvedValue([
      {
        title: "Rewrite+",
        url: "https://galge.seiya-saiga.com/game/key/rewrite_plus.html",
      },
    ]);

    renderCard();

    expect(
      await screen.findByText(
        "No walkthrough loaded yet. Search automatically from Seiya-Saiga's directory or enter a guide URL directly.",
      ),
    ).toBeInTheDocument();

    expect(await screen.findByText("Rewrite+")).toBeInTheDocument();
    expect(screen.getByText("Seiya-Saiga match")).toBeInTheDocument();
  });

  it("renders routes, save markers, and checkable choices when walkthrough is loaded", async () => {
    vi.mocked(api.walkthroughGet).mockResolvedValue(mockWalkthrough());

    renderCard();

    expect(await screen.findByText("Walkthrough")).toBeInTheDocument();
    // Default is translated when translation is available
    expect(await screen.findByText("Common - Kotori, Chihaya")).toBeInTheDocument();
    expect(screen.getByText("From Save 01")).toBeInTheDocument();
    expect(
      screen.getByText("Go home because the guy's attitude is annoying"),
    ).toBeInTheDocument();
    expect(screen.getByText("Common Route BAD End")).toBeInTheDocument();

    // Check glossary badge
    expect(screen.getByText("2 VNDB name mappings")).toBeInTheDocument();
  });

  it("toggles choice checkbox item on click", async () => {
    const user = userEvent.setup();
    vi.mocked(api.walkthroughGet).mockResolvedValue(mockWalkthrough());

    renderCard();

    const checkbox = (await screen.findByRole("checkbox")) as HTMLInputElement;
    expect(checkbox.checked).toBe(false);

    await user.click(checkbox);
    expect(api.walkthroughToggleItem).toHaveBeenCalledWith("g1", "c_1");
  });

  it("allows switching language mode back to original Japanese", async () => {
    const user = userEvent.setup();
    vi.mocked(api.walkthroughGet).mockResolvedValue(mockWalkthrough());

    renderCard();

    const jaButton = await screen.findByRole("button", { name: "Japanese (original)" });
    await user.click(jaButton);

    expect(await screen.findByText("共通・小鳥、ちはや")).toBeInTheDocument();
    expect(screen.getByText("男の態度が気に食わないのでもう帰る")).toBeInTheDocument();
  });
  it("allows searching Seiya-Saiga catalog and displays results", async () => {
    const user = userEvent.setup();
    vi.mocked(api.walkthroughSearch).mockResolvedValue([
      {
        title: "Rewrite - PS3",
        url: "https://galge.seiya-saiga.com/game/key/rewrite_ps3.html",
      },
    ]);

    renderCard();

    const input = await screen.findByPlaceholderText(
      "Search game titles (Japanese / Romaji) on Seiya-Saiga...",
    );
    await user.type(input, "rewrite");

    const searchBtn = screen.getByRole("button", { name: "Search" });
    await user.click(searchBtn);

    expect(api.walkthroughSearch).toHaveBeenCalledWith("rewrite");
    expect(await screen.findByText("Rewrite - PS3")).toBeInTheDocument();
    expect(screen.getByText("Search results (1)")).toBeInTheDocument();
  });
});
