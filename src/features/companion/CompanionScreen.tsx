import { useCallback, useEffect, useRef, useState } from "react";
import { Link, useSearchParams } from "react-router";
import { useI18n } from "@/app/useI18n";
import { Button } from "@/components/Button";
import { Icon } from "@/components/Icon";
import { Sheet } from "@/components/Sheet";
import { api } from "@/lib/ipc/client";
import type { LlmProfiles } from "@/lib/ipc/types";
import type { ChatMessage } from "@/lib/ipc/chat";
import {
  companionApi,
  type CompanionSession,
  type SessionOptions,
} from "@/lib/ipc/companion";
import { ChatMessageView } from "@/features/chat/ChatMessageView";
import { CompanionSetup } from "./CompanionSetup";
import { CompanionVision } from "./CompanionVision";
import { CompanionMemory } from "./CompanionMemory";
import { personalityName, SessionPersonality } from "./CompanionPersonality";
import { companionWindowApi } from "@/lib/ipc/companionWindow";
import { OpenOverlayButton, windowAction } from "./OverlayActions";
import { companionError } from "./errors";
import {
  acceptSnapshot,
  failureCode,
  performAction,
  sendQuestion,
  useCompanionStore,
} from "./store";

function isLocal(url: string): boolean {
  try {
    const parsed = new URL(url);
    return (
      parsed.protocol === "http:" &&
      ["localhost", "127.0.0.1", "[::1]"].includes(parsed.hostname)
    );
  } catch {
    return false;
  }
}

export function CompanionScreen({ overlay = false }: { overlay?: boolean }) {
  const { t, locale } = useI18n();
  const [params, setParams] = useSearchParams();
  const snapshot = useCompanionStore((state) => state.snapshot);
  const connected = useCompanionStore((state) => state.connected);
  const connectionError = useCompanionStore((state) => state.connectionError);
  const pending = useCompanionStore((state) => state.actionPending);
  const actionError = useCompanionStore((state) => state.error);
  const draft = useCompanionStore((state) => state.draft);
  const [setup, setSetup] = useState(false);
  const [sessions, setSessions] = useState<CompanionSession[]>([]);
  const [historyId, setHistoryId] = useState<string | null>(null);
  const [history, setHistory] = useState<{ id: string; messages: ChatMessage[] } | null>(
    null,
  );
  const [historyError, setHistoryError] = useState<string | null>(null);
  const [provider, setProvider] = useState<LlmProfiles | null>(null);
  const [visible, setVisible] = useState(30);
  const [deleteConfirm, setDeleteConfirm] = useState(false);
  const [optionsOpen, setOptionsOpen] = useState(false);
  const [memoryOpen, setMemoryOpen] = useState(false);
  const [memoryPassage, setMemoryPassage] = useState<ChatMessage | null>(null);
  const memoryRequested = !overlay && params.get("memory") === "1";
  const closeMemory = useCallback(() => {
    setMemoryOpen(false);
    setMemoryPassage(null);
    if (params.has("memory")) {
      const next = new URLSearchParams(params);
      next.delete("memory");
      void setParams(next, { replace: true });
    }
  }, [params, setParams]);
  // Sheet must keep focus in place while streaming snapshots refresh the page.
  const closeOptions = useCallback(() => {
    setOptionsOpen(false);
    setDeleteConfirm(false);
  }, []);
  const end = useRef<HTMLDivElement>(null);
  const follow = useRef(true);
  const active = snapshot?.session && snapshot.lifecycle !== "off";
  const session = historyId
    ? sessions.find((item) => item.id === historyId)
    : snapshot?.session;
  const viewingLive = !historyId && Boolean(active);
  const gameId = params.get("game");
  const showSetup =
    setup ||
    !session ||
    Boolean(gameId && !historyId && (!active || gameId !== session.options.gameId));
  const sessionId = session?.id;
  const live = !historyId ? snapshot?.live : null;
  const busy = Boolean(viewingLive && snapshot?.busy);
  const showingSession = Boolean(session && (overlay ? active : !showSetup));

  useEffect(() => {
    let alive = true;
    const refresh = () => {
      void api
        .aiLlmProfiles()
        .then((profiles) => {
          if (alive) setProvider(profiles);
        })
        .catch(() => {});
    };
    refresh();
    window.addEventListener("focus", refresh);
    return () => {
      alive = false;
      window.removeEventListener("focus", refresh);
    };
  }, []);

  useEffect(() => {
    if (overlay) return;
    let alive = true;
    void companionApi
      .list()
      .then((list) => {
        if (alive) setSessions(list);
      })
      .catch((error) => {
        if (alive) setHistoryError(failureCode(error));
      });
    return () => {
      alive = false;
    };
  }, [overlay, snapshot?.session?.id, snapshot?.session?.state]);

  useEffect(() => {
    if (!sessionId) return;
    let alive = true;
    void companionApi
      .messages(sessionId)
      .then((messages) => {
        if (alive) {
          setHistory({ id: sessionId, messages });
          setHistoryError(null);
        }
      })
      .catch((error) => {
        if (alive) {
          setHistory({ id: sessionId, messages: [] });
          setHistoryError(failureCode(error));
        }
      });
    return () => {
      alive = false;
    };
  }, [sessionId, live?.id, busy, snapshot?.session?.state]);

  useEffect(() => {
    if (follow.current)
      end.current?.scrollIntoView?.({ block: "end", behavior: "instant" });
  }, [live?.content, history?.messages.length]);

  const profile = provider?.profiles.find((profile) => profile.id === provider.active);
  const usable = profile && (profile.hasKey || isLocal(profile.baseUrl));
  const error = actionError ?? snapshot?.error ?? historyError;
  const saved = history && history.id === sessionId ? history.messages : [];
  const messages = saved.map((message) => (live?.id === message.id ? live : message));
  if (live && !messages.some((message) => message.id === live.id)) messages.push(live);

  async function begin(options: SessionOptions) {
    const started = await performAction(async () => {
      if (active && snapshot?.session) {
        acceptSnapshot(await companionApi.control(snapshot.session.id, "end"));
      }
      return companionApi.start(crypto.randomUUID(), options);
    });
    if (started) {
      void setParams({});
      setSetup(false);
      setHistoryId(null);
      setVisible(30);
      setDeleteConfirm(false);
      useCompanionStore.setState({ draft: "", submitted: null });
      follow.current = true;
    }
  }

  async function remove() {
    if (!session || (active && session.id === snapshot?.session?.id)) return;
    useCompanionStore.setState({ actionPending: true, error: null });
    try {
      await companionApi.delete(session.id);
      acceptSnapshot(await companionApi.snapshot());
      setSessions(await companionApi.list());
      setHistoryId(null);
      setDeleteConfirm(false);
      setSetup(true);
      closeOptions();
    } catch (error) {
      useCompanionStore.setState({ error: failureCode(error) });
    } finally {
      useCompanionStore.setState({ actionPending: false });
    }
  }

  return (
    <section className={overlay ? "companion companion--overlay" : "companion"}>
      <header className="companion__header">
        <div className="companion__heading">
          <h1
            title={
              showingSession
                ? (session?.gameTitle ?? t("companion.removedGame"))
                : undefined
            }
          >
            {showingSession
              ? (session?.gameTitle ?? t("companion.removedGame"))
              : t("companion.title")}
          </h1>
          {showingSession && (
            <p className="companion__muted" role="status">
              {viewingLive && snapshot
                ? t(`companion.sourceState.${snapshot.source.state}`)
                : t("companion.ended")}
            </p>
          )}
        </div>
        <div className="companion__actions">
          {!overlay && <OpenOverlayButton compact />}
          {showingSession && viewingLive && session && snapshot && (
            <Button
              variant="ghost"
              size="icon"
              aria-label={t(
                snapshot.lifecycle === "paused" ? "companion.resume" : "companion.pause",
              )}
              title={t(
                snapshot.lifecycle === "paused" ? "companion.resume" : "companion.pause",
              )}
              disabled={pending}
              onClick={() =>
                void performAction(() =>
                  companionApi.control(
                    session.id,
                    snapshot.lifecycle === "paused" ? "resume" : "pause",
                  ),
                )
              }
            >
              <Icon name={snapshot.lifecycle === "paused" ? "play" : "pause"} />
            </Button>
          )}
          {snapshot && (
            <Button
              variant="ghost"
              size="icon"
              aria-label={t("companion.options")}
              title={t("companion.options")}
              aria-haspopup="dialog"
              aria-expanded={optionsOpen}
              onClick={() => setOptionsOpen(true)}
            >
              <Icon name="settings" />
            </Button>
          )}
          {overlay && (
            <Button
              variant="ghost"
              size="icon"
              aria-label={t("companion.hideOverlay")}
              title={t("companion.hideOverlayHint")}
              onClick={() => void windowAction(companionWindowApi.hide)}
            >
              <Icon name="eyeOff" />
            </Button>
          )}
        </div>
      </header>

      {connectionError && (
        <div className="companion__notice" role="alert">
          <p>{t("companion.connectionError")}</p>
          <Button onClick={() => void performAction(companionApi.snapshot)}>
            {t("companion.retry")}
          </Button>
        </div>
      )}
      {error && (
        <p className="companion__error" role="alert">
          {companionError(t, error)}
        </p>
      )}
      {!snapshot ? (
        <p role="status">{t("companion.loading")}</p>
      ) : (
        <>
          {showSetup && !overlay && (
            <div className="companion__provider">
              <span>
                {profile
                  ? t("companion.provider", { name: profile.name, model: profile.model })
                  : t("companion.noProvider")}
              </span>
              <Link to="/settings">{t("companion.openSettings")}</Link>
            </div>
          )}
          {snapshot.capabilities.backgroundRuntime === "unsupported" && (
            <p className="companion__muted">{t("companion.mobileLimit")}</p>
          )}
          {overlay && (!session || !active) ? (
            <div className="companion__empty">
              <Icon name="chat" size={28} />
              <h2>{t("companion.overlayEmpty")}</h2>
              <p>{t("companion.overlaySetupHint")}</p>
              <Button
                variant="primary"
                onClick={() => void windowAction(() => companionWindowApi.showMain())}
              >
                {t("companion.openMain")}
              </Button>
            </div>
          ) : showSetup && !overlay ? (
            <CompanionSetup
              key={gameId ?? snapshot.session?.id ?? "new"}
              gameId={gameId}
              initial={snapshot.session?.options}
              replacing={Boolean(active)}
              pending={pending || !connected || snapshot.busy}
              onStart={begin}
              onCancel={
                session
                  ? () => {
                      void setParams({});
                      setSetup(false);
                    }
                  : undefined
              }
            />
          ) : (
            session && (
              <div className="companion__session">
                {viewingLive && <CompanionVision snapshot={snapshot} overlay={overlay} />}
                {session.replayExcluded && (
                  <p className="companion__notice">{t("memory.replayExcluded")}</p>
                )}
                {session.reason &&
                  !["user", "ended_by_user"].includes(session.reason) && (
                    <p className="companion__notice">
                      {t(`companion.reason.${session.reason}`)}
                    </p>
                  )}
                <div
                  className="companion__conversation"
                  aria-label={t("companion.conversation")}
                  onScroll={(event) => {
                    const area = event.currentTarget;
                    follow.current =
                      area.scrollHeight - area.scrollTop - area.clientHeight < 80;
                  }}
                >
                  {messages.length > visible && (
                    <Button
                      variant="ghost"
                      onClick={() => setVisible((count) => count + 30)}
                    >
                      {t("companion.olderMessages")}
                    </Button>
                  )}
                  {history?.id !== session.id ? (
                    <p role="status">{t("companion.loading")}</p>
                  ) : (
                    messages.length === 0 && (
                      <p className="companion__empty">{t("companion.emptyHint")}</p>
                    )
                  )}
                  {messages.slice(-visible).map((message) => (
                    <div className="companion-memory__message" key={message.id}>
                      <ChatMessageView
                        key={message.id}
                        message={message}
                        compactTools
                        onOpenGame={
                          overlay
                            ? (id) =>
                                void windowAction(() =>
                                  companionWindowApi.showMain(
                                    `/game/${encodeURIComponent(id)}`,
                                  ),
                                )
                            : undefined
                        }
                      />
                      {!overlay &&
                        message.status === "complete" &&
                        session.options.knowledge !== "metadata" && (
                          <Button
                            variant="ghost"
                            onClick={() => {
                              setMemoryPassage(message);
                              setMemoryOpen(true);
                            }}
                          >
                            {t("memory.remember")}
                          </Button>
                        )}
                    </div>
                  ))}
                  {busy && !live && <p role="status">{t("companion.replying")}</p>}
                  <div ref={end} />
                </div>
                {viewingLive ? (
                  <form
                    className="companion__composer"
                    onSubmit={(event) => {
                      event.preventDefault();
                      if (
                        (!busy || snapshot.vision.automatic) &&
                        connected &&
                        snapshot.lifecycle === "active" &&
                        usable
                      ) {
                        follow.current = true;
                        void sendQuestion(session.id);
                      }
                    }}
                  >
                    {!usable && (
                      <p className="companion__muted">
                        {t("companion.providerRequired")}{" "}
                        {overlay ? (
                          <Button
                            variant="ghost"
                            onClick={() =>
                              void windowAction(() =>
                                companionWindowApi.showMain("/settings"),
                              )
                            }
                          >
                            {t("companion.openSettings")}
                          </Button>
                        ) : (
                          <Link to="/settings">{t("companion.openSettings")}</Link>
                        )}
                      </p>
                    )}
                    <label className="companion__field">
                      <span>{t("companion.message")}</span>
                      <textarea
                        rows={2}
                        maxLength={16000}
                        value={draft}
                        placeholder={t("companion.placeholder")}
                        onChange={(event) =>
                          useCompanionStore.setState({ draft: event.target.value })
                        }
                        onKeyDown={(event) => {
                          if (
                            (event.ctrlKey || event.metaKey) &&
                            event.key === "Enter" &&
                            !event.nativeEvent.isComposing
                          ) {
                            event.preventDefault();
                            event.currentTarget.form?.requestSubmit();
                          }
                        }}
                      />
                    </label>
                    {busy && !snapshot.vision.automatic ? (
                      <Button
                        size="icon"
                        className="companion__send"
                        aria-label={t("companion.stopReply")}
                        title={t("companion.stopReply")}
                        disabled={pending}
                        onClick={() =>
                          void performAction(() =>
                            companionApi.control(session.id, "cancel_turn"),
                          )
                        }
                      >
                        <Icon name="stop" />
                      </Button>
                    ) : (
                      <Button
                        type="submit"
                        variant="primary"
                        size="icon"
                        className="companion__send"
                        aria-label={t("companion.send")}
                        title={`${t("companion.send")} (${t("companion.sendHint")})`}
                        disabled={
                          pending ||
                          (busy && !snapshot.vision.automatic) ||
                          !connected ||
                          !usable ||
                          snapshot.lifecycle !== "active" ||
                          !draft.trim()
                        }
                      >
                        <Icon name="send" />
                      </Button>
                    )}
                  </form>
                ) : active ? (
                  <div className="companion__archive-actions">
                    <Button onClick={() => setHistoryId(null)}>
                      {t("companion.returnLive")}
                    </Button>
                  </div>
                ) : null}
              </div>
            )
          )}
        </>
      )}
      <Sheet
        open={optionsOpen && !memoryRequested}
        title={t("companion.options")}
        onClose={closeOptions}
      >
        <div className="companion companion--options">
          {error && (
            <p className="companion__error" role="alert">
              {companionError(t, error)}
            </p>
          )}
          {session && (
            <div className="companion__options-section">
              <h3>{session.gameTitle ?? t("companion.removedGame")}</h3>
              <p>{session.options.playthroughLabel}</p>
              <p>
                {personalityName(t, session.options)} ·{" "}
                {t(`companion.knowledge.${session.options.knowledge}`)}
              </p>
              {session.options.boundary && <p>{session.options.boundary}</p>}
              <p className="companion__muted">
                {t(`companion.knowledgeHint.${session.options.knowledge}`)}
              </p>
              {!viewingLive && (
                <p className="companion__muted">{t("companion.historyHint")}</p>
              )}
            </div>
          )}
          {viewingLive && session && optionsOpen && (
            <SessionPersonality
              key={`${session.id}:${session.options.profile?.id ?? session.options.persona}:${session.options.profile?.version ?? 0}`}
              id={session.id}
              options={session.options}
              disabled={pending || !connected}
            />
          )}
          <div className="companion__options-section">
            <h3>{t("memory.title")}</h3>
            <p className="companion__muted">{t("memory.intro")}</p>
            <Button
              onClick={() => {
                closeOptions();
                if (overlay)
                  void windowAction(() =>
                    companionWindowApi.showMain("/companion?memory=1"),
                  );
                else setMemoryOpen(true);
              }}
            >
              {t("memory.open")}
            </Button>
          </div>
          <div className="companion__provider">
            <span>
              {profile
                ? t("companion.provider", { name: profile.name, model: profile.model })
                : t("companion.noProvider")}
            </span>
            {overlay ? (
              <Button
                variant="ghost"
                onClick={() =>
                  void windowAction(() => companionWindowApi.showMain("/settings"))
                }
              >
                {t("companion.openSettings")}
              </Button>
            ) : (
              <Link to="/settings">{t("companion.openSettings")}</Link>
            )}
          </div>
          {!overlay && sessions.length > 0 && (
            <label className="companion__field">
              <span>{t("companion.history")}</span>
              <select
                aria-label={t("companion.selectSession")}
                value={historyId ?? ""}
                onChange={(event) => {
                  void setParams({});
                  setHistoryId(event.target.value || null);
                  setSetup(false);
                  setVisible(30);
                  closeOptions();
                  follow.current = true;
                }}
              >
                <option value="">
                  {t(active ? "companion.liveSession" : "companion.latestSession")}
                </option>
                {sessions
                  .filter((item) => item.id !== snapshot?.session?.id)
                  .map((item) => (
                    <option key={item.id} value={item.id}>
                      {item.gameTitle ?? t("companion.removedGame")} ·{" "}
                      {item.options.playthroughLabel} ·{" "}
                      {new Date(item.createdAt).toLocaleDateString(locale)}
                    </option>
                  ))}
              </select>
            </label>
          )}
          {viewingLive && session?.options.source === "luna" && snapshot && (
            <div className="companion__options-section">
              <h3>{t("companion.preview")}</h3>
              <p className="companion__muted">
                {t(
                  snapshot.processId
                    ? "companion.processBound"
                    : "companion.manualBinding",
                )}
              </p>
              {snapshot.source.lastLineAt != null && (
                <small>
                  {t("companion.lastText", {
                    time: new Date(snapshot.source.lastLineAt).toLocaleTimeString(locale),
                  })}
                </small>
              )}
              {snapshot.source.preview.map((line, index) => (
                <blockquote key={index} className="companion__dialogue">
                  {line}
                </blockquote>
              ))}
            </div>
          )}
          {snapshot?.capabilities.overlay === "unsupported" && (
            <p className="companion__muted">{t("companion.overlayUnsupported")}</p>
          )}
          <div className="companion__actions">
            {overlay ? (
              <Button
                onClick={() => void windowAction(() => companionWindowApi.showMain())}
              >
                {t("companion.openMain")}
              </Button>
            ) : (
              <>
                {!showSetup && (
                  <Button
                    disabled={pending || snapshot?.busy}
                    onClick={() => {
                      closeOptions();
                      setSetup(true);
                    }}
                  >
                    <Icon name="plus" />
                    {t("companion.newSession")}
                  </Button>
                )}
                <Link className="btn btn--ghost" to="/chat">
                  {t("companion.regularChat")}
                </Link>
              </>
            )}
            {viewingLive && session && (
              <Button
                disabled={pending}
                onClick={() => {
                  closeOptions();
                  void performAction(() => companionApi.control(session.id, "end"));
                }}
              >
                <Icon name="stop" />
                {t("companion.end")}
              </Button>
            )}
          </div>
          {!overlay &&
            session &&
            !viewingLive &&
            (deleteConfirm ? (
              <div className="companion__notice">
                <p>{t("companion.deleteConfirm")}</p>
                <div className="companion__actions">
                  <Button
                    variant="danger"
                    disabled={pending}
                    onClick={() => void remove()}
                  >
                    {t("companion.delete")}
                  </Button>
                  <Button disabled={pending} onClick={() => setDeleteConfirm(false)}>
                    {t("common.cancel")}
                  </Button>
                </div>
              </div>
            ) : (
              <Button variant="ghost" onClick={() => setDeleteConfirm(true)}>
                {t("companion.delete")}
              </Button>
            ))}
        </div>
      </Sheet>
      {(memoryOpen || memoryRequested) && !overlay && (
        <CompanionMemory
          session={session ?? null}
          passage={memoryPassage}
          onClose={closeMemory}
        />
      )}
    </section>
  );
}
