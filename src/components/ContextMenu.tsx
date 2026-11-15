import { useEffect, useLayoutEffect, useRef, useState } from "react";

/** One entry in the menu. `null` renders a divider. */
export type MenuItem = null | {
  label: string;
  onSelect: () => void;
  /** Shown greyed and not clickable. */
  disabled?: boolean;
};

interface Position {
  x: number;
  y: number;
}

/** Viewport-clamped custom right-click menu replacing WebView2's disabled native menu. */
export function ContextMenu({
  position,
  items,
  onClose,
}: {
  position: Position;
  items: MenuItem[];
  onClose: () => void;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const returnFocus = useRef(
    document.activeElement instanceof HTMLElement ? document.activeElement : null,
  );
  const [placed, setPlaced] = useState<Position>(position);

  // Measure after paint, then nudge inside the viewport. useLayoutEffect so the
  // first frame the user sees is already in the right place.
  useLayoutEffect(() => {
    const box = ref.current?.getBoundingClientRect();
    if (!box) return;
    const margin = 8;
    const x = Math.min(position.x, window.innerWidth - box.width - margin);
    const y = Math.min(position.y, window.innerHeight - box.height - margin);
    setPlaced({ x: Math.max(margin, x), y: Math.max(margin, y) });
    ref.current?.querySelector<HTMLButtonElement>("button:not(:disabled)")?.focus();
  }, [position]);

  useEffect(() => {
    const previousFocus = returnFocus.current;
    const close = (event: Event): void => {
      // Ignore the gesture that opened the menu. React registers this listener
      // on the same document the opening event is still propagating through, so
      // without this the menu closes in the frame it opened.
      if (ref.current?.contains(event.target as Node)) return;
      onClose();
    };
    const onKey = (event: KeyboardEvent): void => {
      if (event.key === "Escape" || event.key === "Tab") {
        event.preventDefault();
        event.stopImmediatePropagation();
        onClose();
      } else if (["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) {
        event.preventDefault();
        event.stopImmediatePropagation();
        const buttons = Array.from(
          ref.current?.querySelectorAll<HTMLButtonElement>("button:not(:disabled)") ?? [],
        );
        const current = buttons.findIndex((button) => button === document.activeElement);
        const next =
          event.key === "Home"
            ? 0
            : event.key === "End"
              ? buttons.length - 1
              : (current + (event.key === "ArrowDown" ? 1 : -1) + buttons.length) %
                buttons.length;
        buttons[next]?.focus();
      }
    };
    // A pointerdown anywhere else dismisses, which is what every menu does.
    window.addEventListener("pointerdown", close);
    window.addEventListener("blur", close);
    window.addEventListener("keydown", onKey, true);
    window.addEventListener("resize", close);
    return () => {
      window.removeEventListener("pointerdown", close);
      window.removeEventListener("blur", close);
      window.removeEventListener("keydown", onKey, true);
      window.removeEventListener("resize", close);
      if (previousFocus?.isConnected) previousFocus.focus();
    };
  }, [onClose]);

  return (
    <div
      ref={ref}
      className="ctxmenu"
      role="menu"
      style={{ left: placed.x, top: placed.y }}
      // The menu itself must not be dismissed by the pointerdown that opened it,
      // and a click on a row should not also reach the surface underneath.
      onPointerDown={(event) => {
        event.stopPropagation();
      }}
      onContextMenu={(event) => {
        event.preventDefault();
        event.stopPropagation();
      }}
    >
      {items.map((item, index) =>
        item === null ? (
          // Dividers are structural, so the index is a stable enough key here.
          <div className="ctxmenu__divider" key={`d${index}`} role="separator" />
        ) : (
          <button
            key={item.label}
            type="button"
            role="menuitem"
            className="ctxmenu__item"
            disabled={item.disabled ?? false}
            onClick={() => {
              onClose();
              item.onSelect();
            }}
          >
            {item.label}
          </button>
        ),
      )}
    </div>
  );
}
