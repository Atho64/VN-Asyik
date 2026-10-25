import type { ReactNode } from "react";

import { useI18n } from "@/app/useI18n";
import type { GameStatus } from "@/lib/ipc/types";

interface ChipProps {
  children: ReactNode;
  selected?: boolean;
  onClick?: () => void;
  className?: string;
  /** Accessible name when the visible label alone would be ambiguous. */
  ariaLabel?: string;
}

/** A selectable filter chip. Renders a real button so it is keyboard usable. */
export function Chip({ children, selected, onClick, className, ariaLabel }: ChipProps) {
  const classes = ["chip"];
  if (className) classes.push(className);

  if (!onClick) {
    return <span className={classes.join(" ")}>{children}</span>;
  }

  return (
    <button
      type="button"
      className={classes.join(" ")}
      aria-pressed={selected ?? false}
      aria-label={ariaLabel}
      onClick={onClick}
    >
      {children}
    </button>
  );
}

/**
 * Status chip. The status is communicated by the label text and a small ring
 * whose colour carries the same meaning, so it never depends on colour alone.
 */
export function StatusChip({ status }: { status: GameStatus }) {
  const { t } = useI18n();
  return (
    <span className={`chip chip--${status.replace("_", "-")}`}>
      <span className="chip__status" aria-hidden="true" />
      {t(`status.${status}`)}
    </span>
  );
}
