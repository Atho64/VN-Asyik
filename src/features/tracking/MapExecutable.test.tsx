import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { AppContext } from "@/app/AppProvider";
import { useAppStore } from "@/app/store";
import { createTranslator } from "@/i18n";

const mocks = vi.hoisted(() => ({
  map: vi.fn().mockResolvedValue(undefined),
  open: vi.fn(),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: mocks.open }));
vi.mock("@/lib/ipc/client", () => ({
  api: {
    trackingAndroidApps: vi.fn().mockResolvedValue([
      { package: "dev.eden.eden_emulator.nightly", label: "Eden Nightly" },
      { package: "com.tyranor.next", label: "Tyranor Next" },
    ]),
    trackingMapRunnerTarget: mocks.map,
  },
  IpcError: class extends Error {},
}));
const { MapExecutable } = await import("./MapExecutable");
const t = createTranslator("en");

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
  });
});

describe("Android application mapping", () => {
  it("maps the exact installed fork package without desktop paths or arguments", async () => {
    const onMapped = vi.fn().mockResolvedValue(undefined);
    render(
      <AppContext.Provider value={{ t, locale: "en", ready: true, bootError: null }}>
        <MapExecutable games={[]} fixedGameId="my-game" onMapped={onMapped} />
      </AppContext.Provider>,
    );
    await screen.findByRole("option", {
      name: "Eden Nightly · dev.eden.eden_emulator.nightly",
    });
    await userEvent.selectOptions(
      screen.getByRole("combobox", { name: "Installed app or emulator" }),
      "dev.eden.eden_emulator.nightly",
    );
    await userEvent.click(screen.getByRole("button", { name: "Link app" }));
    await waitFor(() =>
      expect(mocks.map).toHaveBeenCalledWith({
        gameId: "my-game",
        runnerKind: "custom",
        package: "dev.eden.eden_emulator.nightly",
        exePath: null,
        launchTarget: null,
        titleMatch: null,
        extraArgs: null,
      }),
    );
    expect(mocks.open).not.toHaveBeenCalled();
    expect(onMapped).toHaveBeenCalledOnce();
    expect(screen.queryByText("Extra arguments")).toBeNull();
  });
});
