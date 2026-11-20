import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.hoisted(() => vi.fn());
const listen = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen }));

const {
  achievementsList,
  achievementsEvaluate,
  achievementsNoteDiscoverImport,
  listenAchievementsUnlocked,
  requestAchievementsRefresh,
  ACHIEVEMENTS_REFRESH_EVENT,
} = await import("./achievements");

const view = {
  slug: "first-session",
  tier: "bronze",
  group: "A",
  unlocked: true,
  progress: 1,
  target: 1,
  unlockedAt: 1_700_000_000_000,
};

beforeEach(() => {
  vi.resetAllMocks();
});

describe("achievementsList", () => {
  it("parses the camelCase catalog the backend returns", async () => {
    invoke.mockResolvedValue([view]);
    await expect(achievementsList()).resolves.toEqual([view]);
    expect(invoke).toHaveBeenCalledWith("achievements_list", undefined);
  });

  it("rejects a snake_case payload instead of silently dropping the date", async () => {
    // The regression this guards: a `unlocked_at` key would parse as undefined
    // and every unlocked card would lose its date without an error.
    invoke.mockResolvedValue([{ ...view, unlocked_at: view.unlockedAt, unlockedAt: undefined }]);
    await expect(achievementsList()).rejects.toThrow();
  });

  it("rejects an unknown tier", async () => {
    invoke.mockResolvedValue([{ ...view, tier: "platinum" }]);
    await expect(achievementsList()).rejects.toThrow();
  });
});

describe("achievementsEvaluate", () => {
  it("parses the bare slug array the command returns", async () => {
    invoke.mockResolvedValue(["first-session", "first-game"]);
    await expect(achievementsEvaluate()).resolves.toEqual(["first-session", "first-game"]);
    expect(invoke).toHaveBeenCalledWith("achievements_evaluate", undefined);
  });

  it("rejects an object payload: the command returns an array, the event an object", async () => {
    // `achievements_evaluate` is `Vec<String>`; only the emitted event wraps it
    // in `{ slugs }`. Accepting the object here would hide a contract drift.
    invoke.mockResolvedValue({ slugs: ["first-session"] });
    await expect(achievementsEvaluate()).rejects.toThrow();
  });
});

describe("achievementsNoteDiscoverImport", () => {
  it("accepts the null a void command resolves to", async () => {
    invoke.mockResolvedValue(null);
    await expect(achievementsNoteDiscoverImport()).resolves.toBeUndefined();
    expect(invoke).toHaveBeenCalledWith("achievements_note_discover_import", undefined);
  });
});

describe("listenAchievementsUnlocked", () => {
  it("validates the payload before calling back, and skips empty or malformed events", async () => {
    let handler: ((event: { payload: unknown }) => void) | undefined;
    listen.mockImplementation(async (_event: string, cb: typeof handler) => {
      handler = cb;
      return vi.fn();
    });
    const onUnlocked = vi.fn();
    await listenAchievementsUnlocked(onUnlocked);
    expect(listen).toHaveBeenCalledWith("vn:achievements-unlocked", expect.any(Function));

    handler?.({ payload: { slugs: ["first-session"] } });
    expect(onUnlocked).toHaveBeenCalledWith(["first-session"]);

    handler?.({ payload: { slugs: [] } });
    handler?.({ payload: { slugs: "first-session" } });
    handler?.({ payload: null });
    expect(onUnlocked).toHaveBeenCalledTimes(1);
  });
});

describe("requestAchievementsRefresh", () => {
  it("dispatches the window event the shell listener coalesces", () => {
    const seen = vi.fn();
    window.addEventListener(ACHIEVEMENTS_REFRESH_EVENT, seen);
    requestAchievementsRefresh();
    window.removeEventListener(ACHIEVEMENTS_REFRESH_EVENT, seen);
    expect(seen).toHaveBeenCalledTimes(1);
  });
});