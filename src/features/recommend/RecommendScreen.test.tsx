import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { AppContext } from "@/app/AppProvider";
import { createTranslator } from "@/i18n";

vi.mock("@/lib/ipc/client", () => ({
  api: {
    aiRecommendSettings: vi.fn().mockResolvedValue({
      hasTavilyKey: true,
      tavilyKeyHint: "tvly-…abc",
      hasChatKey: true,
      supported: true,
      chatModel: "test-model",
    }),
    forYouProfile: vi.fn().mockResolvedValue({
      considered: 0,
      ready: false,
      tags: [],
      developers: [],
      staff: [],
      voices: [],
      topGames: [],
    }),
    forYouSearch: vi.fn(),
    openExternal: vi.fn(),
  },
  IpcError: class extends Error {},
}));

const { RecommendScreen } = await import("./RecommendScreen");

beforeEach(() => {
  vi.clearAllMocks();
});

function renderScreen(locale: "en" | "id" = "en") {
  return render(
    <AppContext.Provider
      value={{ t: createTranslator(locale), locale, ready: true, bootError: null }}
    >
      <RecommendScreen />
    </AppContext.Provider>,
  );
}

describe("the discover screen's two tabs", () => {
  it("opens on Ask with the key management and form, and For you one tab away", async () => {
    renderScreen();
    // The tab row reuses the library's switcher pattern.
    const tablist = screen.getByRole("tablist");
    expect(tablist.className).toBe("tabs");
    expect(screen.getByRole("tab", { name: "Ask" }).getAttribute("aria-selected")).toBe("true");
    expect(await screen.findByText("Search key stored")).toBeInTheDocument();

    await userEvent.click(screen.getByRole("tab", { name: "For you" }));
    expect(screen.getByRole("tab", { name: "For you" }).getAttribute("aria-selected")).toBe("true");
    expect(screen.getByRole("tab", { name: "Ask" }).getAttribute("aria-selected")).toBe("false");
    // The For-you panel replaced the ask flow, key row included.
    expect(screen.queryByText("Search key stored")).toBeNull();
    expect(screen.getByRole("heading", { name: "For you" })).toBeInTheDocument();
  });

  it("names both tabs in Indonesian too", () => {
    renderScreen("id");
    expect(screen.getByRole("tab", { name: "Tanya" })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "Untukmu" })).toBeInTheDocument();
  });
});
