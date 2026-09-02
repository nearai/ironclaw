// Opaque browser-profile and tab identity for run-completion arbitration
// (2026-08-13 design §5.5). Neither identifier contains account or route
// data; both are random, minted locally, and only ever correlated
// server-side against host-owned records.

const BROWSER_INSTANCE_KEY = "ironclaw:run-completions:browser-instance";
const TAB_KEY = "ironclaw:run-completions:tab";

function randomOpaqueId(prefix: string): string {
  const bytes = new Uint8Array(16);
  if (globalThis.crypto?.getRandomValues) {
    globalThis.crypto.getRandomValues(bytes);
  } else {
    for (let index = 0; index < bytes.length; index += 1) {
      bytes[index] = Math.floor(Math.random() * 256);
    }
  }
  const hex = Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("");
  return `${prefix}-${hex}`;
}

/** Stable per-browser-profile id (localStorage; survives restarts). */
export function browserInstanceId(): string {
  try {
    const existing = window.localStorage.getItem(BROWSER_INSTANCE_KEY);
    if (existing) return existing;
    const minted = randomOpaqueId("rbi");
    window.localStorage.setItem(BROWSER_INSTANCE_KEY, minted);
    return minted;
  } catch (_) {
    // Storage unavailable (private mode edge cases): a per-page id still
    // arbitrates correctly, it just cannot dedupe across restarts.
    return ephemeralBrowserId;
  }
}

/** Per-tab id (sessionStorage; survives reloads of the same tab). */
export function tabId(): string {
  try {
    const existing = window.sessionStorage.getItem(TAB_KEY);
    if (existing) return existing;
    const minted = randomOpaqueId("rtb");
    window.sessionStorage.setItem(TAB_KEY, minted);
    return minted;
  } catch (_) {
    return ephemeralTabId;
  }
}

const ephemeralBrowserId = randomOpaqueId("rbi");
const ephemeralTabId = randomOpaqueId("rtb");

// Monotonic browser state revision (§4): bumped on every reported state
// change so the server can reject grants issued against stale state.
let stateRevisionCounter = Date.now();
let focusEpochCounter = 0;

export function nextStateRevision(): number {
  stateRevisionCounter += 1;
  return stateRevisionCounter;
}

export function currentStateRevision(): number {
  return stateRevisionCounter;
}

export function bumpFocusEpoch(): number {
  focusEpochCounter += 1;
  return focusEpochCounter;
}

export function currentFocusEpoch(): number {
  return focusEpochCounter;
}
