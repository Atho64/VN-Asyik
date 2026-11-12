import { describe, expect, it } from "vitest";

import { contrastRatio, fromHex } from "./palette";
import {
  DEFAULT_THEME_TOKENS,
  THEME_PRESETS,
  applyThemeVars,
  ensureContrast,
  readableTokens,
  resetAppearanceForTest,
  resolveAccent,
  resolveThemeVars,
  themeContrast,
  updateAppearance,
  type Theme,
  type ThemeInput,
  type ThemeTokens,
} from "./theme";

const NO_MEDIA = { reducedTransparency: false, moreContrast: false, reduceEffects: false };

function input(patch: Partial<ThemeInput> = {}): ThemeInput {
  return {
    theme: null,
    mode: "dark",
    accent: null,
    tint: null,
    gameTint: true,
    media: NO_MEDIA,
    ...patch,
  };
}

function tokens(patch: Partial<ThemeTokens> = {}): ThemeTokens {
  return { ...DEFAULT_THEME_TOKENS.dark, ...patch };
}

function theme(patch: Partial<ThemeTokens> = {}, base: "dark" | "light" = "dark"): Theme {
  return { id: "custom:test", name: "Test", base, tokens: tokens(patch) };
}

describe("resolveThemeVars", () => {
  it("writes nothing at all when there is no theme and no accent", () => {
    // The default look must stay byte-identical to the stylesheet, or every
    // existing user gets a silent redesign on upgrade.
    expect(resolveThemeVars(input())).toEqual({});
  });

  it("carries a theme's surfaces, text and derived scrim", () => {
    const vars = resolveThemeVars(input({ theme: theme({ bg: "#2e3440" }) }));
    expect(vars["--bg"]).toBe("#2e3440");
    expect(vars["--text"]).toBe(DEFAULT_THEME_TOKENS.dark.text);
    expect(vars["--scrim-mid"]).toBe("rgb(46 52 64 / 0.7)");
  });

  it("follows the base for alpha ramps, not the chosen colours", () => {
    const light = resolveThemeVars(input({ theme: theme({}, "light"), mode: "light" }));
    expect(light["--border"]).toBe("rgb(16 18 28 / 0.12)");
    const dark = resolveThemeVars(input({ theme: theme() }));
    expect(dark["--border"]).toBe("rgb(255 255 255 / 0.1)");
  });
});

describe("readableTokens", () => {
  it("lifts unreadable text until it clears AA", () => {
    const clamped = readableTokens(tokens({ bg: "#000000", text: "#111111" }));
    const ratio = contrastRatio(
      fromHex(clamped.text) ?? { r: 0, g: 0, b: 0 },
      fromHex(clamped.bg) ?? { r: 0, g: 0, b: 0 },
    );
    expect(ratio).toBeGreaterThanOrEqual(4.5);
    expect(clamped.text).not.toBe("#111111");
  });

  it("darkens text on a light background instead of pushing it to white", () => {
    const clamped = readableTokens(
      tokens({ bg: "#ffffff", bgElevated: "#ffffff", text: "#eeeeee" }),
    );
    const ratio = contrastRatio(
      fromHex(clamped.text) ?? { r: 0, g: 0, b: 0 },
      { r: 255, g: 255, b: 255 },
    );
    expect(ratio).toBeGreaterThanOrEqual(4.5);
    expect(clamped.text).not.toBe("#ffffff");
  });

  it("leaves a theme that already reads well alone", () => {
    const original = tokens();
    expect(readableTokens(original)).toEqual(original);
  });

  it("accepts faint text at 3:1 that body text would have to lift", () => {
    // #5a5a5a on black is ~3.04:1: enough for the faint role, not enough for
    // body text. Both targets must be enforced, or faint text silently becomes
    // as loud as the body copy.
    const faintish = tokens({
      bg: "#000000",
      bgElevated: "#000000",
      textFaint: "#5a5a5a",
      text: "#5a5a5a",
    });
    const clamped = readableTokens(faintish);
    expect(clamped.textFaint).toBe("#5a5a5a");
    expect(clamped.text).not.toBe("#5a5a5a");
  });
});

describe("ensureContrast", () => {
  it("returns the input untouched when it already passes", () => {
    const fg = { r: 255, g: 255, b: 255 };
    const bg = { r: 0, g: 0, b: 0 };
    expect(ensureContrast(fg, bg, 4.5)).toEqual(fg);
  });
});

describe("resolveAccent", () => {
  const tint = { accent: "#ff0000", accentSoft: "#ff00002e", onAccent: "#ffffff" };

  it("lets the reader's accent beat the focused game's palette", () => {
    const accent = resolveAccent(input({ accent: "#65d8c5", tint }));
    expect(accent?.accent).not.toBe("#ff0000");
  });

  it("uses the game palette when the reader picked no accent", () => {
    expect(resolveAccent(input({ tint }))).toEqual(tint);
  });

  it("ignores the game palette when the switch is off", () => {
    expect(resolveAccent(input({ tint, gameTint: false }))).toBeNull();
  });

  it("falls back to nothing so the theme's own accent shows", () => {
    expect(resolveAccent(input())).toBeNull();
  });
});

describe("accessibility media", () => {
  it("keeps reduced transparency winning over a glassy theme", () => {
    // An inline property beats the stylesheet's media query, so the resolver has
    // to apply the reduced value itself or the setting silently stops working.
    const vars = resolveThemeVars(
      input({ theme: theme({ glass: 1 }), media: { ...NO_MEDIA, reducedTransparency: true } }),
    );
    expect(vars["--glass-blur"]).toBe("0px");
    expect(vars["--glass-saturate"]).toBe("1");
    expect(vars["--glass-bg"]).toBe(DEFAULT_THEME_TOKENS.dark.surfaceSolid);
  });

  it("raises muted text to full text under more contrast", () => {
    const vars = resolveThemeVars(
      input({ theme: theme(), media: { ...NO_MEDIA, moreContrast: true } }),
    );
    expect(vars["--text-muted"]).toBe(DEFAULT_THEME_TOKENS.dark.text);
  });

  it("keeps the Reduce effects switch working under a theme", () => {
    // `:root[data-reduce-effects="true"]` is a stylesheet rule, and the theme
    // writes inline properties, so the switch would silently stop working.
    const vars = resolveThemeVars(
      input({ theme: theme({ glass: 1 }), media: { ...NO_MEDIA, reduceEffects: true } }),
    );
    expect(vars["--glass-blur"]).toBe("0px");
    expect(vars["--glass-bg"]).toBe(DEFAULT_THEME_TOKENS.dark.surfaceSolid);
  });

  it("keeps a glassy theme glassy when neither query applies", () => {
    const vars = resolveThemeVars(input({ theme: theme({ glass: 1 }) }));
    expect(vars["--glass-blur"]).toBe("24px");
  });
});

describe("shape and density", () => {
  it("changes only space tokens for density", () => {
    const comfortable = resolveThemeVars(input({ theme: theme({ density: "comfortable" }) }));
    const compact = resolveThemeVars(input({ theme: theme({ density: "compact" }) }));
    const changed = Object.keys(comfortable).filter((key) => comfortable[key] !== compact[key]);
    expect(changed.length).toBeGreaterThan(0);
    expect(changed.every((key) => key.startsWith("--space-"))).toBe(true);
  });

  it("changes only radius tokens for corners", () => {
    const soft = resolveThemeVars(input({ theme: theme({ radius: "soft" }) }));
    const sharp = resolveThemeVars(input({ theme: theme({ radius: "sharp" }) }));
    const changed = Object.keys(soft).filter((key) => soft[key] !== sharp[key]);
    expect(changed.length).toBeGreaterThan(0);
    expect(changed.every((key) => key.startsWith("--radius-"))).toBe(true);
  });

  it("zeroes glass at strength 0", () => {
    const vars = resolveThemeVars(input({ theme: theme({ glass: 0 }) }));
    expect(vars["--glass-blur"]).toBe("0px");
  });
});

describe("presets", () => {
  it("all clear AA for body text against their own background", () => {
    for (const preset of THEME_PRESETS) {
      expect(themeContrast(readableTokens(preset.tokens)), preset.id).toBeGreaterThanOrEqual(4.5);
    }
  });

  it("have unique ids", () => {
    const ids = THEME_PRESETS.map((preset) => preset.id);
    expect(new Set(ids).size).toBe(ids.length);
  });
});

describe("applyThemeVars", () => {
  it("clears a property the next render no longer sets", () => {
    const root = document.createElement("div");
    applyThemeVars(root, { "--bg": "#000000", "--text": "#ffffff" });
    expect(root.style.getPropertyValue("--bg")).toBe("#000000");
    applyThemeVars(root, { "--text": "#ffffff" });
    expect(root.style.getPropertyValue("--bg")).toBe("");
    expect(root.style.getPropertyValue("--text")).toBe("#ffffff");
  });
});

describe("updateAppearance", () => {
  it("restores the reader's accent when the game tint is cleared", () => {
    // The regression this exists for: leaving a game route used to call
    // removeProperty("--accent"), which deleted the reader's own accent too.
    resetAppearanceForTest();
    updateAppearance({ accent: "#65d8c5" });
    updateAppearance({ tint: { accent: "#ff0000", accentSoft: "#ff00002e", onAccent: "#fff" } });
    expect(document.documentElement.style.getPropertyValue("--accent")).not.toBe("#ff0000");

    updateAppearance({ tint: null });
    expect(document.documentElement.style.getPropertyValue("--accent")).not.toBe("");
    resetAppearanceForTest();
  });

  it("leaves the stylesheet alone when nothing is set", () => {
    resetAppearanceForTest();
    updateAppearance({});
    expect(document.documentElement.style.getPropertyValue("--bg")).toBe("");
  });
});
