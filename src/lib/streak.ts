/**
 * Reading streaks over the per-day playtime list.
 *
 * Input is the same shape `playtime_by_day` returns and the heatmap renders:
 * `[dayIndex, ms]` tuples where a day index is days since epoch of the *local*
 * date. Both functions are pure and take "today" as an argument, so a test can
 * pin the calendar instead of racing the real clock.
 */

/** Day indexes the reader actually recorded time on, ascending. */
function readDays(days: Array<[number, number]>): number[] {
  return days
    .filter(([, ms]) => ms > 0)
    .map(([day]) => day)
    .sort((a, b) => a - b);
}

/**
 * The current streak: consecutive reading days ending today, or yesterday —
 * a streak survives until the day is over, so not having read *yet* today is
 * not the same as having broken it.
 */
export function currentStreak(days: Array<[number, number]>, todayIndex: number): number {
  const read = new Set(readDays(days));
  // Start from yesterday when today has no time yet; only a day with nothing
  // on it before that ends the streak.
  let cursor = read.has(todayIndex) ? todayIndex : todayIndex - 1;
  let streak = 0;
  while (read.has(cursor)) {
    streak += 1;
    cursor -= 1;
  }
  return streak;
}

/** The longest run of consecutive reading days over the whole history. */
export function longestStreak(days: Array<[number, number]>): number {
  const read = readDays(days);
  let longest = 0;
  let run = 0;
  let previous = Number.NaN;
  for (const day of read) {
    run = day === previous + 1 ? run + 1 : 1;
    previous = day;
    if (run > longest) longest = run;
  }
  return longest;
}
