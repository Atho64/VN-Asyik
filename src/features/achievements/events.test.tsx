import { beforeEach, describe, expect, it, vi } from "vitest";
import { act, render, waitFor } from "@testing-library/react";

import { AppContext } from "@/app/AppProvider";
import { useAppStore } from "@/app/store";
import { createTranslator } from "@/i18n";
import { buildUnlockToast, useAchievementUnlockToasts, useAchievementsRefresh } from "./events";

const mocks = vi.hoisted(() => ({
  listenAchievementsUnlocked: vi.fn(),
  achievementsEvaluate: vi.fn(),
}));
vi.mock("@/lib/ipc/achievements", () => ({
  listenAchievementsUnlocked: mocks.listenAchievementsUnlocked,
  achievementsEvaluate: mocks.achievementsEvaluate,
  ACHIEVEMENTS_REFRESH_EVENT: "vn:achievements-refresh",
}));

const t = createTranslator("en");

function ToastHarness() {
  useAchievementUnlockToasts();
  return null;
}
function RefreshHarness() {
  useAchievementsRefresh();
  return null;
}

function renderHarness(node: React.ReactNode) {
  return render(
    <AppContext.Provider value={{ t, locale: "en", ready: true, bootError: null }}>
      {node}
    </AppContext.Provider>,
  );
}

beforeEach(() => {
  vi.resetAllMocks();
  useAppStore.setState({ toasts: [] });
});

describe("buildUnlockToast", () => {
  it("names a single unlock", () => {
    const { kind, message } = buildUnlockToast(["first-session"], t);
    expect(kind).toBe("success");
    expect(message).toBe("Achievement unlocked: First Steps");
  });

  it("summarizes a batch in one toast naming a few, never one per slug", () => {
    const slugs = ["first-session", "first-game", "first-finish", "first-note", "first-route"];
    const { message } = buildUnlockToast(slugs, t);
    expect(message).toBe(
      "5 new achievements: First Steps, First on the Shelf, First Finish",
    );
  });

  it("stays one toast even for the whole retroactive catalog", () => {
    const slugs = Array.from({ length: 65 }, () => "first-session");
    const { message } = buildUnlockToast(slugs, t);
    expect(message.startsWith("65 new achievements:")).toBe(true);
  });
});

describe("useAchievementUnlockToasts", () => {
  it("pushes one batched toast when the backend reports unlocks", async () => {
    let handler: ((slugs: string[]) => void) | undefined;
    mocks.listenAchievementsUnlocked.mockImplementation(async (cb: typeof handler) => {
      handler = cb;
      return vi.fn();
    });
    renderHarness(<ToastHarness />);
    await waitFor(() => expect(handler).toBeDefined());

    act(() => handler?.(["first-session", "first-game", "first-finish", "first-note"]));
    const toasts = useAppStore.getState().toasts;
    expect(toasts).toHaveLength(1);
    expect(toasts[0]?.kind).toBe("success");
    expect(toasts[0]?.message).toContain("4 new achievements");
  });
});

describe("useAchievementsRefresh", () => {
  it("coalesces a burst of refresh events into one evaluate", async () => {
    mocks.achievementsEvaluate.mockResolvedValue([]);
    renderHarness(<RefreshHarness />);
    act(() => {
      window.dispatchEvent(new CustomEvent("vn:achievements-refresh"));
      window.dispatchEvent(new CustomEvent("vn:achievements-refresh"));
      window.dispatchEvent(new CustomEvent("vn:achievements-refresh"));
    });
    await waitFor(() => expect(mocks.achievementsEvaluate).toHaveBeenCalledTimes(1));
    // A second tick must not re-run: the burst already settled into one call.
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 5));
    });
    expect(mocks.achievementsEvaluate).toHaveBeenCalledTimes(1);
  });
});