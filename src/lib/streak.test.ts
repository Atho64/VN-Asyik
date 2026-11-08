import { describe, expect, it } from "vitest";

import { currentStreak, longestStreak } from "./streak";

/** A day list from `[dayIndex, ms]` tuples; 0 ms is what an empty day stores. */
const days = (...pairs: Array<[number, number]>): Array<[number, number]> => pairs;

describe("currentStreak", () => {
  it("counts consecutive days up to today", () => {
    const list = days([10, 100], [11, 200], [12, 300]);
    expect(currentStreak(list, 12)).toBe(3);
  });

  it("survives until the day is over: reading yesterday only is still a streak", () => {
    const list = days([10, 100], [11, 200]);
    expect(currentStreak(list, 12)).toBe(2);
  });

  it("ends at the first missed day", () => {
    const list = days([8, 100], [9, 100], [10, 0], [12, 100]);
    expect(currentStreak(list, 12)).toBe(1);
  });

  it("is zero when the last two days have no time", () => {
    const list = days([9, 100]);
    expect(currentStreak(list, 12)).toBe(0);
  });

  it("is zero for an empty history", () => {
    expect(currentStreak([], 12)).toBe(0);
  });

  it("ignores days that carry a zero bucket", () => {
    const list = days([10, 0], [11, 50], [12, 60]);
    expect(currentStreak(list, 12)).toBe(2);
  });
});

describe("longestStreak", () => {
  it("finds the longest run anywhere in the history", () => {
    const list = days([1, 10], [2, 10], [3, 10], [7, 10], [8, 10], [20, 10]);
    expect(longestStreak(list)).toBe(3);
  });

  it("is zero for an empty history", () => {
    expect(longestStreak([])).toBe(0);
  });

  it("counts a single day as a one-day streak", () => {
    expect(longestStreak(days([5, 10]))).toBe(1);
  });

  it("covers the whole history, not just the heatmap window", () => {
    const list = days([0, 10], [1, 10], [2, 10], [3, 10], [500, 10]);
    expect(longestStreak(list)).toBe(4);
  });

  it("survives unsorted input", () => {
    const list = days([3, 10], [1, 10], [2, 10]);
    expect(longestStreak(list)).toBe(3);
  });
});
