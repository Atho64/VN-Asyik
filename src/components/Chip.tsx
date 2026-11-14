import type { ReactNode } from "react";

import { useI18n } from "@/app/useI18n";
import type { GameStatus } from "@/lib/ipc/types";
import { Icon, type IconName } from "./Icon";

interface ChipProps {
  children: ReactNode;
  selected?: boolean;
  onClick?: () => void;
  className?: string;
  /** Accessible name when the visible label alone would be ambiguous. */
  ariaLabel?: string;
  role?: "tab";
  /**
   * A chip whose choice is currently overridden by something else. Disabled
   * rather than hidden: the control still explains what it would do, and a
   * disabled button is announced as unavailable instead of silently ignoring
   * the click.
   */
  disabled?: boolean;
}

/** A selectable filter chip. Renders a real button so it is keyboard usable. */
export function Chip({
  children,
  selected,
  onClick,
  className,
  ariaLabel,
  role,
  disabled,
}: ChipProps) {
  const classes = ["chip"];
  if (className) classes.push(className);

  if (!onClick) {
    return <span className={classes.join(" ")}>{children}</span>;
  }

  return (
    <button
      type="button"
      role={role}
      className={classes.join(" ")}
      aria-selected={role === "tab" ? (selected ?? false) : undefined}
      aria-pressed={role === "tab" ? undefined : (selected ?? false)}
      aria-label={ariaLabel}
      disabled={disabled}
      onClick={onClick}
    >
      {children}
    </button>
  );
}

const statusIcons: Record<GameStatus, IconName> = {
  playing: "guide",
  completed: "check",
  wishlist: "clock",
  on_hold: "pause",
  dropped: "close",
};

export function StatusChip({ status }: { status: GameStatus }) {
  const { t } = useI18n();
  return (
    <span className={`status-chip status-chip--${status.replace("_", "-")}`}>
      <Icon name={statusIcons[status]} className="status-chip__icon" />
      <span className="status-chip__label">{t(`status.${status}`)}</span>
    </span>
  );
}
