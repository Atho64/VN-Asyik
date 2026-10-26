/**
 * Time formatting for display.
 *
 * The database stores UTC milliseconds. Display is locale-aware but the shape is
 * ours: playtime reads as `12h 40m`, never as `12.66 hours`, because the number
 * people compare across games is whole minutes.
 */

export const MS_PER_SECOND = 1_000;
export const MS_PER_MINUTE = 60_000;
export const MS_PER_HOUR = 3_600_000;
export const MS_PER_DAY = 86_400_000;

/**
 * Whether a duration is real but shorter than one minute.
 *
 * The tracker records in short slices (roughly four seconds of counted time per
 * poll), so a genuine session regularly lands between one and 59 seconds. Flooring
 * that to whole minutes printed "0 min" for time the user actually spent reading,
 * which reads as a tracking failure rather than a short session. Mirrors
 * `vn_core::time_fmt::is_sub_minute`; the two must agree.
 */
export function isSubMinute(ms: number): boolean {
  return Number.isFinite(ms) && ms > 0 && ms < MS_PER_MINUTE;
}

/** Whole seconds, floored, for the sub-minute case. */
export function secondsOf(ms: number): number {
  return Number.isFinite(ms) && ms > 0 ? Math.floor(ms / MS_PER_SECOND) : 0;
}

/** `h:mm`, the compact form for dense rows and cards. */
export function formatDuration(ms: number): string {
  // A sub-minute slice gets `0:04`, not a bare `0:00`: the compact form keeps the
  // `m:ss` shape zero-padded so the column stays aligned either way.
  if (isSubMinute(ms)) return `0:${String(secondsOf(ms)).padStart(2, "0")}`;
  if (!Number.isFinite(ms) || ms <= 0) return "0:00";
  const totalMinutes = Math.floor(ms / MS_PER_MINUTE);
  const hours = Math.floor(totalMinutes / 60);
  const minutes = totalMinutes % 60;
  return `${hours}:${minutes.toString().padStart(2, "0")}`;
}

/** `12h 40m`, the readable form for detail headers and stats. */
export function formatDurationLong(ms: number, locale = "en"): string {
  if (!Number.isFinite(ms) || ms <= 0) return locale === "id" ? "0 menit" : "0 min";
  // Under a minute, report seconds. This is the fix for sessions shown as
  // "0 menit" while carrying real counted time.
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

/**
 * Build a UTC timestamp from a local date string (`YYYY-MM-DD`) plus a time.
 *
 * Uses the browser's local timezone on purpose: users enter the session they
 * remember, in the clock they remember it by.
 */
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
