/**
 * Model builders for the two wrapped cards.
 *
 * The renderer knows nothing about `YearReview` or `LibraryWrapped`; it draws a
 * `ShareCardModel`. Keeping the translation and figure selection here means the
 * year and all-time cards share one layout without sharing one data shape.
 */

import type { Locale, Translator } from "@/i18n";
import type { Game, LibraryWrapped, YearReview } from "@/lib/ipc/types";
import { formatDuration } from "@/lib/time";
import {
  shareCardDay,
  shareCardMonth,
  type ShareCardFact,
  type ShareCardModel,
} from "./shareCardCanvas";

/**
 * A game-shaped placeholder for the artwork hook when a card has no top game.
 *
 * `useArtwork` is a hook, so it cannot be skipped conditionally. The placeholder
 * has no VN id and no cover, which means it performs no IPC call and resolves to
 * the title-hash fallback palette.
 */
export const SHARE_PLACEHOLDER_GAME: Game = {
  meta: {
    id: "share-card-placeholder",
    updated_at: 0,
    device_id_device: "",
    deleted: false,
  },
  title: "VN Asyik",
  title_original: null,
  vndb_id: null,
  status: "playing",
  rating: null,
  notes: null,
  length_minutes: null,
  manual_playtime_ms: 0,
  accent_color: null,
  cover_url: null,
  cover_sexual: null,
  cover_violence: null,
  custom_cover_path: null,
  custom_cover_source_vndb: false,
  started_at: null,
  finished_at: null,
};

function peakHour(hourMs: number[]): { index: number; ms: number } {
  let index = 0;
  for (let cursor = 1; cursor < hourMs.length; cursor += 1) {
    if ((hourMs[cursor] ?? 0) > (hourMs[index] ?? 0)) index = cursor;
  }
  return { index, ms: hourMs[index] ?? 0 };
}

function hourPeakLabel(
  hourMs: number[],
  t: Translator,
  locale: Locale,
): string {
  const peak = peakHour(hourMs);
  return t("review.hourPeak", {
    hour: String(peak.index).padStart(2, "0"),
    duration: formatDuration(peak.ms, locale),
  });
}

/** The per-year card. */
export function yearShareModel(
  review: YearReview,
  t: Translator,
  locale: Locale,
  accent: string,
  backdropUrl: string | null,
): ShareCardModel {
  const facts: ShareCardFact[] = [];
  if (review.longestSession) {
    facts.push({
      label: t("review.longestSession"),
      value: formatDuration(review.longestSession.countedMs, locale),
    });
  }
  const month = shareCardMonth(review, locale);
  if (month) {
    facts.push({ label: t("review.busiestMonth"), value: month });
  }
  facts.push({
    label: t("review.topRoute"),
    value: review.topRoute?.name ?? t("review.noRoute"),
  });
  const day = shareCardDay(review, locale);
  if (day) {
    facts.push({ label: t("review.busiestDay"), value: day });
  }

  return {
    badge: String(review.year),
    eyebrow: t("review.shareEyebrow"),
    totalMs: review.totalMs,
    subline: t("review.shareSubline", {
      days: t("review.days", { count: review.activeDays }),
      sessions: t("review.sessionsCount", { count: review.sessionCount }),
    }),
    figures: [
      { value: String(review.endingsReached), label: t("review.endings") },
      { value: String(review.gamesFinished), label: t("review.gamesFinished") },
      {
        value: t("review.days", { count: review.longestStreakDays }),
        label: t("review.longestStreak"),
      },
    ],
    topGames: review.topGames.map((game) => ({
      title: game.title,
      countedMs: game.countedMs,
    })),
    facts,
    hourMs: review.hourMs,
    hourPeak: hourPeakLabel(review.hourMs, t, locale),
    accent,
    backdropUrl,
    filename: `vn-asyik-${review.year}-wrapped`,
  };
}

/** The library-wide card. */
export function allTimeShareModel(
  wrapped: LibraryWrapped,
  t: Translator,
  locale: Locale,
  accent: string,
  backdropUrl: string | null,
): ShareCardModel {
  const facts: ShareCardFact[] = [];
  if (wrapped.longestSession) {
    facts.push({
      label: t("review.longestSession"),
      value: formatDuration(wrapped.longestSession.countedMs, locale),
    });
  }
  facts.push({
    label: t("review.longestStreak"),
    value: t("review.days", { count: wrapped.longestStreakDays }),
  });
  facts.push({
    label: t("review.topRoute"),
    value: wrapped.topRoute?.name ?? t("review.noRoute"),
  });
  if (wrapped.mostReadYear) {
    facts.push({
      label: t("wrapped.mostReadYear"),
      value: String(wrapped.mostReadYear.year),
    });
  }

  return {
    badge: t("wrapped.allTimeBadge"),
    eyebrow: t("wrapped.eyebrow"),
    totalMs: wrapped.totalMs,
    subline: t("wrapped.subline", {
      since: wrapped.firstYear
        ? String(wrapped.firstYear)
        : t("wrapped.unknownYear"),
      games: t("wrapped.gamesCount", { count: wrapped.gamesTracked }),
      sessions: t("wrapped.sessionsCount", { count: wrapped.sessionCount }),
    }),
    figures: [
      { value: String(wrapped.activeDays), label: t("review.activeDays") },
      { value: String(wrapped.endingsReached), label: t("review.endings") },
      { value: String(wrapped.gamesFinished), label: t("review.gamesFinished") },
    ],
    topGames: wrapped.topGames.map((game) => ({
      title: game.title,
      countedMs: game.countedMs,
    })),
    facts,
    hourMs: wrapped.hourMs,
    hourPeak: hourPeakLabel(wrapped.hourMs, t, locale),
    accent,
    backdropUrl,
    filename: "vn-asyik-all-time-wrapped",
  };
}
