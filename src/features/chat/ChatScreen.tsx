import { useEffect, useRef, useState } from "react";
import { Link, useSearchParams } from "react-router";
import { useI18n } from "@/app/useI18n";
import { useAppStore } from "@/app/store";
import { Button } from "@/components/Button";
import { Icon } from "@/components/Icon";
import { api, IpcError } from "@/lib/ipc/client";
import {
  chatApi,
  type ChatEvent,
  type ChatMessage,
  type ChatOptions,
  type ChatThread,
} from "@/lib/ipc/chat";
import { hookApi, type HookStatusView } from "@/lib/ipc/hook";
import { scriptApi, type ScriptDocument } from "@/lib/ipc/script";
import type { LlmProfiles } from "@/lib/ipc/types";
import { ChatMessageView } from "./ChatMessageView";

/** The companion's personalities, mirroring `vn_core::chat::Persona::ALL`. */
const CHAT_PERSONAS = [
  { id: "companion", label: "vnchat.persona.companion" },
  { id: "kawaii", label: "vnchat.persona.kawaii" },
  { id: "analyst", label: "vnchat.persona.analyst" },
  { id: "playful", label: "vnchat.persona.playful" },
  { id: "concise", label: "vnchat.persona.concise" },
] as const;

export function applyChatEvent(
  message: ChatMessage | null,
  event: ChatEvent,
): ChatMessage | null {
  if (event.type === "started") return event.message;
  if (!message) return null;
  if (event.type === "delta")
    return { ...message, content: message.content + event.text };
  if (event.type === "sources") return { ...message, sources: event.sources };
  const index = message.tools.findIndex(
    (trace) => trace.id === event.trace.id && trace.status === "running",
  );
  return {
    ...message,
    tools:
      index < 0
        ? [...message.tools, event.trace]
        : message.tools.map((trace, i) => (i === index ? event.trace : trace)),
  };
}

const localEndpoint = (url: string) => {
  try {
    const parsed = new URL(url);
    return (
      parsed.protocol === "http:" &&
      ["localhost", "127.0.0.1", "[::1]"].includes(parsed.hostname)
    );
  } catch {
    return false;
  }
};

export function ChatScreen() {
  const { t, locale } = useI18n();
  const cards = useAppStore((state) => state.cards);
  const [params] = useSearchParams();
  const [threads, setThreads] = useState<ChatThread[]>([]);
  const [current, setCurrent] = useState<ChatThread | null>(null);
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [live, setLive] = useState<ChatMessage | null>(null);
  const [provider, setProvider] = useState<LlmProfiles | null>(null);
  const [hookStatus, setHookStatus] = useState<HookStatusView | null>(null);
  const [scriptDocs, setScriptDocs] = useState<{
    gameId: string;
    docs: ScriptDocument[];
  } | null>(null);
  const [atBottom, setAtBottom] = useState(true);
  const queryGame = params.get("game");
  const [lastQueryGame, setLastQueryGame] = useState(queryGame);
  const [options, setOptions] = useState<ChatOptions>({
    gameId: queryGame,
    spoilers: "none",
    boundary: "",
    allowOnline: false,
    language: locale,
    persona: "companion",
  });
  const [draft, setDraft] = useState("");
  const [title, setTitle] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  if (lastQueryGame !== queryGame) {
    setLastQueryGame(queryGame);
    if (!current && messages.length === 0 && !busy && queryGame) {
      setOptions((previous) =>
        previous.gameId === queryGame
          ? previous
          : { ...previous, gameId: queryGame },
      );
    }
  }
  const [cancellable, setCancellable] = useState(false);
  const [visible, setVisible] = useState(30);
  const [deleting, setDeleting] = useState(false);
  const endRef = useRef<HTMLDivElement>(null);
  const draftRef = useRef<HTMLTextAreaElement>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  const follow = useRef(true);
  const requestId = useRef<string | null>(null);
  const mounted = useRef(false);
  const selection = useRef(0);
  const operation = useRef(false);
  const errorCode = (caught: unknown) =>
    caught instanceof IpcError ? caught.message : "unknown";
  const errorLabel = (code: string) => {
    const key = `vnchat.error.${code}`;
    const value = t(key);
    return value === key ? t("vnchat.error.unknown") : value;
  };

  /* Height follows the content up to the CSS max-height, so the box starts as a
   * single line instead of reserving three empty ones. */
  function grow() {
    const box = draftRef.current;
    if (!box) return;
    box.style.height = "auto";
    box.style.height = `${box.scrollHeight}px`;
  }

  useEffect(() => {
    mounted.current = true;
    const fetchHook = () => {
      hookApi
        .status()
        .then((status) => {
          if (!mounted.current) return;
          setHookStatus((prev) => {
            if (
              prev &&
              prev.connected === status.connected &&
              prev.lineCount === status.lineCount &&
              prev.lines.at(-1)?.at === status.lines.at(-1)?.at &&
              prev.lines.at(-1)?.text === status.lines.at(-1)?.text
            ) {
              return prev;
            }
            return status;
          });
        })
        .catch(() => {});
    };
    fetchHook();
    const hookTimer = setInterval(fetchHook, 3000);

    void Promise.all([chatApi.list(), api.aiLlmProfiles()])
      .then(([list, profiles]) => {
        if (mounted.current) {
          setThreads(list);
          setProvider(profiles);
          setLoading(false);
        }
      })
      .catch((caught) => {
        if (mounted.current) {
          setError(errorCode(caught));
          setLoading(false);
        }
      });
    return () => {
      mounted.current = false;
      clearInterval(hookTimer);
      if (requestId.current) void chatApi.cancel(requestId.current).catch(() => {});
    };
  }, []);

  useEffect(() => {
    grow();
  }, [draft]);

  useEffect(() => {
    const gameId = options.gameId;
    if (!gameId) return;
    let active = true;
    scriptApi
      .list(gameId)
      .then((docs) => {
        if (active && mounted.current) setScriptDocs({ gameId, docs });
      })
      .catch(() => {
        if (active && mounted.current) setScriptDocs(null);
      });
    return () => {
      active = false;
    };
  }, [options.gameId]);

  useEffect(() => {
    if (follow.current)
      endRef.current?.scrollIntoView?.({ block: "end", behavior: "instant" });
  }, [live?.content, live?.tools.length, messages.length]);

  async function open(thread: ChatThread) {
    if (busy) return;
    const version = ++selection.current;
    setLoading(true);
    setError(null);
    setDeleting(false);
    try {
      const saved = await chatApi.messages(thread.id);
      if (version !== selection.current || !mounted.current) return;
      setCurrent(thread);
      setTitle(thread.title);
      setOptions(thread.options);
      setMessages(saved);
      setLive(null);
      setVisible(30);
      setDraft("");
      follow.current = true;
      setAtBottom(true);
    } catch (caught) {
      if (version === selection.current && mounted.current) {
        setError(errorCode(caught));
      }
    } finally {
      if (version === selection.current && mounted.current) setLoading(false);
    }
  }

  function newChat() {
    ++selection.current;
    setCurrent(null);
    setTitle("");
    setMessages([]);
    setLive(null);
    setDraft("");
    setError(null);
    setDeleting(false);
    setVisible(30);
    follow.current = true;
    setAtBottom(true);
    setOptions({
      gameId: params.get("game"),
      spoilers: "none",
      boundary: "",
      allowOnline: false,
      language: locale,
      persona: "companion",
    });
  }

  async function send(retry = false) {
    if (
      operation.current ||
      busy ||
      loading ||
      (!retry && !draft.trim()) ||
      (options.spoilers === "boundary" && !options.boundary.trim()) ||
      !usable
    )
      return;
    operation.current = true;
    setBusy(true);
    setError(null);
    setLive(null);
    setCancellable(false);
    follow.current = true;
    setAtBottom(true);
    let thread = current;
    const text = draft.trim();
    try {
      if (!thread) {
        thread = await chatApi.create({ ...options, language: locale });
        if (!mounted.current) return;
        setCurrent(thread);
        setThreads((previous) => [thread as ChatThread, ...previous]);
      } else if (messages.length === 0) {
        thread = await chatApi.update(thread.id, thread.title, {
          ...options,
          language: locale,
        });
        if (!mounted.current) return;
        setCurrent(thread);
      }
      requestId.current = thread.id;
      setCancellable(true);
      const result = await chatApi.send(thread.id, text, retry, (event) => {
        if (!mounted.current) return;
        if (event.type === "started") {
          setCancellable(true);
          if (!retry) setDraft("");
          if (retry) setMessages((previous) => previous.slice(0, -1));
          else
            setMessages((previous) => [
              ...previous,
              {
                id: `pending-${event.message.id}`,
                role: "user",
                content: text,
                status: "complete",
                tools: [],
                sources: [],
                error: null,
                tokens: null,
                createdAt: Date.now(),
              },
            ]);
        }
        setLive((previous) => applyChatEvent(previous, event));
      });
      if (!mounted.current) return;
      setLive(null);
      if (result.error && result.status !== "cancelled") setError(result.error);
      const [saved, list] = await Promise.all([
        chatApi.messages(thread.id),
        chatApi.list(),
      ]);
      if (mounted.current) {
        setMessages(saved);
        setThreads(list);
        const updated = list.find((item) => item.id === thread?.id);
        if (updated) {
          setCurrent(updated);
          setTitle(updated.title);
        }
      }
    } catch (caught) {
      if (mounted.current) {
        setError(errorCode(caught));
        setLive(null);
        if (thread) {
          try {
            const saved = await chatApi.messages(thread.id);
            if (mounted.current) setMessages(saved);
          } catch {
            /* Original failure stays visible. */
          }
        }
      }
    } finally {
      operation.current = false;
      requestId.current = null;
      if (mounted.current) {
        setBusy(false);
        setCancellable(false);
      }
    }
  }

  async function remove() {
    if (!current || operation.current) return;
    operation.current = true;
    setBusy(true);
    try {
      await chatApi.delete(current.id);
      setThreads((previous) => previous.filter((item) => item.id !== current.id));
      newChat();
    } catch (caught) {
      setError(errorCode(caught));
    } finally {
      operation.current = false;
      setBusy(false);
    }
  }
  async function rename() {
    if (!current || operation.current) return;
    operation.current = true;
    setBusy(true);
    try {
      const updated = await chatApi.update(current.id, title, options);
      setCurrent(updated);
      setThreads((previous) =>
        previous.map((item) => (item.id === updated.id ? updated : item)),
      );
    } catch (caught) {
      setError(errorCode(caught));
    } finally {
      operation.current = false;
      setBusy(false);
    }
  }

  /* The composer needs an endpoint that can answer: the active profile, with a
   * stored key or a local runtime that does not need one. A saved profile that
   * is not the active one does not count. */
  const activeProfile =
    provider?.profiles.find((profile) => profile.id === provider.active) ?? null;
  const usable = Boolean(
    activeProfile &&
    (activeProfile.hasKey || localEndpoint(activeProfile.baseUrl)),
  );
  const configured = provider !== null && usable;
  const locked = messages.length > 0 || busy;
  const last = messages.at(-1);
  return (
    <section className="vnchat">
      <header className="vnchat__header">
        <div>
          <h1 className="page-title">{t("vnchat.title")}</h1>
          <p className="page-sub">{t("vnchat.intro")}</p>
        </div>
        <Button variant="secondary" onClick={newChat} disabled={busy || loading}>
          <Icon name="plus" />
          {t("vnchat.new")}
        </Button>
      </header>
      <div className="vnchat__layout">
        <aside className="vnchat__sidebar">
          <label className="field">
            <span className="field__label">{t("vnchat.conversations")}</span>
            <select
              className="input"
              value={current?.id ?? ""}
              disabled={busy || loading}
              onChange={(event) => {
                const thread = threads.find((item) => item.id === event.target.value);
                if (thread) void open(thread);
                else newChat();
              }}
            >
              <option value="">{t("vnchat.new")}</option>
              {threads.map((thread) => (
                <option key={thread.id} value={thread.id}>
                  {thread.title || t("vnchat.untitled")}
                </option>
              ))}
            </select>
          </label>
          {current && (
            <details className="vnchat__manage">
              <summary>{t("vnchat.manage")}</summary>
              <label className="field">
                <span className="field__label">{t("vnchat.name")}</span>
                <input
                  className="input"
                  value={title}
                  maxLength={120}
                  disabled={busy}
                  onChange={(event) => setTitle(event.target.value)}
                />
              </label>
              <div className="vnchat__manage-actions">
                <Button variant="ghost" disabled={busy} onClick={() => void rename()}>
                  {t("vnchat.rename")}
                </Button>
                <Button variant="ghost" disabled={busy} onClick={() => setDeleting(true)}>
                  {t("vnchat.delete")}
                </Button>
              </div>
              {deleting && (
                <div className="notice">
                  <p>{t("vnchat.deleteConfirm")}</p>
                  <div className="vnchat__manage-actions">
                    <Button disabled={busy} onClick={() => void remove()}>
                      {t("vnchat.delete")}
                    </Button>
                    <Button variant="ghost" onClick={() => setDeleting(false)}>
                      {t("common.cancel")}
                    </Button>
                  </div>
                </div>
              )}
            </details>
          )}

          {/* Context is the sidebar's primary job: what is this chat allowed to
              see, and how much may it reveal. They lock after the first message
              because a later narrowing would leave older answers leaking. */}
          <div className="vnchat__group vnchat__group--scope">
            <span className="vnchat__label">{t("vnchat.scope")}</span>
            <select
              className="vnchat__select"
              aria-label={t("vnchat.scope")}
              value={options.gameId ?? ""}
              disabled={locked}
              onChange={(event) =>
                setOptions((previous) => ({
                  ...previous,
                  gameId: event.target.value || null,
                }))
              }
            >
              <option value="">{t("vnchat.allLibrary")}</option>
              {options.gameId &&
                !cards.some((card) => card.game.meta.id === options.gameId) && (
                  <option value={options.gameId}>{t("vnchat.missingGame")}</option>
                )}
              {cards.map((card) => (
                <option key={card.game.meta.id} value={card.game.meta.id}>
                  {card.game.title}
                </option>
              ))}
            </select>
          </div>

          <div className="vnchat__group vnchat__group--rules">
            <span className="vnchat__label">{t("vnchat.spoilers")}</span>
            <div
              className="vnchat__segments"
              role="group"
              aria-label={t("vnchat.spoilers")}
            >
              {(
                [
                  ["none", "vnchat.spoilers.none"],
                  ["boundary", "vnchat.spoilers.boundary"],
                  ["all", "vnchat.spoilers.all"],
                ] as const
              ).map(([value, label]) => (
                <button
                  key={value}
                  type="button"
                  className="vnchat__segment"
                  aria-pressed={options.spoilers === value}
                  disabled={locked}
                  onClick={() =>
                    setOptions((previous) => ({ ...previous, spoilers: value }))
                  }
                >
                  {t(label)}
                </button>
              ))}
            </div>
            {options.spoilers === "boundary" && (
              <input
                className="vnchat__select"
                aria-label={t("vnchat.boundary")}
                aria-invalid={!locked && !options.boundary.trim()}
                value={options.boundary}
                maxLength={500}
                disabled={locked}
                placeholder={t("vnchat.boundaryHint")}
                onChange={(event) =>
                  setOptions((previous) => ({
                    ...previous,
                    boundary: event.target.value,
                  }))
                }
              />
            )}
            {locked && <p className="vnchat__hint">{t("vnchat.locked")}</p>}
          </div>

          <div className="vnchat__group vnchat__group--rules">
            <span className="vnchat__label">{t("vnchat.persona")}</span>
            <select
              className="vnchat__select"
              aria-label={t("vnchat.persona")}
              value={options.persona}
              disabled={locked}
              onChange={(event) =>
                setOptions((previous) => ({
                  ...previous,
                  persona: event.target
                    .value as typeof CHAT_PERSONAS[number]["id"],
                }))
              }
            >
              {CHAT_PERSONAS.map((persona) => (
                <option key={persona.id} value={persona.id}>
                  {t(persona.label)}
                </option>
              ))}
            </select>
            <p className="vnchat__hint">{t("vnchat.personaHint")}</p>
          </div>

          <div className="vnchat__group vnchat__group--rules">
            <label className="vnchat__check">
              <input
                type="checkbox"
                checked={options.allowOnline}
                disabled={locked}
                onChange={(event) =>
                  setOptions((previous) => ({
                    ...previous,
                    allowOnline: event.target.checked,
                  }))
                }
              />
              <span>{t("vnchat.online")}</span>
            </label>
            <p className="vnchat__hint">
              {t(
                options.spoilers === "all"
                  ? "vnchat.onlineHint"
                  : "vnchat.onlineRestricted",
              )}
            </p>
          </div>

          <div className="vnchat__group vnchat__group--rules">
            <div className="vnchat__status-row">
              <span className="vnchat__label">{t("vnchat.hookStatus")}</span>
              <span
                className="vnchat__pill"
                data-active={hookStatus?.connected ? "true" : "false"}
              >
                {hookStatus?.connected
                  ? t("vnchat.hookConnected", { count: hookStatus.lineCount })
                  : t("vnchat.hookDisconnected")}
              </span>
            </div>
            <p className="vnchat__hint">
              {hookStatus?.connected
                ? t("vnchat.hookHintActive")
                : t("vnchat.hookHintInactive")}
            </p>
          </div>

          {options.gameId && scriptDocs?.gameId === options.gameId && (
            <div className="vnchat__group vnchat__group--rules">
              {(() => {
                const currentDocs = scriptDocs.docs;
                const readyDoc = currentDocs.find((d) => Boolean(d.compaction?.digest));
                const latestDoc = currentDocs[0];
                return (
                  <>
                    <div className="vnchat__status-row">
                      <span className="vnchat__label">{t("vnchat.digestStatus")}</span>
                      <span
                        className="vnchat__pill"
                        data-active={readyDoc ? "true" : "false"}
                      >
                        {readyDoc
                          ? t("vnchat.digestReady")
                          : latestDoc
                            ? t("vnchat.digestProgress", {
                                done: latestDoc.compaction?.doneChunks ?? 0,
                                total:
                                  latestDoc.compaction?.chunkCount ??
                                  latestDoc.chunkCount,
                              })
                            : t("vnchat.digestNone")}
                      </span>
                    </div>
                    {readyDoc && (
                      <p className="vnchat__hint">
                        {t(
                          options.spoilers === "all"
                            ? "vnchat.digestHintAll"
                            : "vnchat.digestHintLocked",
                        )}
                      </p>
                    )}
                  </>
                );
              })()}
            </div>
          )}

          <div className="vnchat__provider">
            <span>{t("vnchat.provider")}</span>
            <strong title={activeProfile?.model ?? undefined}>
              {activeProfile?.model ?? "…"}
            </strong>
            <Link to="/settings">{t("vnchat.configure")}</Link>
          </div>
        </aside>
        <div className="vnchat__conversation">
          {loading ? (
            <div className="vnchat__empty">
              <div className="skeleton skeleton--row" />
              <p role="status">{t("common.loading")}</p>
            </div>
          ) : (
            <div
              className="vnchat__scroll"
              ref={scrollRef}
              onScroll={() => {
                const box = scrollRef.current;
                if (box) {
                  const near =
                    box.scrollHeight - box.scrollTop - box.clientHeight < 100;
                  follow.current = near;
                  setAtBottom(near);
                }
              }}
            >
              {messages.length === 0 && !live && (
                <div className="vnchat__empty">
                  <Icon name="chat" size={30} />
                  <h2>{t("vnchat.emptyTitle")}</h2>
                  <p>{t("vnchat.emptyBody")}</p>
                  <div className="vnchat__prompts">
                    {(
                      [
                        "library",
                        options.gameId ? "story" : "storyGeneral",
                        "taste",
                      ] as const
                    ).map((key) => (
                      <button
                        type="button"
                        className="vnchat__prompt"
                        key={key}
                        onClick={() => setDraft(t(`vnchat.prompt.${key}`))}
                      >
                        {t(`vnchat.prompt.${key}`)}
                      </button>
                    ))}
                  </div>
                </div>
              )}
              {messages.length > 0 && (
                <div className="vnchat__thread">
                  {messages.length > visible && (
                    <Button
                      variant="ghost"
                      onClick={() => {
                        follow.current = false;
                        setAtBottom(false);
                        setVisible((previous) => previous + 30);
                      }}
                    >
                      {t("vnchat.older")}
                    </Button>
                  )}
                  {messages.slice(-visible).map((message) => (
                    <ChatMessageView key={message.id} message={message} />
                  ))}
                  {live && <ChatMessageView message={live} />}
                  <div ref={endRef} />
                </div>
              )}
              {messages.length === 0 && live && (
                <div className="vnchat__thread">
                  <ChatMessageView message={live} />
                  <div ref={endRef} />
                </div>
              )}
            </div>
          )}
          <div className="vnchat__composer">
            <div className="vnchat__composer-inner">
              {!atBottom && (messages.length > 0 || live) && (
                <div className="vnchat__jump">
                  <Button
                    variant="secondary"
                    onClick={() => {
                      follow.current = true;
                      setAtBottom(true);
                      endRef.current?.scrollIntoView?.({
                        block: "end",
                        behavior: "smooth",
                      });
                    }}
                  >
                    {t("vnchat.jumpLatest")}
                  </Button>
                </div>
              )}
              {!configured && !loading && (
                <p className="notice">
                  {t("vnchat.needProvider")}{" "}
                  <Link to="/settings">{t("vnchat.configure")}</Link>
                </p>
              )}
              {error && (
                <p className="notice notice--danger" role="alert">
                  {errorLabel(error)}
                </p>
              )}
              <form
                onSubmit={(event) => {
                  event.preventDefault();
                  void send();
                }}
              >
                <label className="sr-only" htmlFor="vnchat-message">
                  {t("vnchat.message")}
                </label>
                <div className="vnchat__field">
                  <textarea
                    id="vnchat-message"
                    ref={draftRef}
                    className="vnchat__input"
                    placeholder={t("vnchat.placeholder")}
                    value={draft}
                    maxLength={16000}
                    disabled={busy || loading}
                    onChange={(event) => {
                      setDraft(event.target.value);
                      grow();
                    }}
                    onKeyDown={(event) => {
                      if (
                        (event.ctrlKey || event.metaKey) &&
                        event.key === "Enter" &&
                        !event.nativeEvent.isComposing
                      ) {
                        event.preventDefault();
                        void send();
                      }
                    }}
                    rows={1}
                  />
                  {busy ? (
                    <button
                      type="button"
                      className="vnchat__send vnchat__send--stop"
                      disabled={!cancellable}
                      title={t("vnchat.stop")}
                      aria-label={t("vnchat.stop")}
                      onClick={() => {
                        if (requestId.current)
                          void chatApi
                            .cancel(requestId.current)
                            .catch((caught) => setError(errorCode(caught)));
                      }}
                    >
                      <Icon name="stop" size={14} />
                      <span>{t("vnchat.stop")}</span>
                    </button>
                  ) : (
                    <button
                      type="submit"
                      className="vnchat__send"
                      disabled={
                        !configured ||
                        loading ||
                        !draft.trim() ||
                        (options.spoilers === "boundary" && !options.boundary.trim())
                      }
                      title={t("vnchat.send")}
                      aria-label={t("vnchat.send")}
                    >
                      <Icon name="send" size={16} />
                    </button>
                  )}
                </div>
                {!busy && last && ["error", "cancelled"].includes(last.status) && (
                  <div className="vnchat__meta">
                    <Button
                      variant="ghost"
                      disabled={!configured}
                      onClick={() => void send(true)}
                    >
                      {t("vnchat.retry")}
                    </Button>
                  </div>
                )}
              </form>
              <div className="vnchat__composer-foot">
                <details className="vnchat__footnote">
                  <summary>{t("vnchat.accuracyLabel")}</summary>
                  <p>{t("vnchat.accuracy")}</p>
                </details>
                <p className="vnchat__send-hint">{t("vnchat.sendHint")}</p>
              </div>
            </div>
          </div>
        </div>
      </div>
    </section>
  );
}
