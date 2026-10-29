import { useEffect, useRef, useState, type ReactNode } from "react";
import { open } from "@tauri-apps/plugin-dialog";

import { useAppStore } from "@/app/store";
import { useI18n } from "@/app/useI18n";
import { Button } from "@/components/Button";
import { Card, CardHeader } from "@/components/Card";
import { Icon } from "@/components/Icon";
import { IpcError } from "@/lib/ipc/client";
import {
  DEFAULT_AGENT_COUNT,
  scriptApi,
  type ScriptDocument,
} from "@/lib/ipc/script";
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

/**
 * Scripts the user extracted themselves and imported for this game.
 *
 * This is the disassemble-it-yourself path: VN Asyik never opens the game's
 * archives here, it only reads the text file the user points at. The card says
 * so, and it says where the script goes — into the companion chat, summarized
 * chunk by chunk so a whole novel fits a model's context without inventing
 * content.
 */
export function ScriptCard({ gameId }: { gameId: string }) {
  const { t, locale } = useI18n();
  const pushToast = useAppStore((state) => state.pushToast);

  const [docs, setDocs] = useState<ScriptDocument[] | null>(null);
  const [importing, setImporting] = useState(false);
  // The document currently being summarized, if any. One at a time: the calls
  // are sequential by design (each summarizes the next pending chunks), and a
  // second concurrent loop would race the same progress row.
  const [compactingId, setCompactingId] = useState<string | null>(null);
  const stopRef = useRef(false);
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

  useEffect(() => {
    let cancelled = false;
    void scriptApi
      .list(gameId)
      .then((next) => {
        if (!cancelled) setDocs(next);
      })
      .catch((caught: unknown) => {
        if (!cancelled) {
          setDocs([]);
          pushToast(
            "error",
            caught instanceof IpcError ? caught.message : t("common.error"),
          );
        }
      });
    return () => {
      cancelled = true;
      stopRef.current = true;
    };
  }, [gameId, pushToast, t]);

  const refresh = async (): Promise<void> => {
    setDocs(await scriptApi.list(gameId));
  };

  /** Persist a new agent count. Disabled mid-run because a layout change
   * restarts an unfinished summary (the stored progress belongs to the old
   * agent layout). */
  const changeAgents = async (next: number): Promise<void> => {
    try {
      const saved = await scriptApi.saveAgentSettings({ agents: next });
      setAgents(saved.agents);
    } catch (caught) {
      pushToast(
        "error",
        caught instanceof IpcError ? caught.message : t("common.error"),
      );
    }
  };

  const pick = async (): Promise<void> => {
    setImporting(true);
    try {
      const chosen = await open({
        multiple: true,
        directory: false,
        title: t("script.pickTitle"),
        filters: [
          {
            name: t("script.pickFilter"),
            extensions: ["txt", "json", "log", "csv"],
          },
        ],
      });
      // One file or many: the backend merges them in file-name order into a
      // single document, so part files do not have to be combined by hand.
      if (!chosen || chosen.length === 0) return;
      await scriptApi.import(gameId, chosen);
      await refresh();
      pushToast("success", t("script.imported"));
    } catch (caught) {
      pushToast(
        "error",
        caught instanceof IpcError ? caught.message : t("common.error"),
      );
    } finally {
      setImporting(false);
    }
  };

  const remove = async (id: string): Promise<void> => {
    try {
      await scriptApi.delete(id);
      await refresh();
      pushToast("success", t("script.deleted"));
    } catch (caught) {
      pushToast(
        "error",
        caught instanceof IpcError ? caught.message : t("common.error"),
      );
    }
  };

  /**
   * Summarize until the digest exists. Each call covers a bounded batch of
   * chunks, so progress is visible between calls and a stop request (or the
   * card unmounting) takes effect without abandoning a completed batch — the
   * backend resumes from `doneChunks` on the next run.
   *
   * `rebuild` starts over instead of resuming: that is the only way to replace
   * a finished digest written by an older summarizer, whose stored chunk
   * summaries cannot be reused with the current prompts.
   */
  const summarize = async (doc: ScriptDocument, rebuild = false): Promise<void> => {
    if (compactingId !== null) return;
    setCompactingId(doc.id);
    stopRef.current = false;
    try {
      for (let first = true; ; first = false) {
        if (stopRef.current) return;
        const info = await scriptApi.compact(doc.id, undefined, rebuild && first);
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
        // Done chunks without a digest should not occur; guard the loop anyway.
        if (info.doneChunks >= info.chunkCount) return;
      }
    } catch (caught) {
      // Compaction needs the chat provider; say that in app words rather than
      // the raw error code when that is what is missing. The backend holds a
      // per-document lease, so a run started before this card mounted (the
      // loop survives unmount until its batch ends) reports busy instead of
      // racing the same progress row.
      pushToast(
        "error",
        caught instanceof IpcError && caught.code === "chat_key_missing"
          ? t("script.error.chatKeyMissing")
          : caught instanceof IpcError && caught.code === "compaction_busy"
            ? t("script.busy")
            : caught instanceof IpcError
              ? caught.message
              : t("common.error"),
      );
    } finally {
      setCompactingId(null);
    }
  };

  const compactionLine = (doc: ScriptDocument): ReactNode => {
    const info = doc.compaction;
    if (!info) {
      return <span className="script-row__state">{t("script.notSummarized")}</span>;
    }
    if (info.digest) {
      return (
        <span className="script-row__state script-row__state--done">
          <Icon name="check" size={12} />
          {info.digestOutdated ? t("script.digestOld") : t("script.digestReady")}
        </span>
      );
    }
    const percent =
      info.chunkCount > 0 ? Math.round((info.doneChunks / info.chunkCount) * 100) : 0;
    const label =
      info.agentCount > 1
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
      <span className="script-row__state">
        {label}
        <span
          className="script-row__meter"
          aria-hidden="true"
          data-percent={percent}
          style={{ width: `${Math.max(percent, 4)}%` }}
        />
      </span>
    );
  };

  return (
    <Card>
      <CardHeader
        title={t("script.title")}
        aside={
          <Button variant="ghost" disabled={importing} onClick={() => void pick()}>
            {importing ? t("script.importing") : t("script.import")}
          </Button>
        }
      />
      <p className="card__intro">{t("script.intro")}</p>

      <div className="script-agents">
        <span className="script-agents__label" id="script-agents-label">
          {t("script.agents")}
        </span>
        <select
          className="select"
          aria-labelledby="script-agents-label"
          title={t("script.agentsHint")}
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

      {docs === null ? null : docs.length === 0 ? (
        <p className="body">{t("script.empty")}</p>
      ) : (
        <ul className="script-list">
          {docs.map((doc) => {
            const compacting = compactingId === doc.id;
            const needsWork =
              doc.compaction === null ||
              (doc.compaction.digest === null &&
                doc.compaction.doneChunks < doc.compaction.chunkCount);
            return (
              <li key={doc.id} className="script-row">
                <div className="script-row__main">
                  <span className="row__title" title={doc.sourcePath}>
                    {basename(doc.sourcePath)}
                  </span>
                  <span className="row__sub">
                    {t(`script.kind.${doc.sourceKind}`)} ·{" "}
                    {t("script.lines", { count: doc.lineCount })} ·{" "}
                    {formatBytes(doc.byteSize)} · {formatDateTime(doc.createdAt, locale)}
                  </span>
                  {compactionLine(doc)}
                </div>
                <div className="script-row__actions">
                  {compacting ? (
                    <Button
                      variant="ghost"
                      onClick={() => {
                        stopRef.current = true;
                      }}
                    >
                      {t("script.stop")}
                    </Button>
                  ) : doc.compaction?.digest ? (
                    <Button
                      variant="secondary"
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
                    <Button variant="secondary" onClick={() => void summarize(doc)}>
                      {doc.compaction && doc.compaction.doneChunks > 0
                        ? t("script.summarizeContinue")
                        : t("script.summarize")}
                    </Button>
                  ) : null}
                  <Button variant="ghost" onClick={() => void remove(doc.id)}>
                    <Icon name="trash" size={14} />
                    {t("script.remove")}
                  </Button>
                </div>
              </li>
            );
          })}
        </ul>
      )}

      <p className="field__hint">{t("script.agentsHint")}</p>
      <p className="field__hint">{t("script.localOnly")}</p>
    </Card>
  );
}
