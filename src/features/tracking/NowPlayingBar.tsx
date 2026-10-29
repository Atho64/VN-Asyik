import { useEffect, useState } from "react";
import { Link } from "react-router";

import { useAppStore } from "@/app/store";
import { useI18n } from "@/app/useI18n";
import { Button } from "@/components/Button";
import { Icon } from "@/components/Icon";
import { api, IpcError, startPolling } from "@/lib/ipc/client";
import { formatDuration } from "@/lib/time";
import type { AndroidSessionView, TrackingStatus } from "@/lib/ipc/types";

/**
 * Live playtime bar.
 *
 * Rendered when a desktop run is open, a mapped game is detected, or an Android
 * per-game session is active. Shows active and away time as separate figures.
 */
export function NowPlayingBar() {
  const { t, locale } = useI18n();
  const pushToast = useAppStore((state) => state.pushToast);
  const cards = useAppStore((state) => state.cards);
  const setCards = useAppStore((state) => state.setCards);
  const isAndroid = useAppStore((state) => state.platform?.os === "android");
  // The platform is only known after boot. Polling before that would ask the
  // desktop command on Android (or the reverse) for as long as the bar is mounted.
  const ready = useAppStore((state) => state.platform !== null);
  const [status, setStatus] = useState<TrackingStatus | null>(null);
  const [androidSession, setAndroidSession] = useState<AndroidSessionView | null>(null);
  const [busy, setBusy] = useState(false);
  const [dismissed, setDismissed] = useState(false);

  useEffect(() => {
    if (!ready) return undefined;
    let cancelled = false;

    const read = async (): Promise<void> => {
      try {
        if (isAndroid) {
          const next = await api.trackingAndroidStatus();
          if (!cancelled) setAndroidSession(next);
        } else {
          const next = await api.trackingStatus();
          if (!cancelled) setStatus(next);
        }
      } catch {
        // Tracking status is a convenience; a failure must not spam the UI.
      }
    };

    const reconcileOnReturn = async (): Promise<void> => {
      if (!isAndroid || document.visibilityState === "hidden") return;
      try {
        const next = await api.trackingAndroidSession();
        if (!cancelled) {
          setAndroidSession(next);
          if (!next.active && next.countedMs > 0) {
            setCards(await api.listGames());
          }
        }
      } catch {
        // Ignore
      }
    };

    const onReturn = (): void => {
      void reconcileOnReturn();
    };

    // One request in flight at a time: a fixed interval queued a second status
    // call behind a slow one and let an older answer land last.
    const stopPolling = startPolling(read, 1_000);
    window.addEventListener("focus", onReturn);
    document.addEventListener("visibilitychange", onReturn);
    return () => {
      cancelled = true;
      stopPolling();
      window.removeEventListener("focus", onReturn);
      document.removeEventListener("visibilitychange", onReturn);
    };
  }, [isAndroid, ready, setCards]);

  if (dismissed) return null;

  if (isAndroid) {
    if (!androidSession?.active || !androidSession.gameId) return null;
    const game = cards.find((card) => card.game.meta.id === androidSession.gameId);
    const title = game?.game.title ?? androidSession.gameTitle ?? androidSession.gameId;
    const activeMs = androidSession.elapsedMs ?? androidSession.countedMs;

    const stopAndroid = async (): Promise<void> => {
      setBusy(true);
      try {
        const next = await api.trackingAndroidStop();
        setAndroidSession(next);
        setCards(await api.listGames());
      } catch (caught) {
        pushToast(
          "error",
          caught instanceof IpcError ? caught.message : t("common.error"),
        );
      } finally {
        setBusy(false);
      }
    };

    return (
      <div className="glass nowplaying" role="status" aria-live="polite">
        <span
          aria-hidden="true"
          style={{
            width: 8,
            height: 8,
            borderRadius: "50%",
            background: "var(--success)",
          }}
        />
        <div style={{ minWidth: 0, flex: 1 }}>
          <div
            className="row__title"
            style={{ overflow: "hidden", textOverflow: "ellipsis" }}
          >
            {title}
          </div>
          <div className="caption tnum">
            {`${t("tracking.tracking")} · ${t("tracking.active")} ${formatDuration(activeMs, locale)}`}
          </div>
        </div>
        <div style={{ display: "flex", gap: "var(--space-2)" }}>
          <Button
            variant="ghost"
            onClick={() => void stopAndroid()}
            disabled={busy}
          >
            {t("tracking.stop")}
          </Button>
        </div>
      </div>
    );
  }

  if (!status || !status.available) return null;

  const running = status.phase !== "idle" && status.game_id !== null;
  const detected = !running && status.candidates.length > 0;
  if (!running && !detected) return null;

  const game = cards.find((card) => card.game.meta.id === status.game_id);
  const title = game?.game.title ?? (detected ? status.candidates[0]?.exe_name : null);
  if (!title) return null;

  const act = async (action: () => Promise<void>): Promise<void> => {
    setBusy(true);
    try {
      await action();
      setStatus(await api.trackingStatus());
    } catch (caught) {
      pushToast(
        "error",
        caught instanceof IpcError ? caught.message : t("common.error"),
      );
    } finally {
      setBusy(false);
    }
  };

  return (
    <div
      className="glass nowplaying"
      role="status"
      aria-live="polite"
    >
      <span
        aria-hidden="true"
        style={{
          width: 8,
          height: 8,
          borderRadius: "50%",
          background:
            status.phase === "tracking"
              ? "var(--success)"
              : status.phase === "paused"
                ? "var(--warning)"
                : "var(--text-faint)",
        }}
      />

      <div style={{ minWidth: 0, flex: 1 }}>
        <div className="row__title" style={{ overflow: "hidden", textOverflow: "ellipsis" }}>
          {detected ? `${t("tracking.detected")}: ${title}` : title}
        </div>
        <div className="caption tnum">
          {detected
            ? t("tracking.startHint")
            : `${t(`tracking.${status.phase}`) } · ${t("tracking.active")} ${formatDuration(
                status.active_ms,
              )}${
                status.afk_ms > 0
                  ? ` · ${t("tracking.away")} ${formatDuration(status.afk_ms, locale)}`
                  : ""
              }`}
        </div>
      </div>

      {running ? (
        <div style={{ display: "flex", gap: "var(--space-2)" }}>
          {status.phase === "paused" ? (
            <Button
              variant="secondary"
              onClick={() => void act(() => api.trackingResume())}
              disabled={busy}
            >
              {t("tracking.resume")}
            </Button>
          ) : (
            <Button
              variant="secondary"
              onClick={() => void act(() => api.trackingPause())}
              disabled={busy}
            >
              {t("tracking.pause")}
            </Button>
          )}
          <Button
            variant="ghost"
            onClick={() => void act(() => api.trackingStop())}
            disabled={busy}
          >
            {t("tracking.stop")}
          </Button>
        </div>
      ) : (
        <Link className="btn btn--secondary" to="/tracking">
          {t("nav.tracking")}
        </Link>
      )}

      <button
        type="button"
        aria-label={t("common.dismiss")}
        onClick={() => setDismissed(true)}
        style={{
          color: "var(--text-muted)",
          display: "grid",
          placeItems: "center",
          minWidth: 32,
          minHeight: 32,
        }}
      >
        <Icon name="close" size={14} />
      </button>
    </div>
  );
}
