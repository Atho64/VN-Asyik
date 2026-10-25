import { useEffect } from "react";

import { useAppStore, type ToastKind } from "@/app/store";
import { useI18n } from "@/app/useI18n";
import { Icon } from "./Icon";

const DISMISS_AFTER_MS = 6000;

/**
 * Toasts for outcomes the user must know about: a save failed, an import skipped
 * duplicates. Not used for decoration or for things the UI already shows.
 */
export function Toasts() {
  const toasts = useAppStore((state) => state.toasts);
  const dismiss = useAppStore((state) => state.dismissToast);
  const { t } = useI18n();

  return (
    <div className="toasts" role="status" aria-live="polite">
      {toasts.map((toast) => (
        <ToastItem
          key={toast.id}
          kind={toast.kind}
          message={toast.message}
          onDismiss={() => dismiss(toast.id)}
          dismissLabel={t("common.dismiss")}
        />
      ))}
    </div>
  );
}

function ToastItem({
  kind,
  message,
  onDismiss,
  dismissLabel,
}: {
  kind: ToastKind;
  message: string;
  onDismiss: () => void;
  dismissLabel: string;
}) {
  useEffect(() => {
    const timer = window.setTimeout(onDismiss, DISMISS_AFTER_MS);
    return () => window.clearTimeout(timer);
  }, [onDismiss]);

  return (
    <div className={`toast toast--${kind}`}>
      <Icon name={kind === "error" ? "alert" : "check"} size={16} />
      <span>{message}</span>
      <button
        type="button"
        onClick={onDismiss}
        aria-label={dismissLabel}
        style={{
          marginLeft: "auto",
          color: "var(--text-muted)",
          display: "grid",
          placeItems: "center",
          minWidth: 24,
          minHeight: 24,
        }}
      >
        <Icon name="close" size={14} />
      </button>
    </div>
  );
}
