import { describe, expect, it } from "vitest";

import { achievementUnit, formatProgressValue } from "./progress";

describe("achievement progress units", () => {
  it("classifies time, day, and count slugs like the backend", () => {
    for (const slug of [
      "playtime-10h",
      "playtime-1000h",
      "day-5h",
      "session-8h",
      "night-owl",
      "early-bird",
      "route-10h",
    ]) {
      expect(achievementUnit(slug)).toBe("ms");
    }
    for (const slug of [
      "streak-3",
      "streak-100",
      "marathon-10h-5d",
      "streak-2h-7d",
      "comeback",
      "anniversary-1y",
      "journal-streak-7",
    ]) {
      expect(achievementUnit(slug)).toBe("days");
    }
    for (const slug of [
      "first-session",
      "sessions-100",
      "endings-vn-3",
      "tags-10",
      "library-50",
      "shot-first",
    ]) {
      expect(achievementUnit(slug)).toBe("count");
    }
  });

  it("formats milliseconds as compact durations per locale", () => {
    expect(formatProgressValue("playtime-100h", 360_000_000, "en")).toBe("100h");
    expect(formatProgressValue("playtime-100h", 360_000_000, "id")).toBe("100 j");
    expect(formatProgressValue("session-4h", 5_400_000, "en")).toBe("1h 30m");
    expect(formatProgressValue("session-4h", 5_400_000, "id")).toBe("1 j 30 mnt");
  });

  it("leaves day and count progress as plain integers", () => {
    expect(formatProgressValue("streak-7", 3, "en")).toBe("3");
    expect(formatProgressValue("sessions-100", 40, "id")).toBe("40");
  });
});