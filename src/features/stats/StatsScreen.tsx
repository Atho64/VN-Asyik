import { useEffect, useMemo, useState } from "react";

import { useAppStore } from "@/app/store";
import { useI18n } from "@/app/useI18n";
import { TastePanel } from "@/features/stats/TastePanel";
import { TimeStats } from "@/features/stats/TimeStats";
import { Card, CardHeader } from "@/components/Card";
import { EmptyState } from "@/components/EmptyState";
import { api } from "@/lib/ipc/client";
import { statusOrder } from "@/lib/library";
import { MS_PER_DAY, MS_PER_MINUTE, dayIndex, formatDurationLong } from "@/lib/time";
import type { StatusBreakdown } from "@/lib/ipc/types";

/** 53 weeks x 7 days, the standard contribution-graph span. */
const WEEKS = 53;

interface DayCell {
  day: number;
  ms: number;
}

/**
 * Stats.
 *
 * Composition is editorial, not a stat-card row: one number large enough to read
 * across a room, then supporting detail. Every figure is aggregated from recorded
 * sessions by the backend, so an empty library shows an empty state instead of
 * zeros dressed up as insight (antislop R-17, R-38).
 */
export function StatsScreen() {
  const { t, locale } = useI18n();
  const cards = useAppStore((state) => state.cards);
  const loaded = useAppStore((state) => state.libraryLoaded);

  const [breakdown, setBreakdown] = useState<StatusBreakdown | null>(null);
  const [byDay, setByDay] = useState<DayCell[]>([]);
  const [failed, setFailed] = useState(false);
  // Captured once: the heatmap window must not shift between renders. Day
  // indexes are of the *local* date — the backend buckets by the user's own
  // calendar day (`playtime_by_day`), so a late-night session lands on the
  // cell the reader experienced, not on the UTC date the timestamp carries.
  const [todayIndex] = useState(() =>
    dayIndex(Date.now() - new Date().getTimezoneOffset() * 60_000),
  );

  useEffect(() => {
    let cancelled = false;
    void Promise.all([api.statusBreakdown(), api.playtimeByDay()])
      .then(([statusRows, days]) => {
        if (cancelled) return;
        setBreakdown(statusRows);
        setByDay(days.map(([day, ms]) => ({ day, ms })));
      })
      .catch(() => {
        if (!cancelled) setFailed(true);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const totalMs = useMemo(
    () => cards.reduce((sum, card) => sum + card.total_ms, 0),
    [cards],
  );
  const trackedGames = useMemo(
    () => cards.filter((card) => card.total_ms > 0).length,
    [cards],
  );
  const afkMs = useMemo(() => cards.reduce((sum, card) => sum + card.afk_ms, 0), [cards]);
  const top = useMemo(
    () =>
      [...cards]
        .filter((card) => card.total_ms > 0)
        .sort((a, b) => b.total_ms - a.total_ms)
        .slice(0, 8),
    [cards],
  );

  const heatmap = useMemo(() => {
    if (byDay.length === 0) return null;
    const byIndex = new Map(byDay.map((entry) => [entry.day, entry.ms]));
    const today = todayIndex;
    const first = today - WEEKS * 7 + 1;
    const cells: DayCell[] = [];
    for (let day = first; day <= today; day += 1) {
      cells.push({ day, ms: byIndex.get(day) ?? 0 });
    }
    const max = Math.max(1, ...cells.map((cell) => cell.ms));
    return { cells, max };
  }, [byDay, todayIndex]);

  if (!loaded) {
    return (
      <div aria-busy="true" aria-label={t("stats.loading")}>
        <div
          className="skeleton"
          style={{ height: 72, width: "40%", marginBottom: 24 }}
        />
        <div className="skeleton" style={{ height: 160 }} />
      </div>
    );
  }

  if (failed) {
    return <EmptyState title={t("stats.errorTitle")} body={t("common.error")} />;
  }

  if (totalMs === 0) {
    return <EmptyState title={t("stats.emptyTitle")} body={t("stats.emptyBody")} />;
  }

  const maxTop = top[0]?.total_ms ?? 1;
  const breakdownTotal = breakdown
    ? breakdown.playing +
      breakdown.wishlist +
      breakdown.completed +
      breakdown.on_hold +
      breakdown.dropped
    : 0;

  const countFor = (status: string): number => {
    if (!breakdown) return 0;
    switch (status) {
      case "playing":
        return breakdown.playing;
      case "wishlist":
        return breakdown.wishlist;
      case "completed":
        return breakdown.completed;
      case "on_hold":
        return breakdown.on_hold;
      default:
        return breakdown.dropped;
    }
  };

  const level = (ms: number, max: number): number => {
    if (ms <= 0) return 0;
    return Math.min(4, Math.max(1, Math.ceil((ms / max) * 4)));
  };

  return (
    <div className="stats-page">
      <header className="page-head">
        <h1 className="page-title">{t("stats.title")}</h1>
      </header>
      <TimeStats />
      <TastePanel />
      {/* The total and the status split are one thought ("how much, and of
          what"), so they share one card like every other block on this page.
          Previously they sat as bare text on the backdrop while the sections
          under them were cards, which read as a different page. */}
      <Card>
        <CardHeader title={t("stats.overview")} />
        <div className="stats__lede">
          <div>
            <p className="caption">{t("stats.totalTime")}</p>
            <p className="stats__number">
              {locale === "id"
                ? formatDurationLong(totalMs, locale)
                : `${Math.floor(totalMs / 3_600_000)}`}
            </p>
            {locale === "en" ? (
              <p className="stats__unit">
                hours
                {totalMs % 3_600_000 > 0
                  ? ` · ${Math.floor((totalMs % 3_600_000) / MS_PER_MINUTE)} min`
                  : ""}
              </p>
            ) : null}
            <p className="body" style={{ marginTop: "var(--space-3)" }}>
              {t("stats.gamesTracked")}: <span className="tnum">{trackedGames}</span>
            </p>
            {afkMs > 0 ? (
              <p className="caption" style={{ marginTop: "var(--space-1)" }}>
                {t("stats.awayTime")}:{" "}
                <span className="tnum">{formatDurationLong(afkMs, locale)}</span>
              </p>
            ) : null}
          </div>

          <div className="stats__rows">
            <p className="caption" style={{ marginBottom: 4 }}>
              {t("stats.byStatus")}
            </p>
            {statusOrder.map((status) => {
              const count = countFor(status);
              const ratio = breakdownTotal > 0 ? count / breakdownTotal : 0;
              return (
                <div className="stats__row" key={status}>
                  <span>{t(`status.${status}`)}</span>
                  <span className="stats__bar">
                    <span
                      className="stats__bar-fill"
                      style={{ width: `${Math.round(ratio * 100)}%`, display: "block" }}
                    />
                  </span>
                  <span className="tnum caption">{count}</span>
                </div>
              );
            })}
          </div>
        </div>
      </Card>

      <Card>
        <CardHeader title={t("stats.topGames")} />
        <div className="stats__rows">
          {top.map((card) => (
            <div className="stats__row" key={card.game.meta.id}>
              <span
                style={{
                  overflow: "hidden",
                  textOverflow: "ellipsis",
                  whiteSpace: "nowrap",
                }}
                title={card.game.title}
              >
                {card.game.title}
              </span>
              <span className="stats__bar">
                <span
                  className="stats__bar-fill"
                  style={{
                    width: `${Math.round((card.total_ms / maxTop) * 100)}%`,
                    display: "block",
                  }}
                />
              </span>
              <span className="tnum caption">
                {formatDurationLong(card.total_ms, locale)}
              </span>
            </div>
          ))}
        </div>
      </Card>

      <Card>
        <CardHeader
          title={t("stats.heatmap")}
          aside={
            <span className="caption" aria-hidden="true">
              {t("stats.heatmapLegend")} · {t("stats.heatmapLegendMore")}
            </span>
          }
        />
        {heatmap ? (
          <div className="heatmap" role="img" aria-label={t("stats.heatmap")}>
            {heatmap.cells.map((cell) => (
              <span
                key={cell.day}
                className="heatmap__cell"
                data-level={level(cell.ms, heatmap.max)}
                title={`${new Date(cell.day * MS_PER_DAY).toLocaleDateString(
                  locale === "id" ? "id-ID" : "en-GB",
                  // A day index encodes a calendar date as-if-UTC; rendering
                  // it in UTC shows that exact date in every timezone.
                  { timeZone: "UTC" },
                )}: ${formatDurationLong(cell.ms, locale)}`}
              />
            ))}
          </div>
        ) : (
          <p className="body">{t("stats.emptyBody")}</p>
        )}
      </Card>
    </div>
  );
}
