// The app-root run-completion orchestrator (2026-08-13 design §5–§9).
//
// One instance per authenticated page:
//   boot    — durable unread snapshot seeds the badge cache, then the
//             owner-scoped logical stream subscribes from the snapshot's
//             resume sequence over the SHARED session socket (no second
//             connection, no event-specific route);
//   notices — the profile intent is derived from merged tab state and
//             submitted over authenticated HTTP (never the socket);
//   grants  — only this browser profile applies a grant naming it, on one
//             tab (profile-local test-and-set), and acknowledges over HTTP
//             after the effect actually happened;
//   clears  — every tab drops local surfaces and the worker closes OS
//             notifications by thread tag (§9.3).
//
// Dynamically imported by its hook so none of this rides the eager /chat
// bundle.

import {
  acknowledgeRunCompletion,
  fetchUnreadRunCompletions,
  reportRunCompletionThreadRead,
  submitRunCompletionIntent,
} from "../api";
import { sessionEventClient } from "../session-events/client";
import { toast } from "../toast";
import { observedThroughSequence, resumeCursorFor } from "./evidence";
import { browserInstanceId, currentStateRevision, tabId } from "./ids";
import {
  claimOnce,
  hasObservedRun,
  profileIntentFor,
  recordObservedRun,
  releaseClaimsFor,
  reportStateChange,
  setRouteThread,
  startCoordination,
} from "./coordination";
import { compareSequences } from "./sequence";
import {
  type RunCompletionGrant,
  type RunCompletionNotice,
  noticeFromWire,
  parseRunCompletionEvent,
} from "./protocol";
import {
  applyClear,
  applyNotice,
  applySettled,
  noticeForRun,
  rebaseFromSnapshot,
  resetRunCompletionStore,
  runCompletionSnapshot,
  unreadForThread,
} from "./store";

type OrchestratorOptions = {
  /** Presentation copy, resolved by the caller's i18n. */
  inAppMessage: (unreadForThread: number) => string;
  /** SPA navigation for toast click-through. */
  navigateToThread: (threadId: string) => void;
};

let subscription: { unsubscribe: () => void } | null = null;
let stopCoordinationFn: (() => void) | null = null;
let options: OrchestratorOptions | null = null;
let watchingGrants = new Map<string, RunCompletionGrant>();
let started = false;
// The thread whose history the focused view has confirmed rendered (§9.3):
// `thread_read` evidence is issued only for this thread, never from route
// presence alone, so a notice cannot be settled before its reply is on
// screen.
let historyRenderedThreadId: string | null = null;
// Consecutive non-retryable subscription failures since the last delivered
// event. Bounds the rebase-then-resubscribe recovery so a persistently
// rejected selector degrades to snapshot-only instead of looping.
let terminalSubscriptionFailures = 0;
const MAX_TERMINAL_RESUBSCRIBES = 3;
// Bounded tolerance between the revision a grant was issued against and the
// profile's current revision: focus churn during the arbitration window is
// normal; a large gap means the grant no longer describes this browser.
const STALE_REVISION_TOLERANCE = 64;
// The `thread_read` evidence currently in flight (`<thread>:<sequence>`), so a
// badge change while the request is outstanding cannot re-fire the same
// drain and race its own settlement.
let threadReadInFlight: string | null = null;
// Set while the shared socket is reconnecting; the first `open` afterwards
// rebases the badge cache from the durable snapshot (§7.6), because clears
// and grants that fired during the gap ride the notice's own sequence and are
// not replayed by the resumed subscription.
let transportReconnecting = false;

export function startRunCompletions(opts: OrchestratorOptions): () => void {
  if (started) {
    options = opts;
    return stopRunCompletions;
  }
  started = true;
  options = opts;
  stopCoordinationFn = startCoordination();
  void boot();
  return stopRunCompletions;
}

export function stopRunCompletions() {
  started = false;
  subscription?.unsubscribe();
  subscription = null;
  stopCoordinationFn?.();
  stopCoordinationFn = null;
  watchingGrants = new Map();
  historyRenderedThreadId = null;
  threadReadInFlight = null;
  terminalSubscriptionFailures = 0;
  transportReconnecting = false;
  // The badge cache is per-signed-in-owner state: clear it so a following
  // sign-in never briefly renders the previous account's notices, and so a
  // stale boot()/rebase() resolving after stop repopulates nothing visible
  // (the next start rebases from the durable snapshot anyway).
  resetRunCompletionStore();
}

async function boot() {
  let resume = "0";
  try {
    resume = await rebase();
  } catch (_) {
    resume = "0";
  }
  subscribeStream(resume);
}

async function rebase(): Promise<string> {
  const snapshot = await fetchUnreadRunCompletions();
  const notices = Array.isArray(snapshot?.notices)
    ? snapshot.notices
        .map((wire: unknown) => noticeFromWire(wire))
        .filter((notice: RunCompletionNotice | null): notice is RunCompletionNotice =>
          Boolean(notice),
        )
    : [];
  const resume =
    typeof snapshot?.resume_sequence === "string" ? snapshot.resume_sequence : "0";
  rebaseFromSnapshot(notices, resume);
  return resume;
}

function subscribeStream(fromSequence: string) {
  if (!started) return;
  const client = sessionEventClient();
  subscription = client.subscribe(
    { kind: "run_completions" },
    {
      onEvent: ({ body }) => {
        terminalSubscriptionFailures = 0;
        const event = parseRunCompletionEvent(body);
        if (!event) return;
        if (event.type === "notice") {
          void handleNotice(event.notice);
        } else if (event.type === "grant") {
          void handleGrant(event.grant);
        } else {
          applyClear(event.clear.notice_id, event.clear.sequence);
          releaseClaimsFor([event.clear.notice_id]);
          void closeOsNotificationsByTag(event.clear.thread_tag);
        }
      },
      onError: (error) => {
        if (error.retryable) return;
        // The shared client dropped this registration (a non-retryable
        // selector failure is terminal for that subscription, e.g. a cursor
        // beyond retention). Rebase from the durable snapshot and register
        // a fresh subscription from its head — bounded, so a selector the
        // server keeps rejecting degrades to snapshot-only.
        terminalSubscriptionFailures += 1;
        if (terminalSubscriptionFailures > MAX_TERMINAL_RESUBSCRIBES) return;
        void rebase()
          .then((resume) => subscribeStream(resume))
          .catch(() => undefined);
      },
      onStatus: (status) => {
        if (status === "reconnecting") {
          transportReconnecting = true;
        } else if (status === "open" && transportReconnecting) {
          transportReconnecting = false;
          void rebase().catch(() => undefined);
        }
      },
    },
    // Resume strictly after the snapshot's head (`rc:` cursor namespace):
    // the snapshot already seeded everything at or before it, so the
    // replay carries only what happened since.
    { idPrefix: "rc", fromCursor: resumeCursorFor(fromSequence) },
  );
}

async function handleNotice(notice: RunCompletionNotice) {
  applyNotice(notice);
  if (notice.read) return;
  const decision = profileIntentFor(notice);
  // §8.1 fast path: the focused thread view already rendered this exact
  // reply — reply_observed settles without any presentation.
  const submissionKey = `intent:${notice.notice_id}:${decision.intent}:${decision.stateRevision}`;
  if (!claimOnce(submissionKey)) return;
  try {
    const response = await submitRunCompletionIntent({
      noticeId: notice.notice_id,
      browserInstanceId: browserInstanceId(),
      tabId: tabId(),
      stateRevision: decision.stateRevision,
      focusEpoch: decision.focusEpoch,
      intent: decision.intent,
    });
    if (Array.isArray(response?.settled_notice_ids)) {
      applySettled(response.settled_notice_ids);
    }
  } catch (_) {
    // Intent submission is best-effort: the server falls back to push or
    // in-app-unread on silence, which is the design's safe default.
  }
}

async function handleGrant(grant: RunCompletionGrant) {
  if (grant.browser_instance_id !== browserInstanceId()) return;
  // Stale grants are rejected rather than applied (§5.6): the profile's
  // state moved past the revision the grant was issued against.
  if (grant.state_revision < currentStateRevision() - STALE_REVISION_TOLERANCE) {
    await acknowledge(grant, "stale_state");
    return;
  }
  if (grant.surface === "in_app" && !pageIsVisible()) {
    // §5.6: an in-app grant is presented by a tab the user can see. A hidden
    // tab leaves the claim to a visible one; if none claims before the grant
    // expires, arbitration regresses and falls back.
    return;
  }
  const applyKey = `grant:${grant.grant_id}`;
  if (!claimOnce(applyKey)) return;
  const notice = noticeById(grant.notice_id);
  switch (grant.surface) {
    case "no_surface_watching_thread": {
      // Lease: the focused thread view has until expiry to confirm the
      // exact reply rendered. If it already has, settle immediately.
      if (notice && hasObservedRun(notice.run_id)) {
        await acknowledge(grant, "reply_rendered");
        applyClear(grant.notice_id);
        return;
      }
      watchingGrants.set(grant.notice_id, grant);
      return;
    }
    case "in_app": {
      const message = options?.inAppMessage(notice?.unread_count_for_thread ?? 1);
      try {
        toast(message ?? "An agent run finished.", { tone: "info" });
        reportStateChange();
        await acknowledge(grant, "presented");
      } catch (_) {
        await acknowledge(grant, "effect_failed");
      }
      return;
    }
    case "local_os": {
      const shown = await showOsNotification(notice);
      await acknowledge(grant, shown ? "presented" : "effect_failed");
      return;
    }
    default:
      // Unknown surface from a newer server: report the effect failed so
      // arbitration falls back rather than silently suppressing.
      await acknowledge(grant, "effect_failed");
  }
}

function noticeById(noticeId: string): RunCompletionNotice | null {
  for (const notice of runCompletionSnapshot().notices) {
    if (notice.notice_id === noticeId) return notice;
  }
  return null;
}

function pageIsVisible(): boolean {
  return typeof document === "undefined" || document.visibilityState === "visible";
}

/**
 * Acknowledge a grant. Resolves to the notice ids the server settled on
 * this evidence (`null` when the acknowledgement was lost or rejected), so
 * a caller that needs read evidence can tell a settled lease from one the
 * server had already expired or re-granted.
 */
async function acknowledge(
  grant: RunCompletionGrant,
  outcome: "reply_rendered" | "presented" | "stale_state" | "effect_failed",
): Promise<string[] | null> {
  try {
    const response = await acknowledgeRunCompletion({
      noticeId: grant.notice_id,
      grantId: grant.grant_id,
      stateRevision: currentStateRevision(),
      outcome,
    });
    const settled = Array.isArray(response?.settled_notice_ids)
      ? response.settled_notice_ids
      : null;
    if (settled && outcome === "reply_rendered") {
      applySettled(settled);
    }
    return settled;
  } catch (_) {
    // A lost acknowledgement may duplicate presentation later but cannot
    // silently lose the notice (§5.6).
    return null;
  }
}

async function showOsNotification(
  notice: RunCompletionNotice | null,
): Promise<boolean> {
  if (
    typeof Notification === "undefined" ||
    Notification.permission !== "granted" ||
    typeof navigator === "undefined" ||
    !("serviceWorker" in navigator)
  ) {
    return false;
  }
  try {
    const registration = await navigator.serviceWorker.getRegistration();
    if (!registration) return false;
    const threadPath = notice
      ? `/chat/${encodeURIComponent(notice.thread_id)}`
      : "/";
    // Fixed, localized copy only (the same strings as the in-app toast): OS
    // surfaces never carry generated content.
    const body =
      options?.inAppMessage(notice?.unread_count_for_thread ?? 1) ??
      "An agent run finished.";
    await registration.showNotification("IronClaw", {
      body,
      tag: notice?.thread_tag || undefined,
      data: { url: threadPath },
      icon: "/assets/web-app-manifest-192x192.png",
      badge: "/assets/web-app-manifest-192x192.png",
    });
    return true;
  } catch (_) {
    return false;
  }
}

async function closeOsNotificationsByTag(tag: string) {
  if (!tag || typeof navigator === "undefined" || !("serviceWorker" in navigator)) {
    return;
  }
  try {
    const registration = await navigator.serviceWorker.getRegistration();
    if (!registration) return;
    const notifications = await registration.getNotifications({ tag });
    for (const notification of notifications) notification.close();
  } catch (_) {
    // Best-effort close; a sleeping worker clears on its next wake (§9.3).
  }
}

// ---- Evidence reporting from the chat surface ----

/** The chat route announces which thread is on screen. */
export function reportActiveThread(threadId: string | null) {
  if (threadId !== historyRenderedThreadId) {
    // A different (or no) thread is on screen: its history is not yet
    // confirmed rendered, so thread-read evidence waits for that signal.
    historyRenderedThreadId = null;
  }
  setRouteThread(threadId);
}

/**
 * The focused thread view finished loading its history for `threadId`
 * (§9.3: "a focused thread view confirms finalized replies through
 * sequence N"). Unlocks thread-read evidence for that thread and settles
 * whatever is already unread for it.
 */
export function reportThreadHistoryRendered(threadId: string, renderedRunIds: string[] = []) {
  if (!threadId) return;
  // The replies in the loaded history are rendered evidence, exactly like a
  // live finalization: they bound how far `thread_read` may advance.
  for (const runId of renderedRunIds) recordObservedRun(runId);
  historyRenderedThreadId = threadId;
  reportThreadViewed(threadId);
}

/**
 * The focused thread view consumed a finalized reply for `runId` (§9.3).
 * Settles a pending watching-thread lease, or answers a later notice with
 * `reply_observed`.
 */
export function reportReplyRendered(runId: string) {
  if (!runId) return;
  recordObservedRun(runId);
  if (typeof document !== "undefined") {
    if (document.visibilityState !== "visible" || !document.hasFocus()) return;
  }
  const notice = noticeForRun(runId);
  if (!notice) return;
  const lease = watchingGrants.get(notice.notice_id);
  if (lease) {
    watchingGrants.delete(notice.notice_id);
    void acknowledge(lease, "reply_rendered").then((settled) => {
      if (settled?.includes(notice.notice_id)) return;
      // The lease had already expired or been re-granted (the server
      // answers 409 and mints no read evidence): fall back to the
      // reply-observed intent, which carries this browser's own identity.
      submitReplyObserved(notice);
    });
    return;
  }
  submitReplyObserved(notice);
}

function submitReplyObserved(notice: RunCompletionNotice) {
  void submitRunCompletionIntent({
    noticeId: notice.notice_id,
    browserInstanceId: browserInstanceId(),
    tabId: tabId(),
    stateRevision: currentStateRevision(),
    focusEpoch: 0,
    intent: "reply_observed",
  })
    .then((response) => {
      if (Array.isArray(response?.settled_notice_ids)) {
        applySettled(response.settled_notice_ids);
      }
    })
    .catch(() => undefined);
}

/**
 * The focused thread view rendered its history through the newest
 * finalized reply: advance read state for every unread completion of this
 * thread at or below the greatest known sequence (§7.8, §9.3).
 */
export function reportThreadViewed(threadId: string) {
  if (!threadId) return;
  // Display is not read (§9.3): route presence or a badge change alone never
  // settles a notice — only a focused view whose history for THIS thread has
  // been confirmed rendered.
  if (historyRenderedThreadId !== threadId) return;
  if (typeof document !== "undefined") {
    if (document.visibilityState !== "visible" || !document.hasFocus()) return;
  }
  // Advance only through completions whose reply this tab has rendered
  // (history load or live finalization); a notice that arrived for the
  // open thread while its reply is still in flight stays unread.
  const throughSequence = observedThroughSequence(threadId);
  if (!throughSequence) return;
  const inFlightKey = `${threadId}:${throughSequence}`;
  if (threadReadInFlight === inFlightKey) return;
  threadReadInFlight = inFlightKey;
  const settledLocally = unreadForThread(threadId)
    .filter((notice) => compareSequences(notice.sequence, throughSequence) <= 0)
    .map((notice) => notice.notice_id);
  void reportRunCompletionThreadRead({
    threadId,
    throughSequence,
    browserInstanceId: browserInstanceId(),
  })
    .then((response) => {
      const settled = Array.isArray(response?.settled_notice_ids)
        ? response.settled_notice_ids
        : settledLocally;
      applySettled(settled);
      releaseClaimsFor(settled);
    })
    .catch(() => undefined)
    .finally(() => {
      if (threadReadInFlight === inFlightKey) threadReadInFlight = null;
    });
}
