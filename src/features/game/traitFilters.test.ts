import { describe, expect, it } from "vitest";
import {
  MAX_SAFE_SPOILER,
  formatBirthday,
  formatBloodType,
  groupTraits,
  isTraitVisible,
} from "./traitFilters";
import type { CharacterTrait } from "@/lib/ipc/types";

function trait(overrides: Partial<CharacterTrait>): CharacterTrait {
  return {
    id: "i1",
    name: "Blue",
    group_name: "Hair",
    spoiler: 0,
    sexual: false,
    lie: false,
    ...overrides,
  };
}

describe("traitFilters", () => {
  it("shows a safe trait and withholds a spoiler one at the default ceiling", () => {
    expect(isTraitVisible(trait({ spoiler: 0 }), MAX_SAFE_SPOILER)).toBe(true);
    expect(isTraitVisible(trait({ spoiler: 1 }), MAX_SAFE_SPOILER)).toBe(false);
    expect(isTraitVisible(trait({ spoiler: 2 }), MAX_SAFE_SPOILER)).toBe(false);
  });

  it("treats a missing spoiler level as safe", () => {
    // VNDB omits the field rather than sending 0. Treating absent as unsafe
    // would hide most of a character's hair and eyes.
    expect(isTraitVisible(trait({ spoiler: null }), MAX_SAFE_SPOILER)).toBe(true);
  });

  it("groups traits in the order VNDB emits them", () => {
    const groups = groupTraits([
      trait({ group_name: "Hair", name: "Blue" }),
      trait({ group_name: "Eyes", name: "Red" }),
      trait({ group_name: "Hair", name: "Twin Tails" }),
    ]);
    expect(groups).toEqual([
      ["Hair", ["Blue", "Twin Tails"]],
      ["Eyes", ["Red"]],
    ]);
  });

  it("excludes spoiler traits from the default grouping", () => {
    const groups = groupTraits([
      trait({ group_name: "Hair", name: "Blue", spoiler: 0 }),
      trait({ group_name: "Subject of", name: "Murder", spoiler: 2 }),
    ]);
    expect(groups).toEqual([["Hair", ["Blue"]]]);
  });

  it("includes them once the ceiling is raised", () => {
    const groups = groupTraits(
      [
        trait({ group_name: "Hair", name: "Blue", spoiler: 0 }),
        trait({ group_name: "Subject of", name: "Murder", spoiler: 2 }),
      ],
      2,
    );
    expect(groups).toEqual([
      ["Hair", ["Blue"]],
      ["Subject of", ["Murder"]],
    ]);
  });

  it("keeps an ungrouped trait instead of dropping it", () => {
    // A null group must not silently vanish: it is information the reader paid
    // a request for.
    expect(groupTraits([trait({ group_name: null, name: "Mysterious" })])).toEqual([
      ["Other", ["Mysterious"]],
    ]);
  });

  it("de-duplicates a repeated trait name inside one group", () => {
    const groups = groupTraits([
      trait({ name: "Blue" }),
      trait({ name: "Blue", id: "i2" }),
    ]);
    expect(groups).toEqual([["Hair", ["Blue"]]]);
  });

  it("formats a birthday in both locales", () => {
    expect(formatBirthday([5, 13], "en")).toBe("May 13");
    expect(formatBirthday([5, 13], "id")).toBe("Mei 13");
    expect(formatBirthday([10, 30], "en")).toBe("October 30");
  });

  it("refuses a birthday that is not a real month and day", () => {
    expect(formatBirthday(null, "en")).toBeNull();
    expect(formatBirthday([0, 5], "en")).toBeNull();
    expect(formatBirthday([13, 5], "en")).toBeNull();
    expect(formatBirthday([5], "en")).toBeNull();
  });

  it("upper-cases a blood type and treats blank as absent", () => {
    expect(formatBloodType("a")).toBe("A");
    expect(formatBloodType(" ab ")).toBe("AB");
    expect(formatBloodType("")).toBeNull();
    expect(formatBloodType(null)).toBeNull();
  });
});
