import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { open } from "@tauri-apps/plugin-dialog";
import { AppContext } from "@/app/AppProvider";
import { useAppStore } from "@/app/store";
import { createTranslator } from "@/i18n";
import { ScriptCard } from "./ScriptCard";
import { scriptApi } from "@/lib/ipc/script";
import { IpcError } from "@/lib/ipc/client";
import type { ScriptDocument } from "@/lib/ipc/script";

vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn(),
}));

vi.mock("@/lib/ipc/script", () => ({
  DEFAULT_AGENT_COUNT: 3,
  scriptApi: {
    import: vi.fn(),
    list: vi.fn(),
    delete: vi.fn(),
    compact: vi.fn(),
    agentSettings: vi.fn(),
    saveAgentSettings: vi.fn(),
  },
}));

function doc(overrides: Partial<ScriptDocument> = {}): ScriptDocument {
  return {
    id: "d1",
    gameId: "g1",
    engine: "kirikiri",
    sourceKind: "plain_text",
    sourcePath: "C:\\games\\Saku\\script.txt",
    sourceSha256: "ab".repeat(32),
    lineCount: 4_512,
    byteSize: 486_400,
    createdAt: 1_760_000_000_000,
    compaction: null,
    ...overrides,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  useAppStore.setState({ toasts: [], platform: null });
  vi.mocked(open).mockResolvedValue(null);
  vi.mocked(scriptApi.list).mockResolvedValue([]);
  vi.mocked(scriptApi.delete).mockResolvedValue(undefined);
  vi.mocked(scriptApi.import).mockResolvedValue(doc());
  vi.mocked(scriptApi.agentSettings).mockResolvedValue({ agents: 3 });
  vi.mocked(scriptApi.saveAgentSettings).mockImplementation(async ({ agents }) => ({
    agents,
  }));
});

function renderCard() {
  return render(
    <AppContext.Provider
      value={{ t: createTranslator("en"), locale: "en", ready: true, bootError: null }}
    >
      <ScriptCard gameId="g1" />
    </AppContext.Provider>,
  );
}

describe("ScriptCard", () => {
  it("shows the empty state when nothing is imported", async () => {
    renderCard();

    expect(await screen.findByText("No script imported yet. Extract one from the game with your own tools, then import the text file here.")).toBeInTheDocument();
  });

  it("lists a document inside its own row container with kind and size", async () => {
    vi.mocked(scriptApi.list).mockResolvedValue([doc()]);
    renderCard();

    // The list and row carry the layout (AGENTS.md §11): pin the classes, not
    // just the text, so an unreadable row cannot pass as a working card.
    const list = await waitFor(() => {
      const found = document.querySelector(".script-list");
      if (!found) throw new Error("script list missing");
      return found;
    });
    expect(list.querySelector(".script-row")).not.toBeNull();
    expect(screen.getByText("script.txt")).toBeInTheDocument();
    expect(screen.getByText(/Plain text/)).toBeInTheDocument();
    expect(screen.getByText(/4512 lines/)).toBeInTheDocument();
    // 486_400 bytes is exactly 475 KB, so the formatter's rounding is stable.
    expect(screen.getByText(/475.0 KB/)).toBeInTheDocument();
    expect(screen.getByText("Not summarized")).toBeInTheDocument();
  });

  it("shows the digest state without a summarize button once done", async () => {
    vi.mocked(scriptApi.list).mockResolvedValue([
      doc({
        compaction: {
          modelId: "qwen",
          chunkCount: 3,
          doneChunks: 3,
          agentCount: 1,
          compactorVersion: 2,
          currentVersion: 2,
          digestOutdated: false,
          digest: "The story…",
        },
      }),
    ]);
    renderCard();

    expect(await screen.findByText("Digest ready")).toBeInTheDocument();
    expect(
      document.querySelector(".script-row__state--done"),
    ).not.toBeNull();
    expect(screen.queryByRole("button", { name: "Summarize" })).not.toBeInTheDocument();
  });

  it("offers a rebuild for a digest written by an older summarizer", async () => {
    vi.mocked(scriptApi.list).mockResolvedValue([
      doc({
        compaction: {
          modelId: "qwen",
          chunkCount: 3,
          doneChunks: 3,
          agentCount: 1,
          compactorVersion: 1,
          currentVersion: 2,
          digestOutdated: true,
          digest: "A short old summary.",
        },
      }),
    ]);
    vi.mocked(scriptApi.compact).mockResolvedValue({
      modelId: "qwen",
      chunkCount: 3,
      doneChunks: 3,
      agentCount: 1,
      compactorVersion: 2,
      currentVersion: 2,
      digestOutdated: false,
      digest: "CHARACTERS: ...\nCHRONOLOGY: ...",
    });

    renderCard();
    // The stale state is said out loud instead of passing as current.
    expect(await screen.findByText("Older digest")).toBeInTheDocument();
    await userEvent
      .setup()
      .click(await screen.findByRole("button", { name: "Rebuild digest" }));

    await waitFor(() => {
      // Stored progress must be discarded, not resumed against the old prompts.
      expect(scriptApi.compact).toHaveBeenCalledWith("d1", undefined, true);
    });
    expect(await screen.findByText("Digest ready")).toBeInTheDocument();
  });

  it("summarizes in batches until the digest exists, showing progress", async () => {
    vi.mocked(scriptApi.list).mockResolvedValue([
      doc({
        compaction: { modelId: "qwen", chunkCount: 2, doneChunks: 0, agentCount: 1, compactorVersion: 2, currentVersion: 2, digestOutdated: false, digest: null },
      }),
    ]);
    vi.mocked(scriptApi.compact)
      .mockResolvedValueOnce({
        modelId: "qwen",
        chunkCount: 2,
        doneChunks: 1,
        agentCount: 1,
        compactorVersion: 2,
        currentVersion: 2,
        digestOutdated: false,
        digest: null,
      })
      .mockResolvedValueOnce({
        modelId: "qwen",
        chunkCount: 2,
        doneChunks: 2,
        agentCount: 1,
        compactorVersion: 2,
        currentVersion: 2,
        digestOutdated: false,
        digest: "A one-page outline.",
      });

    renderCard();
    await userEvent.setup().click(await screen.findByRole("button", { name: "Summarize" }));

    await waitFor(() => {
      expect(scriptApi.compact).toHaveBeenCalledTimes(2);
    });
    expect(await screen.findByText("Digest ready")).toBeInTheDocument();
    const toasts = useAppStore.getState().toasts;
    expect(toasts.some((toast) => toast.message === "Digest ready")).toBe(true);
  });

  it("maps a missing provider key to an in-app instruction", async () => {
    vi.mocked(scriptApi.list).mockResolvedValue([doc()]);
    vi.mocked(scriptApi.compact).mockRejectedValue(
      new IpcError("chat_key_missing", "chat_key_missing"),
    );

    renderCard();
    await userEvent.setup().click(await screen.findByRole("button", { name: "Summarize" }));

    await waitFor(() => {
      const toasts = useAppStore.getState().toasts;
      expect(
        toasts.some((toast) =>
          toast.message.includes("Set an AI provider (or a local endpoint) in Settings first."),
        ),
      ).toBe(true);
    });
  });

  it("imports the chosen file and refreshes the list", async () => {
    vi.mocked(open).mockResolvedValue(["C:\\games\\Saku\\dialogue.json"]);
    vi.mocked(scriptApi.import).mockResolvedValue(
      doc({ sourceKind: "json_vntp", sourcePath: "C:\\games\\Saku\\dialogue.json" }),
    );
    vi.mocked(scriptApi.list)
      .mockResolvedValueOnce([])
      .mockResolvedValueOnce([
        doc({ sourceKind: "json_vntp", sourcePath: "C:\\games\\Saku\\dialogue.json" }),
      ]);

    renderCard();
    await userEvent.setup().click(await screen.findByRole("button", { name: "Import script" }));

    await waitFor(() => {
      expect(scriptApi.import).toHaveBeenCalledWith("g1", ["C:\\games\\Saku\\dialogue.json"]);
    });
    expect(await screen.findByText("dialogue.json")).toBeInTheDocument();
    // The kind label lives in the row's sub line, not only in the card intro.
    expect(
      document.querySelector(".script-row .row__sub")?.textContent,
    ).toContain("JSON (VNTP)");
  });

  it("imports several chosen files as one merged document", async () => {
    // The dialog is multi-select: a script extracted as part files imports in
    // one pick, and every chosen path reaches the same import call.
    vi.mocked(open).mockResolvedValue([
      "C:\\games\\Saku\\scene10.txt",
      "C:\\games\\Saku\\scene2.txt",
    ]);
    vi.mocked(scriptApi.import).mockResolvedValue(doc({ lineCount: 9_001 }));

    renderCard();
    await userEvent.setup().click(await screen.findByRole("button", { name: "Import script" }));

    await waitFor(() => {
      expect(scriptApi.import).toHaveBeenCalledTimes(1);
    });
    expect(scriptApi.import).toHaveBeenCalledWith("g1", [
      "C:\\games\\Saku\\scene10.txt",
      "C:\\games\\Saku\\scene2.txt",
    ]);
  });

  it("removes a document", async () => {
    vi.mocked(scriptApi.list)
      .mockResolvedValueOnce([doc()])
      .mockResolvedValueOnce([]);
    renderCard();

    await userEvent
      .setup()
      .click(await screen.findByRole("button", { name: /Remove/ }));

    await waitFor(() => {
      expect(scriptApi.delete).toHaveBeenCalledWith("d1");
    });
    const toasts = useAppStore.getState().toasts;
    expect(toasts.some((toast) => toast.message === "Script removed")).toBe(true);
  });

  it("shows and saves the agent count", async () => {
    renderCard();

    const select = await screen.findByLabelText("Agents");
    expect(select).toHaveValue("3");

    await userEvent.setup().selectOptions(select, "5");
    await waitFor(() => {
      expect(scriptApi.saveAgentSettings).toHaveBeenCalledWith({ agents: 5 });
    });
    // The select reflects the value the backend normalized and returned.
    await waitFor(() => {
      expect(select).toHaveValue("5");
    });
  });

  it("locks the agent count while a summary is running", async () => {
    // A layout change restarts an unfinished summary, so the select is locked
    // for exactly as long as the loop owns the document.
    vi.mocked(scriptApi.list).mockResolvedValue([
      doc({
        compaction: { modelId: "qwen", chunkCount: 1, doneChunks: 0, agentCount: 1, compactorVersion: 2, currentVersion: 2, digestOutdated: false, digest: null },
      }),
    ]);
    let finish:
      | ((value: {
          modelId: string;
          chunkCount: number;
          doneChunks: number;
          agentCount: number;
          compactorVersion: number;
          currentVersion: number;
          digestOutdated: boolean;
          digest: string;
        }) => void)
      | undefined;
    vi.mocked(scriptApi.compact).mockImplementation(
      () =>
        new Promise((resolve) => {
          finish = resolve;
        }),
    );

    renderCard();
    const select = await screen.findByLabelText("Agents");
    await userEvent.setup().click(await screen.findByRole("button", { name: "Summarize" }));

    await waitFor(() => {
      expect(select).toBeDisabled();
    });
    finish?.({
      modelId: "qwen",
      chunkCount: 1,
      doneChunks: 1,
      agentCount: 1,
      compactorVersion: 2,
      currentVersion: 2,
      digestOutdated: false,
      digest: "Done.",
    });
    await waitFor(() => {
      expect(select).toBeEnabled();
    });
    expect(scriptApi.saveAgentSettings).not.toHaveBeenCalled();
  });

  it("counts the agents in the progress line", async () => {
    vi.mocked(scriptApi.list).mockResolvedValue([
      doc({
        compaction: { modelId: "qwen", chunkCount: 40, doneChunks: 12, agentCount: 3, compactorVersion: 2, currentVersion: 2, digestOutdated: false, digest: null },
      }),
    ]);
    renderCard();

    expect(
      await screen.findByText("Summarized 12 of 40 chunks · 3 agents"),
    ).toBeInTheDocument();
  });
});
