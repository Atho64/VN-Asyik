/**
 * Time formatting for display.
 *
 * The database stores UTC milliseconds. Display is locale-aware but the shape is
 * ours: playtime reads as `12h 40m`, never as `12.66 hours`, because the number
 * people compare across games is whole minutes.
 */

import type { Locale } from "@/i18n";

export const MS_PER_SECOND = 1_000;
export const MS_PER_MINUTE = 60_000;
export const MS_PER_HOUR = 3_600_000;
export const MS_PER_DAY = 86_400_000;

/** True for nonzero durations under one minute (`1s..59s`); mirrors `vn_core::time_fmt::is_sub_minute`. */
export function isSubMinute(ms: number): boolean {
  return Number.isFinite(ms) && ms > 0 && ms < MS_PER_MINUTE;
}

/** Whole seconds, floored, for the sub-minute case. */
export function secondsOf(ms: number): number {
  return Number.isFinite(ms) && ms > 0 ? Math.floor(ms / MS_PER_SECOND) : 0;
}

/** Unit labels for the compact duration, per locale. */
const COMPACT_UNITS: Record<
  Locale,
  { h: string; m: string; s: string; sep: string }
> = {
  en: { h: "h", m: "m", s: "s", sep: "" },
  // Indonesian uses `dtk` (`d` already denotes days) and space-separated units.
  id: { h: "j", m: "mnt", s: "dtk", sep: " " },
};

/** Formats compact durations with explicit units (`45s`, `12m`, `1h 20m` / `45 dtk`, `12 mnt`, `1 j 20 mnt`). */
export function formatDuration(ms: number, locale: Locale = "en"): string {
  const unit = COMPACT_UNITS[locale] ?? COMPACT_UNITS.en;
  const totalSeconds = Number.isFinite(ms) ? Math.floor(Math.max(ms, 0) / MS_PER_SECOND) : 0;
  if (totalSeconds <= 0) return `0${unit.sep}${unit.s}`;
  if (totalSeconds < 60) return `${totalSeconds}${unit.sep}${unit.s}`;
  const totalMinutes = Math.floor(totalSeconds / 60);
  if (totalMinutes < 60) return `${totalMinutes}${unit.sep}${unit.m}`;
  const hours = Math.floor(totalMinutes / 60);
  const minutes = totalMinutes % 60;
  if (minutes === 0) return `${hours}${unit.sep}${unit.h}`;
  return `${hours}${unit.sep}${unit.h} ${minutes}${unit.sep}${unit.m}`;
}

/** `12h 40m`, the readable form for detail headers and stats. */
export function formatDurationLong(ms: number, locale = "en"): string {
  if (!Number.isFinite(ms) || ms <= 0) return locale === "id" ? "0 menit" : "0 min";
  // Report sub-minute sessions in seconds so short active tracking does not show 0 min.
  if (isSubMinute(ms)) {
    const seconds = secondsOf(ms);
    return locale === "id" ? `${seconds} detik` : `${seconds} s`;
  }
  const totalMinutes = Math.floor(ms / MS_PER_MINUTE);
  const hours = Math.floor(totalMinutes / 60);
  const minutes = totalMinutes % 60;
  if (hours === 0) {
    return locale === "id" ? `${minutes} menit` : `${minutes} min`;
  }
  const hourUnit = locale === "id" ? "jam" : "h";
  if (minutes === 0) return `${hours}${hourUnit}`;
  return `${hours}${hourUnit} ${minutes}m`;
}

/** Progress toward the VNDB-reported length, 0..1, or `null` when unknown. */
export function lengthProgress(
  playedMs: number,
  lengthMinutes: number | null,
): number | null {
  if (!lengthMinutes || lengthMinutes <= 0) return null;
  const ratio = playedMs / (lengthMinutes * MS_PER_MINUTE);
  return Math.max(0, Math.min(1, ratio));
}

/** Local date, e.g. `14 Mar 2026`. */
export function formatDate(ms: number, locale = "en"): string {
  if (!Number.isFinite(ms)) return "-";
  return new Intl.DateTimeFormat(locale === "id" ? "id-ID" : "en-GB", {
    day: "numeric",
    month: "short",
    year: "numeric",
  }).format(new Date(ms));
}

/** Local date and time for session rows. */
export function formatDateTime(ms: number, locale = "en"): string {
  if (!Number.isFinite(ms)) return "-";
  return new Intl.DateTimeFormat(locale === "id" ? "id-ID" : "en-GB", {
    day: "numeric",
    month: "short",
    hour: "2-digit",
    minute: "2-digit",
  }).format(new Date(ms));
}

/** UTC day index, matching `vn_core::time_fmt::utc_day_index`. */
export function dayIndex(ms: number): number {
  return Math.floor(ms / MS_PER_DAY);
}

/** `HH:MM` from an `<input type="time">` value. */
export function parseTimeInput(value: string): { hours: number; minutes: number } | null {
  const match = /^(\d{1,2}):(\d{2})$/.exec(value.trim());
  if (!match) return null;
  const hours = Number(match[1]);
  const minutes = Number(match[2]);
  if (hours < 0 || hours > 23 || minutes < 0 || minutes > 59) return null;
  return { hours, minutes };
}

/** Parses local `YYYY-MM-DD` and `HH:MM` into a UTC timestamp (uses local wall-clock timezone). */
export function localDateTimeToMs(date: string, time: string): number | null {
  const dateMatch = /^(\d{4})-(\d{2})-(\d{2})$/.exec(date.trim());
  const parsedTime = parseTimeInput(time);
  if (!dateMatch || !parsedTime) return null;
  const [, year, month, day] = dateMatch;
  const local = new Date(
    Number(year),
    Number(month) - 1,
    Number(day),
    parsedTime.hours,
    parsedTime.minutes,
    0,
    0,
  );
  const ms = local.getTime();
  return Number.isFinite(ms) ? ms : null;
}
