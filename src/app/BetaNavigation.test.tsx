import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import { MemoryRouter } from "react-router";
import { AppContext } from "./AppProvider";
import { useAppStore } from "./store";
import { createTranslator } from "@/i18n";
import { AppShell } from "./AppShell";

vi.mock("@/components/Backdrop", () => ({ Backdrop: () => null, focusArt: vi.fn() }));
vi.mock("@/features/tracking/NowPlayingBar", () => ({ NowPlayingBar: () => null }));
vi.mock("@/features/companion/CompanionBar", () => ({
  CompanionBar: () => <p>Active companion</p>,
}));
vi.mock("@/features/library/AddGameSheet", () => ({ AddGameSheet: () => null }));
vi.mock("@/lib/ipc/companionWindow", () => ({
  companionWindowApi: { listenMain: vi.fn(async () => () => {}) },
}));
beforeEach(() => {
  useAppStore.setState({
    prefs: { ...useAppStore.getState().prefs, beta_features: false },
    paletteOpen: false,
    toasts: [],
  });
});
it("gates desktop navigation, mobile More, the session bar and command palette together", async () => {
  render(
    <AppContext.Provider
      value={{ t: createTranslator("en"), locale: "en", ready: true, bootError: null }}
    >
      <MemoryRouter>
        <AppShell />
      </MemoryRouter>
    </AppContext.Provider>,
  );
  for (const name of ["Recommendations", "VN Chat", "Companion"])
    expect(screen.queryByRole("link", { name })).toBeNull();
  expect(screen.queryByText("Active companion")).toBeNull();
  act(() => useAppStore.setState({ paletteOpen: true }));
  expect(screen.queryByRole("option", { name: /VN Chat/ })).toBeNull();
  act(() => useAppStore.setState({ paletteOpen: false }));
  await userEvent.click(screen.getByRole("button", { name: "More" }));
  const more = screen.getByRole("navigation", { name: "More" });
  for (const name of ["Recommendations", "VN Chat", "Companion"])
    expect(within(more).queryByRole("link", { name })).toBeNull();
  act(() =>
    useAppStore.setState({
      prefs: { ...useAppStore.getState().prefs, beta_features: true },
    }),
  );
  for (const name of ["Recommendations", "VN Chat", "Companion"])
    expect(within(more).getByRole("link", { name })).toBeInTheDocument();
  expect(screen.getByText("Active companion")).toBeInTheDocument();
  act(() =>
    useAppStore.setState({
      prefs: { ...useAppStore.getState().prefs, beta_features: false },
    }),
  );
  expect(screen.queryByText("Active companion")).toBeNull();
});
