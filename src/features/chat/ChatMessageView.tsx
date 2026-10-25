import { Link } from "react-router";
import { useI18n } from "@/app/useI18n";
import type { ChatMessage, ChatTrace } from "@/lib/ipc/chat";
import { renderMarkdown, safeHref } from "./markdown";

/** Sources are links only when they are plain http(s) without credentials. */
export function safeSourceUrl(raw: string | null): string | null {
  return safeHref(raw ?? "");
}

function ToolActivity({ trace }: { trace: ChatTrace }) {
  const { t } = useI18n();
  const names: Record<string, string> = {
    search_library: t("vnchat.tool.library"),
    get_game: t("vnchat.tool.game"),
    library_stats: t("vnchat.tool.stats"),
    play_history: t("vnchat.tool.history"),
    taste_profile: t("vnchat.tool.taste"),
    search_vndb: t("vnchat.tool.vndb"),
    search_web: t("vnchat.tool.web"),
  };
  return (
    <details className="vnchat__tool">
      <summary>
        <span>{names[trace.name] ?? t("vnchat.tool.unknown")}</span>
        <span className="vnchat__tool-status" data-status={trace.status}>
          {t(`vnchat.status.${trace.status}`)}
        </span>
      </summary>
      <pre>{trace.name}</pre>
      <pre>{trace.arguments}</pre>
      {trace.result != null && <pre>{JSON.stringify(trace.result, null, 2)}</pre>}
    </details>
  );
}

export function ChatMessageView({ message }: { message: ChatMessage }) {
  const { t, locale } = useI18n();
  const assistant = message.role === "assistant";
  return (
    <article className={`vnchat__message vnchat__message--${message.role}`}>
      <div className="vnchat__message-head">
        <strong>{t(assistant ? "vnchat.assistant" : "vnchat.you")}</strong>
        <time dateTime={new Date(message.createdAt).toISOString()}>
          {new Date(message.createdAt).toLocaleTimeString(locale, {
            hour: "2-digit",
            minute: "2-digit",
          })}
        </time>
      </div>
      {message.tools.length > 0 && (
        <div className="vnchat__tools" aria-label={t("vnchat.tools")}>
          {message.tools.map((trace, index) => (
            <ToolActivity key={`${trace.id}-${index}`} trace={trace} />
          ))}
        </div>
      )}
      {/* Rendered to React nodes, never to HTML: a model reply cannot inject
          markup, load a remote image, or pass a non-http(s) link. */}
      <div className="vnchat__answer">
        {message.content ? (
          renderMarkdown(message.content)
        ) : message.status === "running" ? (
          <p className="md-p">{t("vnchat.thinking")}</p>
        ) : null}
      </div>
      {message.sources.length > 0 && (
        <details className="vnchat__sources">
          <summary>{t("vnchat.sources", { count: message.sources.length })}</summary>
          <ol>
            {message.sources.map((source) => {
              const url = safeSourceUrl(source.url);
              return (
                <li key={source.id}>
                  {url ? (
                    <a href={url} target="_blank" rel="noreferrer noopener">
                      {source.title}
                    </a>
                  ) : source.gameId ? (
                    <Link to={`/game/${encodeURIComponent(source.gameId)}`}>
                      {t("vnchat.librarySource")}
                    </Link>
                  ) : (
                    t("vnchat.librarySource")
                  )}
                </li>
              );
            })}
          </ol>
        </details>
      )}
      {message.status === "cancelled" && (
        <p className="vnchat__status-note">
          {message.content.trim() ? t("vnchat.stopped") : t("vnchat.stoppedEmpty")}
        </p>
      )}
      {message.status === "error" && message.content.trim() !== "" && (
        <p className="vnchat__status-note vnchat__status-note--warn">
          {t("vnchat.failedPartial")}
        </p>
      )}
      {message.tokens !== null && (
        <p className="vnchat__status-note">
          {t("vnchat.tokens", { count: message.tokens })}
        </p>
      )}
    </article>
  );
}
