/* Per-conversation composer draft store.
 *
 * Keyed by thread id, or NEW_DRAFT_KEY for the landing / new-conversation
 * composer. Backed by localStorage (best-effort, same try/catch shape as
 * lib/thread-state.js) so an unsent draft survives navigating away from a
 * conversation — including the new-conversation screen, whose composer
 * previously dropped its draft on unmount.
 */

import { authScope } from "../../../lib/auth-scope.js";

export const NEW_DRAFT_KEY = "__new__";

const STORAGE_PREFIX = "ironclaw:v2-draft:";

// Namespaced by the authenticated user so one user's unsent text can't be
// restored for another in the same browser (the new-conversation slot is
// shared by key, so this scoping is what isolates it across sessions).
function storageKey(key) {
  return `${STORAGE_PREFIX}${authScope()}:${key || NEW_DRAFT_KEY}`;
}

/** Read the saved draft for a key, or "" when none / storage is unavailable. */
export function getDraft(key) {
  try {
    return window.localStorage.getItem(storageKey(key)) || "";
  } catch (_) {
    // Private mode / quota — drafts are best-effort.
    return "";
  }
}

/** Persist (or, when text is empty, clear) the draft for a key. */
export function setDraft(key, text) {
  try {
    if (text) {
      window.localStorage.setItem(storageKey(key), text);
    } else {
      window.localStorage.removeItem(storageKey(key));
    }
  } catch (_) {
    // Best-effort — never block the composer on storage failure.
  }
}

/** Clear the draft for a key (e.g. after a successful send). */
export function clearDraft(key) {
  setDraft(key, "");
}

/** Remove every persisted draft. Called on sign-out so unsent text can't
 * leak to a different user signing in on the same browser. */
export function clearAllDrafts() {
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
}
