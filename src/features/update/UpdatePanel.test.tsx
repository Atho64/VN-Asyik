import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import { AppContext } from "@/app/AppProvider";
import { useAppStore } from "@/app/store";
import { createTranslator } from "@/i18n";
import type { UpdateView } from "@/lib/ipc/types";

const mocks = vi.hoisted(() => ({
  updateCheck: vi.fn(),
  updateInstall: vi.fn(),
  openExternal: vi.fn(),
}));

vi.mock("@/lib/ipc/client", () => ({
  api: {
    updateCheck: mocks.updateCheck,
    updateInstall: mocks.updateInstall,
    openExternal: mocks.openExternal,
  },
  IpcError: class IpcError extends Error {
    code: string;
    constructor(code: string, message: string) {
      super(message);
      this.code = code;
    }
  },
}));

const { UpdateBanner, UpdatePanel } = await import("./UpdatePanel");
const { IpcError } = await import("@/lib/ipc/client");

const RELEASE_URL = "https://github.com/Atho64/VN-Asyik/releases/tag/v0.2.5";

function updateState(overrides: Partial<UpdateView> = {}): UpdateView {
  return {
    current: "0.2.4",
    latest: "0.2.5",
    available: true,
    url: RELEASE_URL,
    asset: null,
    checkedAt: 1_700_000_000_000,
    error: null,
    canInstall: false,
    ...overrides,
  };
}

function renderWithLocale(
  ui: React.ReactElement,
  locale: "en" | "id" = "en",
) {
  return render(
    <AppContext.Provider
      value={{
        t: createTranslator(locale),
        locale,
        ready: true,
        bootError: null,
      }}
    >
      {ui}
    </AppContext.Provider>,
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  mocks.openExternal.mockResolvedValue(undefined);
  mocks.updateInstall.mockResolvedValue("C:\\Users\\test\\Downloads\\setup.exe");
  useAppStore.setState({
    update: null,
    toasts: [],
  });
});

describe("UpdateBanner", () => {
  it("opens the GitHub release page instead of downloading when self-install is unsupported", async () => {
    useAppStore.setState({ update: updateState({ canInstall: false }) });

    renderWithLocale(<UpdateBanner />, "id");

    expect(screen.getByRole("status")).toHaveTextContent(
      "Versi 0.2.5 sudah tersedia.",
    );
    const button = screen.getByRole("button", { name: "Buka di GitHub" });
    await userEvent.click(button);

    expect(mocks.openExternal).toHaveBeenCalledWith(RELEASE_URL);
    expect(mocks.updateInstall).not.toHaveBeenCalled();
  });

  it("downloads and runs the installer directly when self-install is supported", async () => {
    useAppStore.setState({
      update: updateState({
        canInstall: true,
        asset: { name: "VN.Asyik_0.2.5_x64-setup.exe", url: "https://x/s.exe", size: 10 },
      }),
    });

    renderWithLocale(<UpdateBanner />, "en");

    const button = screen.getByRole("button", { name: "Download and install" });
    await userEvent.click(button);

    expect(mocks.updateInstall).toHaveBeenCalledTimes(1);
    expect(mocks.openExternal).not.toHaveBeenCalled();
    expect(useAppStore.getState().toasts.at(-1)?.kind).toBe("success");
  });

  it("stays hidden when up to date or when a non-installable update lacks a release URL", () => {
    useAppStore.setState({ update: updateState({ available: false }) });
    const first = renderWithLocale(<UpdateBanner />);
    expect(first.container).toBeEmptyDOMElement();
    first.unmount();

    useAppStore.setState({
      update: updateState({ available: true, canInstall: false, url: null }),
    });
    const second = renderWithLocale(<UpdateBanner />);
    expect(second.container).toBeEmptyDOMElement();
  });

  it("surfaces an error toast when opening the GitHub release link fails", async () => {
    mocks.openExternal.mockRejectedValueOnce(
      new IpcError("open_failed", "could not open browser"),
    );
    useAppStore.setState({ update: updateState({ canInstall: false }) });

    renderWithLocale(<UpdateBanner />, "en");
    await userEvent.click(screen.getByRole("button", { name: "Open on GitHub" }));

    expect(useAppStore.getState().toasts.at(-1)).toMatchObject({
      kind: "error",
      message: "could not open browser",
    });
  });
});

describe("UpdatePanel", () => {
  it("shows Open on GitHub on Android / non-installable builds and avoids auto-download", async () => {
    useAppStore.setState({ update: updateState({ canInstall: false }) });

    renderWithLocale(<UpdatePanel />, "en");

    expect(screen.getByText("Version 0.2.5 is available.")).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Download and install" }),
    ).toBeNull();

    await userEvent.click(screen.getByRole("button", { name: "Open on GitHub" }));
    expect(mocks.openExternal).toHaveBeenCalledWith(RELEASE_URL);
    expect(mocks.updateInstall).not.toHaveBeenCalled();
  });

  it("refreshes the update status when Check now is clicked", async () => {
    const fresh = updateState({ canInstall: false, latest: "0.2.6" });
    mocks.updateCheck.mockResolvedValueOnce(fresh);
    useAppStore.setState({ update: updateState({ canInstall: false }) });

    renderWithLocale(<UpdatePanel />, "en");

    await userEvent.click(screen.getByRole("button", { name: "Check now" }));
    expect(mocks.updateCheck).toHaveBeenCalledWith(true);
    expect(useAppStore.getState().update?.latest).toBe("0.2.6");
  });
});
