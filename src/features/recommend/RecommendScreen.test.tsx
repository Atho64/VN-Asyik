import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { AppContext } from "@/app/AppProvider";
import { createTranslator } from "@/i18n";

vi.mock("@/lib/ipc/client", () => ({
  api: {
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

describe("Discover screen", () => {
  it("renders the VNDB For-you panel directly without AI tabs or API key setup", async () => {
    renderScreen();
    expect(
      screen.getByRole("heading", { name: "What to read next" }),
    ).toBeInTheDocument();
    expect(screen.queryByRole("tablist")).toBeNull();
    expect(screen.queryByText("Search key stored")).toBeNull();
    expect(await screen.findByRole("heading", { name: "For you" })).toBeInTheDocument();
  });

  it("uses Indonesian copy when locale is id", async () => {
    renderScreen("id");
    expect(
      screen.getByRole("heading", { name: "Apa yang dibaca berikutnya" }),
    ).toBeInTheDocument();
    expect(await screen.findByRole("heading", { name: "Untukmu" })).toBeInTheDocument();
  });
});
