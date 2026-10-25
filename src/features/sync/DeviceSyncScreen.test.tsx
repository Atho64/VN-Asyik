import { beforeEach, describe, expect, it, vi } from "vitest";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import { AppContext } from "@/app/AppProvider";
import { useAppStore } from "@/app/store";
import { createTranslator } from "@/i18n";
import type { DataImportPreview, DataImportResult } from "@/lib/ipc/types";

const mocks = vi.hoisted(() => ({
  status: vi.fn(),
  conflicts: vi.fn(),
  exportBackup: vi.fn(),
  exportTransfer: vi.fn(),
  shareBackup: vi.fn(),
  shareTransfer: vi.fn(),
  open: vi.fn(),
  apply: vi.fn(),
  cancel: vi.fn(),
  run: vi.fn(),
  listGames: vi.fn(),
  getPrefs: vi.fn(),
}));

vi.mock("@/lib/ipc/client", () => ({
  api: {
    syncStatus: mocks.status,
    syncConflicts: mocks.conflicts,
    dataExportFile: mocks.exportBackup,
    syncExportFile: mocks.exportTransfer,
    dataShareFile: mocks.shareBackup,
    syncShareFile: mocks.shareTransfer,
    dataOpenFile: mocks.open,
    dataApplyImport: mocks.apply,
    dataCancelImport: mocks.cancel,
    syncRun: mocks.run,
    listGames: mocks.listGames,
    getPrefs: mocks.getPrefs,
  },
  IpcError: class extends Error {},
}));

const { DeviceSyncScreen } = await import("./DeviceSyncScreen");
const status = {
  deviceId: "c05139e8-6d2b-4c5c-9882-749efad1ad6a",
  folder: null,
  lastSyncAtMs: null,
  games: 2,
  chapters: 0,
  sessions: 3,
};
const outcome = { peers: 1, pulled: 2, pushed: 3, conflicts: [], skipped: [] };
const backup: DataImportPreview = {
  id: "draft-backup",
  kind: "backup",
  createdAtMs: Date.UTC(2026, 9, 21, 13, 20),
  games: 2,
  chapters: 4,
  sessions: 7,
  endings: 3,
  sameDevice: true,
  canRestore: true,
};
const transfer: DataImportPreview = {
  ...backup,
  id: "draft-transfer",
  kind: "transfer",
  sameDevice: false,
  canRestore: false,
};
const summary = { games: 2, chapters: 4, sessions: 7, endings: 3, sessions_skipped: 1 };
const restoredPrefs = {
  theme: "light",
  language: "id",
  nsfw: "hide",
  reduce_effects: true,
} as const;

beforeEach(() => {
  vi.resetAllMocks();
  mocks.status.mockResolvedValue(status);
  mocks.conflicts.mockResolvedValue([]);
  mocks.exportBackup.mockResolvedValue(true);
  mocks.exportTransfer.mockResolvedValue(true);
  mocks.shareBackup.mockResolvedValue(undefined);
  mocks.shareTransfer.mockResolvedValue(undefined);
  mocks.open.mockResolvedValue(backup);
  mocks.apply.mockResolvedValue({ kind: "transfer", outcome });
  mocks.cancel.mockResolvedValue(undefined);
  mocks.run.mockResolvedValue(outcome);
  mocks.listGames.mockResolvedValue([]);
  mocks.getPrefs.mockResolvedValue(restoredPrefs);
  useAppStore.setState({
    platform: {
      os: "android",
      kind: "mobile",
      first_class: true,
      process_tracking: "supported",
      chapter_detection: "unavailable",
      discord_presence: "unavailable",
      cloud_sync: "unavailable",
    },
    prefs: { theme: "dark", language: "en", nsfw: "blur", reduce_effects: false },
    toasts: [],
    libraryLoaded: false,
    cards: [],
  });
});

function renderScreen() {
  return render(
    <AppContext.Provider
      value={{ t: createTranslator("en"), locale: "en", ready: true, bootError: null }}
    >
      <DeviceSyncScreen />
    </AppContext.Provider>,
  );
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((complete) => {
    resolve = complete;
  });
  return { promise, resolve };
}

describe("backup and device transfer", () => {
  it("offers one export and import action on Android without a filesystem path", () => {
    renderScreen();
    expect(
      screen.getByRole("heading", { name: "Backup & transfer" }),
    ).toBeInTheDocument();
    expect(screen.getAllByRole("button", { name: "Export file" })).toHaveLength(1);
    expect(screen.getAllByRole("button", { name: "Import file" })).toHaveLength(1);
    expect(screen.queryByRole("textbox")).toBeNull();
    expect(screen.queryByRole("button", { name: "Sync now" })).toBeNull();
    expect(screen.queryByRole("radio")).toBeNull();
  });

  it("exports one library file without asking for a mode", async () => {
    renderScreen();
    expect(screen.queryByRole("radio")).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "Export file" }));
    expect(mocks.exportBackup).toHaveBeenCalledOnce();
    expect(mocks.exportTransfer).not.toHaveBeenCalled();
    expect(useAppStore.getState().toasts[0]?.message).toBe("File saved");
  });

  it("shares the same library format on Android", async () => {
    renderScreen();
    await userEvent.click(screen.getByRole("button", { name: "Share file" }));
    expect(mocks.shareBackup).toHaveBeenCalledOnce();
    expect(mocks.shareTransfer).not.toHaveBeenCalled();
  });

  it.each(["export", "import"] as const)(
    "keeps %s picker cancellation quiet",
    async (action) => {
      mocks.exportBackup.mockResolvedValue(false);
      mocks.open.mockResolvedValue(null);
      renderScreen();
      const button = screen.getByRole("button", {
        name: action === "import" ? "Import file" : "Export file",
      });
      await userEvent.click(button);
      await waitFor(() => expect(button).toBeEnabled());
      expect(mocks.apply).not.toHaveBeenCalled();
      expect(mocks.listGames).not.toHaveBeenCalled();
      expect(screen.queryByRole("dialog")).toBeNull();
      expect(useAppStore.getState().toasts).toHaveLength(0);
      expect(screen.queryByRole("alert")).toBeNull();
    },
  );

  it("previews old backups with merge as the default", async () => {
    renderScreen();
    await userEvent.click(screen.getByRole("button", { name: "Import file" }));
    const dialog = await screen.findByRole("dialog", { name: "Preview import" });
    expect(
      within(dialog)
        .getAllByRole("definition")
        .map((row) => row.textContent),
    ).toEqual([expect.stringContaining("2026"), "2", "4", "7", "3"]);
    expect(
      within(dialog).getByText(/Merging keeps newer changes and deletions/),
    ).toBeInTheDocument();
    expect(within(dialog).getByRole("button", { name: "Merge data" })).toBeEnabled();
    expect(screen.queryByRole("button", { name: "Restore backup" })).toBeNull();
    expect(mocks.apply).not.toHaveBeenCalled();
    expect(mocks.getPrefs).not.toHaveBeenCalled();
  });
  it("restores a backup only after confirmation and refreshes all four preferences", async () => {
    mocks.apply.mockResolvedValue({ kind: "backup", summary });
    renderScreen();
    await userEvent.click(screen.getByRole("button", { name: "Import file" }));
    await userEvent.click(await screen.findByText("Advanced options"));
    await userEvent.click(screen.getByRole("checkbox"));
    expect(screen.getByText(/even if this backup is older/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Restore backup" }));
    await waitFor(() => expect(useAppStore.getState().libraryLoaded).toBe(true));
    expect(mocks.apply).toHaveBeenCalledExactlyOnceWith("draft-backup", "restore");
    expect(mocks.getPrefs).toHaveBeenCalledOnce();
    expect(useAppStore.getState().prefs).toEqual(restoredPrefs);
    expect(mocks.cancel).not.toHaveBeenCalled();
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(useAppStore.getState().toasts[0]?.message).toBe(
      "Imported 2 games, 7 sessions, skipped 1 duplicates",
    );
  });

  it("merges a transfer only after confirmation and refreshes status/conflicts while preserving preferences", async () => {
    mocks.open.mockResolvedValue(transfer);
    mocks.apply.mockResolvedValue({ kind: "transfer", outcome });
    renderScreen();
    await screen.findByText("No conflicts yet");
    await userEvent.click(screen.getByRole("button", { name: "Import file" }));
    const dialog = await screen.findByRole("dialog", { name: "Preview import" });
    expect(
      within(dialog).getByText(/Merging keeps newer changes and deletions/),
    ).toBeInTheDocument();
    expect(mocks.apply).not.toHaveBeenCalled();
    await userEvent.click(within(dialog).getByRole("button", { name: "Merge data" }));
    await waitFor(() => expect(useAppStore.getState().libraryLoaded).toBe(true));
    expect(mocks.apply).toHaveBeenCalledExactlyOnceWith("draft-transfer", "merge");
    expect(mocks.status).toHaveBeenCalledTimes(2);
    expect(mocks.conflicts).toHaveBeenCalledTimes(2);
    expect(mocks.getPrefs).not.toHaveBeenCalled();
    expect(useAppStore.getState().prefs.theme).toBe("dark");
    expect(screen.getByRole("heading", { name: "Last run" })).toBeInTheDocument();
  });

  it("allows a same-device file to merge without requiring a second device", async () => {
    mocks.open.mockResolvedValue({ ...transfer, sameDevice: true });
    renderScreen();
    await userEvent.click(screen.getByRole("button", { name: "Import file" }));
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByRole("button", { name: "Merge data" })).toBeEnabled();
    expect(within(dialog).queryByText("Advanced options")).toBeNull();
    await userEvent.click(within(dialog).getByRole("button", { name: "Merge data" }));
    expect(mocks.apply).toHaveBeenCalledExactlyOnceWith("draft-transfer", "merge");
  });

  it.each(["close", "escape", "scrim", "unmount"] as const)(
    "discards the draft on %s",
    async (action) => {
      const view = renderScreen();
      await userEvent.click(screen.getByRole("button", { name: "Import file" }));
      const dialog = await screen.findByRole("dialog");
      if (action === "close")
        await userEvent.click(
          within(dialog).getAllByRole("button", { name: "Close" })[1]!,
        );
      if (action === "escape") await userEvent.keyboard("{Escape}");
      if (action === "scrim") fireEvent.mouseDown(dialog.parentElement!);
      if (action === "unmount") view.unmount();
      await waitFor(() =>
        expect(mocks.cancel).toHaveBeenCalledExactlyOnceWith("draft-backup"),
      );
      expect(mocks.apply).not.toHaveBeenCalled();
      expect(screen.queryByRole("dialog")).toBeNull();
    },
  );

  it("discards a picker result arriving after the tab has unmounted", async () => {
    const pending = deferred<DataImportPreview | null>();
    mocks.open.mockReturnValue(pending.promise);
    const view = renderScreen();
    await userEvent.click(screen.getByRole("button", { name: "Import file" }));
    view.unmount();
    await act(async () => pending.resolve(backup));
    expect(mocks.cancel).toHaveBeenCalledExactlyOnceWith("draft-backup");
    expect(mocks.apply).not.toHaveBeenCalled();
    expect(useAppStore.getState().toasts).toHaveLength(0);
  });

  it("locks duplicate picker and confirmation clicks before React rerenders", async () => {
    const pendingOpen = deferred<DataImportPreview | null>();
    const pendingApply = deferred<DataImportResult>();
    mocks.open.mockReturnValue(pendingOpen.promise);
    mocks.apply.mockReturnValue(pendingApply.promise);
    renderScreen();
    const open = screen.getByRole("button", { name: "Import file" });
    act(() => {
      open.click();
      open.click();
    });
    expect(mocks.open).toHaveBeenCalledOnce();
    await act(async () => pendingOpen.resolve(backup));
    const confirm = screen.getByRole("button", { name: "Merge data" });
    act(() => {
      confirm.click();
      confirm.click();
    });
    expect(mocks.apply).toHaveBeenCalledOnce();
    await userEvent.keyboard("{Escape}");
    fireEvent.mouseDown(screen.getByRole("dialog").parentElement!);
    fireEvent.click(
      within(screen.getByRole("dialog")).getAllByRole("button", { name: "Close" })[0]!,
    );
    expect(screen.getByRole("dialog")).toBeInTheDocument();
    expect(mocks.cancel).not.toHaveBeenCalled();
    await act(async () => pendingApply.resolve({ kind: "backup", summary }));
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("does not start another picker until preview cancellation finishes", async () => {
    const pendingCancel = deferred<void>();
    mocks.cancel.mockReturnValue(pendingCancel.promise);
    renderScreen();
    await userEvent.click(screen.getByRole("button", { name: "Import file" }));
    await userEvent.keyboard("{Escape}");
    const open = screen.getByRole("button", { name: "Import file" });
    expect(open).toBeDisabled();
    fireEvent.click(open);
    expect(mocks.open).toHaveBeenCalledOnce();
    await act(async () => pendingCancel.resolve());
    expect(open).toBeEnabled();
  });

  it("shows malformed/rejected file errors and lets the user choose again", async () => {
    mocks.open.mockRejectedValueOnce(new Error("unsupported payload"));
    renderScreen();
    await userEvent.click(screen.getByRole("button", { name: "Import file" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      /not a supported backup or transfer/,
    );
    expect(mocks.apply).not.toHaveBeenCalled();
    await userEvent.click(screen.getByRole("button", { name: "Choose file again" }));
    expect(
      await screen.findByRole("dialog", { name: "Preview import" }),
    ).toBeInTheDocument();
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("reopens a consumed draft after an apply failure instead of reusing its token", async () => {
    mocks.apply.mockRejectedValueOnce(new Error("stale_import"));
    mocks.open
      .mockResolvedValueOnce(backup)
      .mockResolvedValueOnce({ ...backup, id: "fresh-draft" });
    renderScreen();
    await userEvent.click(screen.getByRole("button", { name: "Import file" }));
    await userEvent.click(await screen.findByRole("button", { name: "Merge data" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "The file could not be applied",
    );
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(mocks.listGames).not.toHaveBeenCalled();
    await userEvent.click(screen.getByRole("button", { name: "Choose file again" }));
    await userEvent.click(await screen.findByRole("button", { name: "Merge data" }));
    expect(mocks.apply.mock.calls).toEqual([
      ["draft-backup", "merge"],
      ["fresh-draft", "merge"],
    ]);
  });

  it("reports persisted import success even if refreshing fails and retries only the views", async () => {
    mocks.listGames.mockRejectedValueOnce(new Error("reload failed"));
    renderScreen();
    await userEvent.click(screen.getByRole("button", { name: "Import file" }));
    await userEvent.click(await screen.findByRole("button", { name: "Merge data" }));
    expect(
      await screen.findByText(
        /The data was applied, but some views could not be refreshed/,
      ),
    ).toBeInTheDocument();
    expect(useAppStore.getState().toasts[0]?.kind).toBe("success");
    expect(screen.queryByRole("alert")).toBeNull();
    expect(screen.queryByRole("button", { name: "Choose file again" })).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "Try again" }));
    await waitFor(() => expect(useAppStore.getState().libraryLoaded).toBe(true));
    expect(mocks.apply).toHaveBeenCalledOnce();
    expect(screen.queryByText(/some views could not be refreshed/)).toBeNull();
  });

  it("does not claim clean conflicts before loading and retries a status failure", async () => {
    mocks.status.mockRejectedValueOnce(new Error("database unavailable"));
    renderScreen();
    expect(screen.getByRole("status")).toHaveTextContent("Loading");
    expect(screen.queryByText("No conflicts yet")).toBeNull();
    expect(await screen.findByRole("alert")).toBeInTheDocument();
    expect(screen.queryByText("No conflicts yet")).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "Try again" }));
    expect(await screen.findByText("No conflicts yet")).toBeInTheDocument();
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("keeps desktop folder sync separate and refreshes cards after syncing", async () => {
    useAppStore.setState({
      platform: { ...useAppStore.getState().platform!, os: "windows", kind: "desktop" },
    });
    renderScreen();
    expect(screen.getByRole("heading", { name: "Folder sync" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Share file" })).toBeNull();
    const sync = screen.getByRole("button", { name: "Sync now" });
    expect(sync).toBeDisabled();
    await screen.findByText("No conflicts yet");
    await userEvent.type(screen.getByRole("textbox"), "D:/Library sync");
    await userEvent.click(sync);
    await waitFor(() => expect(useAppStore.getState().libraryLoaded).toBe(true));
    expect(mocks.run).toHaveBeenCalledExactlyOnceWith("D:/Library sync");
    expect(mocks.getPrefs).not.toHaveBeenCalled();
  });
});
