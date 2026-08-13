// Same-profile tab coordination for run-completion presentation
// (2026-08-13 design §5.5, §5.6, §9.2).
//
// Every tab broadcasts its own `TabNotificationState` over a
// `BroadcastChannel`; each tab merges the freshest report per tab and
// derives the SAME profile-level intent for a notice, so any tab can submit
// it (submission and presentation are deduplicated with localStorage
// test-and-set keys — the profile-local ledger; the service worker's push
// dedupe covers the sleeping-profile path). The server remains the only
// arbiter across profiles and devices, and accepts at most one current
// intent per notice and browser profile, so duplicate submissions replace
// rather than multiply.
//
// The service worker stays the authority for push display, notification
// click routing, and tag closing; pages own the live-stream arbitration
// bookkeeping because they hold the session stream and the authenticated
// HTTP session the mutations require (§5.5: the worker never receives
// session credentials).

import { bumpFocusEpoch, currentFocusEpoch, nextStateRevision, tabId } from "./ids";
import type { RunCompletionIntentKind, RunCompletionNotice } from "./protocol";

const CHANNEL_NAME = "ironclaw-run-completions";
// A tab report older than this is treated as gone (closed tab, frozen
// background page). Fresh reports arrive on every focus/visibility/route
// transition plus a keepalive.
const TAB_STATE_TTL_MS = 20_000;
const KEEPALIVE_INTERVAL_MS = 8_000;
// §5.4: at most 128 recent observed run IDs per tab state.
const MAX_OBSERVED_RUN_IDS = 128;
// Presentation/submission ledger entries expire so storage stays bounded.
const LEDGER_TTL_MS = 6 * 60 * 60 * 1000;
const LEDGER_PREFIX = "ironclaw.rc.ledger.";

export type TabNotificationState = {
  tabId: string;
  stateRevision: number;
  focusEpoch: number;
  route: { kind: "thread"; threadId: string } | { kind: "other" };
  visibility: "visible" | "hidden";
  focused: boolean;
  observedRunIds: string[];
  reportedAt: number;
};

type CoordinationMessage =
  | { type: "tab_state"; state: TabNotificationState }
  | { type: "tab_closed"; tabId: string };

let channel: BroadcastChannel | null = null;
let keepalive: ReturnType<typeof setInterval> | null = null;
const peerStates = new Map<string, TabNotificationState>();
const observedRunIds: string[] = [];
let currentRoute: TabNotificationState["route"] = { kind: "other" };
let started = false;

function nowMs(): number {
  return Date.now();
}

function localState(): TabNotificationState {
  return {
    tabId: tabId(),
    stateRevision: nextStateRevision(),
    focusEpoch: currentFocusEpoch(),
    route: currentRoute,
    visibility:
      typeof document !== "undefined" && document.visibilityState === "hidden"
        ? "hidden"
        : "visible",
    focused: typeof document !== "undefined" ? document.hasFocus() : false,
    observedRunIds: [...observedRunIds],
    reportedAt: nowMs(),
  };
}

function broadcast(state: TabNotificationState) {
  peerStates.set(state.tabId, state);
  try {
    channel?.postMessage({ type: "tab_state", state } satisfies CoordinationMessage);
  } catch (_) {
    // A closed channel only affects cross-tab dedupe; the server still
    // collapses duplicate intents by browser profile.
  }
}

export function reportStateChange() {
  if (!started) return;
  broadcast(localState());
}

export function setRouteThread(threadId: string | null) {
  currentRoute = threadId ? { kind: "thread", threadId } : { kind: "other" };
  reportStateChange();
}

export function recordObservedRun(runId: string) {
  if (!runId) return;
  const existing = observedRunIds.indexOf(runId);
  if (existing !== -1) observedRunIds.splice(existing, 1);
  observedRunIds.push(runId);
  while (observedRunIds.length > MAX_OBSERVED_RUN_IDS) observedRunIds.shift();
  reportStateChange();
}

export function hasObservedRun(runId: string): boolean {
  return observedRunIds.includes(runId);
}

export function startCoordination(): () => void {
  if (started) return stopCoordination;
  started = true;
  if (typeof BroadcastChannel !== "undefined") {
    channel = new BroadcastChannel(CHANNEL_NAME);
    channel.onmessage = (event) => {
      const message = event.data as CoordinationMessage | null;
      if (!message || typeof message !== "object") return;
      if (message.type === "tab_state" && message.state?.tabId) {
        peerStates.set(message.state.tabId, message.state);
      } else if (message.type === "tab_closed" && message.tabId) {
        peerStates.delete(message.tabId);
      }
    };
  }
  const onFocus = () => {
    bumpFocusEpoch();
    reportStateChange();
  };
  const onBlur = () => reportStateChange();
  const onVisibility = () => reportStateChange();
  const onPageHide = () => {
    try {
      channel?.postMessage({
        type: "tab_closed",
        tabId: tabId(),
      } satisfies CoordinationMessage);
    } catch (_) {
      // Best effort; TTL expiry covers the silent-close case.
    }
  };
  window.addEventListener("focus", onFocus);
  window.addEventListener("blur", onBlur);
  document.addEventListener("visibilitychange", onVisibility);
  window.addEventListener("pagehide", onPageHide);
  keepalive = setInterval(reportStateChange, KEEPALIVE_INTERVAL_MS);
  reportStateChange();
  return stopCoordination;

  function stopCoordination() {
    started = false;
    window.removeEventListener("focus", onFocus);
    window.removeEventListener("blur", onBlur);
    document.removeEventListener("visibilitychange", onVisibility);
    window.removeEventListener("pagehide", onPageHide);
    if (keepalive) clearInterval(keepalive);
    keepalive = null;
    channel?.close();
    channel = null;
    peerStates.clear();
  }
}

function liveStates(): TabNotificationState[] {
  const deadline = nowMs() - TAB_STATE_TTL_MS;
  const local = localState();
  peerStates.set(local.tabId, local);
  return Array.from(peerStates.values()).filter(
    (state) => state.reportedAt >= deadline,
  );
}

export type ProfileIntentDecision = {
  intent: RunCompletionIntentKind;
  /** The tab that would present, when one is needed. */
  presentingTabId: string | null;
  stateRevision: number;
  focusEpoch: number;
};

/**
 * The §5.6 profile-level intent for one notice, derived from every live
 * tab's state. Deterministic across tabs given the same reports, so any
 * tab may submit it.
 */
export function profileIntentFor(notice: RunCompletionNotice): ProfileIntentDecision {
  const states = liveStates();
  const revision = Math.max(...states.map((state) => state.stateRevision), 0);
  const epoch = Math.max(...states.map((state) => state.focusEpoch), 0);
  const focused = states
    .filter((state) => state.focused && state.visibility === "visible")
    // Deterministic focus-claim tie-break: a tab on the notice's thread
    // outranks a tab elsewhere; then smallest tab id (§5.6).
    .sort((a, b) => {
      const aOnThread =
        a.route.kind === "thread" && a.route.threadId === notice.thread_id;
      const bOnThread =
        b.route.kind === "thread" && b.route.threadId === notice.thread_id;
      if (aOnThread !== bOnThread) return aOnThread ? -1 : 1;
      return a.tabId < b.tabId ? -1 : 1;
    });
  const decisionBase = { stateRevision: revision, focusEpoch: epoch };
  const winner = focused[0] ?? null;
  if (winner) {
    const onThread =
      winner.route.kind === "thread" && winner.route.threadId === notice.thread_id;
    if (onThread && winner.observedRunIds.includes(notice.run_id)) {
      return { intent: "reply_observed", presentingTabId: winner.tabId, ...decisionBase };
    }
    if (onThread) {
      return { intent: "watching_thread", presentingTabId: winner.tabId, ...decisionBase };
    }
    return { intent: "in_app", presentingTabId: winner.tabId, ...decisionBase };
  }
  if (states.length > 0) {
    // Tabs exist, none focused: local OS presentation, if permission and
    // server-side selection/enrollment allow (the server validates; the
    // browser only asserts permission here — never prompts, §6.1).
    if (
      typeof Notification !== "undefined" &&
      Notification.permission === "granted"
    ) {
      return { intent: "local_os", presentingTabId: null, ...decisionBase };
    }
    return { intent: "unavailable", presentingTabId: null, ...decisionBase };
  }
  return { intent: "unavailable", presentingTabId: null, ...decisionBase };
}

/**
 * Profile-local test-and-set ledger (§9.2): at most one tab submits one
 * intent revision or applies one grant. localStorage writes are synchronous
 * within a profile, which is the exact scope the dedupe needs; the durable
 * server records stay authoritative if storage is unavailable.
 */
export function claimOnce(key: string): boolean {
  const fullKey = `${LEDGER_PREFIX}${key}`;
  try {
    if (window.localStorage.getItem(fullKey)) return false;
    window.localStorage.setItem(fullKey, String(nowMs()));
    pruneLedger();
    return true;
  } catch (_) {
    // No shared storage: fall back to per-tab claims. The server's
    // idempotent operations absorb the duplicates.
    if (localClaims.has(fullKey)) return false;
    localClaims.add(fullKey);
    return true;
  }
}

export function releaseClaimsFor(noticeId: string) {
  try {
    const doomed: string[] = [];
    for (let index = 0; index < window.localStorage.length; index += 1) {
      const key = window.localStorage.key(index);
      if (key?.startsWith(LEDGER_PREFIX) && key.includes(noticeId)) {
        doomed.push(key);
      }
    }
    for (const key of doomed) window.localStorage.removeItem(key);
  } catch (_) {
    // Ledger pruning is best-effort.
  }
}

function pruneLedger() {
  try {
    const deadline = nowMs() - LEDGER_TTL_MS;
    const doomed: string[] = [];
    for (let index = 0; index < window.localStorage.length; index += 1) {
      const key = window.localStorage.key(index);
      if (!key?.startsWith(LEDGER_PREFIX)) continue;
      const stamp = Number(window.localStorage.getItem(key) ?? 0);
      if (!Number.isFinite(stamp) || stamp < deadline) doomed.push(key);
    }
    for (const key of doomed) window.localStorage.removeItem(key);
  } catch (_) {
    // Best-effort bound.
  }
}

const localClaims = new Set<string>();

export function resetCoordinationForTests() {
  peerStates.clear();
  observedRunIds.length = 0;
  currentRoute = { kind: "other" };
  localClaims.clear();
}
