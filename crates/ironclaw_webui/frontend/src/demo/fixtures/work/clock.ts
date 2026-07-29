// Shared time helpers for the work-surface DEMO fixtures. Everything is
// derived from a single `NOW` captured at module load so the whole dataset
// stays internally consistent ("8 minutes ago" in one panel matches the
// timestamp another panel renders for the same event).

export const NOW = Date.now();
export const MINUTE = 60_000;
export const HOUR = 3_600_000;
export const DAY = 86_400_000;

/** ISO timestamp `msAgo` in the past. */
export function iso(msAgo: number): string {
  return new Date(NOW - msAgo).toISOString();
}

/** ISO timestamp `msAhead` in the future (schedules, next fires). */
export function isoIn(msAhead: number): string {
  return new Date(NOW + msAhead).toISOString();
}

/** Local calendar date (YYYY-MM-DD) `daysAgo` days in the past. */
export function dateStamp(daysAgo: number): string {
  return new Date(NOW - daysAgo * DAY).toISOString().slice(0, 10);
}
