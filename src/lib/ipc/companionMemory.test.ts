import { beforeEach, expect, it, vi } from "vitest";
const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
const { memoryApi, memoryViewSchema } = await import("./companionMemory");
beforeEach(() => vi.resetAllMocks());
it("passes explicit scope and optimistic revision to Rust", async () => {
  invoke.mockResolvedValue(null);
  await memoryApi.forget({ kind: "reading", id: "session" }, 17);
  expect(invoke).toHaveBeenCalledWith("companion_memory_forget", {
    target: { kind: "reading", id: "session" },
    revision: 17,
  });
});
it("rejects malformed memory capabilities instead of silently assuming enabled", () => {
  expect(
    memoryViewSchema.safeParse({ settings: { revision: 1 }, items: [], jobs: [] })
      .success,
  ).toBe(false);
});
