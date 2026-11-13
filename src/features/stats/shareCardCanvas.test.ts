import { describe, expect, it } from "vitest";

import type { YearReview } from "@/lib/ipc/types";
import { MS_PER_DAY } from "@/lib/time";
import {
  SHARE_CARD_SIZES,
  fitText,
  shareCardDay,
  shareCardFilename,
  shareCardMonth,
  shareDuration,
  type ShareCardModel,
} from "./shareCardCanvas";

const review = {
  year: 2024,
  totalMs: 7_200_000,
  busiestMonth: { month: 5, countedMs: 3_600_000 },
  busiestDay: {
    day: Math.floor(Date.UTC(2024, 4, 10) / MS_PER_DAY),
    countedMs: 3_600_000,
  },
} as unknown as YearReview;

const model = {
  filename: "vn-asyik-2024-wrapped",
} as ShareCardModel;

describe("share card helpers", () => {
  it("keeps the post and story sizes distinct", () => {
    expect(SHARE_CARD_SIZES.post).toEqual({ width: 1080, height: 1350 });
    expect(SHARE_CARD_SIZES.story).toEqual({ width: 1080, height: 1920 });
  });

  it("names the file after the year and format", () => {
    expect(shareCardFilename(model, "post")).toBe("vn-asyik-2024-wrapped.png");
    expect(shareCardFilename(model, "story")).toBe(
      "vn-asyik-2024-wrapped-story.png",
    );
  });

  it("uses the compact duration in both locales", () => {
    expect(shareDuration(7_200_000, "en")).toBe("2h");
    expect(shareDuration(7_200_000, "id")).toBe("2 j");
  });

  it("renders the busiest month and local day", () => {
    expect(shareCardMonth(review, "en")).toBe("May");
    expect(shareCardDay(review, "en")).toBe("10 May");
  });

  it("has no month or day for a quiet year", () => {
    const quiet = { ...review, busiestMonth: null, busiestDay: null };
    expect(shareCardMonth(quiet, "en")).toBeNull();
    expect(shareCardDay(quiet, "en")).toBeNull();
  });

  it("truncates a title that does not fit", () => {
    // Ten pixels per character: "abc…" is 40, "abcd…" is 50, so 45 keeps three.
    const ctx = {
      measureText: (text: string) => ({ width: text.length * 10 }),
    } as unknown as CanvasRenderingContext2D;
    expect(fitText(ctx, "abcdef", 45)).toBe("abc…");
    expect(fitText(ctx, "abc", 45)).toBe("abc");
  });
});
