import { useEffect, useRef, type RefObject } from "react";

const FOCUSABLE_SELECTOR =
  "input, textarea, select, button, [href], [tabindex]:not([tabindex='-1'])";

/**
 * Traps keyboard Tab navigation inside an open modal dialog, handles Escape,
 * and restores focus to the trigger element when the dialog closes or unmounts.
 */
export function useModalFocusTrap(
  panelRef: RefObject<HTMLElement | null>,
  open: boolean,
  onClose: () => void,
  initialFocusRef?: RefObject<HTMLElement | null>,
): void {
  const previouslyFocused = useRef<HTMLElement | null>(null);

  useEffect(() => {
    if (!open) return undefined;

    previouslyFocused.current = document.activeElement as HTMLElement | null;
    const panel = panelRef.current;
    const initialTarget =
      initialFocusRef?.current ?? panel?.querySelector<HTMLElement>(FOCUSABLE_SELECTOR);
    initialTarget?.focus();

    const onKeyDown = (event: KeyboardEvent): void => {
      if (event.key === "Escape") {
        event.preventDefault();
        onClose();
        return;
      }
      if (event.key !== "Tab" || !panel) return;
      const items = [...panel.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR)].filter(
        (element) => !element.hasAttribute("disabled"),
      );
      if (items.length === 0) return;
      const first = items[0];
      const last = items[items.length - 1];
      if (!first || !last) return;
      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first.focus();
      }
    };

    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("keydown", onKeyDown);
      previouslyFocused.current?.focus();
    };
  }, [initialFocusRef, onClose, open, panelRef]);
}
