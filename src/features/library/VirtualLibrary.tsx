import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
} from "react";
import {
  defaultRangeExtractor,
  measureElement,
  type Range,
  type Virtualizer,
} from "@tanstack/react-virtual";

import { CoverCard, GameRow } from "@/components/CoverCard";
import type { GameCard } from "@/lib/ipc/types";
import { useVirtualCollection } from "./useVirtualCollection";

export type LibraryView = "grid" | "list" | "shelf";

interface Layout {
  columns: number;
  cardWidth: number;
  gap: number;
  rowHeight: number;
  margin: number;
}

const emptyLayout: Layout = { columns: 1, cardWidth: 0, gap: 0, rowHeight: 1, margin: 0 };

function sameLayout(a: Layout, b: Layout): boolean {
  return Object.keys(a).every((key) => a[key as keyof Layout] === b[key as keyof Layout]);
}

function numberProperty(style: CSSStyleDeclaration, property: string): number {
  return Number.parseFloat(style.getPropertyValue(property)) || 0;
}

/**
 * Translate a wheel gesture into horizontal movement for a horizontal scroller.
 *
 * The shelf is `overflow-x: auto`, and the browser only moves such an element for
 * a horizontal wheel delta. A mouse wheel sends `deltaY` only, so a shelf under a
 * mouse could not be scrolled at all: the gesture fell through to `.app__main`,
 * which has no vertical overflow while the shelf fits the viewport, so nothing
 * moved anywhere. Trackpads worked (they send `deltaX`), which is why this looked
 * like a mouse-only bug.
 *
 * This function only does arithmetic and the write; the snap toggle lives in
 * `shelfWheelScroll`, which must suspend snapping before calling this.
 *
 * Returns the number of pixels actually consumed. `0` means the gesture was left
 * alone so the page can use it, which is what keeps reaching the end of the shelf
 * from trapping the wheel.
 */
export function shelfWheelDelta(
  element: Pick<HTMLElement, "scrollLeft" | "scrollWidth" | "clientWidth">,
  event: Pick<WheelEvent, "deltaX" | "deltaY" | "shiftKey">,
  speed = 1,
): number {
  const { deltaX, deltaY } = event;
  // A horizontal gesture, or a trackpad two-finger swipe, already scrolls natively.
  // Taking it over would double the movement the browser is about to apply. A
  // shift-wheel is the browser's own way to express "horizontal", including when
  // it arrives as a bare deltaY.
  if (event.shiftKey) return 0;
  if (Math.abs(deltaX) > Math.abs(deltaY)) return 0;
  if (deltaY === 0) return 0;

  const maxScroll = element.scrollWidth - element.clientWidth;
  if (maxScroll <= 0) return 0;
  const wantingForward = deltaY > 0;
  // At the boundary the gesture belongs to the page, not the shelf.
  if (wantingForward && element.scrollLeft >= maxScroll - 1) return 0;
  if (!wantingForward && element.scrollLeft <= 0) return 0;

  const target = Math.min(Math.max(element.scrollLeft + deltaY * speed, 0), maxScroll);
  const from = element.scrollLeft;
  if (target === from) return 0;
  element.scrollLeft = target;
  return element.scrollLeft - from;
}

/** Idle delay before snapping is restored after the last wheel event. */
export const WHEEL_SNAP_IDLE_MS = 220;

/**
 * Suspend scroll snapping for a wheel burst, write the scroll, then restore it.
 *
 * Two ordering details make this work, and both were wrong on earlier attempts:
 *
 * 1. Snapping is disabled *before* the write. With `scroll-snap-type: x` active the
 *    browser re-anchors `scrollLeft` to the nearest snap point immediately, and the
 *    first card sits at offset 4, so a +120 step became 4 every time.
 * 2. It stays off for the whole burst, not just the current task. Restoring on the
 *    next frame is not enough: the shelf re-renders in response to the scroll
 *    event, after that frame, so the value was re-anchored as soon as snapping
 *    returned. The restore waits for `WHEEL_SNAP_IDLE_MS` of quiet instead.
 *
 * Touch flings and keyboard navigation never reach this path, so they keep their
 * snapping (arrow keys go through `scrollToIndex`, which lands on a card boundary
 * and tolerates a snap).
 */
export function shelfWheelScroll(
  element: HTMLElement,
  event: Pick<WheelEvent, "deltaX" | "deltaY" | "shiftKey">,
  pending = new WeakMap<HTMLElement, number>(),
): number {
  // Snapping must be off BEFORE the write, not after: `shelfWheelDelta` assigns
  // `scrollLeft`, and with snap still active the browser re-anchors that value to
  // the nearest card immediately. Turning it off afterwards leaves the revert in
  // place, which is why the wheel still did nothing on the first attempt.
  //
  // The inline value is captured once, on the first event of a burst. Reading it
  // on every event would capture the "none" this function wrote a moment ago, so
  // the restore would put "none" back and leave snapping off forever.
  const timers = pending;
  const suspended = timers.has(element);
  const previousSnap = suspended ? "" : element.style.scrollSnapType;
  element.style.scrollSnapType = "none";
  const consumed = shelfWheelDelta(element, event);
  if (consumed === 0) {
    // Nothing moved. Only undo our own change if no burst is in progress.
    if (!suspended) element.style.scrollSnapType = previousSnap;
    return 0;
  }
  const previousTimer = timers.get(element);
  if (previousTimer !== undefined) clearTimeout(previousTimer);
  const handle = setTimeout(() => {
    element.style.scrollSnapType = previousSnap;
    timers.delete(element);
  }, WHEEL_SNAP_IDLE_MS);
  timers.set(element, handle as unknown as number);
  return consumed;
}

/**
 * Grid and list rows use the shell's scrolling main element. The cinematic shelf
 * retains its horizontal scroller. CSS probes resolve the existing fluid tracks
 * and token spacing, so the virtual layout follows the same mobile breakpoints.
 */
export function VirtualLibrary({
  cards,
  view,
}: {
  cards: GameCard[];
  view: LibraryView;
}) {
  const hostRef = useRef<HTMLDivElement>(null);
  const probeRef = useRef<HTMLDivElement>(null);
  const [layout, setLayout] = useState(emptyLayout);
  const [shelfMeasure, setShelfMeasure] = useState({ width: 0, height: 0 });
  const [focusedId, setFocusedId] = useState<string | null>(null);
  const pendingFocus = useRef<{ id: string; last: boolean } | null>(null);
  const horizontal = view === "shelf";
  const columns = view === "grid" ? layout.columns : 1;
  const count = Math.ceil(cards.length / columns);
  const focusedIndex = useMemo(
    () =>
      focusedId === null
        ? -1
        : cards.findIndex((card) => card.game.meta.id === focusedId),
    [cards, focusedId],
  );

  const rangeExtractor = useCallback(
    (range: Range): number[] => {
      const visible = defaultRangeExtractor(range);
      const row = Math.floor(focusedIndex / columns);
      // A wheel/touch scroll must never unmount the element holding keyboard focus.
      if (row >= 0 && row < range.count && !visible.includes(row)) visible.push(row);
      return visible.sort((a, b) => a - b);
    },
    [columns, focusedIndex],
  );

  const getScrollElement = useCallback(
    () =>
      horizontal
        ? hostRef.current
        : (hostRef.current?.closest<HTMLElement>(".app__main") ?? null),
    [horizontal],
  );
  const estimateSize = useCallback(
    () => (horizontal ? layout.cardWidth : layout.rowHeight),
    [horizontal, layout.cardWidth, layout.rowHeight],
  );
  // Keep key extraction stable during scrolling; changing its identity forces
  // TanStack to rebuild measurements for every game rather than the visible set.
  const getItemKey = useCallback(
    (index: number) => `${columns}:${cards[index * columns]?.game.meta.id ?? index}`,
    [cards, columns],
  );
  const measureRow = useCallback(
    (
      element: HTMLDivElement,
      entry: ResizeObserverEntry | undefined,
      instance: Virtualizer<HTMLElement, HTMLDivElement>,
    ) => {
      if (horizontal) {
        const height = entry?.borderBoxSize?.[0]?.blockSize ?? element.offsetHeight;
        if (height > 0)
          setShelfMeasure((current) => {
            const nextHeight = Math.max(
              current.width === layout.cardWidth ? current.height : 0,
              height,
            );
            return current.width === layout.cardWidth && current.height === nextHeight
              ? current
              : { width: layout.cardWidth, height: nextHeight };
          });
      }
      return (
        measureElement(element, entry, instance) ||
        (horizontal ? layout.cardWidth : layout.rowHeight)
      );
    },
    [horizontal, layout.cardWidth, layout.rowHeight],
  );

  const virtualizer = useVirtualCollection({
    count,
    getScrollElement,
    horizontal,
    enabled: layout.cardWidth > 0,
    estimateSize,
    getItemKey,
    gap: layout.gap,
    scrollMargin: layout.margin,
    overscan: 3,
    rangeExtractor,
    measureElement: measureRow,
  });

  useLayoutEffect(() => {
    const host = hostRef.current;
    const probe = probeRef.current;
    const main = host?.closest<HTMLElement>(".app__main");
    if (!host || !probe || !main) return;

    const readLayout = (): void => {
      const root = getComputedStyle(document.documentElement);
      const style = getComputedStyle(probe);
      const tracks = style.gridTemplateColumns
        .split(" ")
        .map(Number.parseFloat)
        .filter(Number.isFinite);
      const columnGap = numberProperty(style, "column-gap");
      const gridColumns = Math.max(1, tracks.length);
      const width = horizontal
        ? (probe.firstElementChild?.getBoundingClientRect().width ?? 0)
        : view === "grid"
          ? (tracks[0] ?? host.clientWidth)
          : host.clientWidth;
      const line =
        numberProperty(root, "--text-xs") * numberProperty(root, "--leading-normal");
      const textHeight = line * 4 + numberProperty(root, "--space-2");
      const listHeight =
        numberProperty(root, "--touch-target") * 1.5 +
        numberProperty(root, "--space-2") * 2 +
        numberProperty(root, "--space-half");
      const rowHeight =
        view === "list" ? listHeight : width * (horizontal ? 4 / 3 : 3 / 2) + textHeight;
      const next = {
        columns: view === "grid" ? gridColumns : 1,
        cardWidth: width,
        gap: horizontal
          ? numberProperty(style, "column-gap")
          : view === "grid"
            ? numberProperty(style, "row-gap")
            : numberProperty(root, "--space-2"),
        rowHeight: Math.max(1, rowHeight),
        margin: horizontal
          ? 0
          : host.getBoundingClientRect().top -
            main.getBoundingClientRect().top -
            main.clientTop +
            main.scrollTop,
      };
      // A browser may report a fractional track; do not accumulate rounding drift.
      if (view === "grid" && tracks.length > 0)
        next.cardWidth = (host.clientWidth - columnGap * (gridColumns - 1)) / gridColumns;
      setLayout((current) => (sameLayout(current, next) ? current : next));
    };

    readLayout();
    const observer =
      typeof ResizeObserver === "undefined" ? null : new ResizeObserver(readLayout);
    observer?.observe(main);
    observer?.observe(host);
    observer?.observe(probe);
    // Toolbar/filter wrapping changes the collection's offset within main.
    for (const sibling of main.children) {
      if (sibling !== host) observer?.observe(sibling);
    }
    window.addEventListener("resize", readLayout);
    return () => {
      observer?.disconnect();
      window.removeEventListener("resize", readLayout);
    };
  }, [horizontal, view]);

  useLayoutEffect(() => {
    // The parent remounts this component for a changed filter, sort, or view.
    const main = hostRef.current?.closest<HTMLElement>(".app__main");
    main?.scrollTo?.({ top: 0, behavior: "instant" });
    if (main) main.scrollTop = 0;
    if (horizontal && hostRef.current) hostRef.current.scrollLeft = 0;
  }, [horizontal]);

  const { measure } = virtualizer;
  useLayoutEffect(() => {
    measure();
  }, [layout.cardWidth, columns, measure]);

  useLayoutEffect(() => {
    const onFocus = (event: FocusEvent): void => {
      if (!(event.target instanceof Element)) return;
      const item = event.target.closest<HTMLElement>("[data-library-game]");
      setFocusedId(
        item && hostRef.current?.contains(item)
          ? (item.dataset.libraryGame ?? null)
          : null,
      );
    };
    document.addEventListener("focusin", onFocus);
    return () => document.removeEventListener("focusin", onFocus);
  }, []);

  useLayoutEffect(() => {
    const desired = pendingFocus.current;
    // A responsive reflow can move a focused card into a newly grouped row.
    const restoreId =
      desired?.id ?? (document.activeElement === document.body ? focusedId : null);
    if (!restoreId) return;
    const item = [
      ...(hostRef.current?.querySelectorAll<HTMLElement>("[data-library-game]") ?? []),
    ].find((element) => element.dataset.libraryGame === restoreId);
    const controls = item?.querySelectorAll<HTMLElement>("a[href],button:not(:disabled)");
    const target = desired?.last ? controls?.[controls.length - 1] : controls?.[0];
    if (!target) return;
    target.focus({ preventScroll: true });
    pendingFocus.current = null;
  });

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>): void => {
    if (!(event.target instanceof HTMLElement)) return;
    const item = event.target.closest<HTMLElement>("[data-library-game]");
    if (!item) return;
    const index = Number(item.dataset.cardIndex);
    const controls = [
      ...item.querySelectorAll<HTMLElement>("a[href],button:not(:disabled)"),
    ];
    const first = controls[0];
    const last = controls[controls.length - 1];
    let next: number | null = null;
    if (event.key === "Tab" && !event.ctrlKey && !event.metaKey && !event.altKey) {
      if (event.shiftKey && event.target === first) next = index - 1;
      if (!event.shiftKey && event.target === last) next = index + 1;
    } else if (
      event.target === first &&
      !event.ctrlKey &&
      !event.metaKey &&
      !event.altKey
    ) {
      switch (event.key) {
        case "ArrowRight":
          next = index + 1;
          break;
        case "ArrowLeft":
          next = index - 1;
          break;
        case "ArrowDown":
          next = index + columns;
          break;
        case "ArrowUp":
          next = index - columns;
          break;
        case "Home":
          next = 0;
          break;
        case "End":
          next = cards.length - 1;
          break;
      }
    }
    const card = next === null ? undefined : cards[next];
    if (!card || next === null) return;
    event.preventDefault();
    pendingFocus.current = {
      id: card.game.meta.id,
      last: event.key === "Tab" && event.shiftKey,
    };
    setFocusedId(card.game.meta.id);
    virtualizer.scrollToIndex(Math.floor(next / columns));
  };

  const { items, total } = virtualizer;

  /**
   * Wheel support for the horizontal shelf. Registered natively rather than via
   * React's `onWheel`, because React attaches wheel listeners as passive at the
   * root, so `preventDefault` there would be ignored and the page would scroll
   * underneath the shelf at the same time.
   */
  useEffect(() => {
    const host = hostRef.current;
    if (!horizontal || !host) return undefined;
    // One timer map for this element, so a burst of wheel events restarts the
    // restore instead of stacking timers and flickering the snap property.
    const pending = new WeakMap<HTMLElement, number>();
    const onWheel = (event: WheelEvent): void => {
      // The handler writes `scrollLeft` and the snap must stay off across the whole
      // burst, not just around the write. See the note on `shelfWheelDelta`.
      const consumed = shelfWheelScroll(host, event, pending);
      if (consumed !== 0) event.preventDefault();
    };
    host.addEventListener("wheel", onWheel, { passive: false });
    return () => {
      host.removeEventListener("wheel", onWheel);
      // A view switch mid-burst must not leave snapping disabled.
      const handle = pending.get(host);
      if (handle !== undefined) clearTimeout(handle);
      host.style.scrollSnapType = "";
    };
  }, [horizontal]);

  return (
    <div
      ref={hostRef}
      className={`library-virtual${horizontal ? " shelf library-virtual--shelf" : ""}`}
      data-view={view}
      onKeyDown={onKeyDown}
    >
      <div
        ref={probeRef}
        className={`library-virtual__probe ${horizontal ? "shelf" : "grid"}`}
        aria-hidden="true"
      >
        {horizontal ? <div className="shelf__item" /> : null}
      </div>
      <div
        className="library-virtual__track"
        style={
          horizontal
            ? {
                width: total,
                height: Math.max(
                  layout.rowHeight,
                  shelfMeasure.width === layout.cardWidth ? shelfMeasure.height : 0,
                ),
              }
            : { height: total }
        }
      >
        {items.map((row) => (
          <div
            key={row.key}
            ref={virtualizer.measureElement}
            data-index={row.index}
            className={
              horizontal
                ? "library-virtual__row shelf__item"
                : `library-virtual__row${view === "grid" ? " grid" : ""}`
            }
            style={
              horizontal
                ? { width: layout.cardWidth, transform: `translateX(${row.start}px)` }
                : {
                    gridTemplateColumns: `repeat(${columns}, minmax(0, 1fr))`,
                    transform: `translateY(${row.start - layout.margin}px)`,
                  }
            }
          >
            {cards
              .slice(row.index * columns, (row.index + 1) * columns)
              .map((card, offset) => (
                <div
                  key={card.game.meta.id}
                  data-library-game={card.game.meta.id}
                  data-card-index={row.index * columns + offset}
                >
                  {view === "list" ? (
                    <GameRow card={card} />
                  ) : (
                    <CoverCard card={card} size={horizontal ? "shelf" : "grid"} />
                  )}
                </div>
              ))}
          </div>
        ))}
      </div>
    </div>
  );
}
