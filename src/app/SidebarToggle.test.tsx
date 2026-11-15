import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { MemoryRouter } from "react-router";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { createTranslator } from "@/i18n";
import { AppContext } from "./AppProvider";
import { useAppStore } from "./store";
import type { UiPrefs } from "@/lib/ipc/types";

const mocks = vi.hoisted(() => ({
  uiPrefsSet: vi.fn(),
  backdropRender: vi.fn(),
  nowPlayingRender: vi.fn(),
}));

vi.mock("@/lib/ipc/client", () => ({
  api: {
    uiPrefsSet: mocks.uiPrefsSet,
  },
  IpcError: class extends Error {},
}));

vi.mock("@/components/Backdrop", () => ({
  Backdrop: () => {
    mocks.backdropRender();
    return null;
  },
  focusArt: vi.fn(),
}));
vi.mock("@/features/tracking/NowPlayingBar", () => ({
  NowPlayingBar: () => {
    mocks.nowPlayingRender();
    return null;
  },
}));
vi.mock("@/features/companion/CompanionBar", () => ({
  CompanionBar: () => null,
}));
vi.mock("@/features/library/AddGameSheet", () => ({ AddGameSheet: () => null }));
vi.mock("@/lib/ipc/companionWindow", () => ({
  companionWindowApi: {
    hide: vi.fn().mockResolvedValue(undefined),
    listenMain: vi.fn().mockResolvedValue(() => undefined),
  },
}));

const { AppShell } = await import("./AppShell");

const t = createTranslator("en");

function renderShell() {
  return render(
    <AppContext.Provider value={{ t, locale: "en", ready: true, bootError: null }}>
      <MemoryRouter>
        <AppShell />
      </MemoryRouter>
    </AppContext.Provider>,
  );
}

describe("desktop sidebar compact toggle", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    window.localStorage.clear();
    mocks.uiPrefsSet.mockImplementation(async (patch: { sidebar_compact?: boolean }) => ({
      sidebar_compact: Boolean(patch.sidebar_compact),
      library_remember_browse: false,
      library_default_sort: "recent",
      library_saved_browse: null,
    }));
    useAppStore.setState({
      sidebarCompact: false,
      uiPrefs: {
        sidebar_compact: false,
        library_remember_browse: false,
        library_default_sort: "recent",
        library_saved_browse: null,
      },
      uiPrefsLoaded: true,
    });
  });

  it("toggles between expanded and compact icon-only mode while preserving accessible labels", async () => {
    renderShell();

    const rail = screen
      .getAllByRole("navigation", { name: "VN Asyik" })
      .find((el) => el.classList.contains("rail"));
    expect(rail).toBeDefined();
    if (!rail) throw new Error("Expected desktop rail navigation");
    const railQueries = within(rail);
    expect(rail).toHaveAttribute("data-compact", "false");

    const collapseBtn = railQueries.getByRole("button", { name: "Collapse sidebar" });
    expect(collapseBtn).toHaveAttribute("aria-expanded", "true");

    fireEvent.click(collapseBtn);

    expect(rail).toHaveAttribute("data-compact", "true");
    const expandBtn = railQueries.getByRole("button", { name: "Expand sidebar" });
    expect(expandBtn).toHaveAttribute("aria-expanded", "false");

    // Accessible link roles and titles remain available for assistive tech and hover tooltips.
    const libraryLink = railQueries.getByRole("link", { name: "Library" });
    expect(libraryLink).toHaveAttribute("title", "Library");
    expect(railQueries.getByRole("link", { name: "Settings" })).toHaveAttribute(
      "title",
      "Settings",
    );

    await waitFor(() => {
      expect(mocks.uiPrefsSet).toHaveBeenCalledWith({ sidebar_compact: true });
    });
    expect(window.localStorage.getItem("vn_asyik_sidebar_compact")).toBe("1");

    fireEvent.click(expandBtn);
    expect(rail).toHaveAttribute("data-compact", "false");
    expect(railQueries.getByRole("link", { name: "Library" })).not.toHaveAttribute(
      "title",
    );
    await waitFor(() => {
      expect(mocks.uiPrefsSet).toHaveBeenLastCalledWith({ sidebar_compact: false });
    });
    expect(window.localStorage.getItem("vn_asyik_sidebar_compact")).toBe("0");
  });

  it("does not rerender artwork and tracking bars when the rail changes width", () => {
    renderShell();
    const artworkRenders = mocks.backdropRender.mock.calls.length;
    const trackingRenders = mocks.nowPlayingRender.mock.calls.length;
    fireEvent.click(screen.getByRole("button", { name: "Collapse sidebar" }));
    fireEvent.click(screen.getByRole("button", { name: "Expand sidebar" }));
    expect(mocks.backdropRender).toHaveBeenCalledTimes(artworkRenders);
    expect(mocks.nowPlayingRender).toHaveBeenCalledTimes(trackingRenders);
  });

  it("serializes saves and never rolls back a newer click when the old save completes", async () => {
    const completions: Array<(prefs: UiPrefs) => void> = [];
    mocks.uiPrefsSet.mockImplementation(
      () => new Promise<UiPrefs>((resolve) => completions.push(resolve)),
    );
    renderShell();
    fireEvent.click(screen.getByRole("button", { name: "Collapse sidebar" }));
    fireEvent.click(screen.getByRole("button", { name: "Expand sidebar" }));
    expect(useAppStore.getState().sidebarCompact).toBe(false);
    expect(mocks.uiPrefsSet).toHaveBeenCalledTimes(1);

    const base = useAppStore.getState().uiPrefs;
    if (!base) throw new Error("Missing UI preferences");
    const firstSave = completions[0];
    if (!firstSave) throw new Error("Missing first save");
    await act(async () => firstSave({ ...base, sidebar_compact: true }));
    expect(useAppStore.getState().sidebarCompact).toBe(false);
    expect(mocks.uiPrefsSet).toHaveBeenCalledTimes(2);
    expect(mocks.uiPrefsSet).toHaveBeenLastCalledWith({ sidebar_compact: false });
    const secondSave = completions[1];
    if (!secondSave) throw new Error("Missing second save");
    await act(async () => secondSave({ ...base, sidebar_compact: false }));
    expect(useAppStore.getState().sidebarCompact).toBe(false);
    expect(window.localStorage.getItem("vn_asyik_sidebar_compact")).toBe("0");
  });
});
