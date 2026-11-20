/**
 * Display units for achievement progress. The backend `progress_and_target`
 * (vn_core::achievements) returns milliseconds for time-based slugs, days for
 * streaks, and plain counts otherwise. This map mirrors that contract so the
 * card never renders raw milliseconds; the bar ratio still uses the raw numbers.
 */

import { formatDuration } from "@/lib/time";
import type { Locale } from "@/i18n";

export type AchievementUnit = "ms" | "days" | "count";

const MS_SLUGS = new Set([
  "playtime-10h",
  "playtime-100h",
  "playtime-500h",
  "playtime-1000h",
  "day-2h",
  "day-5h",
  "day-10h",
  "session-4h",
  "session-8h",
  "night-owl",
  "early-bird",
  "route-10h",
]);

const DAY_SLUGS = new Set([
  "marathon-10h-5d",
  "streak-2h-7d",
  "streak-3",
  "streak-7",
  "streak-30",
  "streak-100",
  "comeback",
  "anniversary-1y",
  "journal-streak-7",
]);

/** The unit a slug's progress and target are measured in. */
export function achievementUnit(slug: string): AchievementUnit {
  if (MS_SLUGS.has(slug)) return "ms";
  if (DAY_SLUGS.has(slug)) return "days";
  return "count";
}

/** A progress value ready for display: compact duration for time, integer otherwise. */
export function formatProgressValue(slug: string, value: number, locale: Locale): string {
  return achievementUnit(slug) === "ms" ? formatDuration(value, locale) : String(value);
}