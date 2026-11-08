import { useCallback, useEffect, useRef, useState } from "react";
import { Button } from "@/components/Button";
import { Sheet } from "@/components/Sheet";
import { useI18n } from "@/app/useI18n";
import type { CompanionSession } from "@/lib/ipc/companion";
import {
  memoryApi,
  type ForgetTarget,
  type Memory,
  type MemorySave,
  type MemorySource,
  type MemoryView,
} from "@/lib/ipc/companionMemory";
import type { ChatMessage } from "@/lib/ipc/chat";
import { useCompanionStore, failureCode } from "./store";
import { companionError } from "./errors";

type Editor = MemorySave & {
  jobId?: string;
  readingContext?: Pick<Memory, "playthroughId" | "knowledge"> | undefined;
};

export function CompanionMemory({
  session,
  passage,
  onClose,
}: {
  session: CompanionSession | null;
  passage: ChatMessage | null;
  onClose: () => void;
}) {
  const { t, locale } = useI18n();
  const sequence = useCompanionStore((s) => s.snapshot?.sequence);
  const [data, setData] = useState<MemoryView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);
  const [query, setQuery] = useState("");
  const [page, setPage] = useState(0);
  const [filter, setFilter] = useState("all");
  const [editor, setEditor] = useState<Editor | null>(null);
  const [source, setSource] = useState<{
    id: string;
    version: number;
    items: MemorySource[];
  } | null>(null);
  const [confirm, setConfirm] = useState<{
    target: ForgetTarget;
    revision: number;
  } | null>(null);
  const [billing, setBilling] = useState<"suggest" | "automatic" | null>(null);
  const order = useRef(0);
  const initialPassage = useRef(false);
  const confirmationRef = useRef<HTMLDivElement>(null);
  const billingRef = useRef<HTMLDivElement>(null);
  const editorRef = useRef<HTMLFormElement>(null);
  useEffect(() => {
    if (confirm) {
      confirmationRef.current?.focus();
      confirmationRef.current?.scrollIntoView?.({ block: "start", behavior: "instant" });
    }
  }, [confirm]);
  useEffect(() => {
    if (billing) {
      billingRef.current?.focus();
      billingRef.current?.scrollIntoView?.({ block: "start", behavior: "instant" });
    }
  }, [billing]);
  useEffect(() => {
    editorRef.current?.scrollIntoView?.({ block: "start", behavior: "instant" });
  }, [editor?.requestId]);
  const sessionId = session?.id ?? null;
  const refresh = useCallback(async () => {
    const ticket = ++order.current;
    try {
      const next = await memoryApi.view(sessionId);
      if (ticket === order.current) setData(next);
    } catch (error) {
      if (ticket === order.current) {
        setData(null);
        setError(failureCode(error));
      }
    }
  }, [sessionId]);
  useEffect(() => {
    const timer = setTimeout(() => void refresh(), 150);
    return () => clearTimeout(timer);
  }, [refresh, sequence]);
  useEffect(
    () => () => {
      order.current++;
    },
    [],
  );

  const newEditor = useCallback(
    (revision: number): Editor => ({
      expectedCompanionId: null,
      id: null,
      expectedVersion: null,
      scope: passage ? "reading" : "shared",
      class: passage ? "episode" : "preference",
      body: passage?.content ?? "",
      pinned: false,
      sessionId,
      messageId: passage?.id ?? null,
      requestId: crypto.randomUUID(),
      expectedRevision: revision,
      readingContext: session
        ? { playthroughId: session.playthroughId, knowledge: session.options.knowledge }
        : undefined,
    }),
    [passage, sessionId, session],
  );
  useEffect(() => {
    if (data && passage && !initialPassage.current) {
      initialPassage.current = true;
      setEditor(newEditor(data.settings.revision));
    }
  }, [data, passage, newEditor]);

  async function action(work: () => Promise<unknown>) {
    setPending(true);
    setError(null);
    setSource(null);
    try {
      await work();
      await refresh();
      return true;
    } catch (error) {
      setError(failureCode(error));
      await refresh();
      return false;
    } finally {
      setPending(false);
    }
  }
  function edit(item: Memory) {
    if (!data) return;
    setEditor({
      expectedCompanionId: item.companionId,
      id: item.id,
      expectedVersion: item.version,
      scope: item.scope,
      class: item.class,
      body: item.body,
      pinned: item.pinned,
      sessionId: null,
      messageId: null,
      requestId: crypto.randomUUID(),
      expectedRevision: data.settings.revision,
      readingContext: item,
    });
    setConfirm(null);
    setSource(null);
  }
  const items =
    data?.items.filter((item) => {
      if (!item.body.toLocaleLowerCase().includes(query.toLocaleLowerCase()))
        return false;
      if (filter === "shared") return item.scope === "shared";
      if (filter === "companion")
        return (
          item.scope === "companion" && item.companionId === data.context?.companionId
        );
      if (filter === "reading")
        return (
          item.scope === "reading" &&
          item.gameId === data.context?.gameId &&
          item.playthroughId === data.context?.playthroughId
        );
      return true;
    }) ?? [];
  const bytes = new TextEncoder().encode(editor?.body ?? "").length;
  const visiblePage = Math.min(page, Math.max(0, Math.ceil(items.length / 20) - 1));

  return (
    <Sheet open title={t("memory.title")} onClose={onClose}>
      <div className="companion-memory">
        <p className="companion__muted">{t("memory.intro")}</p>
        {error && (
          <p className="companion__error" role="alert">
            {companionError(t, error)}
          </p>
        )}
        {!data ? (
          <>
            <p role="status">{t("companion.loading")}</p>
            <Button onClick={() => void refresh()}>{t("companion.retry")}</Button>
          </>
        ) : (
          <>
            <label className="companion-memory__toggle">
              <input
                type="checkbox"
                checked={data.settings.retrievalEnabled}
                disabled={pending}
                onChange={(e) =>
                  void action(() =>
                    memoryApi.settings(
                      e.target.checked,
                      data.settings.autoSummary,
                      data.settings.revision,
                    ),
                  )
                }
              />
              <span>
                {t("memory.use")}
                <small>{t("memory.useHint")}</small>
              </span>
            </label>
            <label className="companion-memory__toggle">
              <input
                type="checkbox"
                checked={data.settings.autoSummary}
                disabled={pending}
                onChange={(e) =>
                  e.target.checked
                    ? setBilling("automatic")
                    : void action(() =>
                        memoryApi.settings(
                          data.settings.retrievalEnabled,
                          false,
                          data.settings.revision,
                        ),
                      )
                }
              />
              <span>
                {t("memory.auto")}
                <small>{t("memory.autoHint")}</small>
              </span>
            </label>
            {data.replayExcluded && (
              <p className="companion__notice">{t("memory.replayExcluded")}</p>
            )}
            {data.settings.gap && (
              <p className="companion__notice">{t(`memory.gap.${data.settings.gap}`)}</p>
            )}
            <p className="companion__muted">
              {t("memory.capacity", { items: data.itemCount, pins: data.pinCount })}
            </p>
            {data.markerCount >= 10000 && (
              <p role="alert">{t("companion.error.memory_capacity")}</p>
            )}
            {billing && (
              <div className="companion__notice" ref={billingRef} tabIndex={-1}>
                <p>
                  {t(
                    billing === "automatic"
                      ? "memory.autoConfirm"
                      : "memory.suggestConfirm",
                  )}
                </p>
                <div className="companion__actions">
                  <Button
                    disabled={pending}
                    onClick={() =>
                      void action(() =>
                        billing === "automatic"
                          ? memoryApi.settings(
                              data.settings.retrievalEnabled,
                              true,
                              data.settings.revision,
                            )
                          : memoryApi.suggest(sessionId!),
                      ).then((ok) => {
                        if (ok) setBilling(null);
                      })
                    }
                  >
                    {t("memory.confirm")}
                  </Button>
                  <Button onClick={() => setBilling(null)}>{t("common.cancel")}</Button>
                </div>
              </div>
            )}
            {editor ? (
              <form
                ref={editorRef}
                className="companion-memory__editor"
                onSubmit={(e) => {
                  e.preventDefault();
                  const request = { ...editor };
                  delete request.readingContext;
                  delete request.jobId;
                  void action(() =>
                    editor.jobId
                      ? memoryApi.review(
                          editor.jobId,
                          editor.body,
                          editor.pinned,
                          editor.expectedRevision,
                        )
                      : memoryApi.save(request),
                  ).then((ok) => {
                    if (ok) setEditor(null);
                  });
                }}
              >
                <h3>
                  {t(
                    editor.jobId
                      ? "memory.review"
                      : editor.id
                        ? "memory.edit"
                        : "memory.remember",
                  )}
                </h3>
                {editor.id && (
                  <p className="companion__notice">{t("memory.editWarning")}</p>
                )}
                {editor.messageId && (
                  <p className="companion__muted">{t("memory.passageHint")}</p>
                )}
                <label className="companion__field">
                  <span>{t("memory.scope")}</span>
                  <select
                    value={editor.scope}
                    disabled={
                      Boolean(editor.id || editor.messageId || editor.jobId) || pending
                    }
                    onChange={(e) => {
                      const scope = e.target.value as Memory["scope"];
                      setEditor({
                        ...editor,
                        scope,
                        expectedCompanionId:
                          scope === "companion"
                            ? (data.context?.companionId ?? null)
                            : null,
                        class: scope === "reading" ? "observation" : "preference",
                      });
                    }}
                  >
                    <option value="shared">{t("memory.scope.shared")}</option>
                    <option value="companion" disabled={!session}>
                      {t("memory.scope.companion")}
                    </option>
                    <option
                      value="reading"
                      disabled={!session || session.options.knowledge === "metadata"}
                    >
                      {t("memory.scope.reading")}
                    </option>
                  </select>
                </label>
                <p className="companion__muted">
                  {t(`memory.scopeHint.${editor.scope}`)}
                </p>
                {editor.scope === "reading" && editor.readingContext && (
                  <p>
                    {t("memory.readingId", { id: editor.readingContext.playthroughId! })}
                    {" · "}
                    {t(`companion.knowledge.${editor.readingContext.knowledge}`)}
                  </p>
                )}
                <label className="companion__field">
                  <span>{t("memory.text")}</span>
                  <textarea
                    rows={5}
                    value={editor.body}
                    onChange={(e) => setEditor({ ...editor, body: e.target.value })}
                  />
                </label>
                <small aria-live="off">{t("memory.bytes", { count: bytes })}</small>
                <label className="companion-memory__toggle">
                  <input
                    type="checkbox"
                    checked={editor.pinned}
                    onChange={(e) => setEditor({ ...editor, pinned: e.target.checked })}
                  />
                  {t("memory.pin")}
                </label>
                <div className="companion__actions">
                  <Button
                    type="submit"
                    variant="primary"
                    disabled={pending || !editor.body.trim() || bytes > 2048}
                  >
                    {t("memory.save")}
                  </Button>
                  <Button disabled={pending} onClick={() => setEditor(null)}>
                    {t("common.cancel")}
                  </Button>
                </div>
              </form>
            ) : (
              <div className="companion__actions">
                <Button
                  variant="primary"
                  disabled={pending || data.itemCount >= 1000}
                  onClick={() =>
                    setEditor({
                      ...newEditor(data.settings.revision),
                      body: "",
                      messageId: null,
                      scope: "shared",
                      class: "preference",
                    })
                  }
                >
                  {t("memory.remember")}
                </Button>
                <Button
                  disabled={
                    pending || !session || session.options.knowledge === "metadata"
                  }
                  onClick={() => setBilling("suggest")}
                >
                  {t("memory.suggest")}
                </Button>
              </div>
            )}
            {data.jobs.length > 0 && (
              <section className="companion-memory__section">
                <h3>{t("memory.suggestions")}</h3>
                {data.jobs.map((job) => (
                  <article className="companion-memory__item" key={job.id}>
                    <p>
                      {t(`memory.job.${job.state}`)} · {job.model}
                    </p>
                    <small>
                      {t("memory.expires", {
                        date: new Date(job.expiresAt).toLocaleDateString(locale),
                      })}{" "}
                      · {t(`companion.knowledge.${job.context.knowledge}`)}
                    </small>
                    {job.error && <p role="status">{companionError(t, job.error)}</p>}
                    {job.output && <p className="companion-memory__body">{job.output}</p>}
                    <div className="companion__actions">
                      {job.state === "review" && (
                        <Button
                          disabled={pending}
                          onClick={() =>
                            setEditor({
                              ...newEditor(data.settings.revision),
                              jobId: job.id,
                              body: job.output,
                              scope: "reading",
                              class: "summary",
                              messageId: null,
                              readingContext: job.context,
                            })
                          }
                        >
                          {t("memory.review")}
                        </Button>
                      )}
                      {(job.state === "paused" || job.state === "pending") && (
                        <Button
                          disabled={pending}
                          onClick={() => void action(() => memoryApi.resume(job.id))}
                        >
                          {t("memory.resumeJob")}
                        </Button>
                      )}
                      <Button
                        disabled={pending}
                        onClick={() =>
                          void action(() =>
                            memoryApi.review(
                              job.id,
                              null,
                              false,
                              data.settings.revision,
                            ),
                          )
                        }
                      >
                        {t("memory.discard")}
                      </Button>
                    </div>
                  </article>
                ))}
              </section>
            )}
            <section className="companion-memory__section">
              <h3>{t("memory.saved")}</h3>
              <label className="companion__field">
                <span>{t("memory.search")}</span>
                <input
                  type="search"
                  value={query}
                  onChange={(e) => {
                    setQuery(e.target.value);
                    setPage(0);
                  }}
                />
              </label>
              <label className="companion__field">
                <span>{t("memory.filter")}</span>
                <select
                  value={filter}
                  onChange={(e) => {
                    setFilter(e.target.value);
                    setPage(0);
                  }}
                >
                  <option value="all">{t("memory.allScopes")}</option>
                  {["shared", "companion", "reading"].map((scope) => (
                    <option key={scope} value={scope}>
                      {t(`memory.scope.${scope}`)}
                    </option>
                  ))}
                </select>
              </label>
              {items.length === 0 && <p>{t("memory.empty")}</p>}
              {items.slice(visiblePage * 20, visiblePage * 20 + 20).map((item) => (
                <article className="companion-memory__item" key={item.id}>
                  <small>
                    {t(`memory.scope.${item.scope}`)} ·{" "}
                    {t(`memory.evidence.${item.evidence}`)}
                    {item.pinned ? ` · ${t("memory.pinned")}` : ""}
                  </small>
                  <p className="companion-memory__body">{item.body}</p>
                  {item.expiresAt && !item.pinned && (
                    <small>
                      {t("memory.expires", {
                        date: new Date(item.expiresAt).toLocaleDateString(locale),
                      })}
                    </small>
                  )}
                  {item.scope === "reading" && (
                    <small>
                      {t(`companion.knowledge.${item.knowledge}`)} ·{" "}
                      {t("memory.readingId", { id: item.playthroughId! })}
                    </small>
                  )}
                  <div className="companion__actions">
                    <Button disabled={pending} onClick={() => edit(item)}>
                      {t("memory.edit")}
                    </Button>
                    <Button
                      disabled={pending}
                      onClick={() =>
                        void action(async () => {
                          const records = await memoryApi.sources(item.id, item.version);
                          setSource({
                            id: item.id,
                            version: item.version,
                            items: records,
                          });
                        })
                      }
                    >
                      {t("memory.sources")}
                    </Button>
                    <Button
                      disabled={pending}
                      variant="ghost"
                      onClick={() =>
                        setConfirm({
                          target: { kind: "item", id: item.id, version: item.version },
                          revision: data.settings.revision,
                        })
                      }
                    >
                      {t("memory.forget")}
                    </Button>
                  </div>
                  {source?.id === item.id && source.version === item.version && (
                    <div className="companion-memory__evidence">
                      {source.items.length === 0 ? (
                        <p>{t("memory.noSource")}</p>
                      ) : (
                        source.items.map((s, index) => (
                          <blockquote key={index}>
                            <small>
                              {t(`memory.source.${s.kind}`)} ·{" "}
                              {t(`companion.knowledge.${s.knowledge}`)}
                            </small>
                            <p>{s.evidence}</p>
                          </blockquote>
                        ))
                      )}
                    </div>
                  )}
                </article>
              ))}
              {items.length > 20 && (
                <div className="companion__actions">
                  <Button
                    disabled={visiblePage === 0}
                    onClick={() => setPage(visiblePage - 1)}
                  >
                    {t("memory.previous")}
                  </Button>
                  <span>
                    {visiblePage + 1} / {Math.ceil(items.length / 20)}
                  </span>
                  <Button
                    disabled={(visiblePage + 1) * 20 >= items.length}
                    onClick={() => setPage(visiblePage + 1)}
                  >
                    {t("memory.next")}
                  </Button>
                </div>
              )}
            </section>
            <details className="companion-memory__section">
              <summary>{t("memory.forgetMore")}</summary>
              <div className="companion__actions">
                {session && (
                  <>
                    <Button
                      onClick={() =>
                        setConfirm({
                          target: { kind: "reading", id: session.id },
                          revision: data.settings.revision,
                        })
                      }
                    >
                      {t("memory.forgetReading")}
                    </Button>
                    <Button
                      onClick={() =>
                        setConfirm({
                          target: { kind: "session", id: session.id },
                          revision: data.settings.revision,
                        })
                      }
                    >
                      {t("memory.forgetSession")}
                    </Button>
                  </>
                )}
                {data.context && (
                  <Button
                    onClick={() =>
                      setConfirm({
                        target: { kind: "companion", id: data.context!.companionId },
                        revision: data.settings.revision,
                      })
                    }
                  >
                    {t("memory.forgetCompanion")}
                  </Button>
                )}
                <Button
                  onClick={() =>
                    setConfirm({
                      target: { kind: "everything" },
                      revision: data.settings.revision,
                    })
                  }
                >
                  {t("memory.forgetAll")}
                </Button>
              </div>
            </details>
            {confirm && (
              <div
                className="companion__notice"
                role="alert"
                ref={confirmationRef}
                tabIndex={-1}
              >
                <h3>{t("memory.forgetConfirm")}</h3>
                <p>{t("memory.forgetWarning")}</p>
                <div className="companion__actions">
                  <Button
                    variant="danger"
                    disabled={pending}
                    onClick={() =>
                      void action(() =>
                        memoryApi.forget(confirm.target, confirm.revision),
                      ).then((ok) => {
                        if (ok) {
                          setConfirm(null);
                          setEditor(null);
                        }
                      })
                    }
                  >
                    {t("memory.forget")}
                  </Button>
                  <Button disabled={pending} onClick={() => setConfirm(null)}>
                    {t("common.cancel")}
                  </Button>
                </div>
              </div>
            )}
          </>
        )}
      </div>
    </Sheet>
  );
}
