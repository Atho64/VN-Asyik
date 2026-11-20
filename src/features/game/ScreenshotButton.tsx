import { useEffect, useRef, useState } from "react";

import { useAppStore } from "@/app/store";
import { useI18n } from "@/app/useI18n";
import { Button } from "@/components/Button";
import { Icon } from "@/components/Icon";
import { api, IpcError } from "@/lib/ipc/client";
import type { CaptureStatus } from "@/lib/ipc/types";
import { requestAchievementsRefresh } from "@/lib/ipc/achievements";

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
  const os = useAppStore((state) => state.platform?.os);
  const [availability, setAvailability] = useState<{
    scope: string | undefined;
    status: CaptureStatus;
  } | null>(null);
  const status =
    availability && availability.scope === gameId ? availability.status : null;
  const [busy, setBusy] = useState(false);
  const refreshAvailability = useRef<() => void>(() => undefined);

  useEffect(() => {
    let active = true;
    let revision = 0;
    const refresh = (): void => {
      if (document.visibilityState === "hidden") return;
      const requested = ++revision;
      void api
        .screenshotStatus()
        .then((next) => {
          if (active && requested === revision) {
            setAvailability({ scope: gameId, status: next });
          }
        })
        .catch(() => undefined);
    };
    refreshAvailability.current = refresh;
    refresh();
    window.addEventListener("focus", refresh);
    document.addEventListener("visibilitychange", refresh);
    return () => {
      active = false;
      window.removeEventListener("focus", refresh);
      document.removeEventListener("visibilitychange", refresh);
      refreshAvailability.current = () => undefined;
    };
  }, [gameId]);

  const capture = async (): Promise<void> => {
    setBusy(true);
    try {
      await api.screenshotCapture(gameId);
      requestAchievementsRefresh();
      pushToast("success", t("screenshot.captured"));
      onCaptured?.();
    } catch (caught) {
      if (!(caught instanceof IpcError && caught.code === "screenshot_cancelled")) {
        pushToast("error", describe(caught, t));
      }
    } finally {
      setBusy(false);
      refreshAvailability.current();
    }
  };

  // Nothing to attribute a shot to: the button would only ever error, so it is
  // not offered at all rather than disabled with no explanation.
  if (status && !status.supported) return null;
  if (gameId && status?.gameId !== gameId) return null;

  return (
    <div className="screenshot-capture">
      <Button
        variant={variant}
        onClick={() => void capture()}
        disabled={!status || busy || Boolean(status.reason)}
        title={
          status?.reason
            ? describeReason(status.reason, t)
            : t(os === "windows" ? "screenshot.shortcutHint" : "screenshot.capture")
        }
        aria-label={t("screenshot.capture")}
      >
        <Icon name="camera" size={16} />
        {t("screenshot.capture")}
      </Button>
      {os === "android" && (
        <p className="caption" role={busy ? "status" : undefined}>
          {t(busy ? "screenshot.androidWaiting" : "screenshot.androidHint")}
        </p>
      )}
    </div>
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
