import { useEffect, useLayoutEffect, useRef, useState } from "react";

/** One entry in the menu. `null` renders a divider. */
export type MenuItem =
  | null
  | {
      label: string;
      onSelect: () => void;
      /** Shown greyed and not clickable. */
      disabled?: boolean;
    };

interface Position {
  x: number;
  y: number;
}

/**
 * The app's own right-click menu.
 *
 * WebView2's built-in menu (Back, Refresh, Save as, Print, Inspect) is turned
 * off at startup, so right-click would otherwise do nothing. This replaces it
 * with entries that mean something here.
 *
 * The menu is positioned in viewport coordinates and flipped when it would
 * overflow the bottom or right edge, because a menu that opens off-screen is
 * the same as no menu.
 */
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
  }, [position]);

  useEffect(() => {
    const close = (event: Event): void => {
      // Ignore the gesture that opened the menu. React registers this listener
      // on the same document the opening event is still propagating through, so
      // without this the menu closes in the frame it opened.
      if (ref.current?.contains(event.target as Node)) return;
      onClose();
    };
    const onKey = (event: KeyboardEvent): void => {
      if (event.key === "Escape") onClose();
    };
    // A pointerdown anywhere else dismisses, which is what every menu does.
    window.addEventListener("pointerdown", close);
    window.addEventListener("blur", close);
    window.addEventListener("keydown", onKey);
    window.addEventListener("resize", close);
    return () => {
      window.removeEventListener("pointerdown", close);
      window.removeEventListener("blur", close);
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("resize", close);
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
