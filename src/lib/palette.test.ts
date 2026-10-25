import { describe, expect, it } from "vitest";

import {
  clampForContrast,
  clampForContrastDark,
  contrastRatio,
  extractPalette,
  fallbackPalette,
  fromHex,
  hashHue,
  luminance,
  mix,
  toHex,
  type Rgb,
} from "./palette";

const SURFACE: Rgb = { r: 11, g: 12, b: 16 };

describe("contrast maths", () => {
  it("computes WCAG relative luminance for known values", () => {
    expect(luminance({ r: 0, g: 0, b: 0 })).toBeCloseTo(0, 5);
    expect(luminance({ r: 255, g: 255, b: 255 })).toBeCloseTo(1, 5);
  });

  it("gives 21:1 for black on white", () => {
    expect(
      contrastRatio({ r: 0, g: 0, b: 0 }, { r: 255, g: 255, b: 255 }),
    ).toBeCloseTo(21, 1);
  });

  it("is symmetric", () => {
    const a: Rgb = { r: 10, g: 120, b: 200 };
    const b: Rgb = { r: 240, g: 240, b: 240 };
    expect(contrastRatio(a, b)).toBeCloseTo(contrastRatio(b, a), 10);
  });

  it("flags the same colour as 1:1", () => {
    expect(contrastRatio(SURFACE, SURFACE)).toBeCloseTo(1, 5);
  });
});

describe("colour helpers", () => {
  it("round-trips hex", () => {
    const colour: Rgb = { r: 139, g: 156, b: 255 };
    expect(fromHex(toHex(colour))).toEqual(colour);
  });

  it("clamps hex channels into range", () => {
    expect(toHex({ r: -5, g: 300, b: 128 })).toBe("#00ff80");
  });

  it("rejects malformed hex", () => {
    expect(fromHex("nope")).toBeNull();
    expect(fromHex("#fff")).toBeNull();
  });

  it("mixes toward the target", () => {
    expect(mix({ r: 0, g: 0, b: 0 }, { r: 100, g: 100, b: 100 }, 0.5)).toEqual({
      r: 50,
      g: 50,
      b: 50,
    });
  });
});

describe("contrast clamping", () => {
  it("lifts a colour until it clears AA on the dark surface", () => {
    const dim: Rgb = { r: 30, g: 20, b: 60 };
    const clamped = clampForContrast(dim, SURFACE);
    expect(contrastRatio(clamped, SURFACE)).toBeGreaterThanOrEqual(4.5);
  });

  it("darkens for a light surface instead of washing out", () => {
    const pale: Rgb = { r: 240, g: 230, b: 200 };
    const light: Rgb = { r: 250, g: 250, b: 252 };
    const clamped = clampForContrastDark(pale, light);
    expect(contrastRatio(clamped, light)).toBeGreaterThanOrEqual(4.5);
  });

  it("leaves an already compliant colour alone", () => {
    const ok: Rgb = { r: 200, g: 220, b: 255 };
    expect(clampForContrast(ok, SURFACE)).toEqual(ok);
  });
});

describe("extractPalette", () => {
  const makePixels = (colours: Array<[number, number, number, number]>): Uint8ClampedArray => {
    const data = new Uint8ClampedArray(colours.length * 4);
    colours.forEach(([r, g, b, a], index) => {
      data[index * 4] = r;
      data[index * 4 + 1] = g;
      data[index * 4 + 2] = b;
      data[index * 4 + 3] = a;
    });
    return data;
  };

  it("returns a usable palette for an empty image", () => {
    const palette = extractPalette(new Uint8ClampedArray(0), 0, 0);
    expect(palette.accent).toMatch(/^#[0-9a-f]{6}$/);
    expect(contrastRatio(fromHex(palette.accent) as Rgb, SURFACE)).toBeGreaterThanOrEqual(
      4.5,
    );
  });

  it("ignores fully transparent pixels", () => {
    const pixels = makePixels([[255, 0, 0, 0]]);
    const palette = extractPalette(pixels, 1, 1);
    expect(palette.accent).toMatch(/^#[0-9a-f]{6}$/);
  });

  it("ignores near-white and near-black so the accent keeps a hue", () => {
    const pixels = makePixels([
      [252, 252, 252, 255],
      [250, 251, 253, 255],
      [4, 4, 6, 255],
      [2, 3, 5, 255],
      [200, 40, 60, 255],
    ]);
    const palette = extractPalette(pixels, 5, 1);
    const rgb = fromHex(palette.accent);
    expect(rgb).not.toBeNull();
    const parsed = rgb as Rgb;
    // The only surviving hue is reddish, so red must dominate.
    expect(parsed.r).toBeGreaterThan(parsed.b);
  });

  it("picks the saturated colour over the merely common one", () => {
    // A large muted grey block plus a smaller vivid blue.
    const grey: [number, number, number, number] = [120, 121, 124, 255];
    const blue: [number, number, number, number] = [30, 90, 230, 255];
    const pixels = makePixels([grey, grey, grey, grey, grey, grey, grey, blue, blue]);
    const palette = extractPalette(pixels, 9, 1);
    const parsed = fromHex(palette.accent) as Rgb;
    expect(parsed.b).toBeGreaterThan(parsed.r);
  });

  it("always returns an accent with AA contrast against the app background", () => {
    const pixels = makePixels(Array.from({ length: 64 }, () => [18, 16, 40, 255]));
    const palette = extractPalette(
      pixels,
      8,
      8,
    );
    expect(
      contrastRatio(fromHex(palette.accent) as Rgb, SURFACE),
    ).toBeGreaterThanOrEqual(4.5);
  });
});

describe("fallbackPalette", () => {
  it("is stable for the same title", () => {
    expect(fallbackPalette("Symphonic Rain")).toEqual(fallbackPalette("Symphonic Rain"));
  });

  it("differs between titles", () => {
    expect(fallbackPalette("Umineko").accent).not.toBe(
      fallbackPalette("Higurashi").accent,
    );
  });

  it("keeps the accent readable on the dark surface", () => {
    for (const title of ["A", "うみねこのなく頃に", "TrymenT", "D.C.5 Future Link"]) {
      const rgb = fromHex(fallbackPalette(title).accent) as Rgb;
      expect(contrastRatio(rgb, SURFACE)).toBeGreaterThanOrEqual(4.5);
    }
  });

  it("hashes deterministically within the hue circle", () => {
    expect(hashHue("abc")).toBe(hashHue("abc"));
    expect(hashHue("abc")).toBeGreaterThanOrEqual(0);
    expect(hashHue("abc")).toBeLessThan(360);
  });

  it("handles Japanese titles without collapsing", () => {
    const a = hashHue("リトルバスターズ");
    const b = hashHue("リトルバスターズ！");
    expect(a).not.toBe(b);
  });
});
