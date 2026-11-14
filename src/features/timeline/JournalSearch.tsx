import { useEffect, useRef, useState, type ReactNode } from "react";
import { Link } from "react-router";

import { useI18n } from "@/app/useI18n";
import { Button } from "@/components/Button";
import { EmptyState } from "@/components/EmptyState";
import { Icon } from "@/components/Icon";
import { useArtwork } from "@/components/useArtwork";
import { readingApi, type JournalSearchResult } from "@/lib/ipc/reading";
import { formatDate } from "@/lib/time";

/**
 * Highlights case-insensitive `query` matches via regex over the raw string
 * (avoiding index drift from length-changing Unicode lowercasing).
 */
function Highlight({ text, query }: { text: string; query: string }) {
  const trimmed = query.trim();
  if (!trimmed) return <>{text}</>;
  const escaped = trimmed.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const pattern = new RegExp(escaped, "gi");
  const parts: ReactNode[] = [];
  let last = 0;
  let key = 0;
  for (const match of text.matchAll(pattern)) {
    const at = match.index ?? 0;
    if (at > last) parts.push(text.slice(last, at));
    parts.push(
      <mark key={key++} className="jsearch__hit">
        {match[0]}
      </mark>,
    );
    last = at + match[0].length;
  }
  if (last === 0) return <>{text}</>;
  if (last < text.length) parts.push(text.slice(last));
  return <>{parts}</>;
}

/** One matching checkpoint. Cover concealment goes through the shared hook. */
function SearchRow({ result, query }: { result: JournalSearchResult; query: string }) {
  const { t, locale } = useI18n();
  const { entry, game } = result;
  const art = useArtwork(game);
  const { policy, imageUrl, fallback } = art;
  const concealed = policy !== "visible";
  const position = [entry.route_name, entry.chapter_name].filter(Boolean).join(" · ");

  return (
    <li>
      <Link to={`/game/${game.meta.id}`} className="jsearch__row">
        <div
          className="jsearch__thumb"
          style={
            imageUrl
              ? undefined
              : { background: `linear-gradient(160deg, ${fallback.accent}44, var(--bg-elevated))` }
          }
        >
          {imageUrl ? (
            <img
              className={concealed ? "cover__art--blurred" : undefined}
              src={imageUrl}
              alt=""
              width={44}
              height={66}
              loading="lazy"
              decoding="async"
              onError={concealed ? undefined : art.imageFailed}
              style={{ width: "100%", height: "100%", objectFit: "cover" }}
            />
          ) : concealed ? (
            <span className="jsearch__concealed" aria-hidden="true">
              <Icon name="eyeOff" size={18} />
            </span>
          ) : null}
        </div>
        <div className="jsearch__body">
          <span className="jsearch__title">{game.title}</span>
          {entry.note.trim() ? (
            <span className="jsearch__note">
              <Highlight text={entry.note} query={query} />
            </span>
          ) : (
            <span className="jsearch__note jsearch__note--empty">
              {t("timeline.eventJournal")}
            </span>
          )}
          <span className="jsearch__meta caption">
            {formatDate(entry.created_at, locale)}
            {position ? ` · ${position}` : ""}
            {entry.photo_image_id ? (
              <Icon name="image" size={12} title={t("reading.photo")} />
            ) : null}
          </span>
          {concealed ? (
            <span className="jsearch__reveal">
              <span>{t("game.explicitHidden")}</span>
              <button
                type="button"
                className="btn btn--secondary"
                onClick={(event) => {
                  event.preventDefault();
                  event.stopPropagation();
                  art.reveal();
                }}
              >
                {t("game.reveal")}
              </button>
            </span>
          ) : null}
        </div>
      </Link>
    </li>
  );
}

/** Debounced, stale-guarded search across journal checkpoints using `(created_at, id)` pagination. */
export function JournalSearch({ query }: { query: string }) {
  const { t } = useI18n();
  const [results, setResults] = useState<JournalSearchResult[] | null>(null);
  const [hasMore, setHasMore] = useState(false);
  const [busy, setBusy] = useState(true);
  const [failed, setFailed] = useState(false);
  const request = useRef(0);

  /** One fetch, guarded so an answer for an old query never lands on a new one. */
  const fetchPage = (before?: { ts: number; id: string }) => {
    const id = ++request.current;
    setBusy(true);
    setFailed(false);
    readingApi
      .search(query, undefined, before)
      .then((page) => {
        if (request.current !== id) return;
        setResults((previous) =>
          before ? [...(previous ?? []), ...page.results] : page.results,
        );
        setHasMore(page.hasMore);
      })
      .catch(() => {
        if (request.current === id) setFailed(true);
      })
      .finally(() => {
        if (request.current === id) setBusy(false);
      });
  };

  useEffect(() => {
    const timer = setTimeout(() => fetchPage(), 250);
    return () => clearTimeout(timer);
    // `fetchPage` closes over the query; a new query is a new search.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [query]);

  const loadMore = () => {
    const last = results?.at(-1);
    if (!last || busy) return;
    fetchPage({ ts: last.entry.created_at, id: last.entry.meta.id });
  };

  if (failed && results === null) {
    return (
      <EmptyState
        title={t("timeline.errorTitle")}
        body={t("common.error")}
        action={<Button onClick={() => fetchPage()}>{t("common.retry")}</Button>}
      />
    );
  }

  if (results === null) {
    return (
      <div aria-busy="true" aria-label={t("timeline.searchLoading")}>
        <div className="skeleton" style={{ height: 64, marginBottom: 12 }} />
        <div className="skeleton" style={{ height: 64, marginBottom: 12 }} />
        <div className="skeleton" style={{ height: 64 }} />
      </div>
    );
  }

  if (results.length === 0) {
    return (
      <EmptyState
        title={t("timeline.searchEmptyTitle")}
        body={t("timeline.searchEmptyBody")}
      />
    );
  }

  return (
    <div className="jsearch">
      <p className="jsearch__count caption tnum" role="status">
        {t("timeline.searchCount", { count: results.length })}
      </p>
      <ul className="jsearch__list">
        {results.map((result) => (
          <SearchRow key={result.entry.meta.id} result={result} query={query} />
        ))}
      </ul>
      {failed ? (
        <div className="reading__error">
          <p className="notice notice--danger" role="alert">
            {t("timeline.errorTitle")}
          </p>
          <Button onClick={loadMore}>{t("common.retry")}</Button>
        </div>
      ) : null}
      {hasMore ? (
        <div className="timeline__pages">
          <Button disabled={busy} onClick={loadMore}>
            {t("timeline.loadOlder")}
          </Button>
        </div>
      ) : null}
    </div>
  );
}