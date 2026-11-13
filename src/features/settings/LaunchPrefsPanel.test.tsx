import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { AppContext } from "@/app/AppProvider";
import { useAppStore } from "@/app/store";
import { createTranslator } from "@/i18n";

const mocks = vi.hoisted(() => ({
  uiPrefsGet: vi.fn(),
  uiPrefsSet: vi.fn(),
  windowLaunchGet: vi.fn(),
  windowLaunchSet: vi.fn(),
  windowLaunchCapture: vi.fn(),
}));

vi.mock("@/lib/ipc/client", () => ({
  api: {
    uiPrefsGet: mocks.uiPrefsGet,
    uiPrefsSet: mocks.uiPrefsSet,
    windowLaunchGet: mocks.windowLaunchGet,
    windowLaunchSet: mocks.windowLaunchSet,
    windowLaunchCapture: mocks.windowLaunchCapture,
  },
  IpcError: class extends Error {
    code = "error";
    constructor(code: string, message: string) {
      super(message);
      this.code = code;
    }
  },
}));

const { LibraryLaunchSection, WindowLaunchSection } = await import("./LaunchPrefsPanel");
const t = createTranslator("en");

function Wrapper({ children }: { children: React.ReactNode }) {
  return (
    <AppContext.Provider value={{ t, locale: "en", ready: true, bootError: null }}>
      {children}
    </AppContext.Provider>
  );
}

describe("LaunchPrefsPanel", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useAppStore.setState({
      uiPrefs: {
        sidebar_compact: false,
        library_remember_browse: false,
        library_default_sort: "recent",
        library_saved_browse: null,
      },
      uiPrefsLoaded: true,
      libraryBrowse: {
        query: "clannad",
        statuses: ["playing"],
        ratedOnly: true,
        view: "list",
        sort: "rating",
      },
    });
  });

  it("saves current library filters when remember-on-launch is enabled and updates default sort", async () => {
    mocks.uiPrefsSet.mockImplementation(async (patch: Record<string, unknown>) => ({
      ...useAppStore.getState().uiPrefs!,
      ...patch,
    }));

    render(
      <Wrapper>
        <LibraryLaunchSection />
      </Wrapper>,
    );

    const sortSelect = screen.getByLabelText("Default sort order");
    fireEvent.change(sortSelect, { target: { value: "playtime" } });

    await waitFor(() => {
      expect(mocks.uiPrefsSet).toHaveBeenCalledWith({
        library_default_sort: "playtime",
      });
    });
    expect(useAppStore.getState().libraryBrowse.sort).toBe("playtime");

    fireEvent.click(screen.getByRole("button", { name: "On" }));

    await waitFor(() => {
      expect(mocks.uiPrefsSet).toHaveBeenLastCalledWith({
        library_remember_browse: true,
        library_saved_browse: {
          statuses: ["playing"],
          rated_only: true,
          view: "list",
          sort: "playtime",
        },
      });
    });
  });

  it("switches desktop window launch mode, captures current coordinates, and applies custom coordinates", async () => {
    mocks.windowLaunchGet.mockResolvedValue({
      mode: "centered",
      x: null,
      y: null,
      current_x: 320,
      current_y: 180,
      supported: true,
    });
    mocks.windowLaunchSet.mockImplementation(async (input: Record<string, unknown>) => ({
      mode: input.mode,
      x: (input.x as number | null | undefined) ?? 320,
      y: (input.y as number | null | undefined) ?? 180,
      current_x: 320,
      current_y: 180,
      supported: true,
    }));
    mocks.windowLaunchCapture.mockResolvedValue({
      mode: "customized",
      x: 410,
      y: 225,
      current_x: 410,
      current_y: 225,
      supported: true,
    });

    render(
      <Wrapper>
        <WindowLaunchSection />
      </Wrapper>,
    );

    const customChip = await screen.findByRole("button", { name: "Customized" });
    fireEvent.click(customChip);

    await waitFor(() => {
      expect(mocks.windowLaunchSet).toHaveBeenCalledWith({
        mode: "customized",
        x: 320,
        y: 180,
        apply_now: false,
      });
    });

    const xField = await screen.findByLabelText("X position (px)");
    const yField = screen.getByLabelText("Y position (px)");
    expect(xField).toHaveValue(320);
    expect(yField).toHaveValue(180);

    fireEvent.click(screen.getByRole("button", { name: "Use current" }));
    await waitFor(() => {
      expect(mocks.windowLaunchCapture).toHaveBeenCalledOnce();
    });
    expect(screen.getByLabelText("X position (px)")).toHaveValue(410);
    expect(screen.getByLabelText("Y position (px)")).toHaveValue(225);

    fireEvent.change(screen.getByLabelText("X position (px)"), { target: { value: "500" } });
    fireEvent.change(screen.getByLabelText("Y position (px)"), { target: { value: "260" } });
    fireEvent.click(screen.getByRole("button", { name: "Apply" }));

    await waitFor(() => {
      expect(mocks.windowLaunchSet).toHaveBeenLastCalledWith({
        mode: "customized",
        x: 500,
        y: 260,
        apply_now: true,
      });
    });
  });

  it("renders nothing when the platform reports window positioning unsupported", async () => {
    mocks.windowLaunchGet.mockResolvedValue({
      mode: "centered",
      x: null,
      y: null,
      current_x: null,
      current_y: null,
      supported: false,
    });

    const { container } = render(
      <Wrapper>
        <WindowLaunchSection />
      </Wrapper>,
    );

    await waitFor(() => {
      expect(mocks.windowLaunchGet).toHaveBeenCalledOnce();
    });
    expect(container).toBeEmptyDOMElement();
  });
});
