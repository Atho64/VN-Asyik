import { describe, expect, it } from "vitest";

import { moveFor, nextIndex, readPad } from "./navigation";

describe("big picture movement", () => {
  it("maps arrows, WASD and the jump keys", () => {
    expect(moveFor("ArrowRight")).toEqual({ kind: "step", delta: 1 });
    expect(moveFor("a")).toEqual({ kind: "step", delta: -1 });
    expect(moveFor("ArrowUp")).toEqual({ kind: "row", delta: -1 });
    expect(moveFor("S")).toEqual({ kind: "row", delta: 1 });
    expect(moveFor("Home")).toEqual({ kind: "edge", to: "first" });
    expect(moveFor("PageDown")).toEqual({ kind: "page", direction: 1 });
    // Enter and Escape are actions, not movement, so they are not in the table.
    expect(moveFor("Enter")).toBeNull();
    expect(moveFor("Escape")).toBeNull();
  });

  it("stops at the ends of a row instead of wrapping into the next one", () => {
    // Five columns, ten games: the last item of row 0 is index 4.
    expect(nextIndex({ kind: "step", delta: 1 }, 4, 10, 5)).toBe(5);
    // A single step from the very end stays on the last game.
    expect(nextIndex({ kind: "step", delta: 1 }, 9, 10, 5)).toBe(9);
    expect(nextIndex({ kind: "step", delta: -1 }, 0, 10, 5)).toBe(0);
  });

  it("keeps the column when moving between rows, and clamps at the edges", () => {
    // Column 2 of row 1 (index 7) up to row 0 is index 2.
    expect(nextIndex({ kind: "row", delta: -1 }, 7, 10, 5)).toBe(2);
    // Up from the first row stays in the same column, not off the grid.
    expect(nextIndex({ kind: "row", delta: -1 }, 2, 10, 5)).toBe(2);
    // Down from the last row lands on the last row's item in that column.
    expect(nextIndex({ kind: "row", delta: 1 }, 2, 10, 5)).toBe(7);
  });

  it("lands inside the grid when the last row is short", () => {
    // Seven games in five columns: row 1 holds indices 5 and 6 only.
    expect(nextIndex({ kind: "row", delta: 1 }, 4, 7, 5)).toBe(6);
    expect(nextIndex({ kind: "step", delta: 1 }, 6, 7, 5)).toBe(6);
  });

  it("jumps to the ends and by a screenful", () => {
    expect(nextIndex({ kind: "edge", to: "last" }, 0, 10, 5)).toBe(9);
    expect(nextIndex({ kind: "edge", to: "first" }, 9, 10, 5)).toBe(0);
    expect(nextIndex({ kind: "page", direction: 1 }, 0, 20, 5)).toBe(15);
    expect(nextIndex({ kind: "page", direction: -1 }, 18, 20, 5)).toBe(3);
  });

  it("survives an empty library", () => {
    expect(nextIndex({ kind: "step", delta: 1 }, 0, 0, 5)).toBe(0);
  });
});

describe("gamepad reading", () => {
  /** A fake pad: buttons by index, axes by index. */
  const pad = (pressed: number[], axes: number[] = []): Gamepad =>
    ({
      connected: true,
      buttons: Array.from({ length: 16 }, (_, index) => ({
        pressed: pressed.includes(index),
      })),
      axes,
    }) as unknown as Gamepad;

  it("reads the D-pad and the left stick", () => {
    expect(readPad([pad([13])]).move).toBe("down");
    expect(readPad([pad([14])]).move).toBe("left");
    expect(readPad([pad([], [0.9, 0])]).move).toBe("right");
    expect(readPad([pad([], [0, -0.9])]).move).toBe("up");
  });

  it("ignores a resting stick", () => {
    // Below half deflection is drift, not intent.
    expect(readPad([pad([], [0.2, -0.1])]).move).toBeNull();
  });

  it("reports a held direction once", () => {
    // The poll runs every frame; a held direction must not repeat.
    expect(readPad([pad([13])], "down").move).toBeNull();
    expect(readPad([pad([13])], "up").move).toBe("down");
  });

  it("reads A and B, and nothing when no pad is connected", () => {
    expect(readPad([pad([0])]).button).toBe("a");
    expect(readPad([pad([1])]).button).toBe("b");
    expect(readPad([null, undefined]).button).toBeNull();
    expect(readPad([]).move).toBeNull();
  });
});
