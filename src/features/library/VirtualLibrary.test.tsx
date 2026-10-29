import { act, fireEvent, render, screen } from "@testing-library/react";
import { StrictMode } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { VirtualLibrary, type LibraryView } from "./VirtualLibrary";
import { LibraryScreen } from "./LibraryScreen";
import { useAppStore } from "@/app/store";
import { createTranslator } from "@/i18n";
import { initialLibraryBrowse } from "@/lib/library";
import type { GameCard } from "@/lib/ipc/types";

vi.mock("@/components/CoverCard", () => ({
  CoverCard: ({ card }: { card: GameCard }) => (
    <a href={`/game/${card.game.meta.id}`}>{card.game.title}</a>
  ),
  GameRow: ({ card }: { card: GameCard }) => (
    <a href={`/game/${card.game.meta.id}`}>{card.game.title}</a>
  ),
}));
vi.mock("@/app/useI18n", () => ({
  useI18n: () => ({ t: createTranslator("en"), locale: "en" }),
}));
vi.mock("./AddGameSheet", () => ({ AddGameSheet: () => null }));
vi.mock("@/lib/ipc/client", () => ({
  api: { listGames: vi.fn() },
  IpcError: class extends Error {},
}));

let width = 960;
let rowHeight = 300;
const callbacks = new Set<ResizeObserverCallback>();
const rowDimensions = new Map<number, number>();
const originalScrollTo = HTMLElement.prototype.scrollTo;
class TestResizeObserver {
  constructor(private callback: ResizeObserverCallback) {
    callbacks.add(callback);
  }
  observe() {}
  unobserve() {}
  disconnect() {
    callbacks.delete(this.callback);
  }
}

const cards: GameCard[] = Array.from({ length: 5_000 }, (_, index) => ({
  game: {
    meta: {
      id: `game-${index}`,
      updated_at: index,
      device_id_device: "test",
      deleted: false,
    },
    title: `Game ${index}`,
    title_original: null,
    vndb_id: null,
    status: "wishlist",
    rating: null,
    notes: null,
    length_minutes: null,
    manual_playtime_ms: 0,
    accent_color: null,
    cover_url: null,
    cover_sexual: null,
    cover_violence: null,
    custom_cover_path: null,
    custom_cover_source_vndb: false,
    started_at: null,
    finished_at: null,
  },
  tracked_ms: 0,
  afk_ms: 0,
  total_ms: 0,
}));

beforeEach(() => {
  width = 960;
  rowHeight = 300;
  callbacks.clear();
  rowDimensions.clear();
  // The library's search/filter set lives in the store (it must outlive a menu
  // change), so a test that types into the search box would otherwise leave the
  // next one rendering a filtered library.
  useAppStore.setState({ libraryBrowse: initialLibraryBrowse });
  vi.stubGlobal("ResizeObserver", TestResizeObserver);
  const originalStyle = window.getComputedStyle;
  vi.spyOn(window, "getComputedStyle").mockImplementation((element) => {
    if (element === document.documentElement) {
      return {
        getPropertyValue: (property: string) =>
          ({
            "--text-xs": "12px",
            "--leading-normal": "1.5",
            "--space-2": "8px",
            "--space-half": "2px",
            "--touch-target": "44px",
          })[property] ?? "",
      } as CSSStyleDeclaration;
    }
    if (element.classList.contains("library-virtual__probe")) {
      return {
        gridTemplateColumns:
          width >= 600 ? "150px 150px 150px 150px 150px 150px" : "174px 174px",
        getPropertyValue: (property: string) =>
          property === "row-gap" ? "16px" : "12px",
      } as CSSStyleDeclaration;
    }
    return originalStyle(element);
  });
  vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockImplementation(function (
    this: HTMLElement,
  ) {
    return width;
  });
  vi.spyOn(HTMLElement.prototype, "clientHeight", "get").mockReturnValue(700);
  vi.spyOn(HTMLElement.prototype, "offsetHeight", "get").mockImplementation(function (
    this: HTMLElement,
  ) {
    return this.classList.contains("library-virtual__row")
      ? (rowDimensions.get(Number(this.dataset.index)) ?? rowHeight)
      : 700;
  });
  vi.spyOn(HTMLElement.prototype, "offsetWidth", "get").mockReturnValue(220);
  vi.spyOn(HTMLElement.prototype, "scrollHeight", "get").mockReturnValue(500_000);
  vi.spyOn(HTMLElement.prototype, "scrollWidth", "get").mockReturnValue(1_000_000);
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (
    this: HTMLElement,
  ) {
    const main = this.closest<HTMLElement>(".app__main");
    const top = this.classList.contains("library-virtual")
      ? 120 - (main?.scrollTop ?? 0)
      : 0;
    return {
      width: this.classList.contains("shelf__item") ? 220 : width,
      height: rowHeight,
      x: 0,
      y: top,
      top,
      left: 0,
      bottom: top + rowHeight,
      right: width,
      toJSON: () => ({}),
    };
  });
  // jsdom has no layout/scroll machinery; retain native offsets and emit its
  // scroll event, while running the real TanStack observers/range calculation.
  HTMLElement.prototype.scrollTo = function (
    options: ScrollToOptions | number = {},
    _y?: number,
  ): void {
    if (typeof options === "number") return;
    if (options.top !== undefined) this.scrollTop = options.top;
    if (options.left !== undefined) this.scrollLeft = options.left;
    this.dispatchEvent(new Event("scroll"));
  };
});

afterEach(() => {
  vi.unstubAllGlobals();
  HTMLElement.prototype.scrollTo = originalScrollTo;
});

function mount(view: LibraryView, selected = cards) {
  const result = render(
    <main className="app__main">
      <VirtualLibrary cards={selected} view={view} />
    </main>,
  );
  return {
    ...result,
    main: result.container.querySelector<HTMLElement>("main")!,
    host: result.container.querySelector<HTMLElement>(".library-virtual")!,
  };
}

describe("shelf wheel handling", () => {
  it("scrolls the shelf on a vertical wheel, and only while it can still move", () => {
    const { host } = mount("shelf");
    // The listener is attached natively and non-passively; jsdom applies neither,
    // so this asserts the component wires the shared helper to the real element.
    const event = new WheelEvent("wheel", {
      deltaY: 120,
      deltaMode: 0,
      bubbles: true,
      cancelable: true,
    });
    act(() => {
      host.scrollLeft = 0;
      host.dispatchEvent(event);
    });
    expect(host.scrollLeft).toBe(120);
    // Consumed, so the page does not scroll at the same time.
    expect(event.defaultPrevented).toBe(true);
  });

  it("does not hijack the wheel in grid or list", () => {
    for (const view of ["grid", "list"] as const) {
      const { host } = mount(view);
      const event = new WheelEvent("wheel", {
        deltaY: 120,
        bubbles: true,
        cancelable: true,
      });
      act(() => {
        host.dispatchEvent(event);
      });
      expect(event.defaultPrevented).toBe(false);
    }
  });
});

describe("virtual library", () => {
  it("attaches scrolling observers after StrictMode's mount cleanup", () => {
    const result = render(
      <StrictMode>
        <main className="app__main">
          <VirtualLibrary cards={cards} view="list" />
        </main>
      </StrictMode>,
    );
    const main = result.container.querySelector<HTMLElement>("main")!;
    expect(screen.getByRole("link", { name: "Game 0" })).toBeInTheDocument();
    act(() => {
      main.scrollTop = 100_000;
      fireEvent.scroll(main);
    });
    expect(screen.queryByRole("link", { name: "Game 0" })).not.toBeInTheDocument();
    expect(screen.getAllByRole("link").length).toBeLessThan(100);
  });
  it.each(["grid", "list", "shelf"] as const)(
    "bounds DOM for 5,000 games in %s and mounts later games when scrolling",
    (view) => {
      const { main, host } = mount(view);
      expect(screen.getAllByRole("link").length).toBeGreaterThan(0);
      expect(screen.getAllByRole("link").length).toBeLessThan(100);
      expect(screen.getByRole("link", { name: "Game 0" })).toBeInTheDocument();
      act(() => {
        if (view === "shelf") host.scrollLeft = 100_000;
        else main.scrollTop = 100_000;
        fireEvent.scroll(view === "shelf" ? host : main);
      });
      expect(screen.getAllByRole("link").length).toBeLessThan(100);
      expect(screen.queryByRole("link", { name: "Game 0" })).not.toBeInTheDocument();
      const later = screen
        .getAllByRole("link")
        .map((link) => Number(link.textContent?.slice(5)));
      expect(Math.min(...later)).toBeGreaterThan(200);
    },
  );

  it("reflows grid columns after responsive resize without rendering the collection", () => {
    const { container } = mount("grid");
    expect(container.querySelector(".library-virtual__row")).toHaveStyle({
      gridTemplateColumns: "repeat(6, minmax(0, 1fr))",
    });
    act(() => {
      width = 360;
      callbacks.forEach((callback) => callback([], {} as ResizeObserver));
    });
    expect(container.querySelector(".library-virtual__row")).toHaveStyle({
      gridTemplateColumns: "repeat(2, minmax(0, 1fr))",
    });
    expect(screen.getAllByRole("link").length).toBeLessThan(40);
  });

  it("retains keyboard focus when a responsive reflow moves its card into a different row", () => {
    mount("grid");
    act(() => screen.getByRole("link", { name: "Game 5" }).focus());
    act(() => {
      width = 360;
      callbacks.forEach((callback) => callback([], {} as ResizeObserver));
    });
    expect(screen.getByRole("link", { name: "Game 5" })).toHaveFocus();
  });

  it.each(["grid", "list"] as const)(
    "positions %s rows from measured heights after a same-column resize, without waiting for another observer event",
    (view) => {
      rowHeight = view === "grid" ? 293 : 105;
      const { container } = mount(view);
      const offset = (row: HTMLElement): number =>
        Number.parseFloat(
          row.style.transform.match(/translateY\(([^p]+)px\)/)?.[1] ?? "0",
        );
      const assertSpacing = (): void => {
        const rows = [
          ...container.querySelectorAll<HTMLElement>(".library-virtual__row"),
        ];
        for (let index = 1; index < rows.length; index += 1) {
          expect(offset(rows[index]!) - offset(rows[index - 1]!)).toBeCloseTo(
            rowHeight + (view === "grid" ? 16 : 8),
            4,
          );
        }
      };
      assertSpacing();
      act(() => {
        width = 900;
        rowHeight = view === "grid" ? 270 : 120;
        callbacks.forEach((callback) => callback([], {} as ResizeObserver));
      });
      // Width changes invalidate cached sizes although existing rows stay mounted.
      // The browser need not emit a second height event after cache invalidation.
      assertSpacing();
    },
  );

  it.each(["grid", "list"] as const)(
    "leaves the token gap between unequal %s row heights and updates following offsets after an observer resize",
    (view) => {
      rowDimensions.set(0, 293);
      rowDimensions.set(1, 105);
      rowDimensions.set(2, 121);
      const { container } = mount(view);
      const rows = [...container.querySelectorAll<HTMLElement>(".library-virtual__row")];
      const offset = (row: HTMLElement): number =>
        Number.parseFloat(
          row.style.transform.match(/translateY\(([^p]+)px\)/)?.[1] ?? "0",
        );
      const gap = view === "grid" ? 16 : 8;
      expect(offset(rows[1]!) - offset(rows[0]!)).toBe(293 + gap);
      expect(offset(rows[2]!) - offset(rows[1]!)).toBe(105 + gap);
      act(() =>
        callbacks.forEach((callback) =>
          callback(
            [
              {
                target: rows[1]!,
                borderBoxSize: [{ inlineSize: 220, blockSize: 150 }],
              } as unknown as ResizeObserverEntry,
            ],
            {} as ResizeObserver,
          ),
        ),
      );
      expect(offset(rows[2]!) - offset(rows[1]!)).toBe(150 + gap);
      expect(offset(rows[3]!) - offset(rows[2]!)).toBe(121 + gap);
    },
  );

  it.each(["grid", "list", "shelf"] as const)(
    "adjusts %s extent when measured content height changes",
    (view) => {
      const { container } = mount(view);
      const track = container.querySelector<HTMLElement>(".library-virtual__track")!;
      const before = Number.parseFloat(track.style.height);
      const firstRow = container.querySelector<HTMLElement>(".library-virtual__row")!;
      act(() =>
        callbacks.forEach((callback) =>
          callback(
            [
              {
                target: firstRow,
                borderBoxSize: [{ inlineSize: 220, blockSize: 600 }],
              } as unknown as ResizeObserverEntry,
            ],
            {} as ResizeObserver,
          ),
        ),
      );
      expect(Number.parseFloat(track.style.height)).toBeGreaterThan(before);
    },
  );

  it("resets scroll when a filter/sort key remounts a small result set", () => {
    const result = mount("list");
    act(() => {
      result.main.scrollTop = 100_000;
      fireEvent.scroll(result.main);
    });
    result.rerender(
      <main className="app__main">
        <VirtualLibrary key="new-filter" cards={cards.slice(4_990)} view="list" />
      </main>,
    );
    expect(result.main.scrollTop).toBe(0);
    expect(screen.getByRole("link", { name: "Game 4990" })).toBeInTheDocument();
    expect(screen.getAllByRole("link").length).toBeLessThanOrEqual(10);
  });

  it("LibraryScreen resets the real main scroller for search and sort changes", () => {
    useAppStore.setState({ cards, libraryLoaded: true, libraryError: null });
    const result = render(
      <main className="app__main">
        <LibraryScreen />
      </main>,
    );
    const main = result.container.querySelector<HTMLElement>("main")!;
    act(() => {
      main.scrollTop = 100_000;
      fireEvent.scroll(main);
    });
    fireEvent.change(screen.getByRole("searchbox"), { target: { value: "Game 4990" } });
    expect(main.scrollTop).toBe(0);
    expect(screen.getAllByRole("link")).toHaveLength(1);
    expect(screen.getByRole("link", { name: "Game 4990" })).toBeInTheDocument();
    fireEvent.change(screen.getByRole("searchbox"), { target: { value: "" } });
    act(() => {
      main.scrollTop = 100_000;
      fireEvent.scroll(main);
    });
    fireEvent.change(screen.getByRole("combobox"), { target: { value: "title" } });
    expect(main.scrollTop).toBe(0);
    expect(screen.getByRole("link", { name: "Game 0" })).toBeInTheDocument();
    expect(screen.getAllByRole("link").length).toBeLessThan(100);
  });

  it("keeps a focused game mounted during pointer scrolling, and keyboard End reaches the final game", () => {
    const { main } = mount("grid");
    const initial = screen.getByRole("link", { name: "Game 0" });
    act(() => initial.focus());
    act(() => {
      main.scrollTop = 100_000;
      fireEvent.scroll(main);
    });
    expect(initial).toHaveFocus();
    expect(initial).toBeInTheDocument();
    act(() => fireEvent.keyDown(initial, { key: "End" }));
    expect(screen.getByRole("link", { name: "Game 4999" })).toHaveFocus();
    expect(screen.getAllByRole("link").length).toBeLessThan(100);
  });

  it("Tab advances into an unmounted neighbour instead of skipping the rest of the library", () => {
    rowHeight = 82;
    mount("list");
    const last = screen.getAllByRole("link").at(-1)!;
    const nextIndex = Number(last.textContent?.slice(5)) + 1;
    act(() => last.focus());
    act(() => fireEvent.keyDown(last, { key: "Tab" }));
    expect(screen.getByRole("link", { name: `Game ${nextIndex}` })).toHaveFocus();
  });
});
