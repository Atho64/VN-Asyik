import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router";
import { AppContext } from "@/app/AppProvider";
import { useAppStore } from "@/app/store";
import { createTranslator } from "@/i18n";
import { api } from "@/lib/ipc/client";

vi.mock("@/lib/ipc/client", () => ({
  api: {
    setBetaFeatures: vi.fn(),
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

const { default: SettingsScreen } = await import("./SettingsScreen");

beforeEach(() => {
  vi.clearAllMocks();
  useAppStore.setState({
    prefs: {
      theme: "dark",
      language: "en",
      nsfw: "blur",
      reduce_effects: false,
      beta_features: false,
    },
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
      {/* Settings links out to the Discord presence page, so a router must exist. */}
      <MemoryRouter>
        <SettingsScreen initialTab={initialTab ?? "general"} />
      </MemoryRouter>
    </AppContext.Provider>,
  );
}

describe("unified settings data tab", () => {
  it("keeps Beta setup hidden until enabled and hides it again on disable", async () => {
    vi.mocked(api.setBetaFeatures).mockImplementation(async (value) => ({
      ...useAppStore.getState().prefs,
      beta_features: value,
    }));
    renderSettings();
    const toggle = screen.getByRole("switch", { name: "Enable Beta features" });
    expect(toggle).toHaveAttribute("aria-checked", "false");
    expect(screen.queryByText("Chat provider")).toBeNull();
    expect(screen.queryByText("Luna live text")).toBeNull();
    await userEvent.click(toggle);
    expect(api.setBetaFeatures).toHaveBeenCalledWith(true);
    expect(screen.getByText("Chat provider")).toBeInTheDocument();
    expect(screen.getByText("Luna live text")).toBeInTheDocument();
    await userEvent.click(toggle);
    expect(api.setBetaFeatures).toHaveBeenLastCalledWith(false);
    expect(screen.queryByText("Chat provider")).toBeNull();
    expect(screen.queryByText("Luna live text")).toBeNull();
  });

  it("retains the disabled state when saving the opt-in fails", async () => {
    vi.mocked(api.setBetaFeatures).mockRejectedValueOnce(new Error("save failed"));
    renderSettings();
    const toggle = screen.getByRole("switch", { name: "Enable Beta features" });
    await userEvent.click(toggle);
    expect(toggle).toHaveAttribute("aria-checked", "false");
    expect(toggle).not.toBeDisabled();
    expect(useAppStore.getState().toasts.at(-1)?.kind).toBe("error");
  });

  it("keeps backup actions in the tab only, with no duplicate shortcut in General", async () => {
    renderSettings();
    const tabs = screen.getByRole("tablist");
    expect(
      within(tabs).getAllByRole("button", { name: "Backup & transfer" }),
    ).toHaveLength(1);
    // The tab is the only way in: no shortcut row duplicating it in General.
    expect(screen.getAllByRole("button", { name: "Backup & transfer" })).toHaveLength(1);
    expect(screen.queryByRole("button", { name: "Export backup" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Import backup" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Export file" })).toBeNull();
    await userEvent.click(
      within(tabs).getByRole("button", { name: "Backup & transfer" }),
    );
    expect(screen.getAllByRole("button", { name: "Export file" })).toHaveLength(1);
    expect(screen.getAllByRole("button", { name: "Import file" })).toHaveLength(1);
    expect(screen.queryByRole("button", { name: "Export backup" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Import backup" })).toBeNull();
  });

  it("does not duplicate the Discord page inside General settings", () => {
    useAppStore.setState({
      platform: {
        ...useAppStore.getState().platform!,
        os: "windows",
        kind: "desktop",
        discord_presence: "supported",
      },
    });
    renderSettings();
    // Discord has its own sidebar destination; General must not repeat it.
    expect(
      screen.queryByRole("heading", { name: "Discord Rich Presence" }),
    ).toBeNull();
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
