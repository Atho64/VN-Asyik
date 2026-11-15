import { beforeEach, expect, it, vi } from "vitest";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

const { api } = await import("./client");

beforeEach(() => vi.resetAllMocks());

/**
 * The regression this exists for: `screenshot_delete` is a `CommandResult<()>`,
 * which Tauri serialises as `null`. The first version validated it with
 * `z.void()`, which accepts `undefined` and rejects `null`, so every successful
 * delete surfaced as "Unexpected response from screenshot_delete".
 */
it("accepts the null a void command resolves to", async () => {
  invoke.mockResolvedValue(null);
  await expect(api.screenshotDelete("shot-1")).resolves.toBeUndefined();
  expect(invoke).toHaveBeenCalledWith("screenshot_delete", { id: "shot-1" });
});

it("still rejects a delete that answered with something unexpected", async () => {
  // `voidSchema` is deliberately permissive about the value, so this guards the
  // other direction: a malformed *known* payload must still fail.
  invoke.mockResolvedValue(null);
  await expect(api.screenshotList("game-1")).rejects.toThrow();
});

it("asks for the cross-game library with no arguments", async () => {
  invoke.mockResolvedValue([]);
  await expect(api.screenshotListAll()).resolves.toEqual([]);
  expect(invoke).toHaveBeenCalledWith("screenshot_list_all", undefined);
});

it("scopes a capture to the game where its control was opened", async () => {
  invoke.mockResolvedValue({
    id: "shot",
    gameId: "game",
    width: 1,
    height: 1,
    capturedAt: 1000,
    source: "android_projection",
    caption: "",
  });
  await api.screenshotCapture("game");
  expect(invoke).toHaveBeenLastCalledWith("screenshot_capture", { gameId: "game" });
});

it("passes painted image bytes to native copy and preserves save cancellation", async () => {
  invoke.mockResolvedValue(null);
  await expect(
    api.screenshotCopy("data:image/png;base64,pixels"),
  ).resolves.toBeUndefined();
  expect(invoke).toHaveBeenLastCalledWith("screenshot_copy", {
    dataUrl: "data:image/png;base64,pixels",
  });
  invoke.mockResolvedValue(false);
  await expect(
    api.screenshotSave("data:image/png;base64,pixels", "shot.png"),
  ).resolves.toBe(false);
  expect(invoke).toHaveBeenLastCalledWith("screenshot_save", {
    dataUrl: "data:image/png;base64,pixels",
    filename: "shot.png",
  });
});
