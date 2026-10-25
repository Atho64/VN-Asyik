import { useCallback, useEffect, useState } from "react";

import { useAppStore } from "@/app/store";
import { useI18n } from "@/app/useI18n";
import { Button } from "@/components/Button";
import { Card, CardHeader } from "@/components/Card";
import { Chip } from "@/components/Chip";
import { api, IpcError } from "@/lib/ipc/client";
import type {
  LogEntry,
  BulkImportOutcome,
  PullResult,
  QueueEntry,
  SyncRow,
  VndbStatus,
} from "@/lib/ipc/types";
import { formatDateTime } from "@/lib/time";
import {
  decideAll,
  summarize,
  toDecisionsPayload,
  undecidedCount,
  type DecisionMap,
  type Resolution,
} from "./syncView";

/**
 * The sync screen.
 *
 * Two things it refuses to do: write anything before the user has seen the diff,
 * and hide a failure. Every conflict is shown with both values, and a queued push
 * that failed stays visible with its error.
 */
export function SyncScreen() {
  const { t, locale } = useI18n();
  const pushToast = useAppStore((state) => state.pushToast);
  const setCards = useAppStore((state) => state.setCards);

  const [status, setStatus] = useState<VndbStatus | null>(null);
  const [token, setToken] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  const [pull, setPull] = useState<PullResult | null>(null);
  const [imported, setImported] = useState<BulkImportOutcome | null>(null);
  const [decisions, setDecisions] = useState<DecisionMap>({});
  const [queue, setQueue] = useState<QueueEntry[]>([]);
  const [log, setLog] = useState<LogEntry[]>([]);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async (): Promise<void> => {
    try {
      const [nextStatus, nextQueue, nextLog] = await Promise.all([
        api.vndbStatus(),
        api.vndbQueue(),
        api.vndbLog(),
      ]);
      setStatus(nextStatus);
      setQueue(nextQueue);
      setLog(nextLog);
      setError(null);
    } catch (caught) {
      setError(caught instanceof IpcError ? caught.message : t("common.error"));
    }
  }, [t]);

  // The initial load is an explicit async task rather than an effect body that
  // calls a stateful callback, which is what the hook rules warn about.
  useEffect(() => {
    let cancelled = false;
    const load = async (): Promise<void> => {
      try {
        const [nextStatus, nextQueue, nextLog] = await Promise.all([
          api.vndbStatus(),
          api.vndbQueue(),
          api.vndbLog(),
        ]);
        if (cancelled) return;
        setStatus(nextStatus);
        setQueue(nextQueue);
        setLog(nextLog);
      } catch (caught) {
        if (!cancelled) {
          setError(caught instanceof IpcError ? caught.message : t("common.error"));
        }
      }
    };
    void load();
    return () => {
      cancelled = true;
    };
  }, [t]);

  const fail = (caught: unknown, fallback: string): void => {
    const message = caught instanceof IpcError ? caught.message : fallback;
    setError(message);
    pushToast("error", message);
  };

  const connect = async (): Promise<void> => {
    setBusy("connect");
    setError(null);
    try {
      await api.vndbConnect(token.trim());
      setToken("");
      await refresh();
    } catch (caught) {
      fail(
        caught,
        caught instanceof IpcError && caught.code === "unauthorized"
          ? t("sync.invalidToken")
          : t("sync.offline"),
      );
    } finally {
      setBusy(null);
    }
  };

  const disconnect = async (): Promise<void> => {
    setBusy("disconnect");
    try {
      await api.vndbDisconnect();
      setPull(null);
      setDecisions({});
      await refresh();
    } catch (caught) {
      fail(caught, t("common.error"));
    } finally {
      setBusy(null);
    }
  };

  const runPull = async (): Promise<void> => {
    setBusy("pull");
    setError(null);
    try {
      const result = await api.vndbPull();
      setPull(result);
      setDecisions({});
      await refresh();
    } catch (caught) {
      fail(
        caught,
        caught instanceof IpcError && caught.code === "throttled"
          ? t("sync.throttled")
          : t("sync.offline"),
      );
    } finally {
      setBusy(null);
    }
  };

  const pushPending = async (): Promise<void> => {
    setBusy("push");
    try {
      const result = await api.vndbPushPending();
      await refresh();
      if (result.failed > 0) {
        pushToast("error", t("sync.queueFailed", { count: result.failed }));
      }
      setCards(await api.listGames());
    } catch (caught) {
      fail(caught, t("sync.offline"));
    } finally {
      setBusy(null);
    }
  };

  const applyDecisions = async (): Promise<void> => {
    if (!pull) return;
    setBusy("apply");
    try {
      const result = await api.vndbApplySync({
        decisions: toDecisionsPayload(decisions),
        push_all_local: false,
      });
      await refresh();
      setCards(await api.listGames());
      pushToast(
        "success",
        t("sync.applied", { updated: result.updated, queued: result.queued.length }),
      );
      // Re-pull so the screen reflects what is left, which is the honest state.
      setPull(await api.vndbPull());
      setDecisions({});
    } catch (caught) {
      fail(caught, t("sync.offline"));
    } finally {
      setBusy(null);
    }
  };

  const decide = (vnId: string, field: string, resolution: Resolution): void => {
    setDecisions((current) => ({
      ...current,
      [vnId]: { ...(current[vnId] ?? {}), [field]: resolution },
    }));
  };

  if (!status) {
    if (error) {
      return (
        <Card>
          <CardHeader title={t("sync.title")} />
          <p className="notice notice--danger" role="alert">
            {error}
          </p>
          <Button onClick={() => void refresh()}>{t("common.retry")}</Button>
        </Card>
      );
    }
    return (
      <div aria-busy="true" aria-label={t("common.loading")}>
        <div className="skeleton" style={{ height: 120, marginBottom: 16 }} />
        <div className="skeleton" style={{ height: 80 }} />
      </div>
    );
  }

  const account = status.account;
  /** Import the entries the last pull reported as missing. */
  async function importMissing() {
    if (!pull || pull.not_imported.length === 0) return;
    setBusy("import");
    setError(null);
    setImported(null);
    try {
      const result = await api.vndbImportMany(pull.not_imported);
      setImported(result);
      pushToast(
        result.failed.length > 0 ? "error" : "success",
        t("sync.importedToast", { count: result.imported }),
      );
      // Re-pull and refresh the library so the counts reflect what is now there
      // rather than what was there before the import.
      setPull(await api.vndbPull());
      setCards(await api.listGames());
    } catch (caught) {
      setError(caught instanceof IpcError ? caught.message : t("common.error"));
    } finally {
      setBusy(null);
    }
  }

  const undecided = pull ? undecidedCount(pull, decisions) : 0;
  const summary = pull ? summarize(pull) : null;

  return (
    <div style={{ display: "grid", gap: "var(--space-5)", maxWidth: "72ch" }}>
      {error ? (
        <p role="alert" style={{ color: "var(--danger)", fontSize: 12 }}>
          {error}
        </p>
      ) : null}

      {/* ---------------- account ---------------- */}
      <Card>
        <CardHeader title={t("sync.title")} />
        <p className="body" style={{ marginBottom: "var(--space-4)" }}>
          {t("sync.intro")}
        </p>

        {!status.storage_supported ? (
          <p className="body" style={{ color: "var(--warning)" }}>
            {t("sync.tokenUnsupported")}
          </p>
        ) : account ? (
          <div style={{ display: "grid", gap: "var(--space-3)" }}>
            <p className="h3">{t("sync.connectedAs", { username: account.username })}</p>
            <div style={{ display: "flex", flexWrap: "wrap", gap: 8 }}>
              <Chip selected={account.permissions.includes("listread")}>
                {t("sync.permRead")}
              </Chip>
              <Chip selected={account.permissions.includes("listwrite")}>
                {t("sync.permWrite")}
              </Chip>
            </div>
            {!account.permissions.includes("listwrite") ? (
              <p className="settings__hint">{t("sync.permMissingWrite")}</p>
            ) : null}
            <div className="captions" style={{ display: "grid", gap: 4 }}>
              <span className="caption">
                {t("sync.lastPull")}:{" "}
                {account.last_pull_at
                  ? formatDateTime(account.last_pull_at, locale)
                  : t("sync.never")}
              </span>
              <span className="caption">
                {t("sync.lastPush")}:{" "}
                {account.last_push_at
                  ? formatDateTime(account.last_push_at, locale)
                  : t("sync.never")}
              </span>
            </div>
            <div style={{ display: "flex", flexWrap: "wrap", gap: 8 }}>
              <Button
                variant="primary"
                onClick={() => void runPull()}
                disabled={busy !== null}
              >
                {busy === "pull" ? t("sync.syncing") : t("sync.syncNow")}
              </Button>
              {status.pending > 0 ? (
                <Button
                  variant="secondary"
                  onClick={() => void pushPending()}
                  disabled={busy !== null}
                >
                  {t("sync.pushPending", { count: status.pending })}
                </Button>
              ) : null}
              <Button
                variant="ghost"
                onClick={() => void disconnect()}
                disabled={busy !== null}
              >
                {t("sync.disconnect")}
              </Button>
            </div>
          </div>
        ) : (
          <form
            className="field"
            onSubmit={(event) => {
              event.preventDefault();
              void connect();
            }}
          >
            <label className="field__label" htmlFor="vndb-token">
              {t("sync.tokenLabel")}
            </label>
            <input
              id="vndb-token"
              className="input"
              type="password"
              autoComplete="off"
              spellCheck={false}
              value={token}
              onChange={(event) => setToken(event.target.value)}
              aria-describedby="vndb-token-help"
            />
            <span id="vndb-token-help" className="settings__hint">
              {t("sync.tokenHelp")}
            </span>
            <Button
              variant="primary"
              type="submit"
              disabled={busy !== null || token.trim().length === 0}
            >
              {busy === "connect" ? t("sync.connecting") : t("sync.connect")}
            </Button>
          </form>
        )}
      </Card>

      {/* ---------------- conflicts ---------------- */}
      {account ? (
        <Card>
          <CardHeader
            title={t("sync.conflictTitle")}
            aside={
              pull ? (
                <span className="caption">
                  {t("sync.remaining", { count: undecided })}
                </span>
              ) : null
            }
          />

          {!pull ? (
            <p className="body">{t("sync.conflictHelp")}</p>
          ) : summary?.clean ? (
            <div style={{ display: "grid", gap: 8 }}>
              <p className="h3">{t("sync.inSyncTitle")}</p>
              <p className="body">{t("sync.inSyncBody")}</p>
            </div>
          ) : (
            <>
              <p className="body" style={{ marginBottom: "var(--space-3)" }}>
                {t("sync.pullSummary", {
                  entries: pull.entries,
                  conflicts: pull.conflicts.length,
                  inSync: pull.in_sync,
                })}
              </p>
              <ul style={{ display: "grid", gap: "var(--space-4)" }}>
                {pull.conflicts.map((row) => (
                  <ConflictBlock
                    key={row.vn_id}
                    row={row}
                    decisions={decisions[row.vn_id] ?? {}}
                    onDecide={decide}
                  />
                ))}
              </ul>
              <div
                style={{
                  marginTop: "var(--space-4)",
                  display: "flex",
                  flexWrap: "wrap",
                  gap: 8,
                  alignItems: "center",
                }}
              >
                <Button
                  variant="primary"
                  onClick={() => void applyDecisions()}
                  disabled={busy !== null || undecided > 0}
                >
                  {busy === "apply" ? t("sync.applying") : t("sync.apply")}
                </Button>
                <span className="caption">{t("sync.bulkHint")}</span>
                <Chip onClick={() => setDecisions(decideAll(pull, "keep_local"))}>
                  {t("sync.allLocal")}
                </Chip>
                <Chip onClick={() => setDecisions(decideAll(pull, "keep_remote"))}>
                  {t("sync.allRemote")}
                </Chip>
              </div>
            </>
          )}

          {summary && summary.notImported > 0 ? (
            <div style={{ marginTop: "var(--space-3)", display: "grid", gap: 8 }}>
              <p className="caption">
                {t("sync.notImported", { count: summary.notImported })}
              </p>
              {/* Telling someone 37 entries are missing and stopping there is a
                  dead end. The pull already knows which ones. */}
              <Button
                variant="secondary"
                disabled={busy === "import"}
                onClick={() => importMissing()}
              >
                {busy === "import" ? t("sync.importing") : t("sync.importAll")}
              </Button>
              {imported ? (
                <p className="caption">
                  {t("sync.importedSummary", {
                    imported: String(imported.imported),
                    failed: String(imported.failed.length),
                  })}
                </p>
              ) : null}
              {imported && imported.failed.length > 0 ? (
                <p className="caption">{imported.failed.slice(0, 3).join("; ")}</p>
              ) : null}
            </div>
          ) : null}
        </Card>
      ) : null}

      {/* ---------------- queue ---------------- */}
      {account ? (
        <Card>
          <CardHeader
            title={t("sync.queueTitle")}
            aside={
              queue.length > 0 ? (
                <Button
                  variant="ghost"
                  onClick={() => void pushPending()}
                  disabled={busy !== null}
                >
                  {t("sync.queueRetry")}
                </Button>
              ) : null
            }
          />
          {queue.length === 0 ? (
            <p className="body">{t("sync.queueEmpty")}</p>
          ) : (
            <ul style={{ display: "grid", gap: 6 }}>
              {queue.map((entry) => (
                <li
                  key={entry.vn_id}
                  style={{
                    display: "flex",
                    justifyContent: "space-between",
                    gap: 12,
                    padding: "6px 0",
                    borderBottom: "1px solid var(--border)",
                  }}
                >
                  <span className="caption">{entry.vn_id}</span>
                  <span className="caption">
                    {t("sync.queueAttempts", { count: entry.attempts })}
                    {entry.last_error ? ` · ${entry.last_error}` : ""}
                  </span>
                </li>
              ))}
            </ul>
          )}
        </Card>
      ) : null}

      {/* ---------------- log ---------------- */}
      {account ? (
        <Card>
          <CardHeader title={t("sync.logTitle")} />
          {log.length === 0 ? (
            <p className="body">{t("sync.logEmpty")}</p>
          ) : (
            <ul style={{ display: "grid", gap: 4 }}>
              {log.slice(0, 20).map((entry) => (
                <li
                  key={entry.id}
                  style={{ display: "flex", gap: 8, fontSize: "var(--text-xs)" }}
                >
                  <span className="caption">{formatDateTime(entry.at, locale)}</span>
                  <span style={{ color: "var(--text-muted)" }}>
                    {t(
                      `sync.kind${entry.kind.charAt(0).toUpperCase()}${entry.kind.slice(1)}`,
                    )}
                  </span>
                  <span>{entry.message}</span>
                </li>
              ))}
            </ul>
          )}
        </Card>
      ) : null}
    </div>
  );
}

interface ConflictBlockProps {
  row: SyncRow;
  decisions: Record<string, Resolution>;
  onDecide: (vnId: string, field: string, resolution: Resolution) => void;
}

/** One VN's field-by-field comparison. */
function ConflictBlock({ row, decisions, onDecide }: ConflictBlockProps) {
  const { t } = useI18n();

  return (
    <li
      style={{
        border: "1px solid var(--border)",
        borderRadius: "var(--radius-m)",
        padding: "var(--space-3)",
        display: "grid",
        gap: "var(--space-2)",
      }}
    >
      <span className="row__title">{row.title}</span>
      {row.conflicts.map((conflict) => {
        const chosen = decisions[conflict.field];
        return (
          <div key={conflict.field} className="vndb-conflict">
            <span className="caption">{t(`field.${conflict.field}`)}</span>
            <span className="vndb-conflict__values">
              <span className="row__sub" title={conflict.local ?? ""}>
                {t("sync.localValue")}: {conflict.local ?? t("sync.unset")}
              </span>
              <span className="row__sub" title={conflict.remote ?? ""}>
                {t("sync.remoteValue")}: {conflict.remote ?? t("sync.unset")}
              </span>
            </span>
            <span className="vndb-conflict__actions">
              <Chip
                selected={chosen === "keep_local"}
                onClick={() => onDecide(row.vn_id, conflict.field, "keep_local")}
              >
                {t("sync.keepLocal")}
              </Chip>
              <Chip
                selected={chosen === "keep_remote"}
                onClick={() => onDecide(row.vn_id, conflict.field, "keep_remote")}
              >
                {t("sync.keepRemote")}
              </Chip>
              <Chip
                selected={chosen === "skip"}
                onClick={() => onDecide(row.vn_id, conflict.field, "skip")}
              >
                {t("sync.skip")}
              </Chip>
            </span>
          </div>
        );
      })}
    </li>
  );
}
