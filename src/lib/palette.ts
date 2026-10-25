/**
 * Accent extraction from cover art.
 *
 * This is the identity motif made operational: the cover is the light source, so
 * the accent has to come from the artwork, never from a fixed brand colour.
 *
 * The algorithm is a coarse median-cut over a heavily subsampled image, then a
 * contrast clamp. It runs on the main thread over at most a 32x32 thumbnail, so
 * it costs well under a frame; the result is cached in the database by the
 * `accent_color` column, so it is computed once per cover in the app's lifetime.
 */

export interface Rgb {
  r: number;
  g: number;
  b: number;
}

export interface AccentPalette {
  /** Accent used for highlights and the focus ring. Contrast-checked. */
  accent: string;
  /** Softer wash for selected surfaces. */
  accentSoft: string;
  /** Text colour that sits on top of `accent`. */
  onAccent: string;
}

/** Relative luminance per WCAG 2.1. */
export function luminance({ r, g, b }: Rgb): number {
  const channel = (value: number): number => {
    const c = value / 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b);
}

/** WCAG contrast ratio between two colours, 1..21. */
export function contrastRatio(a: Rgb, b: Rgb): number {
  const la = luminance(a);
  const lb = luminance(b);
  const lighter = Math.max(la, lb);
  const darker = Math.min(la, lb);
  return (lighter + 0.05) / (darker + 0.05);
}

export function toHex({ r, g, b }: Rgb): string {
  const hex = (v: number): string =>
    Math.max(0, Math.min(255, Math.round(v)))
      .toString(16)
      .padStart(2, "0");
  return `#${hex(r)}${hex(g)}${hex(b)}`;
}

export function fromHex(hex: string): Rgb | null {
  const match = /^#?([0-9a-f]{6})$/i.exec(hex.trim());
  if (!match) return null;
  const value = match[1];
  if (!value) return null;
  return {
    r: Number.parseInt(value.slice(0, 2), 16),
    g: Number.parseInt(value.slice(2, 4), 16),
    b: Number.parseInt(value.slice(4, 6), 16),
  };
}

/** Mix two colours; `amount` is how much of `b` to take, 0..1. */
export function mix(a: Rgb, b: Rgb, amount: number): Rgb {
  const t = Math.max(0, Math.min(1, amount));
  return {
    r: a.r + (b.r - a.r) * t,
    g: a.g + (b.g - a.g) * t,
    b: a.b + (b.b - a.b) * t,
  };
}

/** Raise lightness until the colour clears `target` contrast against `against`. */
export function clampForContrast(colour: Rgb, against: Rgb, target = 4.5): Rgb {
  const white: Rgb = { r: 255, g: 255, b: 255 };
  let candidate = colour;
  for (let step = 0; step <= 20; step += 1) {
    if (contrastRatio(candidate, against) >= target) return candidate;
    candidate = mix(candidate, white, 0.12);
  }
  return white;
}

/** Darken until the colour clears `target` contrast against `against`. */
export function clampForContrastDark(colour: Rgb, against: Rgb, target = 4.5): Rgb {
  const black: Rgb = { r: 0, g: 0, b: 0 };
  let candidate = colour;
  for (let step = 0; step <= 20; step += 1) {
    if (contrastRatio(candidate, against) >= target) return candidate;
    candidate = mix(candidate, black, 0.12);
  }
  return black;
}

/** Stable hash so a title always yields the same fallback hue. */
export function hashHue(text: string): number {
  let hash = 0;
  for (const char of text) {
    hash = (hash * 31 + (char.codePointAt(0) ?? 0)) % 360;
  }
  return hash;
}

/**
 * Palette used when a game has no cover art.
 *
 * Derived from the title hash so every game still gets its own colour and the
 * same game always looks the same. This is the "never show a broken image" rule
 * from DESIGN.md §2, not a decorative gradient.
 */
export function fallbackPalette(title: string): AccentPalette {
  const hue = hashHue(title);
  const base = hslToRgb(hue, 0.46, 0.62);
  const surface: Rgb = { r: 11, g: 12, b: 16 };
  const accent = clampForContrast(base, surface);
  return {
    accent: toHex(accent),
    accentSoft: `${toHex(accent)}2e`,
    onAccent: contrastRatio(accent, { r: 11, g: 12, b: 16 }) > 4.5 ? "#0b0c10" : "#ffffff",
  };
}

function hslToRgb(h: number, s: number, l: number): Rgb {
  const c = (1 - Math.abs(2 * l - 1)) * s;
  const x = c * (1 - Math.abs(((h / 60) % 2) - 1));
  const m = l - c / 2;
  const [r, g, b] =
    h < 60
      ? [c, x, 0]
      : h < 120
        ? [x, c, 0]
        : h < 180
          ? [0, c, x]
          : h < 240
            ? [0, x, c]
            : h < 300
              ? [x, 0, c]
              : [c, 0, x];
  return { r: (r + m) * 255, g: (g + m) * 255, b: (b + m) * 255 };
}

/**
 * Reduce a pixel buffer to a background colour and an accent.
 *
 * `pixels` is RGBA, row-major, `width * height * 4` long. Fully transparent and
 * near-white/near-black pixels are ignored: white would produce a washed accent
 * and black a colourless one, which is exactly the "safe" result the design
 * direction rejects.
 */
export function extractPalette(
  pixels: Uint8ClampedArray,
  width: number,
  height: number,
): AccentPalette {
  const buckets = new Map<number, { count: number; r: number; g: number; b: number }>();

  for (let i = 0; i + 3 < pixels.length; i += 4) {
    const a = pixels[i + 3] ?? 0;
    if (a < 128) continue;
    const r = pixels[i] ?? 0;
    const g = pixels[i + 1] ?? 0;
    const b = pixels[i + 2] ?? 0;

    const max = Math.max(r, g, b);
    const min = Math.min(r, g, b);
    const isNearWhite = min > 226;
    const isNearBlack = max < 32;
    if (isNearWhite || isNearBlack) continue;

    // 5 bits per channel: enough separation, few enough buckets to be exact.
    const key = ((r >> 3) << 10) | ((g >> 3) << 5) | (b >> 3);
    const bucket = buckets.get(key);
    if (bucket) {
      bucket.count += 1;
      bucket.r += r;
      bucket.g += g;
      bucket.b += b;
    } else {
      buckets.set(key, { count: 1, r, g, b });
    }
  }

  if (buckets.size === 0) {
    return fallbackPalette(`${width}x${height}`);
  }

  const ranked = [...buckets.values()]
    .map((bucket) => ({
      count: bucket.count,
      colour: {
        r: bucket.r / bucket.count,
        g: bucket.g / bucket.count,
        b: bucket.b / bucket.count,
      },
    }))
    .sort((a, b) => b.count - a.count);

  // Accent: the most saturated of the leading buckets. Taking the most common
  // bucket outright gives mud on photographic covers; saturation is what reads
  // as "this game's colour".
  const leading = ranked.slice(0, 12);
  const accentBase = leading.reduce((best, entry) => {
    const score = (colour: Rgb): number => {
      const mx = Math.max(colour.r, colour.g, colour.b);
      const mn = Math.min(colour.r, colour.g, colour.b);
      const saturation = mx === 0 ? 0 : (mx - mn) / mx;
      return saturation * 0.8 + (mx / 255) * 0.2;
    };
    return score(entry.colour) > score(best) ? entry.colour : best;
  }, leading[0]?.colour ?? { r: 139, g: 156, b: 255 });

  const surface: Rgb = { r: 11, g: 12, b: 16 };
  const accent = clampForContrast(accentBase, surface);
  const accentHex = toHex(accent);
  const onAccent =
    contrastRatio({ r: 11, g: 12, b: 16 }, accent) >= 4.5 ? "#0b0c10" : "#ffffff";

  return {
    accent: accentHex,
    accentSoft: `${accentHex}2e`,
    onAccent,
  };
}
