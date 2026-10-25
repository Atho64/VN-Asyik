import { describe, expect, it } from "vitest";

import { alternateTitles } from "./alternateTitles";
import type { TitleVariant } from "@/lib/ipc/types";

const variant = (lang: string, title: string, latin: string | null = null): TitleVariant => ({
  lang,
  title,
  latin,
  official: null,
  main: false,
});

describe("alternate titles", () => {
  it("keeps only Japanese and English", () => {
    // VNDB publishes Russian, Chinese, Korean, German and more. A row of five
    // scripts under the title is noise when the reader wants to match a name.
    const shown = alternateTitles("Umineko When They Cry - Question Arcs", null, [
      variant("en", "Umineko When They Cry - Question Arcs"),
      variant("ja", "うみねこのなく頃に"),
      variant("ru", "Когда плачут чайки"),
      variant("zh-Hans", "海猫鸣泣之时"),
    ]);
    expect(shown).toEqual(["うみねこのなく頃に"]);
  });

  it("drops the language code it cannot display rather than guessing", () => {
    // `lang` is nullable in the DTO. Treating null as "probably Japanese" would
    // show a title in a script the filter was meant to exclude.
    expect(alternateTitles("Seed", null, [variant("ru", "Сиид"), { ...variant("", "x"), lang: null }])).toEqual(
      [],
    );
  });

  it("puts the original script before the translation", () => {
    // The Japanese name is the identity: it is what the cover and the save files
    // say. English is the convenience, not the primary.
    const shown = alternateTitles("Seed", null, [
      variant("en", "Sabbat of the Witch"),
      variant("ja", "サノバウィッチ"),
    ]);
    expect(shown).toEqual(["サノバウィッチ", "Sabbat of the Witch"]);
  });

  it("appends aliases after the titles", () => {
    // Aliases are the short names people use; they are not titles and carry no
    // language, so they come last rather than being interleaved.
    const shown = alternateTitles(
      "Umineko When They Cry - Question Arcs",
      null,
      [variant("ja", "うみねこのなく頃に")],
      ["When They Cry 3", "When The Seagulls Cry"],
    );
    expect(shown).toEqual([
      "うみねこのなく頃に",
      "When They Cry 3",
      "When The Seagulls Cry",
    ]);
  });

  it("never repeats the main title", () => {
    // CLANNAD stores "CLANNAD" as its ja, en and zh-Hans title. Printing it three
    // times under the heading is what made the old header look broken.
    const shown = alternateTitles("CLANNAD", null, [
      variant("ja", "CLANNAD"),
      variant("en", "CLANNAD"),
      variant("zh-Hans", "CLANNAD"),
    ]);
    expect(shown).toEqual([]);
  });

  it("uses a romanised form when there is no English title", () => {
    // A Japanese-only release has no `en` entry, so the latin form is the only
    // Latin name that exists.
    const shown = alternateTitles("Seed", null, [
      variant("ja", "この大空に、翼をひろげて", "Kono Oozora ni, Tsubasa wo Hirogete"),
    ]);
    expect(shown).toEqual([
      "この大空に、翼をひろげて",
      "Kono Oozora ni, Tsubasa wo Hirogete",
    ]);
  });

  it("is case-insensitive about duplicates", () => {
    const shown = alternateTitles("Sanoba Witch", null, [], ["sanoba witch"]);
    expect(shown).toEqual([]);
  });

  it("shows every name rather than truncating the list", () => {
    // A cap hides a real name to save vertical space, and the page has room: the
    // row wraps. Showing four of six is worse than showing none of them, because
    // the reader cannot tell a name was dropped.
    const aliases = ["a", "b", "c", "d", "e", "f", "g"];
    const shown = alternateTitles("Seed", null, [], aliases);
    expect(shown).toEqual(aliases);
  });
});
