/* Client-side pinned-thread store.
 *
 * There is no server-side pin concept yet, so pins live in the browser:
 * a Set of pinned thread ids persisted to localStorage. The sidebar reads
 * this store to decide which threads belong under PINNED — replacing the
 * previous behavior where the active thread was implicitly "pinned".
 *
 * Modeled on lib/thread-state.js: a module-level store with a snapshot-on-
 * notify subscription and a React adapter. Persistence is best-effort.
 */

import { React } from "./html.js";

const STORAGE_KEY = "ironclaw:v2-thread-pins";

const subscribers = new Set();
/** @type {Set<string>} */
const pinned = new Set();

function readPersisted() {
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed.filter((id) => typeof id === "string");
  } catch (_) {
    return [];
  }
}

function writePersisted() {
  try {
    if (pinned.size === 0) {
      window.localStorage.removeItem(STORAGE_KEY);
    } else {
      window.localStorage.setItem(STORAGE_KEY, JSON.stringify([...pinned]));
    }
  } catch (_) {
    // Best-effort — never block the UI on storage failure.
  }
}

for (const id of readPersisted()) {
  pinned.add(id);
}

function snapshot() {
  return new Set(pinned);
}

function emit() {
  const snap = snapshot();
  for (const listener of subscribers) {
    try {
      listener(snap);
    } catch (_) {
      // A misbehaving subscriber must not poison the store.
    }
  }
}

/** Is this thread currently pinned? */
export function isPinned(threadId) {
  return pinned.has(threadId);
}

/** Toggle a thread's pinned status, persisting and notifying subscribers. */
export function togglePin(threadId) {
  if (!threadId) return;
  if (pinned.has(threadId)) {
    pinned.delete(threadId);
  } else {
    pinned.add(threadId);
  }
  writePersisted();
  emit();
}

/** Read-only snapshot of the pinned id set. */
export function getPinnedIds() {
  return snapshot();
}

/** Subscribe to pin-set changes. Returns an unsubscribe fn. */
export function subscribePins(listener) {
  subscribers.add(listener);
  return () => {
    subscribers.delete(listener);
  };
}

/** React adapter for the pinned set. Re-renders on any pin/unpin. */
export function usePinnedIds() {
  const [set, setSet] = React.useState(getPinnedIds);
  React.useEffect(() => subscribePins(setSet), []);
  return set;
}
