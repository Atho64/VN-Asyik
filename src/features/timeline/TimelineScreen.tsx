import { useEffect, useMemo, useState } from "react";
import { Link } from "react-router";

import { useAppStore } from "@/app/store";
import { useI18n } from "@/app/useI18n";
import { Button } from "@/components/Button";
import { EmptyState } from "@/components/EmptyState";
import { Icon } from "@/components/Icon";
import { useArtwork } from "@/components/useArtwork";
import { api } from "@/lib/ipc/client";
import type { TimelineEvent } from "@/lib/ipc/types";
import { formatDate, formatDurationLong } from "@/lib/time";
import { JournalSearch } from "./JournalSearch";
import { ScreenshotsPanel } from "@/features/screenshots/ScreenshotsPanel";

/** One chronological day of the journey, events newest first inside it. */
interface TimelineDay {
  key: string;
  label: string;
  events: TimelineEvent[];
}

const dayPart = (value: number): string => String(value).padStart(2, "0");

/**
 * Group events under the local calendar day the reader experienced them. The
 * backend orders by instant; the grouping is presentation, so it uses the
 * viewer's own day — the same choice `playtime_by_day` makes for the heatmap.
 */
function groupByDay(events: TimelineEvent[], locale: string): TimelineDay[] {
  const days: TimelineDay[] = [];
  const index = new Map<string, number>();
  for (const event of events) {
    const date = new Date(event.ts);
    const key = `${date.getFullYear()}-${dayPart(date.getMonth() + 1)}-${dayPart(date.getDate())}`;
    let at = index.get(key);
    if (at === undefined) {
      at = days.length;
      index.set(key, at);
      days.push({ key, label: formatDate(event.ts, locale), events: [] });
    }
    days[at]!.events.push(event);
  }
  return days;
}

/** One event row. Cover concealment goes through the shared artwork hook. */
function TimelineRow({ event }: { event: TimelineEvent }) {
  const { t, locale } = useI18n();
  const art = useArtwork(event.game);
  const { policy, imageUrl, fallback } = art;
  const concealed = policy !== "visible";

  const position = [event.routeName, event.chapterName].filter(Boolean).join(" · ");
  const line = (() => {
    switch (event.kind) {
      case "session":
        return t("timeline.eventSession", {
          time: formatDurationLong(event.countedMs ?? 0, locale),
        });
      case "journal":
        return event.note?.trim() ? event.note : t("timeline.eventJournal");
      case "ending":
        return t("timeline.eventEnding", { name: event.endingName ?? "" });
      default:
        return t("timeline.eventFinished");
    }
  })();

  return (
    <li>
      <Link to={`/game/${event.game.meta.id}`} className="timeline__row">
        <div
          className="timeline__thumb"
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
            <span className="timeline__concealed" aria-hidden="true">
              <Icon name="eyeOff" size={18} />
            </span>
          ) : null}
        </div>
        <div className="timeline__body">
          <span className="timeline__title">{event.game.title}</span>
          <span className="timeline__event">{line}</span>
          {event.kind === "session" && event.note ? (
            <span className="timeline__note">{event.note}</span>
          ) : null}
          {position ? <span className="timeline__position caption">{position}</span> : null}
          {concealed ? (
            <span className="timeline__reveal">
              <span>{t("game.explicitHidden")}</span>
              <button
                type="button"
                className="btn btn--secondary"
                onClick={(event_) => {
                  // Reveal without following the row's link.
                  event_.preventDefault();
                  event_.stopPropagation();
                  art.reveal();
                }}
              >
                {t("game.reveal")}
              </button>
            </span>
          ) : null}
        </div>
        {event.kind === "session" && event.afkMs ? (
          <span className="timeline__aside caption tnum">
            {t("detail.away")}: {formatDurationLong(event.afkMs, locale)}
          </span>
        ) : null}
      </Link>
    </li>
  );
}

/**
 * The reading journey across every game: sessions, journal entries, reached
 * endings and finished games in one feed, newest first.
 *
 * Pages accumulate downward and each is anchored to the cursor of the page
 * before it, so events that arrive while reading never shift the older ones.
 * The backend bounds a page at 50 rows; there is no virtualizer here for the
 * same reason session history has none — a feed you page through, not a grid
 * that renders ten thousand covers.
 */
export function TimelineScreen() {
  const { t, locale } = useI18n();
  const query = useAppStore((state) => state.timelineQuery);
  const setQuery = useAppStore((state) => state.setTimelineQuery);
  const tab = useAppStore((state) => state.timelineTab);
  const setTab = useAppStore((state) => state.setTimelineTab);
  const searching = query.trim().length > 0;
  const [events, setEvents] = useState<TimelineEvent[] | null>(null);
  const [hasMore, setHasMore] = useState(false);
  // The first fetch starts in the mount effect below, so the page opens busy.
  const [busy, setBusy] = useState(true);
  const [failed, setFailed] = useState(false);

  // The feed loads whenever no search is on screen. The cancelled flag (not a
  // ref) makes this safe under StrictMode's mount-cleanup-mount: a ref guard
  // would mark the feed "requested" on the first pass, cancel that fetch in
  // cleanup, then skip the second pass — leaving the page stuck on skeletons.
  useEffect(() => {
    if (searching) return;
    let cancelled = false;
    api
      .timelinePage()
      .then((page) => {
        if (cancelled) return;
        setEvents(page.events);
        setHasMore(page.hasMore);
      })
      .catch(() => {
        if (!cancelled) setFailed(true);
      })
      .finally(() => {
        if (!cancelled) setBusy(false);
      });
    return () => {
      cancelled = true;
    };
  }, [searching]);

  const loadOlder = () => {
    const last = events?.at(-1);
    if (!last || busy) return;
    setBusy(true);
    setFailed(false);
    api
      .timelinePage({ ts: last.ts, id: last.id })
      .then((page) => {
        setEvents((previous) => [...(previous ?? []), ...page.events]);
        setHasMore(page.hasMore);
      })
      .catch(() => setFailed(true))
      .finally(() => setBusy(false));
  };

  /**
   * After a failed first page this refetches from the top; after a failed
   * older page it re-walks the same cursor and appends, so the feed the
   * reader was looking at stays in place either way.
   */
  const retry = () => {
    const last = events?.at(-1);
    setFailed(false);
    setBusy(true);
    api
      .timelinePage(last ? { ts: last.ts, id: last.id } : undefined)
      .then((page) => {
        setEvents((previous) =>
          previous && last ? [...previous, ...page.events] : page.events,
        );
        setHasMore(page.hasMore);
      })
      .catch(() => setFailed(true))
      .finally(() => setBusy(false));
  };

  const days = useMemo(
    () => (events ? groupByDay(events, locale) : []),
    [events, locale],
  );

  const feed = (() => {
    if (failed && events === null) {
      return (
        <EmptyState
          title={t("timeline.errorTitle")}
          body={t("common.error")}
          action={<Button onClick={retry}>{t("common.retry")}</Button>}
        />
      );
    }
    if (events === null) {
      return (
        <div aria-busy="true" aria-label={t("timeline.loading")}>
          <div
            className="skeleton"
            style={{ height: 72, width: "40%", marginBottom: 24 }}
          />
          <div className="skeleton" style={{ height: 160 }} />
        </div>
      );
    }
    if (days.length === 0) {
      return <EmptyState title={t("timeline.emptyTitle")} body={t("timeline.emptyBody")} />;
    }
    return (
      <div className="timeline">
        {days.map((day) => (
          <section key={day.key} className="timeline__day">
            <h2 className="timeline__day-head">{day.label}</h2>
            <ul className="timeline__list">
              {day.events.map((event) => (
                <TimelineRow key={`${event.kind}:${event.id}`} event={event} />
              ))}
            </ul>
          </section>
        ))}
        {failed ? (
          <div className="reading__error">
            <p className="notice notice--danger" role="alert">
              {t("timeline.errorTitle")}
            </p>
            <Button onClick={retry}>{t("common.retry")}</Button>
          </div>
        ) : null}
        {hasMore ? (
          <div className="timeline__pages">
            <Button disabled={busy} onClick={loadOlder}>
              {t("timeline.loadOlder")}
            </Button>
          </div>
        ) : null}
      </div>
    );
  })();

  return (
    <div className="stats-page">
      <header className="page-head">
        <h1 className="page-title">{t("timeline.title")}</h1>
      </header>
      <div className="tabs" role="tablist" aria-label={t("timeline.tabs")}>
        {(
          [
            ["journey", "timeline.tabJourney"],
            ["screenshots", "timeline.tabScreenshots"],
          ] as const
        ).map(([value, labelKey]) => (
          <button
            key={value}
            type="button"
            role="tab"
            aria-selected={tab === value}
            className="tabs__tab"
            onClick={() => setTab(value)}
          >
            {t(labelKey)}
          </button>
        ))}
      </div>
      {tab === "screenshots" ? (
        <ScreenshotsPanel />
      ) : (
        <>
      <div className="timeline__search">
        <div className="search">
          <span className="search__icon">
            <Icon name="search" />
          </span>
          <input
            className="input"
            type="search"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder={t("timeline.searchPlaceholder")}
            aria-label={t("timeline.searchLabel")}
          />
        </div>
      </div>
      {searching ? <JournalSearch query={query.trim()} /> : feed}
        </>
      )}
    </div>
  );
}
