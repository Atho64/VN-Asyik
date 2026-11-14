import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { createTranslator } from "@/i18n";
import { AppContext } from "./AppProvider";
import { useAppStore } from "./store";
import { AppShell } from "./AppShell";

vi.mock("@/lib/ipc/client", () => ({ api: {}, IpcError: class extends Error {} }));
vi.mock("@/components/Backdrop", () => ({ Backdrop: () => null, focusArt: vi.fn() }));
vi.mock("@/features/tracking/NowPlayingBar", () => ({ NowPlayingBar: () => null }));
vi.mock("@/features/companion/CompanionBar", () => ({ CompanionBar: () => null }));
vi.mock("@/lib/ipc/screenshots", () => ({
  listenScreenshotShortcut: vi.fn(async () => () => {}),
}));
vi.mock("@/lib/ipc/companionWindow", () => ({
  companionWindowApi: { listenMain: vi.fn(async () => () => {}) },
}));
vi.mock("@/features/library/AddGameSheet", () => ({
  AddGameSheet: ({ onClose }: { onClose: () => void }) => (
    <div role="dialog" aria-label="Add a game">
      <button onClick={onClose}>Cancel import</button>
    </div>
  ),
}));

function renderShell() {
  return render(
    <AppContext.Provider
      value={{ t: createTranslator("en"), locale: "en", ready: true, bootError: null }}
    >
      <MemoryRouter>
        <Routes>
          <Route element={<AppShell />}>
            <Route path="/" element={<input aria-label="Reading note" />} />
            <Route path="/stats" element={<h1>Reading statistics</h1>} />
          </Route>
        </Routes>
      </MemoryRouter>
    </AppContext.Provider>,
  );
}

describe("command palette shortcuts", () => {
  beforeEach(() => {
    useAppStore.setState({
      paletteOpen: false,
      cards: [],
      toasts: [],
      startupPrefs: null,
    });
  });

  it.each([{ ctrlKey: true }, { metaKey: true }])(
    "opens from an input, accepts a command and closes with %j",
    async (modifier) => {
      renderShell();
      const note = screen.getByRole("textbox", { name: "Reading note" });
      note.focus();
      expect(fireEvent.keyDown(note, { key: "k", code: "KeyK", ...modifier })).toBe(
        false,
      );
      const input = screen.getByRole("textbox", {
        name: "Jump to a screen or search a title",
      });
      await waitFor(() => expect(input).toHaveFocus());
      fireEvent.change(input, { target: { value: "Go to stats" } });
      fireEvent.keyDown(input, { key: "Enter" });
      expect(
        screen.getByRole("heading", { name: "Reading statistics" }),
      ).toBeInTheDocument();
      expect(screen.queryByRole("dialog", { name: "Search commands" })).toBeNull();
    },
  );

  it("toggles once per press and restores focus when dismissed", () => {
    renderShell();
    const note = screen.getByRole("textbox", { name: "Reading note" });
    note.focus();
    fireEvent.keyDown(note, { key: "k", ctrlKey: true });
    fireEvent.keyDown(window, { key: "k", ctrlKey: true, repeat: true });
    expect(screen.getByRole("dialog", { name: "Search commands" })).toBeInTheDocument();
    fireEvent.keyDown(window, { key: "k", ctrlKey: true });
    expect(screen.queryByRole("dialog", { name: "Search commands" })).toBeNull();
    expect(note).toHaveFocus();
    fireEvent.keyDown(note, { key: "k", ctrlKey: true });
    fireEvent.keyDown(document, { key: "Escape" });
    expect(screen.queryByRole("dialog", { name: "Search commands" })).toBeNull();
    expect(note).toHaveFocus();
  });

  it.each([
    { key: "k" },
    { key: "k", ctrlKey: true, altKey: true },
    { key: "K", ctrlKey: true, shiftKey: true },
    { key: "k", ctrlKey: true, isComposing: true },
    { key: "k", ctrlKey: true, repeat: true },
  ])("ignores text, other chords, IME and held keys: %j", (keys) => {
    renderShell();
    fireEvent.keyDown(window, keys);
    expect(screen.queryByRole("dialog", { name: "Search commands" })).toBeNull();
  });

  it("keeps the import sheet open after choosing Add a game", () => {
    renderShell();
    fireEvent.keyDown(window, { key: "k", ctrlKey: true });
    fireEvent.click(screen.getByRole("option", { name: "Add a game" }));
    expect(screen.queryByRole("dialog", { name: "Search commands" })).toBeNull();
    expect(screen.getByRole("dialog", { name: "Add a game" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Cancel import" }));
    expect(screen.queryByRole("dialog", { name: "Add a game" })).toBeNull();
  });

  it("removes the shortcut listener when the shell unmounts", () => {
    const view = renderShell();
    view.unmount();
    fireEvent.keyDown(window, { key: "k", ctrlKey: true });
    expect(useAppStore.getState().paletteOpen).toBe(false);
  });
});
