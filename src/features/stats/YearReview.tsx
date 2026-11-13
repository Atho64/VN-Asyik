import { useEffect, useMemo, useState } from "react";

import { useAppStore } from "@/app/store";
import { useI18n } from "@/app/useI18n";
import { Button } from "@/components/Button";
import { Card, CardHeader } from "@/components/Card";
import { useArtwork } from "@/components/useArtwork";
import { api } from "@/lib/ipc/client";
import { formatRating } from "@/lib/library";
import { MS_PER_DAY, formatDurationLong } from "@/lib/time";
import type { YearReview as YearReviewData } from "@/lib/ipc/types";
import { dayName, monthName } from "./dateLabels";
import { Figure } from "./Figure";
import { ShareCard } from "./ShareCard";
import { SHARE_PLACEHOLDER_GAME, yearShareModel } from "./shareModels";

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

/** Reading rhythm: which weekdays and hours the year's time actually landed in. */
function Rhythm({
  weekdayMs,
  hourMs,
  locale,
}: {
  weekdayMs: number[];
  hourMs: number[];
  locale: "en" | "id";
}) {
  const { t } = useI18n();
  const weekdays = useMemo(() => {
    const formatter = new Intl.DateTimeFormat(locale === "id" ? "id-ID" : "en-GB", {
      weekday: "short",
    });
    // 2024-01-01 was a Monday, matching the Monday-first SQL buckets.
    return Array.from({ length: 7 }, (_, index) =>
      formatter.format(new Date(2024, 0, 1 + index)),
    );
  }, [locale]);
  const weekdayMax = Math.max(1, ...weekdayMs);
  const hourMax = Math.max(1, ...hourMs);
  const weekdayPeak = weekdayMs.reduce(
    (best, ms, index) => (ms > (weekdayMs[best] ?? 0) ? index : best),
    0,
  );
  const hourPeak = hourMs.reduce(
    (best, ms, index) => (ms > (hourMs[best] ?? 0) ? index : best),
    0,
  );

  return (
    <div className="rhythm">
      <div className="rhythm__block">
        <p className="caption">{t("review.weekdayTitle")}</p>
        <div
          className="rhythm__bars"
          role="img"
          aria-label={t("review.weekdayTitle")}
        >
          {weekdayMs.map((ms, index) => (
            <span
              key={weekdays[index]}
              className="rhythm__bar"
              title={`${weekdays[index]}: ${formatDurationLong(ms, locale)}`}
            >
              <span
                className="rhythm__fill"
                style={{ height: `${Math.round((ms / weekdayMax) * 100)}%` }}
              />
            </span>
          ))}
        </div>
        <div className="rhythm__labels" aria-hidden="true">
          {weekdays.map((day) => (
            <span key={day}>{day}</span>
          ))}
        </div>
        <p className="caption rhythm__peak">
          {t("review.weekdayPeak", {
            day: weekdays[weekdayPeak] ?? "",
            duration: formatDurationLong(weekdayMs[weekdayPeak] ?? 0, locale),
          })}
        </p>
      </div>
      <div className="rhythm__block">
        <p className="caption">{t("review.hourTitle")}</p>
        <div
          className="rhythm__bars rhythm__bars--hour"
          role="img"
          aria-label={t("review.hourTitle")}
        >
          {hourMs.map((ms, index) => (
            <span
              key={index}
              className="rhythm__bar"
              title={`${String(index).padStart(2, "0")}:00: ${formatDurationLong(
                ms,
                locale,
              )}`}
            >
              <span
                className="rhythm__fill"
                style={{ height: `${Math.round((ms / hourMax) * 100)}%` }}
              />
            </span>
          ))}
        </div>
        <div className="rhythm__labels rhythm__labels--hour" aria-hidden="true">
          <span>00</span>
          <span>06</span>
          <span>12</span>
          <span>18</span>
          <span>23</span>
        </div>
        <p className="caption rhythm__peak">
          {t("review.hourPeak", {
            hour: String(hourPeak).padStart(2, "0"),
            duration: formatDurationLong(hourMs[hourPeak] ?? 0, locale),
          })}
        </p>
      </div>
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
  const cards = useAppStore((state) => state.cards);
  // Captured once: the chip list must not reshuffle at midnight.
  const [currentYear] = useState(() => new Date().getFullYear());
  const [year, setYear] = useState(currentYear);
  const [review, setReview] = useState<YearReviewData | null>(null);
  const [shareOpen, setShareOpen] = useState(false);
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

  const topGameId = review?.topGames[0]?.gameId;
  const topGame = useMemo(
    () =>
      cards.find((card) => card.game.meta.id === topGameId)?.game ??
      SHARE_PLACEHOLDER_GAME,
    [cards, topGameId],
  );
  const art = useArtwork(topGame);
  const shareModel = useMemo(
    () =>
      review
        ? yearShareModel(
            review,
            t,
            locale,
            art.palette.accent,
            art.localBackdropUrl,
          )
        : null,
    [review, t, locale, art.palette.accent, art.localBackdropUrl],
  );

  return (
    <>
      <Card>
        <CardHeader
          title={t("review.title")}
          aside={
            <div className="review__aside">
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
              {review && review.totalMs > 0 ? (
                <Button variant="secondary" onClick={() => setShareOpen(true)}>
                  {t("review.share")}
                </Button>
              ) : null}
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
              {review.gamesContinued > 0 ? (
                <Figure
                  label={t("review.gamesContinued")}
                  value={String(review.gamesContinued)}
                />
              ) : null}
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
              {review.averageSessionMs !== null ? (
                <Figure
                  label={t("review.averageSession")}
                  value={formatDurationLong(review.averageSessionMs, locale)}
                />
              ) : null}
              {review.longestStreakDays > 0 ? (
                <Figure
                  label={t("review.longestStreak")}
                  value={String(review.longestStreakDays)}
                  hint={t("review.days", { count: review.longestStreakDays })}
                />
              ) : null}
              {review.busiestMonth ? (
                <Figure
                  label={t("review.busiestMonth")}
                  value={monthName(review.busiestMonth.month, locale)}
                  hint={formatDurationLong(review.busiestMonth.countedMs, locale)}
                />
              ) : null}
              {review.busiestDay ? (
                <Figure
                  label={t("review.busiestDay")}
                  value={dayName(review.busiestDay.day, locale)}
                  hint={formatDurationLong(review.busiestDay.countedMs, locale)}
                />
              ) : null}
              {review.topRoute ? (
                <Figure
                  label={t("review.topRoute")}
                  value={review.topRoute.name}
                  hint={formatDurationLong(review.topRoute.countedMs, locale)}
                />
              ) : null}
            </dl>
            {review.weekdayMs.some((ms) => ms > 0) ? (
              <Rhythm
                weekdayMs={review.weekdayMs}
                hourMs={review.hourMs}
                locale={locale}
              />
            ) : null}
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
      {review && shareModel ? (
        <ShareCard
          model={shareModel}
          title={t("review.shareTitle", { year: String(review.year) })}
          open={shareOpen}
          onClose={() => setShareOpen(false)}
        />
      ) : null}
    </>
  );
}
