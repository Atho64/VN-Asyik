import { useEffect, useMemo, useState } from "react";

import { useAppStore } from "@/app/store";
import { useI18n } from "@/app/useI18n";
import { Button } from "@/components/Button";
import { Card, CardHeader } from "@/components/Card";
import { useArtwork } from "@/components/useArtwork";
import { api } from "@/lib/ipc/client";
import { formatRating } from "@/lib/library";
import { formatDurationLong } from "@/lib/time";
import type { LibraryWrapped } from "@/lib/ipc/types";
import { dayName, monthName } from "./dateLabels";
import { Figure } from "./Figure";
import { ShareCard } from "./ShareCard";
import { SHARE_PLACEHOLDER_GAME, allTimeShareModel } from "./shareModels";

/**
 * The library-wide recap: every year summed into one card.
 *
 * The per-year review answers "what did 2024 look like"; this answers "what does
 * the whole shelf look like". It shares the same share-card renderer, so the
 * all-time picture cannot drift from the yearly one.
 */
export function AllTimeWrapped() {
  const { t, locale } = useI18n();
  const cards = useAppStore((state) => state.cards);
  const [wrapped, setWrapped] = useState<LibraryWrapped | null>(null);
  const [failed, setFailed] = useState(false);
  const [shareOpen, setShareOpen] = useState(false);

  useEffect(() => {
    let cancelled = false;
    api.libraryWrapped().then(
      (next) => {
        if (!cancelled) setWrapped(next);
      },
      () => {
        if (!cancelled) setFailed(true);
      },
    );
    return () => {
      cancelled = true;
    };
  }, []);

  const topGameId = wrapped?.topGames[0]?.gameId;
  const topGame = useMemo(
    () =>
      cards.find((card) => card.game.meta.id === topGameId)?.game ??
      SHARE_PLACEHOLDER_GAME,
    [cards, topGameId],
  );
  const art = useArtwork(topGame);
  const shareModel = useMemo(
    () =>
      wrapped
        ? allTimeShareModel(
            wrapped,
            t,
            locale,
            art.palette.accent,
            art.localBackdropUrl,
          )
        : null,
    [wrapped, t, locale, art.palette.accent, art.localBackdropUrl],
  );

  if (failed) {
    return (
      <Card>
        <CardHeader title={t("wrapped.title")} />
        <p className="field__error" role="alert">
          {t("wrapped.error")}
        </p>
      </Card>
    );
  }

  if (!wrapped) {
    return (
      <Card>
        <CardHeader title={t("wrapped.title")} />
        <div aria-busy="true" aria-label={t("wrapped.loading")}>
          <div className="skeleton skeleton--row" />
        </div>
      </Card>
    );
  }

  if (wrapped.totalMs === 0) return null;

  const max = wrapped.topGames[0]?.countedMs ?? 1;

  return (
    <>
      <Card>
        <CardHeader
          title={t("wrapped.title")}
          aside={
            <Button variant="secondary" onClick={() => setShareOpen(true)}>
              {t("wrapped.share")}
            </Button>
          }
        />
        <div>
          <p className="caption">{t("wrapped.totalTime")}</p>
          <p className="stats__number">
            {formatDurationLong(wrapped.totalMs, locale)}
          </p>
          <p className="body" style={{ marginTop: "var(--space-3)" }}>
            {t("wrapped.subline", {
              since: wrapped.firstYear
                ? String(wrapped.firstYear)
                : t("wrapped.unknownYear"),
              games: t("wrapped.gamesCount", { count: wrapped.gamesTracked }),
              sessions: t("wrapped.sessionsCount", {
                count: wrapped.sessionCount,
              }),
            })}
          </p>
        </div>
        <dl className="figures" style={{ marginTop: "var(--space-5)" }}>
            <Figure
              label={t("review.activeDays")}
              value={String(wrapped.activeDays)}
            />
            <Figure
              label={t("review.endings")}
              value={String(wrapped.endingsReached)}
            />
            <Figure
              label={t("review.gamesFinished")}
              value={String(wrapped.gamesFinished)}
            />
            <Figure
              label={t("wrapped.inLibrary")}
              value={String(wrapped.gamesInLibrary)}
            />
            <Figure
              label={t("review.journalEntries")}
              value={String(wrapped.journalEntries)}
            />
            {wrapped.afkMs > 0 ? (
              <Figure
                label={t("stats.awayTime")}
                value={formatDurationLong(wrapped.afkMs, locale)}
              />
            ) : null}
            {wrapped.averageSessionMs !== null ? (
              <Figure
                label={t("review.averageSession")}
                value={formatDurationLong(wrapped.averageSessionMs, locale)}
              />
            ) : null}
            {wrapped.averageRatingDecivotes !== null ? (
              <Figure
                label={t("time.averageRating")}
                value={formatRating(wrapped.averageRatingDecivotes)}
              />
            ) : null}
            {wrapped.longestSession ? (
              <Figure
                label={t("review.longestSession")}
                value={formatDurationLong(wrapped.longestSession.countedMs, locale)}
                hint={wrapped.longestSession.title}
              />
            ) : null}
            {wrapped.longestStreakDays > 0 ? (
              <Figure
                label={t("review.longestStreak")}
                value={String(wrapped.longestStreakDays)}
                hint={t("review.days", { count: wrapped.longestStreakDays })}
              />
            ) : null}
            {wrapped.topRoute ? (
              <Figure
                label={t("review.topRoute")}
                value={wrapped.topRoute.name}
                hint={formatDurationLong(wrapped.topRoute.countedMs, locale)}
              />
            ) : null}
            {wrapped.mostReadYear ? (
              <Figure
                label={t("wrapped.mostReadYear")}
                value={String(wrapped.mostReadYear.year)}
                hint={formatDurationLong(wrapped.mostReadYear.countedMs, locale)}
              />
            ) : null}
            {wrapped.busiestMonth ? (
              <Figure
                label={t("review.busiestMonth")}
                value={monthName(wrapped.busiestMonth.month, locale)}
                hint={formatDurationLong(wrapped.busiestMonth.countedMs, locale)}
              />
            ) : null}
            {wrapped.busiestDay ? (
              <Figure
                label={t("review.busiestDay")}
                value={dayName(wrapped.busiestDay.day, locale)}
                hint={formatDurationLong(wrapped.busiestDay.countedMs, locale)}
              />
            ) : null}
        </dl>
        {wrapped.firstGame ? (
          <p className="caption" style={{ marginTop: "var(--space-3)" }}>
            {t("review.firstGame")}: <strong>{wrapped.firstGame.title}</strong>
          </p>
        ) : null}
        {wrapped.topGames.length > 0 ? (
          <div className="stats__rows">
            <p className="caption" style={{ marginBottom: 4 }}>
              {t("review.topGames")}
            </p>
            {wrapped.topGames.map((game) => (
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
            ))}
          </div>
        ) : null}
      </Card>
      {shareModel ? (
        <ShareCard
          model={shareModel}
          title={t("wrapped.shareTitle")}
          open={shareOpen}
          onClose={() => setShareOpen(false)}
        />
      ) : null}
    </>
  );
}
