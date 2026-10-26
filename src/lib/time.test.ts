import { describe, expect, it } from "vitest";

import {
  MS_PER_DAY,
  dayIndex,
  formatDate,
  formatDateTime,
  formatDuration,
  formatDurationLong,
  lengthProgress,
  localDateTimeToMs,
  parseTimeInput,
} from "./time";

describe("formatDuration", () => {
  it("renders h:mm with padded minutes", () => {
    expect(formatDuration(0)).toBe("0:00");
    expect(formatDuration(5 * 60_000)).toBe("0:05");
    expect(formatDuration(90 * 60_000)).toBe("1:30");
    expect(formatDuration(10 * 3_600_000)).toBe("10:00");
  });

  it("shows a sub-minute session instead of rounding it to nothing", () => {
    // The tracker writes ~4 s slices, so rows between 1 s and 59 s exist and are
    // real playtime. They used to print 0:00.
    expect(formatDuration(4_000)).toBe("0:04");
    expect(formatDuration(1_057)).toBe("0:01");
    expect(formatDuration(59_999)).toBe("0:59");
    // Exactly one minute is not sub-minute.
    expect(formatDuration(60_000)).toBe("0:01");
  });

  it("treats negative and invalid input as zero", () => {
    expect(formatDuration(-1)).toBe("0:00");
    expect(formatDuration(Number.NaN)).toBe("0:00");
  });
});

describe("formatDurationLong", () => {
  it("drops the hour unit below an hour", () => {
    expect(formatDurationLong(5 * 60_000)).toBe("5 min");
    expect(formatDurationLong(60 * 60_000)).toBe("1h");
    expect(formatDurationLong(90 * 60_000)).toBe("1h 30m");
  });

  it("speaks Indonesian when asked", () => {
    expect(formatDurationLong(5 * 60_000, "id")).toBe("5 menit");
    expect(formatDurationLong(60 * 60_000, "id")).toBe("1jam");
  });

  it("reports real sub-minute sessions in seconds, not zero minutes", () => {
    // The bug behind "0 menit" rows next to real timestamps: counted time below a
    // minute floored to 0. It must say how long the session was.
    expect(formatDurationLong(4_000)).toBe("4 s");
    expect(formatDurationLong(1_057)).toBe("1 s");
    expect(formatDurationLong(4_000, "id")).toBe("4 detik");
    // A true zero still reads as nothing played.
    expect(formatDurationLong(0)).toBe("0 min");
  });

  it("does not misreport exactly one minute as seconds", () => {
    expect(formatDurationLong(60_000)).toBe("1 min");
    expect(formatDurationLong(120_000, "id")).toBe("2 menit");
  });
});

describe("lengthProgress", () => {
  it("is null when the length is unknown", () => {
    expect(lengthProgress(3_600_000, null)).toBeNull();
    expect(lengthProgress(3_600_000, 0)).toBeNull();
  });

  it("computes a ratio and clamps at one", () => {
    expect(lengthProgress(30 * 60_000, 60)).toBeCloseTo(0.5, 5);
    expect(lengthProgress(120 * 60_000, 60)).toBe(1);
  });

  it("never goes negative", () => {
    expect(lengthProgress(-100, 60)).toBe(0);
  });
});

describe("dates", () => {
  it("formats a date without throwing on odd input", () => {
    expect(formatDate(Number.NaN)).toBe("-");
    expect(formatDate(0, "en")).toMatch(/\d{4}/);
  });

  it("formats a date and time", () => {
    expect(formatDateTime(1_700_000_000_000, "en")).toMatch(/\d{2}:\d{2}/);
  });

  it("uses UTC day indexes", () => {
    expect(dayIndex(0)).toBe(0);
    expect(dayIndex(MS_PER_DAY)).toBe(1);
    expect(dayIndex(MS_PER_DAY - 1)).toBe(0);
  });
});

describe("time input", () => {
  it("parses HH:MM", () => {
    expect(parseTimeInput("09:05")).toEqual({ hours: 9, minutes: 5 });
    expect(parseTimeInput("23:59")).toEqual({ hours: 23, minutes: 59 });
  });

  it("rejects invalid values", () => {
    expect(parseTimeInput("24:00")).toBeNull();
    expect(parseTimeInput("9pm")).toBeNull();
    expect(parseTimeInput("")).toBeNull();
  });

  it("builds a local timestamp from date and time", () => {
    const ms = localDateTimeToMs("2026-03-14", "20:30");
    expect(ms).not.toBeNull();
    const date = new Date(ms as number);
    expect(date.getFullYear()).toBe(2026);
    expect(date.getMonth()).toBe(2);
    expect(date.getDate()).toBe(14);
    expect(date.getHours()).toBe(20);
    expect(date.getMinutes()).toBe(30);
  });

  it("returns null for a malformed date", () => {
    expect(localDateTimeToMs("14-03-2026", "20:30")).toBeNull();
    expect(localDateTimeToMs("2026-03-14", "nope")).toBeNull();
  });
});
