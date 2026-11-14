/**
 * Single writer for document theme and accent custom properties (`theme -> reader accent -> game tint`).
 * Also accepts media/effect accessibility flags directly because inline custom properties override
 * stylesheet `@media` and `:root[data-reduce-effects]` rules.
 */

import {
  clampForContrast,
  clampForContrastDark,
  contrastRatio,
  fromHex,
  luminance,
  mix,
  toHex,
  type AccentPalette,
  type Rgb,
} from "./palette";

export type ThemeBase = "dark" | "light";

/** Corner treatment. `soft` is the design's own scale. */
export type RadiusScale = "sharp" | "soft" | "round";

/** Spacing treatment. Only the space tokens move; nothing else reflows. */
export type Density = "compact" | "comfortable";

/**
 * Editable theme tokens. Translucency ramps and artwork scrims are derived from
 * `base` and `bg` rather than exposed directly so contrast remains intact.
 */
export interface ThemeTokens {
  bg: string;
  bgElevated: string;
  surfaceSolid: string;
  text: string;
  textMuted: string;
  textFaint: string;
  radius: RadiusScale;
  density: Density;
  /** 0..1. How much glass the surfaces keep. 0 is the reduced-effects look. */
  glass: number;
}

export interface Theme {
  /** `preset:<name>` or `custom:<uuid>`. */
  id: string;
  name: string;
  base: ThemeBase;
  tokens: ThemeTokens;
}

/**
 * Caller-resolved accessibility preferences; read inline here because inline
 * custom properties override `:root[data-reduce-effects]` CSS rules.
 */
export interface ThemeMedia {
  reducedTransparency: boolean;
  moreContrast: boolean;
  reduceEffects: boolean;
}

export const DEFAULT_THEME_TOKENS: Record<ThemeBase, ThemeTokens> = {
  dark: {
    bg: "#101719",
    bgElevated: "#182225",
    surfaceSolid: "#202d30",
    text: "#edf7f4",
    textMuted: "#b6c9c4",
    textFaint: "#91aaa4",
    radius: "soft",
    density: "comfortable",
    glass: 1,
  },
  light: {
    bg: "#f2f7f5",
    bgElevated: "#ffffff",
    surfaceSolid: "#ffffff",
    text: "#172c28",
    textMuted: "#475f59",
    textFaint: "#5c756d",
    radius: "soft",
    density: "comfortable",
    glass: 1,
  },
};

/** Curated themes tuned for different reading conditions; omits Default so built-in light/dark remains untouched. */
export const THEME_PRESETS: Theme[] = [
  {
    id: "preset:nord",
    name: "Nord",
    base: "dark",
    tokens: {
      bg: "#2e3440",
      bgElevated: "#3b4252",
      surfaceSolid: "#434c5e",
      text: "#eceff4",
      textMuted: "#d8dee9",
      textFaint: "#aeb8c8",
      radius: "soft",
      density: "comfortable",
      glass: 0.8,
    },
  },
  {
    id: "preset:sepia",
    name: "Sepia",
    base: "light",
    tokens: {
      bg: "#f4ecdd",
      bgElevated: "#fbf5e9",
      surfaceSolid: "#fbf5e9",
      text: "#3a2f22",
      textMuted: "#6b5a45",
      textFaint: "#857055",
      radius: "soft",
      density: "comfortable",
      glass: 0.55,
    },
  },
  {
    id: "preset:slate",
    name: "Slate",
    base: "dark",
    tokens: {
      bg: "#151a21",
      bgElevated: "#1d242e",
      surfaceSolid: "#252e3a",
      text: "#e6edf5",
      textMuted: "#b3c0d0",
      textFaint: "#8b98a9",
      radius: "sharp",
      density: "compact",
      glass: 0.5,
    },
  },
  {
    id: "preset:contrast",
    name: "High contrast",
    base: "dark",
    tokens: {
      bg: "#000000",
      bgElevated: "#0b0b0b",
      surfaceSolid: "#161616",
      text: "#ffffff",
      textMuted: "#e4e4e4",
      textFaint: "#c2c2c2",
      radius: "sharp",
      density: "comfortable",
      glass: 0,
    },
  },
];

const RADIUS: Record<RadiusScale, Record<string, string>> = {
  sharp: {
    "--radius-xs": "2px",
    "--radius-s": "4px",
    "--radius-m": "6px",
    "--radius-l": "10px",
    "--radius-xl": "12px",
  },
  soft: {
    "--radius-xs": "6px",
    "--radius-s": "8px",
    "--radius-m": "14px",
    "--radius-l": "22px",
    "--radius-xl": "28px",
  },
  round: {
    "--radius-xs": "10px",
    "--radius-s": "14px",
    "--radius-m": "20px",
    "--radius-l": "28px",
    "--radius-xl": "36px",
  },
};

const DENSITY: Record<Density, number> = {
  comfortable: 1,
  compact: 0.85,
};

/** Space tokens, in the order `tokens.css` declares them. */
const SPACE: Record<string, number> = {
  "--space-1": 4,
  "--space-half": 2,
  "--space-2": 8,
  "--space-3": 12,
  "--space-4": 16,
  "--space-5": 24,
  "--space-6": 32,
  "--space-7": 48,
  "--space-8": 64,
};

const BASE_RADIUS = 24;
const BASE_SATURATE = 1.4;

/**
 * Move `fg` away from `bg` until it clears `target`.
 *
 * Direction comes from the background's luminance, not from the theme's name: a
 * "light" theme with a dark custom background still needs its text lightened, and
 * `clampForContrast` alone would push every colour toward white regardless.
 */
export function ensureContrast(fg: Rgb, bg: Rgb, target: number): Rgb {
  if (contrastRatio(fg, bg) >= target) return fg;
  const away: Rgb =
    luminance(bg) < 0.5 ? { r: 255, g: 255, b: 255 } : { r: 0, g: 0, b: 0 };
  let candidate = fg;
  for (let step = 0; step < 20; step += 1) {
    candidate = mix(candidate, away, 0.12);
    if (contrastRatio(candidate, bg) >= target) return candidate;
  }
  return away;
}

/** `r g b` for `rgb(r g b / a)` composition. */
function triplet(hex: string): string {
  const rgb = fromHex(hex);
  if (!rgb) return "0 0 0";
  return `${Math.round(rgb.r)} ${Math.round(rgb.g)} ${Math.round(rgb.b)}`;
}

/**
 * Clamp a theme's text tokens against its own backgrounds.
 *
 * Applied on read, not on write: the stored theme keeps the reader's exact hexes,
 * so switching a preset's background later re-derives readable text instead of
 * silently keeping a colour that was clamped for a different surface.
 */
export function readableTokens(tokens: ThemeTokens): ThemeTokens {
  const bg = fromHex(tokens.bg);
  const elevated = fromHex(tokens.bgElevated);
  if (!bg) return tokens;
  const against = elevated ?? bg;
  const clamp = (value: string, target: number): string => {
    const rgb = fromHex(value);
    if (!rgb) return value;
    const first = ensureContrast(rgb, bg, target);
    return toHex(ensureContrast(first, against, target));
  };
  return {
    ...tokens,
    text: clamp(tokens.text, 4.5),
    textMuted: clamp(tokens.textMuted, 4.5),
    textFaint: clamp(tokens.textFaint, 3),
  };
}

/** Contrast ratio of the theme's body text against its own background. */
export function themeContrast(tokens: ThemeTokens): number {
  const bg = fromHex(tokens.bg);
  const text = fromHex(tokens.text);
  if (!bg || !text) return 0;
  return contrastRatio(text, bg);
}

export interface ThemeInput {
  theme: Theme | null;
  /** The mode actually in force: the theme's base, or the prefs answer. */
  mode: ThemeBase;
  /** The reader's accent, exactly as picked. */
  accent: string | null;
  /** The focused game's palette, or null when nothing is focused. */
  tint: AccentPalette | null;
  /** Whether a game is allowed to tint the interface at all. */
  gameTint: boolean;
  media: ThemeMedia;
}

/**
 * Accent precedence: reader's fixed colour > focused game's tint (when enabled) > theme default.
 */
export function resolveAccent(input: ThemeInput): AccentPalette | null {
  if (!input.accent) {
    return input.gameTint ? (input.tint ?? null) : null;
  }
  const hex = input.accent;
  const adapted =
    input.mode === "dark"
      ? clampForContrast(hexToRgb(hex), hexToRgb(defaultBg("dark")), 4.5)
      : clampForContrastDark(hexToRgb(hex), hexToRgb(defaultBg("light")), 4.5);
  const accent = toHex(adapted);
  return {
    accent,
    accentSoft: `${accent}2e`,
    onAccent: onAccent(accent),
  };
}

function hexToRgb(hex: string): Rgb {
  return fromHex(hex) ?? { r: 0, g: 0, b: 0 };
}

function defaultBg(base: ThemeBase): string {
  return base === "dark" ? "#101719" : "#f2f7f5";
}

/** Whichever of near-black or white contrasts more with the accent. */
function onAccent(hex: string): string {
  const rgb = hexToRgb(hex);
  const dark: Rgb = { r: 16, g: 33, b: 37 };
  const white: Rgb = { r: 255, g: 255, b: 255 };
  return contrastRatio(rgb, dark) >= contrastRatio(rgb, white) ? "#102125" : "#ffffff";
}

/**
 * Returns only custom properties that override `tokens.css` (empty when no theme or accent is set).
 */
export function resolveThemeVars(input: ThemeInput): Record<string, string> {
  const vars: Record<string, string> = {};
  const accent = resolveAccent(input);
  if (accent) {
    vars["--accent"] = accent.accent;
    vars["--accent-soft"] = accent.accentSoft;
    vars["--accent-strong"] = accent.accent;
    vars["--on-accent"] = accent.onAccent;
    vars["--accent-contrast"] = accent.onAccent;
  }

  const theme = input.theme;
  if (!theme) return vars;

  const tokens = readableTokens(theme.tokens);
  const base = theme.base;
  const light = base === "light";

  vars["--bg"] = tokens.bg;
  vars["--bg-elevated"] = tokens.bgElevated;
  vars["--surface-solid"] = tokens.surfaceSolid;

  // Alpha ramps follow the base, not the chosen colours: a light theme with a
  // dark background would otherwise paint white scrims over black surfaces.
  const ink = light ? "16 18 28" : "255 255 255";
  vars["--surface-1"] = `rgb(${ink} / ${light ? 0.04 : 0.05})`;
  vars["--surface-2"] = `rgb(${ink} / ${light ? 0.07 : 0.08})`;
  vars["--surface-3"] = `rgb(${ink} / ${light ? 0.11 : 0.12})`;
  vars["--border"] = `rgb(${ink} / ${light ? 0.12 : 0.1})`;
  vars["--border-strong"] = `rgb(${ink} / ${light ? 0.2 : 0.18})`;
  vars["--field-bg"] = light ? "rgb(255 255 255 / 0.72)" : "rgb(255 255 255 / 0.08)";

  vars["--text"] = tokens.text;
  vars["--text-muted"] = tokens.textMuted;
  vars["--text-faint"] = tokens.textFaint;

  // Derive the artwork scrim from `tokens.bg` so readers cannot set a scrim that makes text unreadable.
  const bg = triplet(tokens.bg);
  vars["--scrim-top"] = `rgb(${bg} / 0.3)`;
  vars["--scrim-mid"] = `rgb(${bg} / 0.7)`;
  vars["--scrim-bottom"] = `rgb(${bg} / 0.94)`;
  vars["--text-shadow-over-art"] = light
    ? "0 1px 2px rgb(255 255 255 / 0.6), 0 2px 12px rgb(255 255 255 / 0.4)"
    : "0 1px 2px rgb(0 0 0 / 0.55), 0 2px 12px rgb(0 0 0 / 0.35)";

  const glass = Math.max(0, Math.min(1, tokens.glass));
  vars["--glass-bg"] = light
    ? `rgb(255 255 255 / ${(0.25 + 0.3 * glass).toFixed(2)})`
    : `rgb(255 255 255 / ${(0.06 * glass).toFixed(3)})`;
  vars["--glass-blur"] = `${Math.round(BASE_RADIUS * glass)}px`;
  vars["--glass-saturate"] = `${(1 + (BASE_SATURATE - 1) * glass).toFixed(2)}`;

  Object.assign(vars, RADIUS[tokens.radius]);

  const scale = DENSITY[tokens.density];
  for (const [name, value] of Object.entries(SPACE)) {
    vars[name] = `${Math.round(value * scale)}px`;
  }

  // Accessibility last: inline properties override stylesheet media queries.
  if (input.media.reducedTransparency || input.media.reduceEffects) {
    vars["--glass-bg"] = tokens.surfaceSolid;
    vars["--glass-blur"] = "0px";
    vars["--glass-saturate"] = "1";
  }
  if (input.media.moreContrast) {
    vars["--border"] = vars["--border-strong"] ?? `rgb(${ink} / 0.2)`;
    vars["--text-muted"] = tokens.text;
  }

  return vars;
}

/** Properties this module has written, so the next render can clear them. */
const written = new Set<string>();

/** Combined inputs from provider, cover hover tint, and route clear; resolved once so writers cannot clobber each other. */
let current: ThemeInput = {
  theme: null,
  mode: "dark",
  accent: null,
  tint: null,
  gameTint: true,
  media: { reducedTransparency: false, moreContrast: false, reduceEffects: false },
};

/** Publish a change and repaint. The only entry point callers should use. */
export function updateAppearance(patch: Partial<ThemeInput>): void {
  current = { ...current, ...patch };
  if (typeof document === "undefined") return;
  applyThemeVars(document.documentElement, resolveThemeVars(current));
}

/** Read back what is currently published, for tests and for `useArtwork`. */
export function currentAppearance(): ThemeInput {
  return current;
}

/** Test seam: drop everything back to no theme, no accent, no tint. */
export function resetAppearanceForTest(): void {
  current = {
    theme: null,
    mode: "dark",
    accent: null,
    tint: null,
    gameTint: true,
    media: { reducedTransparency: false, moreContrast: false, reduceEffects: false },
  };
  if (typeof document !== "undefined") {
    applyThemeVars(document.documentElement, {});
  }
  written.clear();
}

/** Write a resolved map onto the root element, clearing anything it no longer sets. */
export function applyThemeVars(root: HTMLElement, vars: Record<string, string>): void {
  for (const name of written) {
    if (!(name in vars)) root.style.removeProperty(name);
  }
  written.clear();
  for (const [name, value] of Object.entries(vars)) {
    root.style.setProperty(name, value);
    written.add(name);
  }
}

/** Test seam: forget which properties were written. */
export function resetThemeVarsForTest(): void {
  written.clear();
}
