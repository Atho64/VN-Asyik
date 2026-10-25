import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

const { api, IpcError } = await import("./client");
const preview = {
  id: "native-draft",
  kind: "backup",
  createdAtMs: 1_792_590_400_000,
  games: 2,
  chapters: 4,
  sessions: 7,
  endings: 3,
  sameDevice: true,
  canRestore: true,
};

beforeEach(() => {
  vi.resetAllMocks();
});

describe("native data import contract", () => {
  it("validates the lightweight preview and preserves picker cancellation", async () => {
    invoke.mockResolvedValueOnce(preview).mockResolvedValueOnce(null);
    await expect(api.dataOpenFile()).resolves.toEqual(preview);
    await expect(api.dataOpenFile()).resolves.toBeNull();
    expect(invoke).toHaveBeenNthCalledWith(1, "data_open_file", undefined);
  });

  it("accepts the unified library preview", async () => {
    const library = { ...preview, kind: "library" };
    invoke.mockResolvedValue(library);
    await expect(api.dataOpenFile()).resolves.toEqual(library);
  });

  it("passes explicit restore only when requested", async () => {
    invoke.mockResolvedValue({
      kind: "backup",
      summary: { games: 2, chapters: 4, sessions: 7, endings: 3, sessions_skipped: 1 },
    });
    await api.dataApplyImport("native-draft", "restore");
    expect(invoke).toHaveBeenCalledExactlyOnceWith("data_apply_import", {
      id: "native-draft",
      mode: "restore",
    });
  });

  it.each([
    { ...preview, kind: "unsupported" },
    { ...preview, games: -1 },
    { ...preview, sessions: 1.5 },
    { ...preview, createdAtMs: "yesterday" },
  ])("rejects malformed previews before rendering", async (value) => {
    invoke.mockResolvedValue(value);
    await expect(api.dataOpenFile()).rejects.toMatchObject({ code: "contract" });
  });

  it.each([
    {
      kind: "backup",
      summary: { games: 2, chapters: 4, sessions: 7, endings: 3, sessions_skipped: 1 },
    },
    {
      kind: "transfer",
      outcome: { peers: 1, pulled: 2, pushed: 0, conflicts: [], skipped: [] },
    },
  ])("validates apply outcomes while passing only the preview token", async (result) => {
    invoke.mockResolvedValue(result);
    await expect(api.dataApplyImport("native-draft")).resolves.toEqual(result);
    expect(invoke).toHaveBeenCalledExactlyOnceWith("data_apply_import", {
      id: "native-draft",
      mode: "merge",
    });
  });

  it("does not accept the wrong outcome for the detected kind", async () => {
    invoke.mockResolvedValue({
      kind: "backup",
      outcome: { peers: 1, pulled: 2, pushed: 0, conflicts: [], skipped: [] },
    });
    await expect(api.dataApplyImport("native-draft")).rejects.toBeInstanceOf(IpcError);
  });

  it("cancels a native draft and shares the unified library file", async () => {
    invoke.mockResolvedValue(null);
    await expect(api.dataCancelImport("native-draft")).resolves.toBeUndefined();
    await expect(api.dataShareFile()).resolves.toBeUndefined();
    expect(invoke).toHaveBeenNthCalledWith(1, "data_cancel_import", {
      id: "native-draft",
    });
    expect(invoke).toHaveBeenNthCalledWith(2, "data_share_file", undefined);
  });
});
