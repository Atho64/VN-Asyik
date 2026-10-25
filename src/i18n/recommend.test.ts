import { describe, expect, it } from "vitest";

import { translate } from "@/i18n";

/**
 * The recommendation strings are the first in this app to use `{{count}}` with a
 * real plural pair. Getting that wrong renders the literal `{count}`, which is
 * silent: the text is present, just wrong.
 */
describe("recommendation copy", () => {
  it("interpolates the page count", () => {
    const text = translate("en", "recommend.sources", { count: 8 });
    expect(text).toBe("Pages searched (8)");
    expect(text).not.toContain("{");
  });

  it("uses the singular form for one page", () => {
    expect(translate("en", "recommend.sources", { count: 1 })).toBe("Page searched (1)");
  });

  it("interpolates the credit count", () => {
    const text = translate("en", "recommend.cost", { count: 1 });
    expect(text).toBe("This search used 1 Tavily credit.");
    expect(text).not.toContain("{");
  });

  it("pluralises credits", () => {
    expect(translate("en", "recommend.cost", { count: 2 })).toBe(
      "This search used 2 Tavily credits.",
    );
  });

  it("has the same keys in Indonesian", () => {
    // A missing key falls back to the key name, which is visibly broken.
    for (const key of ["recommend.sources", "recommend.cost"]) {
      const text = translate("id", key, { count: 3 });
      expect(text).not.toBe(key);
      expect(text).not.toContain("{");
    }
  });
});
