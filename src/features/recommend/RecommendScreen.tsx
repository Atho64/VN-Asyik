import { useEffect, useState } from "react";

import { useI18n } from "@/app/useI18n";
import { Button } from "@/components/Button";
import { Icon } from "@/components/Icon";
import { api, IpcError } from "@/lib/ipc/client";
import type { RecommendOutcome, RecommendSettings } from "@/lib/ipc/types";

/**
 * "What to read next?" — answered from what people wrote, not from tags.
 *
 * Every suggestion carries the pages it came from. That is the point: a
 * recommendation without a source is a confident guess, so the links are shown
 * rather than collected in a footnote.
 *
 * Layout: one centred column of readable width. The previous version stretched
 * every control across the full window, which put the submit button 1400px from
 * the input it submits and left the screen looking like an unfinished form.
 */
/// Open a citation through Rust's scheme allowlist.
///
/// Tavily supplies these URLs, so they are third-party text and must not reach an
/// `href` the webview can navigate to.
async function openSource(url: string): Promise<void> {
  try {
    await api.openExternal(url);
  } catch {
    // A rejected scheme is not worth an error toast: the link simply does nothing.
  }
}

export function RecommendScreen() {
  const { t } = useI18n();
  const [settings, setSettings] = useState<RecommendSettings | null>(null);
  const [keyInput, setKeyInput] = useState("");
  const [request, setRequest] = useState("");
  const [outcome, setOutcome] = useState<RecommendOutcome | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [savingKey, setSavingKey] = useState(false);

  useEffect(() => {
    // Deferred: setting state directly in an effect body triggers a cascading
    // render, and the settings fetch is async anyway.
    let active = true;
    void (async () => {
      try {
        const next = await api.aiRecommendSettings();
        if (active) setSettings(next);
      } catch (caught) {
        if (active) {
          setError(caught instanceof IpcError ? caught.message : t("common.error"));
        }
      }
    })();
    return () => {
      active = false;
    };
  }, [t]);

  async function saveKey() {
    setSavingKey(true);
    setError(null);
    try {
      setSettings(await api.aiSaveTavilyKey(keyInput));
      setKeyInput("");
    } catch (caught) {
      setError(caught instanceof IpcError ? caught.message : t("common.error"));
    } finally {
      setSavingKey(false);
    }
  }

  async function clearKey() {
    try {
      setSettings(await api.aiClearTavilyKey());
    } catch (caught) {
      setError(caught instanceof IpcError ? caught.message : t("common.error"));
    }
  }

  async function ask(question = request) {
    if (!question.trim() || busy) return;
    setBusy(true);
    setError(null);
    setOutcome(null);
    try {
      setOutcome(await api.aiRecommend(question, "basic"));
    } catch (caught) {
      setError(caught instanceof IpcError ? caught.message : t("common.error"));
    } finally {
      setBusy(false);
    }
  }

  if (!settings) {
    return <p className="settings__hint">{t("common.loading")}</p>;
  }

  const ready = settings.supported && settings.hasTavilyKey;
  const examples = [
    t("recommend.example1"),
    t("recommend.example2"),
    t("recommend.example3"),
  ];

  return (
    <div className="discover">
      <header className="discover__head">
        <h1 className="page-title">{t("recommend.title")}</h1>
        <p className="page-sub">{t("recommend.intro")}</p>
      </header>

      {!settings.supported && <p className="notice">{t("recommend.unsupported")}</p>}

      {error && (
        <p className="notice notice--danger" role="alert">
          {error}
        </p>
      )}

      {/* The key is set once and then forgotten. It used to own a full card,
          permanently showing a destructive "Remove key" button. */}
      {settings.supported && (
        <details className="discover__settings">
          <summary>
            <Icon name="settings" size={14} />
            {t("recommend.settings")}
          </summary>
          <div className="discover__settings-body">
            {settings.hasTavilyKey ? (
              <>
                <p className="discover__keyrow">
                  <span>{t("recommend.keyStored")}</span>
                  <code>{settings.tavilyKeyHint}</code>
                  <Button variant="ghost" onClick={() => void clearKey()}>
                    {t("recommend.removeKey")}
                  </Button>
                </p>
                {!settings.hasChatKey && (
                  <p className="settings__hint">
                    {t("recommend.needsChatKey", { model: settings.chatModel })}
                  </p>
                )}
              </>
            ) : (
              <>
                <p className="settings__hint">{t("recommend.keyHint")}</p>
                <form
                  className="discover__keyform"
                  onSubmit={(event) => {
                    event.preventDefault();
                    if (keyInput.trim().length > 0) void saveKey();
                  }}
                >
                  <input
                    className="input"
                    type="password"
                    value={keyInput}
                    onChange={(event) => setKeyInput(event.target.value)}
                    placeholder={t("recommend.keyPlaceholder")}
                    aria-label={t("recommend.keyTitle")}
                    autoComplete="off"
                    spellCheck={false}
                  />
                  <Button
                    type="submit"
                    disabled={savingKey || keyInput.trim().length === 0}
                  >
                    {savingKey ? t("recommend.saving") : t("recommend.saveKey")}
                  </Button>
                </form>
              </>
            )}
          </div>
        </details>
      )}

      {ready && (
        <form
          className="discover__ask"
          onSubmit={(event) => {
            event.preventDefault();
            void ask();
          }}
        >
          <label className="sr-only" htmlFor="discover-request">
            {t("recommend.placeholder")}
          </label>
          <div className="discover__field">
            <Icon name="search" size={16} />
            <input
              id="discover-request"
              className="discover__input"
              value={request}
              onChange={(event) => setRequest(event.target.value)}
              placeholder={t("recommend.placeholder")}
            />
            <Button type="submit" disabled={busy || request.trim().length === 0}>
              {busy ? t("recommend.asking") : t("recommend.ask")}
            </Button>
          </div>
        </form>
      )}

      {ready && !outcome && !busy && (
        <section className="discover__empty">
          <Icon name="spark" size={22} />
          <h2>{t("recommend.emptyTitle")}</h2>
          <p>{t("recommend.emptyBody")}</p>
          <div className="discover__examples">
            {examples.map((example) => (
              <button
                key={example}
                type="button"
                className="discover__example"
                onClick={() => {
                  setRequest(example);
                  void ask(example);
                }}
              >
                {example}
              </button>
            ))}
          </div>
        </section>
      )}

      {busy && (
        <section className="discover__busy" aria-busy="true">
          <p role="status">{t("recommend.asking")}</p>
          <div className="skeleton skeleton--row" />
          <div className="skeleton skeleton--row" />
          <div className="skeleton skeleton--row" />
        </section>
      )}

      {outcome && (
        <section className="discover__results">
          {/* The note is always shown when present: "nothing found" is an answer. */}
          {outcome.note && <p className="notice">{outcome.note}</p>}

          {outcome.games.length > 0 && (
            <ul className="rec-list">
              {outcome.games.map((game) => (
                <li key={game.title} className="rec-card">
                  <div className="rec-card__head">
                    <h3 className="rec-card__title">{game.title}</h3>
                    {game.owned && (
                      <span className="chip chip--accent">{t("recommend.owned")}</span>
                    )}
                  </div>
                  {game.why && <p className="rec-card__why">{game.why}</p>}
                  {game.sources.length > 0 && (
                    <ul className="rec-card__sources">
                      {game.sources.map((source) => (
                        <li key={source.url}>
                          <button
                            type="button"
                            className="rec-card__source linkbtn"
                            onClick={() => void openSource(source.url)}
                          >
                            {source.title || source.url}
                          </button>
                        </li>
                      ))}
                    </ul>
                  )}
                </li>
              ))}
            </ul>
          )}

          {outcome.answer && (
            <section className="discover__answer">
              <h2>{t("recommend.answerTitle")}</h2>
              <p className="rec-answer">{outcome.answer}</p>
            </section>
          )}

          {outcome.sources.length > 0 && (
            <details className="discover__sources">
              <summary>
                {t("recommend.sources", { count: outcome.sources.length })}
              </summary>
              <ul className="rec-card__sources">
                {outcome.sources.map((source) => (
                  <li key={source.url}>
                    <button
                      type="button"
                      className="rec-card__source linkbtn"
                      onClick={() => void openSource(source.url)}
                    >
                      {source.title || source.url}
                    </button>
                  </li>
                ))}
              </ul>
            </details>
          )}

          <p className="discover__cost">
            {t("recommend.cost", { count: outcome.credits })}
          </p>
        </section>
      )}
    </div>
  );
}
