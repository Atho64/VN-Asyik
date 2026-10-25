import { describe, expect, it } from "vitest";
import { titledNames } from "./alternateTitles";
import type { TitleVariant } from "@/lib/ipc/types";

const variant = (
  lang: string,
  title: string,
  latin: string | null = null,
): TitleVariant => ({
  lang,
  title,
  latin,
  official: null,
  main: null,
});

describe("titledNames", () => {
  it("separates English, romaji and Japanese when all three differ", () => {
    const shown = titledNames("Umineko no Naku Koro ni", null, [
      variant("en", "Umineko When They Cry - Question Arcs"),
      variant("ja", "うみねこのなく頃に", "Umineko no Naku Koro ni"),
    ]);
    expect(shown).toEqual([
      { role: "english", text: "Umineko When They Cry - Question Arcs" },
      { role: "japanese", text: "うみねこのなく頃に" },
    ]);
  });

  it("keeps the romaji when the shown title is something else entirely", () => {
    // The reported case: a Japanese-only release whose library title came from a
    // fan translation, so the romaji is the only usable Latin form.
    const shown = titledNames("Tomoyo After", "智代アフター", [
      variant("ja", "智代アフター", "Tomoyo After ~It's a Wonderful Life~"),
    ]);
    expect(shown).toEqual([
      { role: "romaji", text: "Tomoyo After ~It's a Wonderful Life~" },
      { role: "japanese", text: "智代アフター" },
    ]);
  });

  it("never repeats the title already shown at the top", () => {
    // CLANNAD stores the same word as ja, en and zh-Hans; the old header printed
    // it up to three times.
    const shown = titledNames("CLANNAD", "CLANNAD", [
      variant("en", "CLANNAD"),
      variant("ja", "CLANNAD", "CLANNAD"),
      variant("ko", "클라나드", "CLANNAD"),
    ]);
    expect(shown).toEqual([]);
  });

  it("labels the library's own original title", () => {
    const shown = titledNames("planetarian", "planetarian ～ちいさなほしのゆめ～", []);
    expect(shown).toEqual([
      { role: "original", text: "planetarian ～ちいさなほしのゆめ～" },
    ]);
  });

  it("drops Korean and Chinese titles rather than listing every localisation", () => {
    const shown = titledNames("Seed", null, [
      variant("zh-Hans", "种子"),
      variant("ko", "씨앗"),
      variant("ru", "Сиид"),
    ]);
    expect(shown).toEqual([]);
  });

  it("appends a Latin alias last, labelled as an alias", () => {
    const shown = titledNames("Umineko", null, [], ["When They Cry 3"]);
    expect(shown).toEqual([{ role: "alias", text: "When They Cry 3" }]);
  });

  it("labels a Chinese alias as Chinese, not as a second 'also called'", () => {
    // CLANNAD lists both クラナド and 团子大家族 as aliases with no language
    // field, so the header showed two identical "Also called" rows in different
    // scripts. The script is detected instead.
    const shown = titledNames("CLANNAD", null, [], ["クラナド", "团子大家族"]);
    expect(shown).toEqual([
      { role: "japanese", text: "クラナド" },
      { role: "chinese", text: "团子大家族" },
    ]);
  });

  it("labels a Korean alias as Korean", () => {
    const shown = titledNames("Seed", null, [], ["씨앗"]);
    expect(shown).toEqual([{ role: "korean", text: "씨앗" }]);
  });

  it("treats han mixed with Latin as an alias rather than Chinese", () => {
    const shown = titledNames("Seed", null, [], ["CLANNAD 团子"]);
    expect(shown).toEqual([{ role: "alias", text: "CLANNAD 团子" }]);
  });

  it("de-duplicates an alias that repeats another name", () => {
    const shown = titledNames("X", null, [variant("en", "Ex")], ["ex", "Why"]);
    expect(shown).toEqual([
      { role: "english", text: "Ex" },
      { role: "alias", text: "Why" },
    ]);
  });

  it("survives a title list with null languages", () => {
    const shown = titledNames("Seed", null, [
      { ...variant("", "x"), lang: null },
    ]);
    expect(shown).toEqual([]);
  });
});
