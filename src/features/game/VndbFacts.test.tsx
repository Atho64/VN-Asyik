import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { AppContext } from "@/app/AppProvider";
import { useAppStore } from "@/app/store";
import { createTranslator } from "@/i18n";
import type { DetailPayload } from "@/lib/ipc/types";

const mocks = vi.hoisted(() => ({
  openExternal: vi.fn(),
}));

vi.mock("@/lib/ipc/client", () => ({
  IpcError: class extends Error {},
  api: { openExternal: mocks.openExternal },
}));

const { VndbFacts } = await import("./VndbFacts");
const t = createTranslator("en");

function payload(overrides: Partial<DetailPayload> = {}): DetailPayload {
  return {
    vn_id: "v17",
    title: "Ever17",
    alttitle: null,
    titles: [],
    aliases: [],
    length_minutes: null,
    length: null,
    length_votes: null,
    description: null,
    rating: null,
    average: null,
    votecount: null,
    released: null,
    olang: null,
    devstatus: null,
    languages: [],
    platforms: [],
    tags: [],
    screenshots: [],
    developers: [],
    extlinks: [],
  characters: [],
    ...overrides,
  };
}

function renderFacts(props: { payload: DetailPayload; playtimeMs?: number }) {
  return render(
    <AppContext.Provider value={{ t, locale: "en", ready: true, bootError: null }}>
      <VndbFacts
        payload={props.payload}
        vnId="v17"
        playtimeMs={props.playtimeMs ?? 0}
      />
    </AppContext.Provider>,
  );
}

/** A facts value carries its caveat in a nested span, so its text is split across
 *  elements and an exact string matcher cannot find it. */
function ddContaining(text: string): HTMLElement {
  const match = Array.from(document.querySelectorAll(".facts__value")).find(
    (node) => node.textContent?.startsWith(text),
  );
  if (!match) {
    throw new Error(`no facts value starting with ${text}`);
  }
  return match as HTMLElement;
}

beforeEach(() => {
  vi.clearAllMocks();
  mocks.openExternal.mockResolvedValue(undefined);
  useAppStore.setState({
    prefs: { theme: "dark", language: "en", nsfw: "blur", reduce_effects: false },
  });
});

describe("VNDB facts", () => {
  it("shows the facts a VN page is read for", () => {
    renderFacts({
      payload: payload({
        released: "2002-08-29",
        devstatus: 0,
        developers: [{ id: "p8", name: "KID", original: "キッド", type: "co" }],
        languages: ["en", "ja"],
        platforms: ["win", "psp"],
        length_minutes: 1987,
        length_votes: 203,
        rating: 84.5,
        average: 84.4,
        votecount: 8800,
      }),
    });

    expect(screen.getByText("2002-08-29")).toBeTruthy();
    expect(screen.getByText("Finished")).toBeTruthy();
    expect(screen.getByText("KID · キッド")).toBeTruthy();
    expect(screen.getByText("English, Japanese")).toBeTruthy();
    expect(screen.getByText("Windows, PSP")).toBeTruthy();
    expect(screen.getByText("33h 7m")).toBeTruthy();
    // VNDB reports these in tenths, exactly like the user's own rating, so 84.5
    // is 8.45 and renders as 8.4. Rendering the raw value put "84.5" on screen
    // beside a user rating of "7.5". The value sits in a <dd> beside its vote
    // count, so match on the leading text rather than the whole node.
    expect(ddContaining("8.")).toBeTruthy();
    expect(screen.queryByText(/84\.5/)).toBeNull();
    // The raw average is on the same scale as the score it sits beside.
    expect(screen.getByText(/Raw average 8\.4/)).toBeTruthy();
    // The score is only meaningful with the count behind it.
    expect(screen.getByText(/8800 votes/)).toBeTruthy();
  });

  it("does not invent rows for facts VNDB does not have", () => {
    // An empty label reads as a loading bug, so a sparse entry shows fewer rows.
    renderFacts({ payload: payload({ released: "2010" }) });
    expect(screen.getByText("2010")).toBeTruthy();
    expect(screen.queryByText("Developer")).toBeNull();
    expect(screen.queryByText("Languages")).toBeNull();
    expect(screen.queryByText("Platforms")).toBeNull();
  });

  it("puts the tracked time next to VNDB's estimate", () => {
    // The gap between the two is the interesting number, exactly like the two
    // ratings sitting side by side.
    renderFacts({
      payload: payload({ length_minutes: 1200, length_votes: 40 }),
      playtimeMs: 3_600_000,
    });
    expect(screen.getByText(/Active: 1h/)).toBeTruthy();
    expect(screen.getByText(/40 playtime votes/)).toBeTruthy();
  });

  it("opens the VN page on vndb.org through the validated command", async () => {
    renderFacts({ payload: payload() });
    await userEvent.click(screen.getByRole("button", { name: /Open on VNDB/ }));
    await waitFor(() =>
      expect(mocks.openExternal).toHaveBeenCalledWith("https://vndb.org/v17"),
    );
  });

  it("keeps reference databases behind a disclosure", async () => {
    // A VN has a dozen of these; putting them in the first row buries the useful
    // links and reads as clutter.
    renderFacts({
      payload: payload({
        extlinks: [
          { url: "https://example.invalid/official", label: "Official site", name: "official" },
          {
            url: "https://en.wikipedia.org/wiki/Ever17",
            label: "Wikipedia (en)",
            name: "enwiki",
          },
        ],
      }),
    });
    expect(screen.getByRole("button", { name: "Official site" })).toBeTruthy();
    // Present, but inside the closed disclosure rather than in the open list.
    const wiki = screen.getByRole("button", { name: "Wikipedia (en)" });
    expect(wiki.closest("details")).not.toBeNull();
    expect(wiki.closest("details")?.hasAttribute("open")).toBe(false);
  });

  it("opens an external link through the same command", async () => {
    renderFacts({
      payload: payload({
        extlinks: [
          { url: "https://example.invalid/official", label: "Official site", name: "official" },
        ],
      }),
    });
    await userEvent.click(screen.getByRole("button", { name: "Official site" }));
    expect(mocks.openExternal).toHaveBeenCalledWith("https://example.invalid/official");
  });
});
