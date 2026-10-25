import { describe, expect, it, vi } from "vitest";

import { shelfWheelDelta, shelfWheelScroll, WHEEL_SNAP_IDLE_MS } from "./VirtualLibrary";

/**
 * A scroller double that models `scroll-snap-type: x`.
 *
 * The real bug was not arithmetic: the write was correct and the browser rounded
 * it back to the nearest snap point, so a 120 px wheel step landed on 4 px. A
 * double that just stores whatever is written cannot catch that, which is why the
 * first version of these tests passed against broken code.
 */
function scroller(scrollLeft = 0, scrollWidth = 2000, clientWidth = 1000) {
  const firstSnapPoint = 4;
  const step = 264;
  const element = {
    scrollWidth,
    clientWidth,
    style: { scrollSnapType: "" },
    _left: scrollLeft,
    get scrollLeft() {
      return this._left;
    },
    set scrollLeft(value: number) {
      if (this.style.scrollSnapType === "none") {
        this._left = value;
        return;
      }
      // Snap: pick the nearest multiple of the card step. Reading the property
      // back in the same task still reports the requested value, so snap has to
      // be modelled as a rejection rather than a silent rewrite: that is exactly
      // the behaviour that made the wheel look inert.
      // Snap to the nearest card boundary, clamped to the scrollable range.
      const max = scrollWidth - clientWidth;
      const index = Math.max(0, Math.round((value - firstSnapPoint) / step));
      this._left = Math.min(firstSnapPoint + index * step, max);
    },
  };
  return element;
}

const wheel = (deltaX = 0, deltaY = 0, shiftKey = false) => ({
  deltaX,
  deltaY,
  shiftKey,
});

describe("shelfWheelDelta", () => {
  it("turns a vertical mouse wheel into horizontal movement despite snapping", () => {
    // The real bug: a mouse wheel only sends deltaY, `overflow-x: auto` ignores it,
    // and even after translating it, scroll-snap rounded the write back to 4.
    const element = scroller();
    const host = element as unknown as HTMLElement;
    const pending = new WeakMap<HTMLElement, number>();
    expect(shelfWheelScroll(host, wheel(0, 400), pending)).toBe(400);
    expect(element.scrollLeft).toBe(400);
  });

  it("does not let snapping swallow a small step", () => {
    // Under a card's half-width a naive snap returns to the start, which is why
    // one wheel notch moved nothing at all.
    const element = scroller();
    const host = element as unknown as HTMLElement;
    const pending = new WeakMap<HTMLElement, number>();
    expect(shelfWheelScroll(host, wheel(0, 120), pending)).toBe(120);
    expect(element.scrollLeft).toBe(120);
  });

  it("keeps snapping off for the whole burst, then restores it", () => {
    vi.useFakeTimers();
    try {
      const element = scroller(0);
      const host = element as unknown as HTMLElement;
      const pending = new WeakMap<HTMLElement, number>();
      const previousSnap = element.style.scrollSnapType;
      shelfWheelScroll(host, wheel(0, 120), pending);
      expect(element.style.scrollSnapType).toBe("none");
      // A second notch inside the idle window must not restore snapping: doing so
      // re-anchored the offset, and that was the actual reason the wheel did
      // nothing. Restoring on the next frame was not enough either, because the
      // shelf re-renders in response to the scroll event after that frame.
      vi.advanceTimersByTime(WHEEL_SNAP_IDLE_MS / 2);
      shelfWheelScroll(host, wheel(0, 120), pending);
      expect(element.style.scrollSnapType).toBe("none");
      expect(element.scrollLeft).toBe(240);
      vi.advanceTimersByTime(WHEEL_SNAP_IDLE_MS);
      expect(element.style.scrollSnapType).toBe(previousSnap);
    } finally {
      vi.useRealTimers();
    }
  });

  it("moves backwards on an upward wheel", () => {
    const element = scroller(500);
    // Snapping would re-anchor this; measure the untranslated step.
    element.style.scrollSnapType = "none";
    expect(shelfWheelDelta(element, wheel(0, -200))).toBe(-200);
    expect(element.scrollLeft).toBe(300);
  });

  it("leaves a horizontal gesture to the browser", () => {
    const element = scroller(300);
    expect(shelfWheelDelta(element, wheel(120, 0))).toBe(0);
    expect(element.scrollLeft).toBe(300);
  });

  it("leaves a shift-wheel to the browser, which reads it as horizontal", () => {
    const element = scroller(300);
    expect(shelfWheelDelta(element, wheel(120, 0, true))).toBe(0);
    expect(shelfWheelDelta(element, wheel(0, 120, true))).toBe(0);
  });

  it("clamps at the end instead of overscrolling", () => {
    // 950 of 1000 available: only 50px of travel is left.
    const element = scroller(950, 2000, 1000);
    element.style.scrollSnapType = "none";
    expect(shelfWheelDelta(element, wheel(0, 500))).toBe(50);
    expect(element.scrollLeft).toBe(1000);
  });

  it("hands the gesture back to the page at either end", () => {
    expect(shelfWheelDelta(scroller(1000, 2000, 1000), wheel(0, 120))).toBe(0);
    expect(shelfWheelDelta(scroller(0), wheel(0, -120))).toBe(0);
  });

  it("does nothing when the shelf has no overflow", () => {
    expect(shelfWheelDelta(scroller(0, 1000, 1000), wheel(0, 120))).toBe(0);
    expect(shelfWheelDelta(scroller(0, 800, 1000), wheel(0, 120))).toBe(0);
  });

  it("ignores an empty delta", () => {
    const element = scroller(100);
    expect(shelfWheelDelta(element, wheel(0, 0))).toBe(0);
    expect(element.scrollLeft).toBe(100);
  });

  it("scales the gesture when asked", () => {
    const element = scroller();
    element.style.scrollSnapType = "none";
    expect(shelfWheelDelta(element, wheel(0, 100), 2)).toBe(200);
    expect(element.scrollLeft).toBe(200);
  });
});
