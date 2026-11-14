import { useEffect, useState } from "react";

import { useAppStore } from "@/app/store";
import { useI18n } from "@/app/useI18n";
import { Button } from "@/components/Button";
import { Icon } from "@/components/Icon";
import { api, IpcError } from "@/lib/ipc/client";
import type { CaptureStatus } from "@/lib/ipc/types";

/**
 * Capture button gated by backend `screenshotStatus` capability checks (platform
 * projection, active tracked window) and notifying `onCaptured` on completion.
 */
export function ScreenshotButton({
  gameId,
  onCaptured,
  variant = "secondary",
}: {
  /** Scope capture to this game so shots cannot be filed under an inactive game. */
  gameId?: string;
  onCaptured?: () => void;
  variant?: "secondary" | "ghost";
}) {
  const { t } = useI18n();
  const pushToast = useAppStore((state) => state.pushToast);
  const [status, setStatus] = useState<CaptureStatus | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let active = true;
    void api
      .screenshotStatus()
      .then((next) => {
        if (active) setStatus(next);
      })
      .catch(() => undefined);
    return () => {
      active = false;
    };
  }, []);

  const capture = async (): Promise<void> => {
    setBusy(true);
    try {
      await api.screenshotCapture();
      pushToast("success", t("screenshot.captured"));
      onCaptured?.();
    } catch (caught) {
      pushToast("error", describe(caught, t));
    } finally {
      setBusy(false);
    }
  };

  // Nothing to attribute a shot to: the button would only ever error, so it is
  // not offered at all rather than disabled with no explanation.
  if (status && !status.supported) return null;
  if (gameId && status?.gameId !== gameId) return null;

  return (
    <Button
      variant={variant}
      onClick={() => void capture()}
      disabled={busy || Boolean(status?.reason)}
      title={status?.reason ? describeReason(status.reason, t) : t("screenshot.capture")}
      aria-label={t("screenshot.capture")}
    >
      <Icon name="camera" size={16} />
      {t("screenshot.capture")}
    </Button>
  );
}

function describe(caught: unknown, t: (key: string) => string): string {
  if (!(caught instanceof IpcError)) return t("common.error");
  const key = `screenshot.error.${caught.code}`;
  const translated = t(key);
  return translated === key ? caught.message : translated;
}

function describeReason(reason: string, t: (key: string) => string): string {
  const key = `screenshot.reason.${reason}`;
  const translated = t(key);
  return translated === key ? reason : translated;
}
