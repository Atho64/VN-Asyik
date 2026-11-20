import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router";

import { AppContext } from "@/app/AppProvider";
import { createTranslator } from "@/i18n";
import type { AchievementView } from "@/lib/ipc/types";

const mocks = vi.hoisted(() => ({ list: vi.fn(), listen: vi.fn() }));
vi.mock("@/lib/ipc/achievements", () => ({
  achievementsList: mocks.list,
  listenAchievementsUnlocked: mocks.listen,
}));

const { AchievementStatsCard } = await import("./AchievementStatsCard");

function view(slug: string, unlocked: boolean, unlockedAt: number | null): AchievementView {
  return { slug, tier: "bronze", group: "A", unlocked, progress: 1, target: 1, unlockedAt };
}

const catalog: AchievementView[] = [
  view("first-session", true, 1_700_000_000_000),
  view("first-finish", true, 1_700_000_000_001),
  view("sessions-100", false, null),
];

function renderCard() {
  return render(
    <AppContext.Provider
      value={{ t: createTranslator("en"), locale: "en", ready: true, bootError: null }}
    >
      <MemoryRouter>
        <AchievementStatsCard />
      </MemoryRouter>
    </AppContext.Provider>,
  );
}

beforeEach(() => {
  vi.resetAllMocks();
  mocks.listen.mockImplementation(async () => vi.fn());
  mocks.list.mockResolvedValue(catalog);
});

describe("achievement stats card", () => {
  it("shows the unlocked count, the most recent unlock, and a link to the screen", async () => {
    renderCard();
    expect(await screen.findByText("2 of 3 unlocked")).toBeInTheDocument();
    expect(screen.getByText("Latest: First Finish")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "View all" })).toHaveAttribute(
      "href",
      "/achievements",
    );
  });

  it("says so honestly when nothing is unlocked yet", async () => {
    mocks.list.mockResolvedValue([view("first-session", false, null)]);
    renderCard();
    expect(await screen.findByText("No achievements unlocked yet.")).toBeInTheDocument();
    expect(screen.getByText("0 of 1 unlocked")).toBeInTheDocument();
  });

  it("shows a skeleton while loading", () => {
    mocks.list.mockReturnValue(new Promise(() => undefined));
    const { container } = renderCard();
    expect(container.querySelector(".skeleton")).not.toBeNull();
  });

  it("keeps the link available when the summary fails to load", async () => {
    mocks.list.mockRejectedValue(new Error("db down"));
    renderCard();
    expect(await screen.findByRole("alert")).toHaveTextContent("Could not load achievements");
    expect(screen.getByRole("link", { name: "View all" })).toHaveAttribute(
      "href",
      "/achievements",
    );
  });
});