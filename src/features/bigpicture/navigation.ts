/**
 * Movement and gamepad reading for Big Picture, kept out of the component.
 *
 * The grid is a flat list laid out in rows, so every move is arithmetic on an
 * index. Doing it here means the behaviour can be tested without a browser, and
 * the component only has to know which key arrived.
 */

/** What a key press asks the grid to do. */
export type Move =
  | { kind: "step"; delta: number }
  | { kind: "row"; delta: number }
  | { kind: "edge"; to: "first" | "last" }
  | { kind: "page"; direction: 1 | -1 };

/**
 * Translate a key into a move.
 *
 * Arrows and WASD both work: a gamepad-less TV setup usually has a keyboard on the
 * sofa, and the layout is the same either way. PageUp/PageDown jump a screenful,
 * Home/End jump to the ends, which is what a 200-game library needs.
 */
export function moveFor(key: string): Move | null {
  switch (key) {
    case "ArrowLeft":
    case "a":
    case "A":
      return { kind: "step", delta: -1 };
    case "ArrowRight":
    case "d":
    case "D":
      return { kind: "step", delta: 1 };
    case "ArrowUp":
    case "w":
    case "W":
      return { kind: "row", delta: -1 };
    case "ArrowDown":
    case "s":
    case "S":
      return { kind: "row", delta: 1 };
    case "Home":
      return { kind: "edge", to: "first" };
    case "End":
      return { kind: "edge", to: "last" };
    case "PageUp":
      return { kind: "page", direction: -1 };
    case "PageDown":
      return { kind: "page", direction: 1 };
    default:
      return null;
  }
}

/**
 * The index a move lands on.
 *
 * Horizontal moves clamp rather than wrap: wrapping from the last cover of a row
 * to the first of the next reads as a jump to a different game while the
 * highlight barely moves. Vertical moves clamp to the last row instead of
 * landing in a half-empty one.
 */
export function nextIndex(
  move: Move,
  index: number,
  count: number,
  columns: number,
): number {
  if (count <= 0) return 0;
  const width = Math.max(1, Math.min(columns, count));
  const rows = Math.ceil(count / width);
  const row = Math.floor(index / width);
  const column = index % width;
  switch (move.kind) {
    case "step":
      return Math.max(0, Math.min(count - 1, index + move.delta));
    case "row": {
      const target = row + move.delta;
      // A short last row (or a column that only exists further up) must not land
      // past the end of the list, so every branch clamps to the last game.
      const last = count - 1;
      if (target < 0) return Math.min(column, last);
      if (target >= rows) return Math.min((rows - 1) * width + column, last);
      return Math.min(target * width + column, last);
    }
    case "edge":
      return move.to === "first" ? 0 : count - 1;
    case "page": {
      // A page is a screenful of rows, which is what the viewer can see at once.
      const target = Math.max(
        0,
        Math.min(rows - 1, row + move.direction * Math.max(1, rows - 1)),
      );
      return Math.min(count - 1, target * width + column);
    }
  }
}

/** A direction read from a gamepad, already reduced to a single intent. */
export type PadMove = "left" | "right" | "up" | "down" | null;

/**
 * Read the first connected gamepad.
 *
 * The standard mapping puts the D-pad on buttons 12–15 and the left stick on axes
 * 0–1. A stick is only read past 0.5 so a resting thumb does not walk the grid.
 * The caller debounces: one press, one move.
 */
export function readPad(
  pads: (Gamepad | null | undefined)[],
  previous: PadMove = null,
): { move: PadMove; button: "a" | "b" | null } {
  const pad = pads.find((candidate): candidate is Gamepad =>
    Boolean(candidate?.connected),
  );
  if (!pad) return { move: null, button: null };
  const pressed = (index: number): boolean => Boolean(pad.buttons[index]?.pressed);
  const axis = (index: number): number => pad.axes[index] ?? 0;
  const held: PadMove =
    pressed(12) || axis(1) < -0.5
      ? "up"
      : pressed(13) || axis(1) > 0.5
        ? "down"
        : pressed(14) || axis(0) < -0.5
          ? "left"
          : pressed(15) || axis(0) > 0.5
            ? "right"
            : null;
  // A direction that is still held is not a new press.
  const move = held === previous ? null : held;
  const button = pressed(0) ? "a" : pressed(1) ? "b" : null;
  return { move, button };
}
