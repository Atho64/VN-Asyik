import { describe, expect, it } from "vitest";

import { parseRating } from "./RatingPicker";

describe("parseRating", () => {
  it("reads a whole number as its star value", () => {
    expect(parseRating("7")).toEqual({ kind: "value", decivotes: 70 });
    expect(parseRating("10")).toEqual({ kind: "value", decivotes: 100 });
    expect(parseRating("1")).toEqual({ kind: "value", decivotes: 10 });
  });

  it("reads one decimal place", () => {
    expect(parseRating("7.5")).toEqual({ kind: "value", decivotes: 75 });
    expect(parseRating("7.3")).toEqual({ kind: "value", decivotes: 73 });
    expect(parseRating("10.0")).toEqual({ kind: "value", decivotes: 100 });
  });

  it("accepts a comma, because some keyboards only offer one", () => {
    expect(parseRating("7,5")).toEqual({ kind: "value", decivotes: 75 });
  });

  it("trims surrounding space", () => {
    expect(parseRating("  8.5 ")).toEqual({ kind: "value", decivotes: 85 });
  });

  it("treats empty as no rating, which is different from a bad one", () => {
    // Empty clears the rating; invalid must not, or a typo would silently
    // delete a rating the user had already set.
    expect(parseRating("")).toEqual({ kind: "empty" });
    expect(parseRating("   ")).toEqual({ kind: "empty" });
  });

  it("rejects values outside the scale", () => {
    expect(parseRating("0")).toEqual({ kind: "bad" });
    expect(parseRating("0.9")).toEqual({ kind: "bad" });
    expect(parseRating("10.1")).toEqual({ kind: "bad" });
    expect(parseRating("11")).toEqual({ kind: "bad" });
    expect(parseRating("99")).toEqual({ kind: "bad" });
  });

  it("rejects more than one decimal place", () => {
    expect(parseRating("7.55")).toEqual({ kind: "bad" });
  });

  it("rejects anything that is not a number", () => {
    for (const text of ["abc", "7.", ".5", "-7", "1e1", "7 5", "+7"]) {
      expect(parseRating(text), text).toEqual({ kind: "bad" });
    }
  });
});
