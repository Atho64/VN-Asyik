/** Localized labels for the calendar buckets the stats queries return. */

import { MS_PER_DAY } from "@/lib/time";

/** A calendar month name (`5` becomes `May`). */
export function monthName(month: number, locale: string): string {
  return new Intl.DateTimeFormat(locale === "id" ? "id-ID" : "en-GB", {
    month: "long",
  }).format(new Date(2024, month - 1, 1));
}

/**
 * A local day index as a short date.
 *
 * Day indexes encode a local calendar date as-if-UTC, the heatmap's convention,
 * so rendering in UTC shows the date the reader experienced.
 */
export function dayName(day: number, locale: string): string {
  return new Intl.DateTimeFormat(locale === "id" ? "id-ID" : "en-GB", {
    day: "numeric",
    month: "short",
    timeZone: "UTC",
  }).format(new Date(day * MS_PER_DAY));
}
