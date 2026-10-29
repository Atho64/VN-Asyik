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
import type { ChatSettings } from "@/lib/ipc/types";
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
  const [settings, setSettings] = useState<ChatSettings | null>(null);
  const [hookStatus, setHookStatus] = useState<HookStatusView | null>(null);
  const [options, setOptions] = useState<ChatOptions>({
    gameId: params.get("game"),
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
          if (mounted.current) setHookStatus(status);
        })
        .catch(() => {});
    };
    fetchHook();
    const hookTimer = setInterval(fetchHook, 3000);

    void Promise.all([chatApi.list(), api.aiChatSettings()])
      .then(([list, provider]) => {
        if (mounted.current) {
          setThreads(list);
          setSettings(provider);
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
    } catch (caught) {
      setError(errorCode(caught));
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
      !settings ||
      (!settings.hasKey && !localEndpoint(settings.baseUrl))
    )
      return;
    operation.current = true;
    setBusy(true);
    setError(null);
    setLive(null);
    setCancellable(false);
    follow.current = true;
    let thread = current;
    const text = draft.trim();
    try {
      if (!thread) {
        thread = await chatApi.create({ ...options, language: locale });
        if (!mounted.current) return;
        setCurrent(thread);
        setThreads((previous) => [thread as ChatThread, ...previous]);
      } else {
        thread = await chatApi.update(thread.id, title, { ...options, language: locale });
        if (!mounted.current) return;
        setCurrent(thread);
      }
      requestId.current = thread.id;
      const result = await chatApi.send(thread.id, text, retry, (event) => {
        if (!mounted.current) return;
        if (event.type === "started") {
          setCancellable(true);
          setDraft("");
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

  const configured = settings && (settings.hasKey || localEndpoint(settings.baseUrl));
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
                disabled={busy}
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
            <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: "4px" }}>
              <span className="vnchat__label">{t("vnchat.hookStatus")}</span>
              <span
                style={{
                  fontSize: "0.725rem",
                  padding: "2px 8px",
                  borderRadius: "999px",
                  fontWeight: 600,
                  background: hookStatus?.connected ? "rgba(34, 197, 94, 0.15)" : "rgba(148, 163, 184, 0.15)",
                  color: hookStatus?.connected ? "#22c55e" : "var(--color-text-subtle)",
                  border: hookStatus?.connected ? "1px solid rgba(34, 197, 94, 0.3)" : "1px solid var(--color-border)",
                }}
              >
                {hookStatus?.connected
                  ? t("vnchat.hookConnected", { count: hookStatus.lineCount })
                  : t("vnchat.hookDisconnected")}
              </span>
            </div>
            <p className="vnchat__hint">
              {hookStatus?.connected
                ? t("vnchat.hookHintActive", { count: hookStatus.lines.length })
                : t("vnchat.hookHintInactive")}
            </p>
          </div>

          <div className="vnchat__provider">
            <span>{t("vnchat.provider")}</span>
            <strong title={settings?.model ?? undefined}>{settings?.model ?? "…"}</strong>
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
                if (box)
                  follow.current =
                    box.scrollHeight - box.scrollTop - box.clientHeight < 100;
              }}
            >
              {messages.length === 0 && !live && (
                <div className="vnchat__empty">
                  <Icon name="chat" size={30} />
                  <h2>{t("vnchat.emptyTitle")}</h2>
                  <p>{t("vnchat.emptyBody")}</p>
                  <div className="vnchat__prompts">
                    {["library", "story", "taste"].map((key) => (
                      <button
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
              <details className="vnchat__footnote">
                <summary>{t("vnchat.accuracyLabel")}</summary>
                <p>{t("vnchat.accuracy")}</p>
              </details>
            </div>
          </div>
        </div>
      </div>
    </section>
  );
}
