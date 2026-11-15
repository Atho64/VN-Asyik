import { beforeEach, describe, expect, it, vi } from "vitest";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { AppContext } from "@/app/AppProvider";
import { useAppStore } from "@/app/store";
import { createTranslator } from "@/i18n";

const mocks = vi.hoisted(() => ({
  map: vi.fn().mockResolvedValue(undefined),
  open: vi.fn(),
  windows: vi.fn(),
  mapWindow: vi.fn(),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: mocks.open }));
vi.mock("@/lib/ipc/client", () => ({
  api: {
    trackingAndroidApps: vi.fn().mockResolvedValue([
      { package: "dev.eden.eden_emulator.nightly", label: "Eden Nightly" },
      { package: "com.tyranor.next", label: "Tyranor Next" },
    ]),
    trackingMapRunnerTarget: mocks.map,
    trackingExecutableWindows: mocks.windows,
    trackingMapWindow: mocks.mapWindow,
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

function renderWindows() {
  const platform = useAppStore.getState().platform!;
  useAppStore.setState({ platform: { ...platform, os: "windows", kind: "desktop" } });
  mocks.windows.mockResolvedValue([
    {
      key: "window-1",
      title: "Rewrite+",
      exePath: "C:\\Games\\Rewrite\\SiglusEngine.exe",
    },
    { key: "window-2", title: "Protected game", exePath: null },
  ]);
  mocks.mapWindow.mockResolvedValue({ exe_name: "siglusengine.exe" });
  const onMapped = vi.fn().mockResolvedValue(undefined);
  render(
    <AppContext.Provider value={{ t, locale: "en", ready: true, bootError: null }}>
      <MapExecutable games={[]} fixedGameId="my-game" onMapped={onMapped} />
    </AppContext.Provider>,
  );
  return onMapped;
}

describe("Window executable mapping", () => {
  it("loads only on request, disables unreadable paths and links the selected identity", async () => {
    const onMapped = renderWindows();
    expect(mocks.windows).not.toHaveBeenCalled();
    await userEvent.click(screen.getByRole("button", { name: "Choose a window" }));
    expect(
      await screen.findByRole("option", {
        name: "Protected game · Executable unavailable",
      }),
    ).toBeDisabled();
    expect(screen.getByRole("button", { name: "Link window" })).toBeDisabled();
    expect(
      screen
        .getByRole("combobox", { name: "Open game window" })
        .closest(".map-exe__windows"),
    ).toBeInTheDocument();
    await userEvent.selectOptions(
      screen.getByRole("combobox", { name: "Open game window" }),
      "window-1",
    );
    await userEvent.click(screen.getByRole("button", { name: "Link window" }));
    expect(mocks.mapWindow).toHaveBeenCalledWith("my-game", "window-1");
    expect(mocks.open).not.toHaveBeenCalled();
    expect(onMapped).toHaveBeenCalledOnce();
  });

  it("clears a stale selection on refresh and shows an empty list honestly", async () => {
    renderWindows();
    await userEvent.click(screen.getByRole("button", { name: "Choose a window" }));
    await screen.findByRole("option", { name: "Rewrite+ · SiglusEngine.exe" });
    await userEvent.selectOptions(
      screen.getByRole("combobox", { name: "Open game window" }),
      "window-1",
    );
    mocks.windows.mockResolvedValue([]);
    await userEvent.click(screen.getByRole("button", { name: "Refresh windows" }));
    expect(await screen.findByText(/No eligible windows found/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Link window" })).toBeDisabled();
    expect(mocks.windows).toHaveBeenCalledTimes(2);
  });

  it("keeps the file option available after a window mapping fails", async () => {
    renderWindows();
    mocks.mapWindow.mockRejectedValue(new Error("closed"));
    await userEvent.click(screen.getByRole("button", { name: "Choose a window" }));
    await screen.findByRole("option", { name: "Rewrite+ · SiglusEngine.exe" });
    await userEvent.selectOptions(
      screen.getByRole("combobox", { name: "Open game window" }),
      "window-1",
    );
    await userEvent.click(screen.getByRole("button", { name: "Link window" }));
    expect(await screen.findByRole("alert")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Browse for executable" })).toBeEnabled();
  });

  it("shows loading and discards a response after closing the picker", async () => {
    renderWindows();
    let resolve!: (windows: []) => void;
    mocks.windows.mockReturnValue(
      new Promise((done) => {
        resolve = done;
      }),
    );
    const trigger = screen.getByRole("button", { name: "Choose a window" });
    await userEvent.click(trigger);
    expect(screen.getByRole("status")).toHaveTextContent("Loading windows...");
    await userEvent.click(trigger);
    await act(async () => resolve([]));
    expect(screen.queryByRole("combobox", { name: "Open game window" })).toBeNull();
  });

  it("does not offer the Windows picker on another desktop platform", () => {
    renderWindows();
    act(() =>
      useAppStore.setState({
        platform: { ...useAppStore.getState().platform!, os: "linux" },
      }),
    );
    expect(screen.queryByRole("button", { name: "Choose a window" })).toBeNull();
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
