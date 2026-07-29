// Shared helpers for the system-surface DEMO fixtures (settings, admin,
// extensions, logs). Mirrors the timestamp/id conventions of routes/core.ts
// so the whole tour stays internally consistent and always looks fresh.

export const now = Date.now();
export const MINUTE = 60_000;
export const HOUR = 3_600_000;
export const DAY = 86_400_000;

/** ISO timestamp `msAgo` milliseconds in the past (relative to page load). */
export function iso(msAgo: number): string {
  return new Date(now - msAgo).toISOString();
}

/** ISO timestamp `msAhead` milliseconds in the future (expiries, codes). */
export function isoAhead(msAhead: number): string {
  return new Date(Date.now() + msAhead).toISOString();
}

let idCounter = 0;
/** Deterministic-looking unique id, namespaced away from core.ts ids. */
export function demoId(prefix: string): string {
  idCounter += 1;
  return `${prefix}-demo-${String(idCounter).padStart(4, "0")}`;
}
