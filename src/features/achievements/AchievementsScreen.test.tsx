import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, within } from "@testing-library/react";
import { MemoryRouter } from "react-router";

import { AppContext } from "@/app/AppProvider";
import { initialAchievementsBrowse, useAppStore } from "@/app/store";
import { createTranslator } from "@/i18n";
import type { AchievementView } from "@/lib/ipc/types";

const mocks = vi.hoisted(() => ({ list: vi.fn(), listen: vi.fn() }));
vi.mock("@/lib/ipc/achievements", () => ({
  achievementsList: mocks.list,
  listenAchievementsUnlocked: mocks.listen,
}));
vi.mock("@/lib/ipc/client", () => ({
  IpcError: class extends Error {
    code: string;
    constructor(code: string, message: string) {
      super(message);
      this.code = code;
    }
  },
}));

const { AchievementsScreen } = await import("./AchievementsScreen");
const { IpcError } = await import("@/lib/ipc/client");

function view(
  slug: string,
  tier: AchievementView["tier"],
  group: AchievementView["group"],
  unlocked: boolean,
  progress: number,
  target: number,
  unlockedAt: number | null = null,
): AchievementView {
  return { slug, tier, group, unlocked, progress, target, unlockedAt };
}

const catalog: AchievementView[] = [
  view("first-session", "bronze", "A", true, 1, 1, 1_700_000_000_000),
  view("first-finish", "bronze", "B", true, 1, 1, 1_700_000_000_001),
  view("sessions-100", "silver", "A", false, 40, 100),
  view("playtime-100h", "silver", "A", false, 0, 360_000_000),
  view("shot-first", "bronze", "E", false, 0, 1),
];

function renderScreen(locale: "en" | "id" = "en") {
  return render(
    <AppContext.Provider
      value={{ t: createTranslator(locale), locale, ready: true, bootError: null }}
    >
      <MemoryRouter>
        <AchievementsScreen />
      </MemoryRouter>
    </AppContext.Provider>,
  );
}

beforeEach(() => {
  vi.resetAllMocks();
  mocks.listen.mockImplementation(async () => vi.fn());
  mocks.list.mockResolvedValue(catalog);
  useAppStore.setState({ achievementsBrowse: initialAchievementsBrowse });
});

describe("achievements screen", () => {
  it("shows the overall ring summary and a card per catalog item", async () => {
    const { container } = renderScreen();
    expect(await screen.findByRole("img", { name: "2 of 5 unlocked" })).toBeInTheDocument();
    expect(container.querySelectorAll(".ach-grid").length).toBe(1);
    expect(container.querySelectorAll(".ach-card").length).toBe(5);
    // The ring is the focal point, present as a real progress image.
    expect(container.querySelector(".ach-head__summary .ring")).not.toBeNull();
  });

  it("pins the layout in containers, not only the text", async () => {
    const { container } = renderScreen();
    await screen.findByRole("img", { name: "2 of 5 unlocked" });
    const unlocked = container.querySelectorAll('.ach-card[data-unlocked="true"]');
    const locked = container.querySelectorAll('.ach-card[data-unlocked="false"]');
    expect(unlocked.length).toBe(2);
    expect(locked.length).toBe(3);
    // Unlocked cards carry a date and no progress bar; locked cards carry a bar.
    unlocked.forEach((card) => {
      expect(card.querySelector(".ach-card__date")).not.toBeNull();
      expect(card.querySelector(".ach-card__bar")).toBeNull();
    });
    locked.forEach((card) => {
      expect(card.querySelector(".ach-card__bar")).not.toBeNull();
    });
    // Tier is a label with a material step, present on every card.
    expect(container.querySelectorAll(".ach-card__tier").length).toBe(5);
    // The glyph is inlined as real SVG so currentColor follows the theme.
    expect(container.querySelector(".ach-card__icon svg")).not.toBeNull();
    // The screenshot group carries the honest platform label.
    expect(screen.getByText("Windows/Android")).toBeInTheDocument();
  });

  it("shows an unlock date on unlocked cards and progress on locked cards", async () => {
    const { container } = renderScreen();
    await screen.findByRole("img", { name: "2 of 5 unlocked" });
    const unlockedCard = container.querySelector('.ach-card[data-unlocked="true"]');
    expect(within(unlockedCard as HTMLElement).getByText(/Unlocked/)).toBeInTheDocument();
    const sessions = container.querySelector('.ach-card[data-tier="silver"]');
    const bar = sessions?.querySelector(".ach-card__bar");
    expect(bar).toHaveAttribute("aria-valuenow", "40");
    expect(bar).toHaveAttribute("aria-valuemax", "100");
    expect(within(sessions as HTMLElement).getByText("40/100")).toBeInTheDocument();
  });

  it("renders time-based progress as a duration, never raw milliseconds", async () => {
    const { container } = renderScreen();
    await screen.findByRole("img", { name: "2 of 5 unlocked" });
    const cards = [...container.querySelectorAll(".ach-card")];
    const playtime = cards.find(
      (card) => card.querySelector(".ach-card__name")?.textContent === "A Hundred Hours",
    );
    expect(playtime).toBeTruthy();
    expect(within(playtime as HTMLElement).getByText("0s/100h")).toBeInTheDocument();
    expect(within(playtime as HTMLElement).queryByText(/360000000/)).toBeNull();
  });

  it("renders streak progress in days", async () => {
    mocks.list.mockResolvedValue([view("streak-7", "silver", "A", false, 3, 7)]);
    renderScreen();
    await screen.findByRole("img", { name: "0 of 1 unlocked" });
    expect(screen.getByText("3/7 days")).toBeInTheDocument();
  });

  it("filters by status", async () => {
    const { container } = renderScreen();
    await screen.findByRole("img", { name: "2 of 5 unlocked" });
    fireEvent.click(screen.getByRole("button", { name: "Unlocked" }));
    expect(container.querySelectorAll(".ach-card").length).toBe(2);
    fireEvent.click(screen.getByRole("button", { name: "Locked" }));
    expect(container.querySelectorAll(".ach-card").length).toBe(3);
    fireEvent.click(screen.getByRole("button", { name: "All" }));
    expect(container.querySelectorAll(".ach-card").length).toBe(5);
  });

  it("filters by tier", async () => {
    const { container } = renderScreen();
    await screen.findByRole("img", { name: "2 of 5 unlocked" });
    fireEvent.click(screen.getByRole("button", { name: "Silver" }));
    expect(container.querySelectorAll(".ach-card").length).toBe(2);
    expect(container.querySelectorAll('.ach-card[data-tier="silver"]').length).toBe(2);
  });

  it("searches by name and condition", async () => {
    const { container } = renderScreen();
    await screen.findByRole("img", { name: "2 of 5 unlocked" });
    const search = screen.getByRole("searchbox", { name: "Search achievements" });
    fireEvent.change(search, { target: { value: "Finish" } });
    expect(container.querySelectorAll(".ach-card").length).toBe(1);
    expect(screen.getByText("First Finish")).toBeInTheDocument();
  });

  it("shows a distinct no-match state with a clear-filters action", async () => {
    const { container } = renderScreen();
    await screen.findByRole("img", { name: "2 of 5 unlocked" });
    fireEvent.change(screen.getByRole("searchbox", { name: "Search achievements" }), {
      target: { value: "zzzz-no-match" },
    });
    expect(container.querySelectorAll(".ach-card").length).toBe(0);
    expect(screen.getByText("No matches")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Clear filters" }));
    expect(container.querySelectorAll(".ach-card").length).toBe(5);
  });

  it("shows the empty state with a library action for a fresh library", async () => {
    mocks.list.mockResolvedValue([
      view("first-session", "bronze", "A", false, 0, 1),
      view("first-game", "bronze", "B", false, 0, 1),
    ]);
    renderScreen();
    expect(await screen.findByText("No achievements yet")).toBeInTheDocument();
    const action = screen.getByRole("link", { name: "Go to library" });
    expect(action).toHaveAttribute("href", "/");
  });

  it("shows a loading skeleton while the catalog is fetched", () => {
    mocks.list.mockReturnValue(new Promise(() => undefined));
    const { container } = renderScreen();
    expect(container.querySelector(".ach-grid--loading")).not.toBeNull();
    expect(container.querySelectorAll(".ach-card-skeleton").length).toBeGreaterThan(0);
    expect(screen.queryByRole("img", { name: "2 of 5 unlocked" })).toBeNull();
  });

  it("shows an error state with a working retry", async () => {
    mocks.list.mockRejectedValueOnce(new IpcError("db", "db down"));
    const { container } = renderScreen();
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("db down");
    mocks.list.mockResolvedValue(catalog);
    fireEvent.click(screen.getByRole("button", { name: "Try again" }));
    await screen.findByRole("img", { name: "2 of 5 unlocked" });
    expect(container.querySelectorAll(".ach-card").length).toBe(5);
  });

  it("renders Indonesian copy when the locale is id", async () => {
    renderScreen("id");
    expect(await screen.findByRole("img", { name: "2 dari 5 terbuka" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Pencapaian" })).toBeInTheDocument();
    expect(screen.getByText("Langkah Pertama")).toBeInTheDocument();
  });
});