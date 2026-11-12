import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import { AppContext } from "@/app/AppProvider";
import { useAppStore } from "@/app/store";
import { createTranslator } from "@/i18n";
import { api } from "@/lib/ipc/client";
import type { AppearanceView, ThemeView } from "@/lib/ipc/types";
import { DEFAULT_THEME_TOKENS, THEME_PRESETS, type Theme } from "@/lib/theme";

vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));

const baseAppearance: AppearanceView = {
  accent: null,
  backgroundName: null,
  background: null,
  backgroundDim: 0.45,
  theme: null,
  gameTint: true,
};

vi.mock("@/lib/ipc/client", () => ({
  api: {
    appearanceSet: vi.fn(),
    themeExportFile: vi.fn().mockResolvedValue(true),
    themeImportFile: vi.fn().mockResolvedValue(null),
  },
  IpcError: class extends Error {
    code = "invalid_theme";
  },
}));

const { AppearancePanel } = await import("./AppearancePanel");

/** A preset that must exist; the test fails loudly if the list shrinks. */
function preset(index: number): Theme {
  const found = THEME_PRESETS[index];
  if (!found) throw new Error(`preset ${index} is missing`);
  return found;
}

function renderPanel(appearance: AppearanceView = baseAppearance): void {
  useAppStore.setState({ appearance, toasts: [] });
  render(
    <AppContext.Provider
      value={{
        t: createTranslator("en"),
        locale: "en",
        ready: true,
        bootError: null,
      }}
    >
      <AppearancePanel />
    </AppContext.Provider>,
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(api.appearanceSet).mockImplementation(async (input) => ({
    ...baseAppearance,
    accent: input.accent,
    backgroundDim: input.backgroundDim,
    theme: input.clearTheme ? null : input.theme,
    gameTint: input.gameTint,
  }));
  useAppStore.setState({
    prefs: {
      theme: "dark",
      language: "en",
      nsfw: "blur",
      reduce_effects: false,
      beta_features: false,
    },
  });
});

describe("theme selection", () => {
  it("saves a preset with its own base and shape", async () => {
    renderPanel();
    const nord = preset(0);
    await userEvent.click(screen.getByRole("button", { name: new RegExp(nord.name) }));

    expect(api.appearanceSet).toHaveBeenCalledWith(
      expect.objectContaining({ theme: nord, clearTheme: false }),
    );
  });

  it("clears back to the built-in ramp", async () => {
    renderPanel({ ...baseAppearance, theme: preset(0) });
    await userEvent.click(screen.getByRole("button", { name: "Default" }));

    expect(api.appearanceSet).toHaveBeenCalledWith(
      expect.objectContaining({ theme: null, clearTheme: true }),
    );
  });

  it("seeds a custom theme from the current mode", async () => {
    renderPanel();
    await userEvent.click(screen.getByRole("button", { name: "Custom" }));

    // The draft is published immediately but written after the debounce, so the
    // assertion waits for the write rather than reading a call that has not
    // happened yet.
    await waitFor(
      () => {
        const call = vi.mocked(api.appearanceSet).mock.calls.at(-1)?.[0];
        expect(call?.theme?.tokens).toEqual(DEFAULT_THEME_TOKENS.dark);
        expect(call?.theme?.base).toBe("dark");
      },
      { timeout: 2000 },
    );
  });
});

describe("custom palette", () => {
  it("shows the editor and the measured contrast once a theme is active", async () => {
    renderPanel({ ...baseAppearance, theme: preset(0) });
    expect(screen.getByText("Colours")).toBeInTheDocument();
    expect(screen.getByText(/Body text reads at \d+\.\d:1/)).toBeInTheDocument();
  });

  it("writes a colour change after the debounce, not on every keystroke", async () => {
    const theme = preset(0);
    renderPanel({ ...baseAppearance, theme });

    // `userEvent.type` cannot drive a colour input, so the change is fired the
    // way the browser reports it.
    const field = screen.getByLabelText("Background") as HTMLInputElement;
    fireEvent.change(field, { target: { value: "#123456" } });

    await waitFor(
      () =>
        expect(api.appearanceSet).toHaveBeenCalledWith(
          expect.objectContaining({
            theme: expect.objectContaining({
              tokens: expect.objectContaining({ bg: "#123456" }),
            }),
          }),
        ),
      { timeout: 2000 },
    );
  });

  it("keeps the shape controls on the theme rather than the accent", async () => {
    const theme = preset(0);
    renderPanel({ ...baseAppearance, theme });

    await userEvent.click(screen.getByRole("button", { name: "Round" }));
    await waitFor(
      () =>
        expect(api.appearanceSet).toHaveBeenCalledWith(
          expect.objectContaining({
            theme: expect.objectContaining({
              tokens: expect.objectContaining({ radius: "round" }),
            }),
          }),
        ),
      { timeout: 2000 },
    );
  });
});

describe("sharing", () => {
  it("exports the active theme and confirms", async () => {
    const theme = preset(0);
    renderPanel({ ...baseAppearance, theme });

    await userEvent.click(screen.getByRole("button", { name: "Export theme" }));
    expect(api.themeExportFile).toHaveBeenCalledWith(theme);
    expect(useAppStore.getState().toasts.at(-1)?.message).toBe("Theme file saved");
  });

  it("applies an imported theme immediately", async () => {
    const imported: ThemeView = { ...preset(1), id: "imported:1" };
    vi.mocked(api.themeImportFile).mockResolvedValueOnce(imported);
    renderPanel();

    await userEvent.click(screen.getByRole("button", { name: "Import theme" }));
    await waitFor(() =>
      expect(api.appearanceSet).toHaveBeenCalledWith(
        expect.objectContaining({ theme: imported, clearTheme: false }),
      ),
    );
  });
});

describe("game tint", () => {
  it("is on by default and can be turned off", async () => {
    renderPanel();
    const toggle = screen.getByRole("checkbox");
    expect(toggle).toBeChecked();

    await userEvent.click(toggle);
    expect(api.appearanceSet).toHaveBeenCalledWith(
      expect.objectContaining({ gameTint: false }),
    );
  });
});
