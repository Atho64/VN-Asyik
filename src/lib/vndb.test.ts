import { describe, expect, it } from "vitest";

import { isExplicitHit } from "./vndbSearch";
import { stripFormatting } from "./vndbText";
import type { SearchHit } from "./ipc/types";

function hit(overrides: Partial<SearchHit> = {}): SearchHit {
  return {
    vn_id: "v1",
    title: "Test",
    alttitle: null,
    released: null,
    image_url: null,
    sexual: null,
    violence: null,
    length_minutes: null,
    length: null,
    votecount: null,
    in_library: false,
    ...overrides,
  };
}

describe("isExplicitHit", () => {
  it("shows everything in show mode", () => {
    expect(isExplicitHit(hit({ sexual: 200, violence: 200 }), "show")).toBe(false);
  });

  it("treats a full explicit vote as explicit", () => {
    // Ratings arrive in hundredths: 200 = 2.0 on VNDB's 0..2 scale.
    expect(isExplicitHit(hit({ sexual: 200 }), "blur")).toBe(true);
    expect(isExplicitHit(hit({ violence: 250 }), "hide")).toBe(true);
  });

  it("conceals a partly-rated explicit cover", () => {
    // This is the library's rule (isExplicit: level 1 means "the community voted
    // this explicit at all"). Requiring 2.0 here meant the add sheet showed
    // explicit thumbnails under Hide while the library concealed the same cover.
    // Sanoba Witch is 0.93 sexual on VNDB (v16044).
    expect(isExplicitHit(hit({ sexual: 93 }), "hide")).toBe(true);
    expect(isExplicitHit(hit({ sexual: 100 }), "blur")).toBe(true);
    expect(isExplicitHit(hit({ violence: 100 }), "blur")).toBe(true);
    expect(isExplicitHit(hit({ sexual: 199 }), "blur")).toBe(true);
  });

  it("rounds like the importer, so search and the library never disagree", () => {
    // Rust's flag_level rounds half up: 50..149 becomes level 1, 150..200 level 2.
    expect(isExplicitHit(hit({ sexual: 49 }), "blur")).toBe(false);
    expect(isExplicitHit(hit({ sexual: 50 }), "blur")).toBe(true);
    expect(isExplicitHit(hit({ sexual: 149 }), "blur")).toBe(true);
    expect(isExplicitHit(hit({ sexual: 150 }), "blur")).toBe(true);
  });

  it("treats missing ratings as safe", () => {
    expect(isExplicitHit(hit(), "blur")).toBe(false);
  });
});

describe("stripFormatting", () => {
  it("removes paired codes", () => {
    expect(stripFormatting("[b]Bold[/b]")).toBe("Bold");
    expect(stripFormatting("[url=https://x]link[/url]")).toBe("link");
  });

  it("handles nesting", () => {
    expect(stripFormatting("[b][i]both[/i][/b]")).toBe("both");
  });

  it("leaves plain text untouched", () => {
    expect(stripFormatting("Nothing to strip.")).toBe("Nothing to strip.");
  });

  it("recovers from an unclosed code", () => {
    expect(stripFormatting("[i]unclosed")).toBe("unclosed");
  });

  it("trims surrounding whitespace", () => {
    expect(stripFormatting("  text  ")).toBe("text");
  });

  it("keeps Japanese text intact", () => {
    expect(stripFormatting("[b]うみねこ[/b]")).toBe("うみねこ");
  });
});
