import { useCallback, useEffect, useState } from "react";

import { useAppStore } from "@/app/store";
import { useI18n } from "@/app/useI18n";
import { Button } from "@/components/Button";
import { Card, CardHeader } from "@/components/Card";
import { Chip } from "@/components/Chip";
import { Icon } from "@/components/Icon";
import { api, IpcError } from "@/lib/ipc/client";
import { formatDurationLong } from "@/lib/time";
import type {
  ExecutableCandidate,
  GameExecutable,
  TrackerCapability,
  UsageAccessView,
} from "@/lib/ipc/types";
import { MapExecutable } from "@/features/tracking/MapExecutable";

/**
 * Tracking settings.
 *
 * The mapping is explicit by design: the user says which executable is which
 * game. The folder scan only lists candidates, largest first, and never maps
 * anything on its own, because guessing wrong silently corrupts playtime.
 */
export function TrackingScreen() {
  const { t } = useI18n();
  const cards = useAppStore((state) => state.cards);
  const pushToast = useAppStore((state) => state.pushToast);

  const [capability, setCapability] = useState<TrackerCapability | null>(null);
  const [usageAccess, setUsageAccess] = useState<UsageAccessView | null>(null);
  const [executables, setExecutables] = useState<GameExecutable[]>([]);
  const [folders, setFolders] = useState<string[]>([]);
  const [candidates, setCandidates] = useState<ExecutableCandidate[]>([]);
  const [scanning, setScanning] = useState<string | null>(null);
  const [folderInput, setFolderInput] = useState("");
  const [busy, setBusy] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);

  const refresh = useCallback(async (): Promise<void> => {
    try {
      const [cap, exes, dirs, usage] = await Promise.all([
        api.trackingCapability(),
        api.trackingListExecutables(),
        api.trackingListScanFolders(),
        api.trackingUsageAccess(),
      ]);
      setCapability(cap);
      setExecutables(exes);
      setFolders(dirs);
      setUsageAccess(usage);
      setLoadError(null);
    } catch (caught) {
      setLoadError(caught instanceof IpcError ? caught.message : t("common.error"));
    }
  }, [t]);

  // Initial load as an explicit async task, so the effect body does not call a
  // state-setting callback synchronously.
  useEffect(() => {
    let cancelled = false;
    const load = async (): Promise<void> => {
      try {
        const [cap, exes, dirs, usage] = await Promise.all([
          api.trackingCapability(),
          api.trackingListExecutables(),
          api.trackingListScanFolders(),
          api.trackingUsageAccess(),
        ]);
        if (cancelled) return;
        setCapability(cap);
        setExecutables(exes);
        setFolders(dirs);
        setUsageAccess(usage);
      } catch (caught) {
        if (!cancelled) {
          setLoadError(caught instanceof IpcError ? caught.message : t("common.error"));
        }
      }
    };
    void load();
    return () => {
      cancelled = true;
    };
  }, [t]);

  useEffect(() => {
    const onReturn = (): void => {
      if (document.visibilityState === "visible") void refresh();
    };
    window.addEventListener("focus", onReturn);
    document.addEventListener("visibilitychange", onReturn);
    return () => {
      window.removeEventListener("focus", onReturn);
      document.removeEventListener("visibilitychange", onReturn);
    };
  }, [refresh]);

  const fail = (caught: unknown): void => {
    pushToast("error", caught instanceof IpcError ? caught.message : t("common.error"));
  };

  const toggleEnabled = async (): Promise<void> => {
    if (!capability) return;
    setBusy(true);
    try {
      setCapability(await api.trackingSetEnabled(!capability.enabled));
    } catch (caught) {
      fail(caught);
    } finally {
      setBusy(false);
    }
  };

  const scan = async (path: string): Promise<void> => {
    setScanning(path);
    try {
      setCandidates(await api.trackingScanFolder(path));
    } catch (caught) {
      fail(caught);
    } finally {
      setScanning(null);
    }
  };

  const map = async (gameId: string, exePath: string): Promise<void> => {
    try {
      await api.trackingMapExecutable(gameId, exePath);
      await refresh();
      setCandidates((current) =>
        current.map((candidate) =>
          candidate.path === exePath
            ? { ...candidate, mapped_game_id: gameId }
            : candidate,
        ),
      );
    } catch (caught) {
      fail(caught);
    }
  };

  const remove = async (id: string): Promise<void> => {
    try {
      await api.trackingRemoveExecutable(id);
      await refresh();
    } catch (caught) {
      fail(caught);
    }
  };

  const openUsageSettings = async (): Promise<void> => {
    try {
      await api.trackingOpenUsageSettings();
    } catch (caught) {
      fail(caught);
    }
  };

  const addFolder = async (): Promise<void> => {
    if (folderInput.trim().length === 0) return;
    try {
      await api.trackingAddScanFolder(folderInput.trim());
      setFolderInput("");
      await refresh();
    } catch (caught) {
      fail(caught);
    }
  };

  if (!capability) {
    if (loadError) {
      return (
        <p className="notice notice--danger" role="alert">
          {loadError}
        </p>
      );
    }
    return (
      <div aria-busy="true" aria-label={t("common.loading")}>
        <div className="skeleton" style={{ height: 100, marginBottom: 16 }} />
        <div className="skeleton" style={{ height: 120 }} />
      </div>
    );
  }

  if (!capability.available && !usageAccess?.applicable) {
    return (
      <Card>
        <CardHeader title={t("tracking.title")} />
        <p className="h3" style={{ marginBottom: 8 }}>
          {t("tracking.unavailable")}
        </p>
        <p className="body">{capability.reason ?? t("tracking.unavailableReason")}</p>
      </Card>
    );
  }

  const gameTitle = (gameId: string): string =>
    cards.find((card) => card.game.meta.id === gameId)?.game.title ?? gameId;

  return (
    <div className="tracking-page">
      {/* Android only: playtime comes from the platform usage log, which needs a
          permission that has no runtime dialog. Rendered above the rest because
          without it nothing below can work. */}
      {usageAccess?.applicable ? (
        <Card>
          <CardHeader title={t("tracking.usageAccessTitle")} />
          {usageAccess.granted ? (
            <>
              <p className="body">{t("tracking.usageAccessGranted")}</p>
              <p className="settings__hint">{t("tracking.usageAccessGrantedHint")}</p>
            </>
          ) : (
            <>
              <p className="body" style={{ marginBottom: "var(--space-2)" }}>
                {t("tracking.usageAccessMissing")}
              </p>
              <p className="settings__hint" style={{ marginBottom: "var(--space-3)" }}>
                {usageAccess.reason ?? t("tracking.usageAccessWhy")}
              </p>
              <Button variant="primary" onClick={() => void openUsageSettings()}>
                {t("tracking.usageAccessOpen")}
              </Button>
            </>
          )}
        </Card>
      ) : null}

      {usageAccess?.applicable ? <AndroidSessionPanel /> : null}

      {capability.available ? (
        <>
          <Card>
            <CardHeader title={t("tracking.title")} />
            <p className="body" style={{ marginBottom: "var(--space-4)" }}>
              {t("tracking.intro")}
            </p>

            <div className="settings__row">
              <span className="settings__label">
                {t("tracking.enable")}
                <span className="settings__hint">{t("tracking.enableHint")}</span>
              </span>
              <div className="settings__control">
                {/* State, not a verb: the chip reports whether tracking is on, so the
                    filled style and the label agree and neither has to be guessed. */}
                <Chip
                  selected={capability.enabled}
                  onClick={() => void toggleEnabled()}
                  ariaLabel={t("tracking.enable")}
                >
                  {capability.enabled ? t("tracking.on") : t("tracking.off")}
                </Chip>
              </div>
            </div>
            <p className="caption">
              {t("tracking.poll", {
                seconds: Math.round(capability.poll_interval_ms / 1000),
              })}
            </p>
          </Card>
        </>
      ) : null}

      <Card>
        <CardHeader
          title={t(
            usageAccess?.applicable ? "tracking.linkedApps" : "tracking.executables",
          )}
          aside={
            <span className="caption">
              {t("tracking.mapped", { count: executables.length })}
            </span>
          }
        />
        <MapExecutable games={cards} onMapped={refresh} />
        {executables.length === 0 ? (
          <p className="body">{t("tracking.executablesEmpty")}</p>
        ) : (
          // The track is capped (`minmax(0, 1fr)`): an implicit grid column
          // sizes to the row's max-content, and a long title plus its actions
          // then pushed the whole card wider than a phone's viewport.
          <ul className="executable-list">
            {executables.map((entry) => (
              <li key={entry.id} className="tracking__row tracking__row--divided">
                <Icon name="library" size={14} />
                <span style={{ minWidth: 0, flex: 1 }}>
                  <span
                    className="row__title"
                    style={{
                      display: "flex",
                      alignItems: "center",
                      gap: "var(--space-2)",
                    }}
                  >
                    <span>{gameTitle(entry.game_id)}</span>
                    {entry.runner_kind &&
                    (!usageAccess?.applicable || entry.runner_kind !== "custom") ? (
                      <span
                        className="chip"
                        style={{ fontSize: "var(--font-size-xs)", padding: "2px 6px" }}
                      >
                        {entry.runner_kind.toUpperCase()}
                      </span>
                    ) : null}
                  </span>
                  <span className="row__sub" title={entry.exe_path ?? ""}>
                    {entry.exe_path ?? entry.exe_name}
                    {entry.launch_target ? ` • ${entry.launch_target}` : ""}
                    {entry.title_match ? ` [title: ${entry.title_match}]` : ""}
                  </span>
                </span>
                <Button
                  variant="ghost"
                  onClick={() => void remove(entry.id)}
                  disabled={busy}
                >
                  {t("tracking.remove")}
                </Button>
              </li>
            ))}
          </ul>
        )}
      </Card>

      {capability.available ? (
        <Card>
          <CardHeader title={t("tracking.folders")} />
          <div className="tracking__input-row">
            <input
              className="input"
              value={folderInput}
              onChange={(event) => setFolderInput(event.target.value)}
              placeholder="F:/Visual Novels"
              aria-label={t("tracking.addFolder")}
            />
            <Button variant="secondary" onClick={() => void addFolder()}>
              {t("tracking.addFolder")}
            </Button>
          </div>

          {folders.length === 0 ? null : (
            <ul className="executable-list">
              {folders.map((folder) => (
                <li key={folder} className="tracking__row">
                  <span className="row__sub" style={{ flex: 1 }}>
                    {folder}
                  </span>
                  <Button
                    variant="secondary"
                    onClick={() => void scan(folder)}
                    disabled={scanning !== null}
                  >
                    {scanning === folder ? t("tracking.scanning") : t("tracking.scan")}
                  </Button>
                  <Button
                    variant="ghost"
                    onClick={() => {
                      void api.trackingRemoveScanFolder(folder).then(refresh).catch(fail);
                    }}
                  >
                    {t("tracking.remove")}
                  </Button>
                </li>
              ))}
            </ul>
          )}

          {candidates.length > 0 ? (
            <>
              <p className="caption" style={{ marginBottom: 8 }}>
                {t("tracking.scanHint")}
              </p>
              <ul style={{ display: "grid", gap: 8 }}>
                {candidates.map((candidate) => (
                  <li
                    key={candidate.path}
                    className="tracking__row tracking__row--divided"
                  >
                    <span style={{ minWidth: 0, flex: 1 }}>
                      <span className="row__title">{candidate.exe_name}</span>
                      <span className="row__sub" title={candidate.path}>
                        {candidate.path} · {Math.round(candidate.size_bytes / 1_048_576)}{" "}
                        MB
                      </span>
                    </span>
                    {candidate.mapped_game_id ? (
                      <Chip>{t("tracking.alreadyMapped")}</Chip>
                    ) : (
                      <select
                        className="select"
                        style={{ width: "auto" }}
                        defaultValue=""
                        aria-label={t("tracking.use")}
                        onChange={(event) => {
                          const gameId = event.target.value;
                          if (gameId) void map(gameId, candidate.path);
                        }}
                      >
                        <option value="">{t("tracking.use")}</option>
                        {cards.map((card) => (
                          <option key={card.game.meta.id} value={card.game.meta.id}>
                            {card.game.title}
                          </option>
                        ))}
                      </select>
                    )}
                  </li>
                ))}
              </ul>
            </>
          ) : scanning ? null : folders.length > 0 ? (
            <p className="body">{t("tracking.scanEmpty")}</p>
          ) : null}
        </Card>
      ) : null}
    </div>
  );
}

function AndroidSessionPanel() {
  const { t, locale } = useI18n();
  const cards = useAppStore((state) => state.cards);
  const setCards = useAppStore((state) => state.setCards);
  const [session, setSession] = useState<Awaited<
    ReturnType<typeof api.trackingAndroidSession>
  > | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let active = true;
    const load = async (): Promise<void> => {
      if (document.visibilityState === "hidden") return;
      try {
        const next = await api.trackingAndroidSession();
        if (active) {
          setSession(next);
          setError(next.reason);
        }
      } catch (caught) {
        if (active)
          setError(caught instanceof IpcError ? caught.message : t("common.error"));
      }
    };
    void load();
    const timer = window.setInterval(() => void load(), 2_000);
    return () => {
      active = false;
      window.clearInterval(timer);
    };
  }, [t]);

  const stop = async (): Promise<void> => {
    setBusy(true);
    try {
      const next = await api.trackingAndroidStop();
      setSession(next);
      setError(next.reason);
      setCards(await api.listGames());
    } catch (caught) {
      setError(caught instanceof IpcError ? caught.message : t("common.error"));
    } finally {
      setBusy(false);
    }
  };
  const title = cards.find((card) => card.game.meta.id === session?.gameId)?.game.title;

  return (
    <Card>
      <CardHeader title={t("tracking.androidSession")} />
      <p className="card__intro">{t("tracking.androidSessionHint")}</p>
      {session?.gameId ? (
        <div className="settings__row">
          <span className="settings__label">
            {title ?? session.gameId}
            <span className="settings__hint">{session.package}</span>
            {!error ? (
              <span className="settings__hint">
                {t(
                  session.active
                    ? "tracking.androidSessionActive"
                    : "tracking.androidSessionFinished",
                )}
              </span>
            ) : null}
          </span>
          <span className="tnum">{formatDurationLong(session.countedMs, locale)}</span>
          {session.active ? (
            <Button disabled={busy} onClick={() => void stop()}>
              {t("tracking.stop")}
            </Button>
          ) : null}
        </div>
      ) : (
        <p className="settings__hint">{t("tracking.androidSessionEmpty")}</p>
      )}
      {error ? (
        <p className="notice notice--danger" role="alert">
          {error}
        </p>
      ) : null}
    </Card>
  );
}
