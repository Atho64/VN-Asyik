import { act, render, screen } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import { MemoryRouter, Route, Routes } from "react-router";
import { useEffect } from "react";
import { BetaRoute } from "./BetaRoute";
import { useAppStore } from "./store";

const mounted = vi.fn();
function Feature() {
  useEffect(() => {
    mounted();
  }, []);
  return <p>Experimental screen</p>;
}
beforeEach(() => {
  mounted.mockClear();
  useAppStore.setState({
    prefsLoaded: false,
    prefs: { ...useAppStore.getState().prefs, beta_features: false },
  });
});
function show(path = "/companion", overlay = false) {
  return render(
    <MemoryRouter initialEntries={[path]}>
      <Routes>
        <Route
          path={path}
          element={
            <BetaRoute overlay={overlay}>
              <Feature />
            </BetaRoute>
          }
        />
        <Route path="/settings" element={<p>Settings</p>} />
      </Routes>
    </MemoryRouter>,
  );
}
it.each(["/companion", "/chat", "/recommend"])(
  "blocks direct entry to %s without mounting feature effects",
  (path) => {
    useAppStore.setState({ prefsLoaded: true });
    show(path);
    expect(screen.getByText("Settings")).toBeInTheDocument();
    expect(mounted).not.toHaveBeenCalled();
  },
);
it("waits for saved preferences and removes the mounted feature when disabled", () => {
  show();
  expect(mounted).not.toHaveBeenCalled();
  expect(screen.queryByText("Settings")).toBeNull();
  act(() =>
    useAppStore
      .getState()
      .setPrefs({ ...useAppStore.getState().prefs, beta_features: true }),
  );
  expect(screen.getByText("Experimental screen")).toBeInTheDocument();
  act(() =>
    useAppStore
      .getState()
      .setPrefs({ ...useAppStore.getState().prefs, beta_features: false }),
  );
  expect(screen.queryByText("Experimental screen")).toBeNull();
  expect(screen.getByText("Settings")).toBeInTheDocument();
});
it("leaves a disabled overlay empty instead of navigating it to the main app", () => {
  useAppStore.setState({ prefsLoaded: true });
  show("/companion-overlay", true);
  expect(mounted).not.toHaveBeenCalled();
  expect(screen.queryByText("Settings")).toBeNull();
});
