import { describe, expect, it } from "vitest";
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";

import { en } from "./en";
import { createTranslator, resolveLocale, translate } from "./index";
import { id } from "./id";

/** Repository source root, resolved from this test file's location. */
const SRC = join(import.meta.dirname, "..");

describe("locale resolution", () => {
  it("maps regional tags to a base locale", () => {
    expect(resolveLocale("id-ID")).toBe("id");
    expect(resolveLocale("en-GB")).toBe("en");
    expect(resolveLocale("ID")).toBe("id");
  });

  it("falls back to English for unknown or missing tags", () => {
    expect(resolveLocale("fr")).toBe("en");
    expect(resolveLocale(null)).toBe("en");
    expect(resolveLocale(undefined)).toBe("en");
    expect(resolveLocale("")).toBe("en");
  });
});

describe("dictionary completeness", () => {
  it("has an Indonesian entry for every English key", () => {
    const missing = Object.keys(en).filter(
      (key) => !(key in (id as Record<string, string>)),
    );
    expect(missing).toEqual([]);
  });

  it("has no Indonesian key that English lacks", () => {
    const extra = Object.keys(id).filter(
      (key) => !(key in (en as Record<string, string>)),
    );
    expect(extra).toEqual([]);
  });

  it("never ships an empty string", () => {
    for (const [key, value] of Object.entries(en)) {
      expect(value.trim().length, `en:${key}`).toBeGreaterThan(0);
    }
    for (const [key, value] of Object.entries(id)) {
      expect(value.trim().length, `id:${key}`).toBeGreaterThan(0);
    }
  });

  /**
   * Every `t("literal.key")` in the source must exist in the dictionary.
   *
   * The two parity tests above only compare the locales with each other, so a key
   * that is used in code but defined in neither shipped as the raw key on screen:
   * the token sheet rendered a button labelled `common.cancel` beside "Save Discord
   * Token". `translate` returns the key on a miss by design (a visible bug beats a
   * blank string), and `t` accepts `MessageKey | string`, so neither the runtime
   * nor the type checker caught it. This is the check that does.
   *
   * Only literal first arguments are scanned. Dynamic keys such as
   * `t(\`status.${game.status}\`)` cannot be resolved statically, so the enum-shaped
   * families are listed explicitly below instead of being silently skipped.
   */
  it("defines every key the source asks for", () => {
    const files = readdirSync(SRC, { recursive: true, encoding: "utf8" })
      .filter((name) => /\.(ts|tsx)$/.test(name) && !name.includes(".test."))
      .map((name) => join(SRC, name));

    const used = new Set<string>();
    const literal = /\bt\(\s*["'`]([a-zA-Z][\w.]*)["'`]/g;
    for (const file of files) {
      const source = readFileSync(file, "utf8");
      for (const match of source.matchAll(literal)) {
        const key = match[1];
        if (!key) continue;
        // A template key with an interpolation is dynamic; skip it.
        if (key.includes("${")) continue;
        used.add(key);
      }
    }

    expect(used.size, "the scan must find keys, or it proves nothing").toBeGreaterThan(200);

    // A key may be defined as a plural family (`x_one` / `x_other`) rather than
    // literally, because `t("x", { count })` resolves the suffix at call time.
    const dict = en as Record<string, string>;
    const defined = (key: string): boolean =>
      key in dict || `${key}_one` in dict || `${key}_other` in dict;

    const unknown = [...used].filter((key) => !defined(key));
    expect(unknown).toEqual([]);
  });

  it("defines every plural form a scanned key needs", () => {
    // `t("library.count", { count })` resolves to `library.count_one` /
    // `library.count_other`, and a key can exist in the singular alone, which
    // renders the raw suffixed key for every count above one.
    const pluralBases = Object.keys(en)
      .filter((key) => key.endsWith("_one"))
      .map((key) => key.slice(0, -"_one".length));
    const broken = pluralBases.filter(
      (base) => !(`${base}_other` in (en as Record<string, string>)),
    );
    expect(broken).toEqual([]);
  });
});

describe("interpolation", () => {
  it("substitutes named variables", () => {
    expect(translate("en", "game.rating", { rating: 9 })).toBe("Rated 9 of 10");
  });

  it("leaves unknown placeholders visible rather than blank", () => {
    expect(translate("en", "game.played", {})).toBe("{{time}} played");
  });

  it("keeps a literal dollar-free template intact", () => {
    expect(translate("en", "app.name")).toBe("VN Asyik");
  });
});

describe("plurals", () => {
  it("selects the singular form for one", () => {
    expect(translate("en", "library.count", { count: 1 })).toBe("1 game");
  });

  it("selects the plural form for zero and many", () => {
    expect(translate("en", "library.count", { count: 0 })).toBe("0 games");
    expect(translate("en", "library.count", { count: 42 })).toBe("42 games");
  });

  it("plurals work in Indonesian too", () => {
    expect(translate("id", "library.count", { count: 1 })).toBe("1 game");
    expect(translate("id", "library.count", { count: 3 })).toBe("3 game");
  });
});

describe("fallbacks", () => {
  it("returns the key when it is unknown, so the bug is visible", () => {
    expect(translate("en", "does.not.exist")).toBe("does.not.exist");
  });

  it("falls back to English when a locale is unknown", () => {
    // @ts-expect-error deliberately invalid locale
    expect(translate("de", "nav.library")).toBe("Library");
  });
});

describe("translator", () => {
  it("binds the locale", () => {
    const t = createTranslator("id");
    expect(t("nav.library")).toBe("Koleksi");
  });
});

describe("copy hygiene", () => {
  it("contains no em dashes in either locale", () => {
    for (const [key, value] of Object.entries(en)) {
      expect(value.includes("\u2014"), `en:${key}`).toBe(false);
    }
    for (const [key, value] of Object.entries(id)) {
      expect(value.includes("\u2014"), `id:${key}`).toBe(false);
    }
  });

  it("contains no emoji", () => {
    const emoji = /\p{Extended_Pictographic}/u;
    for (const [key, value] of Object.entries(en)) {
      expect(emoji.test(value), `en:${key}`).toBe(false);
    }
  });
});
