/* Client-side pinned-thread store.
 *
 * There is no server-side pin concept yet, so pins live in the browser:
 * a Set of pinned thread ids persisted to localStorage. The sidebar reads
 * this store to decide which threads belong under PINNED — replacing the
 * previous behavior where the active thread was implicitly "pinned".
 *
 * Pure module (no React import) so it is unit-testable; the React adapter
 * lives with its consumer. Keys are namespaced by the authenticated user
 * (via lib/auth-scope.js) so one user's pins never surface for another in
 * the same browser, matching the draft and history caches.
 */

import { authScope } from "./auth-scope.js";

const STORAGE_PREFIX = "ironclaw:v2-thread-pins:";

const subscribers = new Set();
/** @type {Set<string>} */
const pinned = new Set();
// The scope the in-memory set was last loaded for. The set is reloaded from
// the current scope's storage whenever the authenticated identity changes,
// so a different user reads their own pins (and never the previous user's).
let loadedScope = null;

function storageKey() {
  return `${STORAGE_PREFIX}${authScope()}`;
}

function readPersisted() {
  try {
    const raw = window.localStorage.getItem(storageKey());
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
      window.localStorage.removeItem(storageKey());
    } else {
      window.localStorage.setItem(storageKey(), JSON.stringify([...pinned]));
    }
  } catch (_) {
    // Best-effort — never block the UI on storage failure.
  }
}

// Reload the in-memory set from the current scope's storage when the
// authenticated identity has changed since the last access.
function ensureScope() {
  const scope = authScope();
  if (scope === loadedScope) return;
  pinned.clear();
  for (const id of readPersisted()) pinned.add(id);
  loadedScope = scope;
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
  ensureScope();
  return pinned.has(threadId);
}

/** Toggle a thread's pinned status, persisting and notifying subscribers. */
export function togglePin(threadId) {
  if (!threadId) return;
  ensureScope();
  if (pinned.has(threadId)) {
    pinned.delete(threadId);
  } else {
    pinned.add(threadId);
  }
  writePersisted();
  emit();
}

/** Read-only snapshot of the pinned id set for the current user. */
export function getPinnedIds() {
  ensureScope();
  return snapshot();
}

/** Subscribe to pin-set changes. Returns an unsubscribe fn. */
export function subscribePins(listener) {
  subscribers.add(listener);
  return () => {
    subscribers.delete(listener);
  };
}

/** Remove every persisted pin set (all scopes) and reset the in-memory set.
 * Called on sign-out / identity change so pins can't carry across users. */
export function clearAllPins() {
  pinned.clear();
  loadedScope = authScope();
  try {
    const keys = [];
    for (let i = 0; i < window.localStorage.length; i += 1) {
      const key = window.localStorage.key(i);
      if (key && key.startsWith(STORAGE_PREFIX)) keys.push(key);
    }
    keys.forEach((key) => window.localStorage.removeItem(key));
  } catch (_) {
    // Best-effort.
  }
  emit();
}
