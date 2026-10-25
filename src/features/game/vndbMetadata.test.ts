import { describe, expect, it } from "vitest";

import { createTranslator } from "@/i18n";
import {
  developerLabel,
  developmentStatus,
  formatReleaseDate,
  languageName,
  platformName,
  rankDevelopers,
  splitExtLinks,
  vndbVnUrl,
} from "@/features/game/vndbMetadata";
import type { VndbExtLink, VndbProducer } from "@/lib/ipc/types";

const t = createTranslator("en");
const tId = createTranslator("id");

// The month formatter the detail screen passes in. Stubbed so a test asserts the
// shape the function produces rather than the machine's locale data.
const monthName = (year: number, month: number): string =>
  `${["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"][month - 1]} ${year}`;

describe("language names", () => {
  it("names a known language instead of showing its code", () => {
    expect(languageName("ja", t)).toBe("Japanese");
    expect(languageName("en", t)).toBe("English");
    expect(languageName("pl", t)).toBe("Polish");
  });

  it("keeps a region distinction, because it changes what release you can buy", () => {
    // Flattening zh-Hans and zh-Hant to "Chinese" would hide a real difference.
    // These have their own names rather than a parenthesised code, which is how a
    // store lists them.
    expect(languageName("zh-Hans", t)).toBe("Simplified Chinese");
    expect(languageName("zh-Hant", t)).toBe("Traditional Chinese");
    expect(languageName("zh-Hans", t)).not.toBe(languageName("zh-Hant", t));
  });

  it("names a region that has its own name instead of showing the code", () => {
    // VNDB sends "pt-br"; "Portuguese (br)" is a database key, not a language.
    expect(languageName("pt-br", t)).toBe("Brazilian Portuguese");
    expect(languageName("pt-BR", t)).toBe("Brazilian Portuguese");
  });

  it("passes an unknown language through rather than inventing a name", () => {
    // VNDB adds languages; a new one must still be readable as itself.
    expect(languageName("eo", t)).toBe("eo");
    expect(languageName("xx-YY", t)).toBe("xx-YY");
  });

  it("still qualifies a region for a language with no named variant", () => {
    expect(languageName("en-GB", t)).toBe("English (GB)");
  });

  it("is translated, not hard-coded English", () => {
    expect(languageName("ja", tId)).toBe("Jepang");
  });

  it("handles an empty tag without throwing", () => {
    expect(languageName("  ", t)).toBe("Unknown");
  });
});

describe("platform names", () => {
  it("names known platform codes", () => {
    expect(platformName("win", t)).toBe("Windows");
    expect(platformName("psp", t)).toBe("PSP");
    expect(platformName("and", t)).toBe("Android");
  });

  it("is case-insensitive about the code VNDB sends", () => {
    expect(platformName("WIN", t)).toBe("Windows");
  });

  it("shows an unknown code verbatim rather than dropping the entry", () => {
    // Hiding it would misreport what the VN runs on; guessing would be worse.
    expect(platformName("win9x", t)).toBe("win9x");
  });
});

describe("development status", () => {
  it("maps the three documented values", () => {
    expect(developmentStatus(0, t)).toBe("Finished");
    expect(developmentStatus(1, t)).toBe("In development");
    expect(developmentStatus(2, t)).toBe("Cancelled");
  });

  it("returns null for unknown or missing values instead of claiming finished", () => {
    // `null` and `undefined` both mean "VNDB did not say". Returning "Finished" for
    // them would be a lie on an unreleased entry.
    expect(developmentStatus(null, t)).toBeNull();
    expect(developmentStatus(7, t)).toBeNull();
  });
});

describe("release dates", () => {
  it("formats a full date verbatim", () => {
    expect(formatReleaseDate("2007-08-17", t, monthName)).toBe("2007-08-17");
  });

  it("formats a month-precision date without inventing a day", () => {
    // Padding to the 1st would claim a day nobody knows.
    expect(formatReleaseDate("2007-08", t, monthName)).toBe("Aug 2007");
  });

  it("keeps a year-only date as a year", () => {
    expect(formatReleaseDate("2007", t, monthName)).toBe("2007");
  });

  it("reports the two documented non-date values honestly", () => {
    expect(formatReleaseDate("TBA", t, monthName)).toBe("Not released");
    expect(formatReleaseDate("unknown", t, monthName)).toBe("Date unknown");
  });

  it("passes an unrecognised shape through instead of blanking it", () => {
    // A month/day out of range is not a date this function understands, so the
    // raw string is shown: blanking it would hide that VNDB said something, and
    // interpreting 13 as a month would invent a date.
    expect(formatReleaseDate("2020-13-45", t, monthName)).toBe("2020-13-45");
    expect(formatReleaseDate("sometime", t, monthName)).toBe("sometime");
  });

  it("returns null when there is no date at all", () => {
    expect(formatReleaseDate(null, t, monthName)).toBeNull();
    expect(formatReleaseDate("   ", t, monthName)).toBeNull();
  });
});

describe("developers", () => {
  const company: VndbProducer = { id: "p1", name: "KID", original: "キッド", type: "co" };
  const amateur: VndbProducer = { id: "p2", name: "Fan Group", original: null, type: "ng" };
  const person: VndbProducer = { id: "p3", name: "A Writer", original: null, type: "in" };
  const nameless: VndbProducer = { id: "p4", name: null, original: "  ", type: "co" };

  it("puts companies before amateur groups and individuals", () => {
    // "Who made this" is the company; a fan group is usually a translation credit.
    expect(rankDevelopers([person, amateur, company]).map((d) => d.id)).toEqual([
      "p1",
      "p2",
      "p3",
    ]);
  });

  it("drops entries with no usable name", () => {
    expect(rankDevelopers([nameless, company]).map((d) => d.id)).toEqual(["p1"]);
  });

  it("shows the romanised name and the original script together", () => {
    expect(developerLabel(company)).toBe("KID · キッド");
  });

  it("does not repeat a name that is the same in both scripts", () => {
    expect(developerLabel({ ...company, original: "KID" })).toBe("KID");
  });

  it("uses the original script when that is all there is", () => {
    expect(developerLabel({ id: "p9", name: "", original: "キッド", type: "co" })).toBe(
      "キッド",
    );
    expect(developerLabel(nameless)).toBeNull();
  });
});

describe("external links", () => {
  const link = (name: string, url: string): VndbExtLink => ({
    name,
    url,
    label: name,
  });

  it("separates the page worth clicking from the databases", () => {
    const { primary, reference } = splitExtLinks([
      link("official", "https://example.invalid/game"),
      link("enwiki", "https://en.wikipedia.org/wiki/X"),
      link("steam", "https://store.steampowered.com/app/1"),
      link("vndb", "https://vndb.org/v1"),
    ]);
    expect(primary.map((entry) => entry.name)).toEqual(["official", "steam"]);
    expect(reference.map((entry) => entry.name)).toEqual(["enwiki", "vndb"]);
  });

  it("refuses anything that is not http(s)", () => {
    // A `javascript:` URL from an API response must never become clickable.
    const { primary, reference } = splitExtLinks([
      link("bad", "javascript:alert(1)"),
      link("alsobad", "file:///C:/Windows/System32/calc.exe"),
      link("good", "https://example.invalid"),
    ]);
    expect([...primary, ...reference].map((entry) => entry.name)).toEqual(["good"]);
  });

  it("removes duplicate URLs, which VNDB lists more than once", () => {
    const url = "https://store.steampowered.com/app/1";
    const { primary } = splitExtLinks([link("steam", url), link("steam", url)]);
    expect(primary).toHaveLength(1);
  });

  it("handles a missing url without throwing", () => {
    const { primary, reference } = splitExtLinks([{ name: "x", url: null, label: "x" }]);
    expect(primary).toHaveLength(0);
    expect(reference).toHaveLength(0);
  });
});

describe("VNDB urls", () => {
  it("builds the entry page from the id", () => {
    expect(vndbVnUrl("v17")).toBe("https://vndb.org/v17");
    expect(vndbVnUrl(" v17 ")).toBe("https://vndb.org/v17");
  });

  it("escapes an id that is not a plain vndbid", () => {
    // The id comes from a response body, so it is never concatenated blindly.
    expect(vndbVnUrl("v1/../evil")).toBe("https://vndb.org/v1%2F..%2Fevil");
  });
});
