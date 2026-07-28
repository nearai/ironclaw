// @ts-nocheck
/* Per-thread state store.
 *
 * A thread is in one of a small, named set of states at any time. The
 * sidebar reads this store to render per-row indicators (pinned position,
 * dot colour, label) so a user can see what's happening across all of
 * their threads from anywhere — not just whichever one they happen to
 * have open.
 *
 * Today the only writer is the chat surface, which mirrors gate state
 * for the active thread (see pages/chat/chat.ts). That covers a single
 * thread per browser session — the one you're looking at. The deferred
 * follow-up — a user-scoped SSE channel — will fan out backend state
 * transitions across all of a user's threads and become the canonical
 * writer (at which point the chat.tsx seam goes away).
 *
 * The shape was chosen for that future:
 *   - A Map keyed by thread id, not a Set. The values are a typed enum
 *     (idle is the default and stored as absence). Adding a new state
 *     (e.g. running, failed) is one entry in THREAD_STATE plus one row
 *     in STATE_PRESENTATION on the sidebar — the producer and consumer
 *     don't have to learn about each other.
 *   - No metadata fields. A state name is enough to drive the UI; gate
 *     kind / failure reason / progress numbers belong to the per-thread
 *     event stream, not this cross-thread summary.
 *   - Subscribers receive a snapshot Map; the internal map is never
 *     handed out so a misbehaving listener cannot mutate the store.
 *
 * Persistence: NEEDS_ATTENTION and FAILED survive page refresh via
 * localStorage (key: ironclaw:v2-thread-attention). RUNNING is not persisted;
 * see PERSISTED_STATES below for the full rationale.
 */

import React from "react";

/** Thread state vocabulary. Absence from the map is the implicit `idle`. */
export const THREAD_STATE = Object.freeze({
  RUNNING: "running",
  NEEDS_ATTENTION: "needs_attention",
  FAILED: "failed",
});

/* Only the "user must act" subset survives page refresh.
 *
 * RUNNING is intentionally excluded: across a page lifetime the run almost
 * certainly completed while the tab was closed, so a persisted RUNNING
 * would show a false-positive green dot on reload.
 *
 * NEEDS_ATTENTION is the opposite — a pending gate remains pending until
 * the user acts. Persisting it is exactly what the user expects
 * ("I left an approval hanging; the app remembered").
 *
 * FAILED is not yet populated by any writer but is grouped with
 * NEEDS_ATTENTION for symmetry when it lands. */
const PERSISTED_STATES = new Set([
  THREAD_STATE.NEEDS_ATTENTION,
  THREAD_STATE.FAILED,
]);
const STORAGE_KEY = "ironclaw:v2-thread-attention";

// ── Module state (declared before the functions that mutate it) ────────

const subscribers = new Set();
/** @type {Map<string, string>} */
const states = new Map();

// ── Storage helpers ────────────────────────────────────────────────────

function readPersisted() {
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed.filter(
      (entry) =>
        Array.isArray(entry) &&
        typeof entry[0] === "string" &&
        PERSISTED_STATES.has(entry[1])
    );
  } catch (_) {
    // Private mode, quota, or invalid JSON — persistence is best-effort.
    return [];
  }
}

function writePersisted() {
  const persisted = [];
  for (const [id, state] of states) {
    if (PERSISTED_STATES.has(state)) persisted.push([id, state]);
  }
  try {
    if (persisted.length === 0) {
      window.localStorage.removeItem(STORAGE_KEY);
    } else {
      window.localStorage.setItem(STORAGE_KEY, JSON.stringify(persisted));
    }
  } catch (_) {
    // Best-effort — we never block the UI on storage failure.
  }
}

// Seed from previous session so a pending approval survives page refresh.
for (const [id, state] of readPersisted()) {
  states.set(id, state);
}

// ── Internal helpers ───────────────────────────────────────────────────

function snapshot() {
  return new Map(states);
}

/* Notify subscribers with a fresh snapshot. Pure — no side-effects. */
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

/**
 * Set the state of a thread. Passing `null`/`undefined` clears the
 * entry, returning the thread to the implicit `idle`. No-op when the
 * new state matches the current one (suppresses redundant emits).
 *
 * Writes to localStorage only when the persisted subset (NEEDS_ATTENTION,
 * FAILED) actually changes — RUNNING transitions never touch storage.
 */
export function setThreadState(threadId, state) {
  if (!threadId) return;
  const prevState = states.get(threadId);
  if (state == null) {
    if (!states.delete(threadId)) return;
    if (PERSISTED_STATES.has(prevState)) writePersisted();
    emit();
    return;
  }
  if (prevState === state) return;
  states.set(threadId, state);
  if (PERSISTED_STATES.has(state) || PERSISTED_STATES.has(prevState)) writePersisted();
  emit();
}

/** Convenience: clear the state of a thread (back to implicit idle). */
export function clearThreadState(threadId) {
  setThreadState(threadId, null);
}

/** Read-only snapshot of the current state map. */
export function getThreadStates() {
  return snapshot();
}

/** Subscribe to state-map changes. Returns an unsubscribe fn. */
export function subscribeThreadStates(listener) {
  subscribers.add(listener);
  return () => {
    subscribers.delete(listener);
  };
}

/** React adapter for the state map. Re-renders on any set/clear. */
export function useThreadStates() {
  const [map, setMap] = React.useState(getThreadStates);
  React.useEffect(() => subscribeThreadStates(setMap), []);
  return map;
}
