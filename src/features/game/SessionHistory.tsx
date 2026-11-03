import { useEffect, useId, useState } from "react";

import { useI18n } from "@/app/useI18n";
import { Button } from "@/components/Button";
import { api } from "@/lib/ipc/client";
import type { PlaySession } from "@/lib/ipc/types";
import { formatDateTime, formatDurationLong } from "@/lib/time";

type Cursor = { startedAt: number; id: string } | undefined;

/** Render one bounded page. Cursors keep new sessions from shifting older pages. */
export function SessionHistory({
  gameId,
  sessions,
  notePreviews = {},
  noteVersion = 0,
  total,
  onSelect,
}: {
  gameId: string;
  sessions: PlaySession[];
  notePreviews?: Record<string, string>;
  noteVersion?: number;
  total: number;
  onSelect: (sessionId: string) => void;
}) {
  const { t, locale } = useI18n();
  const countId = useId();
  const [initial] = useState({ index: 0, before: undefined as Cursor, noteVersion });
  const [target, setTarget] = useState(initial);
  const [loaded, setLoaded] = useState({
    target: initial,
    noteVersion,
    sessions,
    notePreviews,
    totalSessions: total,
  });
  const [cursors, setCursors] = useState<Cursor[]>([undefined]);
  const [failure, setFailure] = useState<{
    target: typeof target;
    noteVersion: number;
  } | null>(null);
  const error = failure?.target === target && failure.noteVersion === noteVersion;
  const busy = !error && (loaded.target !== target || loaded.noteVersion !== noteVersion);

  useEffect(() => {
    // The parent already fetched the first page. A note edit refreshes the
    // current cursor, preserving the reader's place in older history.
    if (target === initial && noteVersion === initial.noteVersion) return;
    let cancelled = false;
    void api
      .listGameSessions(gameId, target.before)
      .then((next) => {
        if (cancelled) return;
        setLoaded({ ...next, target, noteVersion });
        setCursors((previous) => [...previous.slice(0, target.index), target.before]);
      })
      .catch(() => {
        if (!cancelled) setFailure({ target, noteVersion });
      });
    return () => {
      cancelled = true;
    };
  }, [gameId, initial, target, noteVersion]);

  const page = loaded.sessions;
  const totalSessions = loaded.totalSessions;
  const pageIndex = loaded.target.index;
  // Never keep displaying an edited/deleted snippet if its refresh failed.
  const previews = loaded.noteVersion === noteVersion ? loaded.notePreviews : {};
  const navigate = (index: number, before: Cursor) =>
    setTarget({ index, before, noteVersion });

  const last = page.at(-1);
  const hasOlder = last && pageIndex * 50 + page.length < totalSessions;
  return (
    <div className="session-history" aria-busy={busy}>
      {page.length === 0 ? (
        <p className="body">{t("detail.sessionsEmpty")}</p>
      ) : (
        <ul className="session-history__list" aria-describedby={countId}>
          {page.map((session) => (
            <li key={session.meta.id}>
              <button
                type="button"
                className="session-history__session"
                aria-label={t("reading.openSession", {
                  date: formatDateTime(session.started_at, locale),
                })}
                aria-describedby={
                  previews[session.meta.id] ? `${countId}-${session.meta.id}` : undefined
                }
                onClick={() => onSelect(session.meta.id)}
              >
                <span className="caption">
                  {formatDateTime(session.started_at, locale)}
                </span>
                <span className="session-history__duration tnum">
                  {formatDurationLong(session.counted_ms, locale)}
                  {session.afk_ms > 0 && (
                    <span className="caption">
                      {t("detail.away")}: {formatDurationLong(session.afk_ms, locale)}
                    </span>
                  )}
                </span>
                <span className="session-history__arrow" aria-hidden="true">
                  ›
                </span>
                {previews[session.meta.id] && (
                  <span
                    className="session-history__note"
                    id={`${countId}-${session.meta.id}`}
                  >
                    {previews[session.meta.id]}
                  </span>
                )}
              </button>
            </li>
          ))}
        </ul>
      )}
      {totalSessions > 0 && (
        <p className="caption" id={countId}>
          {t("reading.sessionsCount", { count: page.length, total: totalSessions })}
        </p>
      )}
      {error && (
        <div className="reading__error">
          <p className="notice notice--danger" role="alert">
            {t("reading.sessionsError")}
          </p>
          <Button onClick={() => setTarget({ ...target })}>{t("common.retry")}</Button>
        </div>
      )}
      {(pageIndex > 0 || hasOlder) && (
        <div className="session-history__pages">
          <Button
            disabled={busy || pageIndex === 0}
            onClick={() => void navigate(pageIndex - 1, cursors[pageIndex - 1])}
          >
            {t("reading.newerSessions")}
          </Button>
          {hasOlder && (
            <Button
              disabled={busy}
              onClick={() =>
                void navigate(pageIndex + 1, {
                  startedAt: last.started_at,
                  id: last.meta.id,
                })
              }
            >
              {t("reading.olderSessions")}
            </Button>
          )}
        </div>
      )}
    </div>
  );
}
