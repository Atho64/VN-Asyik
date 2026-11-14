import { useEffect, useId, useRef, type ReactNode } from "react";
import { createPortal } from "react-dom";

import { useI18n } from "@/app/useI18n";
import { useModalFocusTrap } from "@/lib/useModalFocusTrap";
import { Button } from "./Button";
import { Icon } from "./Icon";

interface SheetProps {
  open: boolean;
  title: string;
  onClose: () => void;
  children: ReactNode;
  footer?: ReactNode;
  /**
   * Allow the panel to grow past its default 560px.
   *
   * Used by the screenshot lightbox, where the content is a picture rather than a
   * column of text: at 560px an 800x600 screenshot is smaller than the tile that
   * opened it, which makes the enlargement pointless.
   */
  wide?: boolean;
}

/**
 * Modal sheet overlay with focus trapping, Escape dismissal, and focus restoration.
 */
export function Sheet({ open, title, onClose, children, footer, wide = false }: SheetProps) {
  const { t } = useI18n();
  const titleId = useId();
  const panelRef = useRef<HTMLDivElement>(null);

  useModalFocusTrap(panelRef, open, onClose);

  useEffect(() => {
    if (!open) return undefined;
    const { overflow } = document.body.style;
    document.body.style.overflow = "hidden";
    return () => {
      document.body.style.overflow = overflow;
    };
  }, [open]);

  if (!open) return null;

  return createPortal(
    <div
      className="scrim"
      role="presentation"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <div
        ref={panelRef}
        className={wide ? "sheet sheet--wide" : "sheet"}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
      >
        <header className="sheet__header">
          <h2 className="h3" id={titleId}>
            {title}
          </h2>
          <Button
            variant="ghost"
            size="icon"
            onClick={onClose}
            aria-label={t("common.close")}
          >
            <Icon name="close" />
          </Button>
        </header>
        <div className="sheet__body">{children}</div>
        {footer ? <footer className="sheet__footer">{footer}</footer> : null}
      </div>
    </div>,
    document.body,
  );
}
