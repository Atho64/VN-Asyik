import { describe, expect, it } from "vitest";
import {
  MAX_SAFE_SPOILER,
  formatBirthday,
  formatBloodType,
  groupTraits,
  isSpoilerCharacter,
  isTraitVisible,
  sortSpoilerLast,
} from "./traitFilters";
import type { CharacterTrait, VndbCharacter } from "@/lib/ipc/types";

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

function person(overrides: Partial<VndbCharacter>): VndbCharacter {
  return {
    id: "c1",
    name: "Someone",
    original: null,
    role: "side",
    spoiler: null,
    image: null,
    traits: [],
    birthday: null,
    height: null,
    weight: null,
    blood_type: null,
    voiced_by: null,
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

  it("treats a null character spoiler as safe", () => {
    // Old cached rows predate the field; VNDB omits the field when untagged.
    expect(isSpoilerCharacter(person({}))).toBe(false);
    expect(isSpoilerCharacter(person({ spoiler: 0 }))).toBe(false);
    expect(isSpoilerCharacter(person({ spoiler: 1 }))).toBe(true);
    expect(isSpoilerCharacter(person({ spoiler: 2 }))).toBe(true);
  });

  it("moves spoiler characters last without re-sorting the safe ones", () => {
    // VNDB's role order is meaningful; the only change is the split.
    const ordered = sortSpoilerLast([
      person({ id: "a", name: "Side" }),
      person({ id: "b", name: "Secret", spoiler: 2 }),
      person({ id: "c", name: "Main" }),
      person({ id: "d", name: "Mild", spoiler: 1 }),
    ]);
    expect(ordered.map((c) => c.name)).toEqual(["Side", "Main", "Secret", "Mild"]);
  });

  it("keeps an all-spoiler cast in its original order", () => {
    const cast = [
      person({ id: "a", name: "First", spoiler: 1 }),
      person({ id: "b", name: "Second", spoiler: 2 }),
    ];
    expect(sortSpoilerLast(cast).map((c) => c.name)).toEqual(["First", "Second"]);
  });
});
