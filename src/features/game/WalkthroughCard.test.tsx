import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
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
      "小鳥": "Kotori",
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
    expect(screen.getByText("Seiya-Saiga Match")).toBeInTheDocument();
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
    expect(screen.getByText("2 characters from VNDB glossary")).toBeInTheDocument();
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

    const jaButton = await screen.findByRole("button", { name: "Japanese (Original)" });
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

    const input = await screen.findByPlaceholderText("Search game titles (Japanese / Romaji) on Seiya-Saiga...");
    await user.type(input, "rewrite");

    const searchBtn = screen.getByRole("button", { name: "Search" });
    await user.click(searchBtn);

    expect(api.walkthroughSearch).toHaveBeenCalledWith("rewrite");
    expect(await screen.findByText("Rewrite - PS3")).toBeInTheDocument();
    expect(screen.getByText("Search Results (1)")).toBeInTheDocument();
  });
});
