import { useCallback, useEffect, useId, useRef, useState, type ReactNode } from "react";
import { open } from "@tauri-apps/plugin-dialog";

import { useAppStore } from "@/app/store";
import { useI18n } from "@/app/useI18n";
import { Button } from "@/components/Button";
import { Card, CardHeader } from "@/components/Card";
import { Icon } from "@/components/Icon";
import { IpcError } from "@/lib/ipc/client";
import { DEFAULT_AGENT_COUNT, scriptApi, type ScriptDocument } from "@/lib/ipc/script";
import { formatDateTime } from "@/lib/time";

/** 1..=10, mirroring `vn_core::script::MAX_AGENTS`. */
const AGENT_CHOICES = Array.from({ length: 10 }, (_, index) => index + 1);

/** `1_048_576` bytes and up reads better as MB than as a six-digit KB count. */
function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

/** Last path segment, on either separator. */
function basename(path: string): string {
  return path.split(/[\\/]/).pop() ?? path;
}

function scriptErrorMessage(caught: unknown, t: ReturnType<typeof useI18n>["t"]): string {
  if (!(caught instanceof IpcError)) return t("common.error");
  // Backend script commands emit code `"script"` with the token in `message`.
  const token =
    caught.code === "script" || caught.code === "ipc" ? caught.message : caught.code;
  const map: Record<string, string> = {
    chat_key_missing: t("script.error.chatKeyMissing"),
    chat_model_missing: t("script.error.chatModelMissing"),
    compaction_busy: t("script.busy"),
    provider_unauthorized: t("script.error.providerUnauthorized"),
    provider_throttled: t("script.error.providerThrottled"),
    provider_offline: t("script.error.providerOffline"),
    provider_budget_exhausted: t("script.error.providerBudgetExhausted"),
    provider_truncated: t("script.error.providerTruncated"),
    provider_empty: t("script.error.providerEmpty"),
    no_files: t("script.error.noFiles"),
    empty_script: t("script.error.emptyScript"),
    invalid_utf8: t("script.error.invalidUtf8"),
    invalid_json: t("script.error.invalidJson"),
    invalid_format: t("script.error.invalidFormat"),
    file_too_large: t("script.error.fileTooLarge"),
    line_too_long: t("script.error.lineTooLong"),
    speaker_too_long: t("script.error.speakerTooLong"),
    too_many_lines: t("script.error.tooManyLines"),
    script_too_large: t("script.error.scriptTooLarge"),
    script_limit: t("script.error.scriptLimit"),
    script_missing: t("script.error.scriptMissing"),
    read_failed: t("script.error.readFailed"),
    agent_settings_invalid: t("script.error.agentSettingsInvalid"),
    beta_disabled: t("script.error.betaDisabled"),
  };
  const mapped = map[token];
  if (mapped !== undefined) return mapped;
  if (token.startsWith("provider_error: ")) {
    return token.slice("provider_error: ".length);
  }
  if (caught.message.includes("no API key is configured")) {
    return t("script.error.chatKeyMissing");
  }
  return caught.message || t("common.error");
}

function hasProgressOrDigest(doc: ScriptDocument): boolean {
  return (
    doc.compaction !== null &&
    (doc.compaction.doneChunks > 0 || doc.compaction.digest !== null)
  );
}

/** User-imported external scripts compacted chunk-by-chunk for chat grounding. */
export function ScriptCard({ gameId }: { gameId: string }) {
  return <ScriptCardBody key={gameId} gameId={gameId} />;
}

function ScriptCardBody({ gameId }: { gameId: string }) {
  const { t, locale } = useI18n();
  const pushToast = useAppStore((state) => state.pushToast);
  const platform = useAppStore((state) => state.platform);
  const agentsLabelId = useId();
  const listIdPrefix = useId();

  const [docs, setDocs] = useState<ScriptDocument[] | null>(null);
  const [listError, setListError] = useState(false);
  const [importing, setImporting] = useState(false);
  // Allow only one sequential compaction loop at a time to avoid racing progress rows.
  const [compactingId, setCompactingId] = useState<string | null>(null);
  const [stopping, setStopping] = useState(false);
  const [deletingId, setDeletingId] = useState<string | null>(null);
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);
  const stopRef = useRef(false);
  const generationRef = useRef(0);
  // Summarizer agent count; null until the setting loads.
  const [agents, setAgents] = useState<number | null>(null);

  useEffect(() => {
    let cancelled = false;
    void scriptApi
      .agentSettings()
      .then((settings) => {
        if (!cancelled) setAgents(settings.agents);
      })
      .catch(() => {
        if (!cancelled) setAgents(DEFAULT_AGENT_COUNT);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const loadDocs = useCallback(async () => {
    const gen = generationRef.current;
    try {
      const next = await scriptApi.list(gameId);
      if (generationRef.current === gen) {
        setListError(false);
        setDocs(next);
      }
    } catch (caught) {
      if (generationRef.current !== gen) return;
      setListError(true);
      setDocs(null);
      pushToast("error", scriptErrorMessage(caught, t));
    }
  }, [gameId, pushToast, t]);

  useEffect(() => {
    let active = true;
    generationRef.current += 1;
    stopRef.current = false;
    void Promise.resolve().then(() => {
      if (active) void loadDocs();
    });
    return () => {
      active = false;
      generationRef.current += 1;
      stopRef.current = true;
    };
  }, [loadDocs]);

  const refreshIfCurrent = async (gen: number): Promise<void> => {
    const next = await scriptApi.list(gameId);
    if (generationRef.current === gen) {
      setDocs(next);
      setListError(false);
    }
  };

  /** Persist a new agent count. Disabled mid-run because a layout change
   * resets an unfinished summary on the next run. */
  const changeAgents = async (next: number): Promise<void> => {
    try {
      const saved = await scriptApi.saveAgentSettings({ agents: next });
      setAgents(saved.agents);
    } catch (caught) {
      pushToast("error", scriptErrorMessage(caught, t));
    }
  };

  const pick = async (): Promise<void> => {
    const gen = generationRef.current;
    setImporting(true);
    try {
      // Android SAF greys out valid JSON files when extension filters assume strict MIME types.
      const chosen = await open({
        multiple: true,
        directory: false,
        title: t("script.pickTitle"),
        ...(platform?.os === "android"
          ? {}
          : {
              filters: [
                {
                  name: t("script.pickFilter"),
                  extensions: ["json"],
                },
              ],
            }),
      });
      if (!chosen || chosen.length === 0) return;
      await scriptApi.import(gameId, chosen);
      if (generationRef.current !== gen) return;
      await refreshIfCurrent(gen);
      if (generationRef.current !== gen) return;
      pushToast("success", t("script.imported"));
    } catch (caught) {
      if (generationRef.current === gen) {
        pushToast("error", scriptErrorMessage(caught, t));
      }
    } finally {
      if (generationRef.current === gen) {
        setImporting(false);
      }
    }
  };

  const executeDelete = async (id: string): Promise<void> => {
    const gen = generationRef.current;
    setDeletingId(id);
    try {
      await scriptApi.delete(id);
      if (generationRef.current !== gen) return;
      setConfirmDeleteId((prev) => (prev === id ? null : prev));
      await refreshIfCurrent(gen);
      if (generationRef.current !== gen) return;
      pushToast("success", t("script.deleted"));
    } catch (caught) {
      if (generationRef.current === gen) {
        pushToast("error", scriptErrorMessage(caught, t));
      }
    } finally {
      if (generationRef.current === gen) {
        setDeletingId(null);
      }
    }
  };

  const requestRemove = (doc: ScriptDocument): void => {
    if (compactingId !== null || deletingId !== null) return;
    if (hasProgressOrDigest(doc)) {
      setConfirmDeleteId(doc.id);
      return;
    }
    void executeDelete(doc.id);
  };

  /**
   * Compact in bounded batches so progress updates between calls and cancellation resumes via `doneChunks`.
   * Pass `rebuild = true` to discard older chunk summaries and re-run from scratch.
   */
  const summarize = async (doc: ScriptDocument, rebuild = false): Promise<void> => {
    if (compactingId !== null || deletingId !== null) return;
    const gen = generationRef.current;
    setConfirmDeleteId(null);
    setCompactingId(doc.id);
    setStopping(false);
    stopRef.current = false;
    try {
      for (let first = true; ; first = false) {
        if (stopRef.current || generationRef.current !== gen) return;
        const info = await scriptApi.compact(doc.id, undefined, rebuild && first);
        if (generationRef.current !== gen) return;
        setDocs((prev) =>
          prev
            ? prev.map((entry) =>
                entry.id === doc.id ? { ...entry, compaction: info } : entry,
              )
            : prev,
        );
        if (info.digest) {
          pushToast("success", t("script.digestReady"));
          return;
        }
        if (info.doneChunks >= info.chunkCount) {
          pushToast("error", t("script.error.reduceIncomplete"));
          return;
        }
      }
    } catch (caught) {
      if (generationRef.current !== gen) return;
      await refreshIfCurrent(gen).catch(() => {});
      pushToast("error", scriptErrorMessage(caught, t));
    } finally {
      if (generationRef.current === gen) {
        setCompactingId(null);
        setStopping(false);
      }
    }
  };

  const compactionLine = (doc: ScriptDocument, compacting: boolean): ReactNode => {
    const info = doc.compaction;
    if (!info) {
      return <span className="script-row__state">{t("script.notSummarized")}</span>;
    }
    if (info.digest) {
      return (
        <span className="script-row__state script-row__state--done" aria-live="polite">
          <Icon name="check" size={12} />
          {info.digestOutdated ? t("script.digestOld") : t("script.digestReady")}
        </span>
      );
    }
    const percent =
      info.chunkCount > 0 ? Math.round((info.doneChunks / info.chunkCount) * 100) : 0;
    const allChunksDone = info.chunkCount > 0 && info.doneChunks >= info.chunkCount;
    const label = allChunksDone
      ? compacting
        ? t("script.finalizing", { count: info.chunkCount })
        : t("script.readyToFinalize", { count: info.chunkCount })
      : info.agentCount > 1
        ? t("script.progressAgents", {
            done: info.doneChunks,
            count: info.chunkCount,
            agents: info.agentCount,
          })
        : t("script.progress", {
            done: info.doneChunks,
            count: info.chunkCount,
          });
    return (
      <span className="script-row__state" aria-live="polite">
        {label}
        <span
          className="script-row__meter"
          role="progressbar"
          aria-valuemin={0}
          aria-valuemax={info.chunkCount}
          aria-valuenow={info.doneChunks}
          aria-label={label}
          data-percent={percent}
        >
          <span
            className="script-row__meter-fill"
            style={{ width: `${Math.max(percent, 4)}%` }}
          />
        </span>
      </span>
    );
  };

  const busyElsewhere = compactingId !== null || deletingId !== null;
  const hasUnfinishedCompaction = Boolean(
    docs?.some(
      (doc) =>
        doc.compaction !== null &&
        doc.compaction.digest === null &&
        doc.compaction.doneChunks > 0,
    ),
  );

  return (
    <Card className="script-card">
      <CardHeader
        title={t("script.title")}
        aside={
          <Button
            variant="secondary"
            disabled={importing || deletingId !== null}
            onClick={() => void pick()}
          >
            {importing ? t("script.importing") : t("script.import")}
          </Button>
        }
      />
      {listError ? (
        <div className="script-error-box">
          <p className="body">{t("script.loadFailed")}</p>
          <Button variant="secondary" onClick={() => void loadDocs()}>
            {t("common.retry")}
          </Button>
        </div>
      ) : docs === null ? (
        <div className="script-loading" aria-busy="true">
          <div className="skeleton skeleton--row" />
        </div>
      ) : docs.length === 0 ? (
        <p className="script-card__empty">{t("script.empty")}</p>
      ) : (
        <ul className="script-list">
          {docs.map((doc) => {
            const compacting = compactingId === doc.id;
            const confirmingDelete = confirmDeleteId === doc.id;
            const needsWork = doc.compaction === null || doc.compaction.digest === null;
            const readyToFinalize =
              doc.compaction !== null &&
              doc.compaction.digest === null &&
              doc.compaction.chunkCount > 0 &&
              doc.compaction.doneChunks >= doc.compaction.chunkCount;
            const titleId = `${listIdPrefix}-${doc.id}`;
            return (
              <li key={doc.id} className="script-row">
                <div className="script-row__main">
                  <span id={titleId} className="row__title" title={doc.sourcePath}>
                    {basename(doc.sourcePath)}
                  </span>
                  <span className="row__sub">
                    {t(`script.kind.${doc.sourceKind}`)} ·{" "}
                    {t("script.lines", { count: doc.lineCount })} ·{" "}
                    {formatBytes(doc.byteSize)} · {formatDateTime(doc.createdAt, locale)}
                  </span>
                  {compactionLine(doc, compacting)}
                </div>
                <div className="script-row__actions">
                  {compacting ? (
                    <Button
                      variant="ghost"
                      disabled={stopping}
                      aria-describedby={titleId}
                      onClick={() => {
                        stopRef.current = true;
                        setStopping(true);
                      }}
                    >
                      {stopping ? t("script.stopping") : t("script.stop")}
                    </Button>
                  ) : doc.compaction?.digest ? (
                    <Button
                      variant="secondary"
                      disabled={busyElsewhere}
                      aria-describedby={titleId}
                      title={
                        doc.compaction.digestOutdated
                          ? t("script.rebuildHint")
                          : undefined
                      }
                      onClick={() => void summarize(doc, true)}
                    >
                      {t("script.rebuild")}
                    </Button>
                  ) : needsWork ? (
                    <Button
                      variant="secondary"
                      disabled={busyElsewhere}
                      aria-describedby={titleId}
                      onClick={() => void summarize(doc)}
                    >
                      {readyToFinalize
                        ? t("script.finalizeDigest")
                        : doc.compaction && doc.compaction.doneChunks > 0
                          ? t("script.summarizeContinue")
                          : t("script.summarize")}
                    </Button>
                  ) : null}

                  {confirmingDelete ? (
                    <>
                      <Button
                        variant="danger"
                        disabled={busyElsewhere}
                        aria-describedby={titleId}
                        onClick={() => void executeDelete(doc.id)}
                      >
                        {t("script.confirmRemove")}
                      </Button>
                      <Button
                        variant="ghost"
                        disabled={deletingId === doc.id}
                        onClick={() => setConfirmDeleteId(null)}
                      >
                        {t("common.cancel")}
                      </Button>
                    </>
                  ) : (
                    <Button
                      variant="ghost"
                      disabled={busyElsewhere}
                      aria-describedby={titleId}
                      onClick={() => requestRemove(doc)}
                    >
                      <Icon name="trash" size={14} />
                      {t("script.remove")}
                    </Button>
                  )}
                </div>
              </li>
            );
          })}
        </ul>
      )}

      {docs && docs.length > 0 ? (
        <p className="script-card__notice">
          {t("script.providerNote")} {t("script.spoilersNote")}
        </p>
      ) : null}

      <details className="script-help">
        <summary>{t("script.settings")}</summary>
        <div className="script-help__body">
          <p>{t("script.intro")}</p>
          <div className="script-agents">
            <label className="script-agents__label" htmlFor={agentsLabelId}>
              {t("script.agents")}
            </label>
            <select
              id={agentsLabelId}
              className="select"
              aria-describedby={`${agentsLabelId}-hint`}
              value={agents ?? DEFAULT_AGENT_COUNT}
              disabled={agents === null || compactingId !== null}
              onChange={(event) => void changeAgents(Number(event.target.value))}
            >
              {AGENT_CHOICES.map((count) => (
                <option key={count} value={count}>
                  {count}
                </option>
              ))}
            </select>
          </div>
          <p id={`${agentsLabelId}-hint`}>
            {t("script.agentsHint")}
            {hasUnfinishedCompaction ? ` ${t("script.agentsResetWarn")}` : null}
          </p>
          <p>{t("script.localOnly")}</p>
        </div>
      </details>
    </Card>
  );
}
