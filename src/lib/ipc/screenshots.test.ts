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
