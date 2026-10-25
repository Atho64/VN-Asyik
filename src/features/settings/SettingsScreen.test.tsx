import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { AppContext } from "@/app/AppProvider";
import { useAppStore } from "@/app/store";
import { createTranslator } from "@/i18n";

vi.mock("@/lib/ipc/client", () => ({
  api: {
    getAfkConfig: vi.fn().mockResolvedValue(null),
    vndbSyncSettings: vi.fn().mockResolvedValue(null),
    syncStatus: vi.fn().mockResolvedValue({
      deviceId: "settings-device",
      folder: null,
      lastSyncAtMs: null,
      games: 0,
      chapters: 0,
      sessions: 0,
    }),
    syncConflicts: vi.fn().mockResolvedValue([]),
  },
  IpcError: class extends Error {},
}));
vi.mock("./ChatPanel", () => ({ ChatPanel: () => <p>Chat provider</p> }));
vi.mock("./LocalePanel", () => ({ LocalePanel: () => <p>Windows locale tools</p> }));
vi.mock("./LunaPanel", () => ({ LunaPanel: () => <p>Luna live text</p> }));
vi.mock("./PresencePanel", () => ({ PresencePanel: () => <p>Discord Presence</p> }));
vi.mock("./TitleParsingPanel", () => ({ TitleParsingPanel: () => <p>Window titles</p> }));

const { SettingsScreen } = await import("./SettingsScreen");

beforeEach(() => {
  vi.clearAllMocks();
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
    toasts: [],
  });
});

function renderSettings(
  initialTab?: "general" | "vndb" | "transfer",
  locale: "en" | "id" = "en",
) {
  return render(
    <AppContext.Provider
      value={{ t: createTranslator(locale), locale, ready: true, bootError: null }}
    >
      <SettingsScreen initialTab={initialTab ?? "general"} />
    </AppContext.Provider>,
  );
}

describe("unified settings data tab", () => {
  it("keeps General to one shortcut and removes duplicate backup actions", async () => {
    renderSettings();
    const tabs = screen.getByRole("tablist");
    expect(
      within(tabs).getAllByRole("button", { name: "Backup & transfer" }),
    ).toHaveLength(1);
    expect(screen.queryByRole("button", { name: "Export backup" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Import backup" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Export file" })).toBeNull();
    expect(screen.getAllByRole("button", { name: "Backup & transfer" })).toHaveLength(2);
    await userEvent.click(
      screen.getAllByRole("button", { name: "Backup & transfer" })[1]!,
    );
    expect(screen.getAllByRole("button", { name: "Export file" })).toHaveLength(1);
    expect(screen.getAllByRole("button", { name: "Import file" })).toHaveLength(1);
    expect(screen.queryByRole("button", { name: "Export backup" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Import backup" })).toBeNull();
  });

  it.each(["android", "windows"] as const)(
    "opens the unified tab directly on %s",
    (os) => {
      useAppStore.setState({
        platform: {
          ...useAppStore.getState().platform!,
          os,
          kind: os === "android" ? "mobile" : "desktop",
        },
      });
      renderSettings("transfer");
      expect(
        screen.getByRole("heading", { name: "Backup & transfer" }),
      ).toBeInTheDocument();
      expect(screen.getAllByRole("button", { name: "Export file" })).toHaveLength(1);
      expect(screen.getAllByRole("button", { name: "Import file" })).toHaveLength(1);
    },
  );

  it("shows Indonesian labels for the unified tab and mode controls", () => {
    renderSettings("transfer", "id");
    expect(
      screen.getByRole("heading", { name: "Cadangan & transfer" }),
    ).toBeInTheDocument();
    expect(screen.queryByRole("radio")).toBeNull();
    expect(screen.getByRole("button", { name: "Impor file" })).toBeInTheDocument();
  });

  it("does not expose Windows locale or window-title controls on Android", () => {
    renderSettings();
    expect(screen.queryByText("Windows locale tools")).toBeNull();
    expect(screen.queryByText("Window titles")).toBeNull();
    expect(screen.getByText("This device")).toBeInTheDocument();
  });
});
