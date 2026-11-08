import { useEffect, useMemo, useState } from "react";

import { useI18n } from "@/app/useI18n";
import { Card, CardHeader } from "@/components/Card";
import { api } from "@/lib/ipc/client";
import { formatRating } from "@/lib/library";
import { MS_PER_DAY, formatDurationLong } from "@/lib/time";
import type { YearReview } from "@/lib/ipc/types";

/**
 * Years the reader actually read in, newest first, plus the current one — the
 * current year belongs in the list even before anything is recorded, or a
 * first-time user would have no chip to press. A day index encodes a calendar
 * date as-if-UTC (the heatmap tooltip relies on the same encoding), so the
 * year comes out of a UTC read.
 */
export function yearsWithDays(
  byDay: Array<{ day: number }>,
  currentYear: number,
): number[] {
  const years = new Set<number>();
  for (const { day } of byDay) {
    years.add(new Date(day * MS_PER_DAY).getUTCFullYear());
  }
  years.add(currentYear);
  return [...years].sort((a, b) => b - a);
}

function Figure({
  label,
  value,
  hint,
}: {
  label: string;
  value: string;
  hint?: string;
}) {
  return (
    <div className="figures__item">
      <dt className="figures__label">{label}</dt>
      <dd className="figures__value tnum">{value}</dd>
      {hint && <dd className="figures__hint">{hint}</dd>}
    </div>
  );
}

/**
 * Year in review: one card that sums up a calendar year of reading. Days and
 * years are the reader's local calendar, the same unit the heatmap beside this
 * card uses, so the two can never disagree about where a session landed.
 *
 * The year chips come from the by-day playtime the Stats page already fetched;
 * the aggregates themselves are one dedicated IPC call per selected year.
 */
export function YearReview({ byDay }: { byDay: Array<{ day: number }> }) {
  const { t, locale } = useI18n();
  // Captured once: the chip list must not reshuffle at midnight.
  const [currentYear] = useState(() => new Date().getFullYear());
  const [year, setYear] = useState(currentYear);
  const [review, setReview] = useState<YearReview | null>(null);
  // The failure is keyed by the year it happened for, so switching years
  // replaces the error instead of comparing against a stale flag.
  const [failure, setFailure] = useState<{ year: number } | null>(null);
  const failed = failure?.year === year;

  useEffect(() => {
    let cancelled = false;
    api
      .yearReview(year)
      .then((next) => {
        if (!cancelled) {
          setReview(next);
          setFailure(null);
        }
      })
      .catch(() => {
        if (!cancelled) setFailure({ year });
      });
    return () => {
      cancelled = true;
    };
  }, [year]);

  const years = useMemo(() => yearsWithDays(byDay, currentYear), [byDay, currentYear]);
  const isEmpty =
    review !== null &&
    review.totalMs === 0 &&
    review.gamesFinished === 0 &&
    review.gamesStarted === 0 &&
    review.endingsReached === 0 &&
    review.journalEntries === 0;

  return (
    <Card>
      <CardHeader
        title={t("review.title")}
        aside={
          <div className="review__years" role="tablist" aria-label={t("review.title")}>
            {years.map((option) => (
              <button
                key={option}
                type="button"
                role="tab"
                className="review__year tnum"
                aria-selected={option === year}
                data-active={option === year}
                onClick={() => setYear(option)}
              >
                {option}
              </button>
            ))}
          </div>
        }
      />
      {failed ? (
        <p className="field__error" role="alert">
          {t("review.errorTitle")}
        </p>
      ) : review === null ? (
        <div aria-busy="true" aria-label={t("review.loading")}>
          <div className="skeleton skeleton--row" />
          <div className="skeleton skeleton--row" />
        </div>
      ) : isEmpty ? (
        <p className="body">{t("review.empty", { year: String(review.year) })}</p>
      ) : (
        <>
          <dl className="figures review__figures">
            <Figure
              label={t("review.totalTime")}
              value={formatDurationLong(review.totalMs, locale)}
            />
            <Figure label={t("review.activeDays")} value={String(review.activeDays)} />
            <Figure label={t("review.sessions")} value={String(review.sessionCount)} />
            <Figure
              label={t("review.endings")}
              value={String(review.endingsReached)}
            />
            <Figure
              label={t("review.journalEntries")}
              value={String(review.journalEntries)}
            />
            <Figure
              label={t("review.gamesStarted")}
              value={String(review.gamesStarted)}
            />
            <Figure
              label={t("review.gamesFinished")}
              value={String(review.gamesFinished)}
            />
            {review.avgRatingDecivotes !== null ? (
              <Figure
                label={t("review.avgRating")}
                value={formatRating(review.avgRatingDecivotes)}
              />
            ) : null}
            {review.longestSession ? (
              <Figure
                label={t("review.longestSession")}
                value={formatDurationLong(review.longestSession.countedMs, locale)}
                hint={review.longestSession.title}
              />
            ) : null}
          </dl>
          {review.firstGame ? (
            <p className="caption review__first">
              {t("review.firstGame")}: <strong>{review.firstGame.title}</strong>
            </p>
          ) : null}
          {review.topGames.length > 0 ? (
            <div className="stats__rows">
              <p className="caption" style={{ marginBottom: 4 }}>
                {t("review.topGames")}
              </p>
              {review.topGames.map((game) => {
                const max = review.topGames[0]?.countedMs ?? 1;
                return (
                  <div className="stats__row" key={game.gameId}>
                    <span
                      style={{
                        overflow: "hidden",
                        textOverflow: "ellipsis",
                        whiteSpace: "nowrap",
                      }}
                      title={game.title}
                    >
                      {game.title}
                    </span>
                    <span className="stats__bar">
                      <span
                        className="stats__bar-fill"
                        style={{
                          width: `${Math.round((game.countedMs / max) * 100)}%`,
                          display: "block",
                        }}
                      />
                    </span>
                    <span className="tnum caption">
                      {formatDurationLong(game.countedMs, locale)}
                    </span>
                  </div>
                );
              })}
            </div>
          ) : null}
        </>
      )}
    </Card>
  );
}
