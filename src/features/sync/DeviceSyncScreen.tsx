import { useCallback, useEffect, useRef, useState } from "react";

import { useAppStore } from "@/app/store";
import { useI18n } from "@/app/useI18n";
import { Button } from "@/components/Button";
import { Card, CardHeader } from "@/components/Card";
import { Chip } from "@/components/Chip";
import { EmptyState } from "@/components/EmptyState";
import { Sheet } from "@/components/Sheet";
import { api, IpcError } from "@/lib/ipc/client";
import type {
  ConflictView,
  DataImportPreview,
  SyncOutcome,
  SyncStatus,
} from "@/lib/ipc/types";
import { formatDateTime } from "@/lib/time";

/** Backup and device transfers share one native picker and a reviewed import. */
export function DeviceSyncScreen() {
  const { t, locale } = useI18n();
  const pushToast = useAppStore((state) => state.pushToast);
  const isAndroid = useAppStore((state) => state.platform?.os === "android");
  const setCards = useAppStore((state) => state.setCards);
  const setPrefs = useAppStore((state) => state.setPrefs);

  const [status, setStatus] = useState<SyncStatus | null>(null);
  const [conflicts, setConflicts] = useState<ConflictView[]>([]);
  const [folder, setFolder] = useState("");
  const [busy, setBusy] = useState(false);
  const [lastOutcome, setLastOutcome] = useState<SyncOutcome | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [restore, setRestore] = useState(false);
  const [preview, setPreview] = useState<DataImportPreview | null>(null);
  const [reopenImport, setReopenImport] = useState(false);
  const [needsRefresh, setNeedsRefresh] = useState<"backup" | "transfer" | null>(null);
  const mountedRef = useRef(false);
  const busyRef = useRef(false);
  const draftRef = useRef<DataImportPreview | null>(null);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      const draft = draftRef.current;
      draftRef.current = null;
      if (draft) void api.dataCancelImport(draft.id).catch(() => undefined);
    };
  }, []);

  const refresh = useCallback(async (): Promise<boolean> => {
    if (!mountedRef.current) return false;
    try {
      const [nextStatus, nextConflicts] = await Promise.all([
        api.syncStatus(),
        api.syncConflicts(50),
      ]);
      if (!mountedRef.current) return false;
      setLoadError(null);
      setStatus(nextStatus);
      setConflicts(nextConflicts);
      setFolder((current) => (current.length > 0 ? current : (nextStatus.folder ?? "")));
      return true;
    } catch (caught) {
      if (mountedRef.current) {
        setLoadError(caught instanceof IpcError ? caught.message : t("common.error"));
      }
      return false;
    } finally {
      if (mountedRef.current) setLoading(false);
    }
  }, [t]);

  useEffect(() => {
    let active = true;
    void Promise.resolve().then(() => {
      if (active) void refresh();
    });
    return () => {
      active = false;
    };
  }, [refresh]);

  const begin = (): boolean => {
    if (busyRef.current || !mountedRef.current) return false;
    busyRef.current = true;
    setBusy(true);
    setError(null);
    setReopenImport(false);
    return true;
  };

  const finish = (): void => {
    busyRef.current = false;
    if (mountedRef.current) setBusy(false);
  };

  const refreshImported = async (kind: "backup" | "transfer"): Promise<void> => {
    const [cards, prefs, sync] = await Promise.allSettled([
      api.listGames(),
      kind === "backup" ? api.getPrefs() : Promise.resolve(null),
      refresh(),
    ]);
    if (!mountedRef.current) return;
    if (cards.status === "fulfilled") setCards(cards.value);
    if (prefs.status === "fulfilled" && prefs.value) setPrefs(prefs.value);
    const failed =
      cards.status === "rejected" ||
      prefs.status === "rejected" ||
      sync.status === "rejected" ||
      !sync.value;
    setNeedsRefresh(failed ? kind : null);
  };

  const runSync = async (): Promise<void> => {
    if (draftRef.current || folder.trim().length === 0 || !begin()) return;
    try {
      const outcome = await api.syncRun(folder.trim());
      if (!mountedRef.current) return;
      setLastOutcome(outcome);
      pushToast("success", t("devices.done"));
      await refreshImported("transfer");
    } catch (caught) {
      // A failed sync must say so. Reporting nothing would look identical to a
      // sync that found no changes.
      if (mountedRef.current) {
        setError(caught instanceof IpcError ? caught.message : t("common.error"));
      }
    } finally {
      finish();
    }
  };

  const exportFile = async (share: boolean): Promise<void> => {
    if (draftRef.current || !begin()) return;
    try {
      if (share) {
        await api.dataShareFile();
      } else {
        const saved = await api.dataExportFile();
        if (saved && mountedRef.current) pushToast("success", t("data.fileSaved"));
      }
    } catch (caught) {
      if (mountedRef.current) {
        setError(caught instanceof IpcError ? caught.message : t("common.error"));
      }
    } finally {
      finish();
    }
  };

  const openImport = async (): Promise<void> => {
    if (draftRef.current || !begin()) return;
    try {
      const next = await api.dataOpenFile();
      if (!mountedRef.current) {
        if (next) await api.dataCancelImport(next.id);
        return;
      }
      draftRef.current = next;
      setRestore(false);
      setPreview(next);
    } catch (caught) {
      if (mountedRef.current) {
        setError(caught instanceof IpcError ? caught.message : t("data.openFailed"));
        setReopenImport(true);
      }
    } finally {
      finish();
    }
  };

  // Sheet's focus trap depends on this callback: keep its identity stable.
  const closePreview = useCallback((): void => {
    if (busyRef.current) return;
    const draft = draftRef.current;
    if (!draft) return;
    busyRef.current = true;
    draftRef.current = null;
    setPreview(null);
    setBusy(true);
    void api
      .dataCancelImport(draft.id)
      .finally(() => {
        busyRef.current = false;
        if (mountedRef.current) setBusy(false);
      })
      .catch(() => undefined);
  }, []);

  const applyImport = async (): Promise<void> => {
    const draft = draftRef.current;
    if (!draft || !begin()) return;
    // Apply consumes the token on success or failure. It cannot safely be reused.
    draftRef.current = null;
    try {
      const result = await api.dataApplyImport(draft.id, restore ? "restore" : "merge");
      if (!mountedRef.current) return;
      setPreview(null);
      if (result.kind === "backup") {
        pushToast(
          "success",
          t("settings.imported", {
            games: result.summary.games,
            sessions: result.summary.sessions,
            skipped: result.summary.sessions_skipped,
          }),
        );
      } else {
        setLastOutcome(result.outcome);
        pushToast("success", t("devices.done"));
      }
      await refreshImported(result.kind);
    } catch (caught) {
      if (mountedRef.current) {
        setPreview(null);
        setError(caught instanceof IpcError ? caught.message : t("data.applyFailed"));
        setReopenImport(true);
      }
    } finally {
      finish();
    }
  };

  return (
    <div className="data-screen">
      <Card>
        <CardHeader title={t("settings.tabTransfer")} />
        <p className="card__intro">{t("data.intro")}</p>

        <div className="data-actions">
          <Button
            disabled={busy || preview !== null}
            onClick={() => void exportFile(false)}
          >
            {t("data.exportFile")}
          </Button>
          {isAndroid ? (
            <Button
              disabled={busy || preview !== null}
              onClick={() => void exportFile(true)}
            >
              {t("devices.shareFile")}
            </Button>
          ) : null}
          <Button disabled={busy || preview !== null} onClick={() => void openImport()}>
            {t("data.importFile")}
          </Button>
        </div>
        <p className="field__hint data-import-hint">{t("data.importHint")}</p>

        {error ? (
          <p className="field__error" role="alert">
            {error}
          </p>
        ) : null}
        {reopenImport ? (
          <div className="data-notice">
            <p className="field__hint">{t("data.reopenHint")}</p>
            <Button disabled={busy} onClick={() => void openImport()}>
              {t("data.reopenFile")}
            </Button>
          </div>
        ) : null}
        {needsRefresh ? (
          <div className="data-notice" role="status">
            <p className="field__hint">{t("data.refreshFailed")}</p>
            <Button
              disabled={busy}
              onClick={() => {
                if (begin()) void refreshImported(needsRefresh).finally(finish);
              }}
            >
              {t("common.retry")}
            </Button>
          </div>
        ) : null}
      </Card>

      {!isAndroid ? (
        <Card>
          <CardHeader title={t("data.folderTitle")} />
          <p className="card__intro">{t("devices.intro")}</p>
          <label className="field" style={{ marginTop: "var(--space-4)" }}>
            <span className="field__label">{t("devices.folder")}</span>
            <input
              className="input"
              value={folder}
              onChange={(event) => setFolder(event.target.value)}
              placeholder={t("devices.folderPlaceholder")}
              spellCheck={false}
            />
            <span className="field__hint">{t("devices.folderHint")}</span>
          </label>

          <div
            style={{
              display: "flex",
              gap: "var(--space-2)",
              marginTop: "var(--space-3)",
            }}
          >
            <Button
              variant="primary"
              onClick={() => void runSync()}
              disabled={busy || preview !== null || folder.trim().length === 0}
            >
              {busy ? t("devices.syncing") : t("devices.sync")}
            </Button>
          </div>
        </Card>
      ) : null}

      {loadError ? (
        <Card>
          <p className="field__error" role="alert">
            {loadError}
          </p>
          <Button disabled={busy || loading} onClick={() => void refresh()}>
            {t("common.retry")}
          </Button>
        </Card>
      ) : loading ? (
        <p className="body" role="status">
          {t("common.loading")}
        </p>
      ) : status ? (
        <Card>
          <CardHeader title={t("devices.statusTitle")} />
          <dl className="stat-list">
            <div className="stat-list__row">
              <dt>{t("devices.thisDevice")}</dt>
              <dd>
                <code>{status.deviceId}</code>
              </dd>
            </div>
            <div className="stat-list__row">
              <dt>{t("devices.lastSync")}</dt>
              <dd>
                {status.lastSyncAtMs === null
                  ? t("devices.never")
                  : formatDateTime(status.lastSyncAtMs, locale)}
              </dd>
            </div>
            <div className="stat-list__row">
              <dt>{t("devices.contents")}</dt>
              <dd>
                {t("devices.counts")
                  .replace("{games}", String(status.games))
                  .replace("{chapters}", String(status.chapters))
                  .replace("{sessions}", String(status.sessions))}
              </dd>
            </div>
          </dl>
        </Card>
      ) : null}

      {lastOutcome ? (
        <Card>
          <CardHeader title={t("devices.resultTitle")} />
          <div style={{ display: "flex", gap: "var(--space-2)", flexWrap: "wrap" }}>
            <Chip>
              {t("devices.peers").replace("{count}", String(lastOutcome.peers))}
            </Chip>
            <Chip>
              {t("devices.pulled").replace("{count}", String(lastOutcome.pulled))}
            </Chip>
            <Chip>
              {t("devices.pushed").replace("{count}", String(lastOutcome.pushed))}
            </Chip>
          </div>
          {lastOutcome.skipped.length > 0 ? (
            <ul
              className="body"
              style={{ marginTop: "var(--space-3)", paddingLeft: "1.1em" }}
            >
              {lastOutcome.skipped.map((skipped) => (
                <li key={skipped.file}>
                  {skipped.file}: {skipped.reason}
                </li>
              ))}
            </ul>
          ) : null}
        </Card>
      ) : null}

      {status && !loading && !loadError ? (
        <Card>
          <CardHeader title={t("devices.conflictsTitle")} />
          {conflicts.length === 0 ? (
            <EmptyState
              title={t("devices.noConflicts")}
              body={t("devices.noConflictsBody")}
            />
          ) : (
            <>
              <p className="card__intro">{t("devices.conflictsIntro")}</p>
              <ul className="conflict-list">
                {conflicts.map((conflict) => (
                  <li
                    key={`${conflict.recordId}-${conflict.createdAtMs}`}
                    className="conflict-row"
                  >
                    <div className="conflict-row__main">
                      <span className="conflict-row__what">
                        {conflict.table} · {conflict.recordId.slice(0, 8)}
                      </span>
                      <span className="conflict-row__when">
                        {formatDateTime(conflict.createdAtMs, locale)}
                      </span>
                    </div>
                    <Chip>{t(`devices.kept.${conflict.kept}`)}</Chip>
                  </li>
                ))}
              </ul>
            </>
          )}
        </Card>
      ) : null}

      <Sheet
        open={preview !== null}
        title={t("data.preview")}
        onClose={closePreview}
        footer={
          preview ? (
            <div className="data-actions">
              <Button disabled={busy} onClick={closePreview}>
                {t("common.close")}
              </Button>
              <Button
                variant="primary"
                disabled={busy}
                onClick={() => void applyImport()}
              >
                {busy ? t("data.applying") : t(restore ? "data.restore" : "data.merge")}
              </Button>
            </div>
          ) : null
        }
      >
        {preview ? (
          <>
            <dl className="stat-list">
              <div className="stat-list__row">
                <dt>{t("data.createdAt")}</dt>
                <dd>
                  {new Intl.DateTimeFormat(locale === "id" ? "id-ID" : "en-GB", {
                    dateStyle: "medium",
                    timeStyle: "short",
                  }).format(new Date(preview.createdAtMs))}
                </dd>
              </div>
              {(["games", "chapters", "sessions", "endings"] as const).map((name) => (
                <div className="stat-list__row" key={name}>
                  <dt>{t(`data.count.${name}`)}</dt>
                  <dd>{preview[name]}</dd>
                </div>
              ))}
            </dl>
            <p className="data-preview-hint">
              {t(restore ? "data.importBehavior.backup" : "data.importBehavior.transfer")}
            </p>
            {preview.canRestore ? (
              <details className="data-advanced">
                <summary>{t("data.advanced")}</summary>
                <label className="data-restore-option">
                  <input
                    type="checkbox"
                    checked={restore}
                    disabled={busy}
                    onChange={(event) => setRestore(event.target.checked)}
                  />
                  <span>{t("data.restoreOption")}</span>
                </label>
              </details>
            ) : null}
          </>
        ) : null}
      </Sheet>
    </div>
  );
}
