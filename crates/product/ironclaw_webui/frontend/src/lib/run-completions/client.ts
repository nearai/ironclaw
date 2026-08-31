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
import { isSessionEventsAdvertised } from "../session-events/transport-flag";
import { toast } from "../toast";
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
  maxSequenceForThread,
  noticeForRun,
  rebaseFromSnapshot,
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
}

async function boot() {
  if (!isSessionEventsAdvertised()) {
    // The deployment does not advertise the session transport: notices
    // still exist durably and surface on next boot with the flag on. The
    // unread snapshot alone still fills the badge.
    try {
      await rebase();
    } catch (_) {
      // Snapshot unavailable: leave the cache empty; nothing to clean up.
    }
    return;
  }
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
        const event = parseRunCompletionEvent(body);
        if (!event) return;
        if (event.type === "notice") {
          void handleNotice(event.notice);
        } else if (event.type === "grant") {
          void handleGrant(event.grant);
        } else {
          applyClear(event.clear.notice_id, event.clear.sequence);
          releaseClaimsFor(event.clear.notice_id);
          void closeOsNotificationsByTag(event.clear.thread_tag);
        }
      },
      onError: (error) => {
        if (!error.retryable) {
          // Cursor unusable (e.g. beyond retention): rebase from the
          // durable snapshot; the client resubscribes from the new cursor.
          void rebase().catch(() => undefined);
        }
      },
    },
    { idPrefix: "rc" },
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

// Bounded tolerance between the revision a grant was issued against and the
// profile's current revision: focus churn during the arbitration window is
// normal; a large gap means the grant no longer describes this browser.
const STALE_REVISION_TOLERANCE = 64;

function noticeById(noticeId: string): RunCompletionNotice | null {
  for (const notice of runCompletionSnapshot().notices) {
    if (notice.notice_id === noticeId) return notice;
  }
  return null;
}

async function acknowledge(
  grant: RunCompletionGrant,
  outcome: "reply_rendered" | "presented" | "stale_state" | "effect_failed",
) {
  try {
    const response = await acknowledgeRunCompletion({
      noticeId: grant.notice_id,
      grantId: grant.grant_id,
      stateRevision: currentStateRevision(),
      outcome,
    });
    if (Array.isArray(response?.settled_notice_ids) && outcome === "reply_rendered") {
      applySettled(response.settled_notice_ids);
    }
  } catch (_) {
    // A lost acknowledgement may duplicate presentation later but cannot
    // silently lose the notice (§5.6).
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
    await registration.showNotification("IronClaw", {
      // Fixed copy only: OS surfaces never carry generated content.
      body: "An agent run finished.",
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
  setRouteThread(threadId);
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
    void acknowledge(lease, "reply_rendered").then(() => {
      applyClear(notice.notice_id);
    });
    return;
  }
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
  if (typeof document !== "undefined") {
    if (document.visibilityState !== "visible" || !document.hasFocus()) return;
  }
  const throughSequence = maxSequenceForThread(threadId);
  if (!throughSequence) return;
  const settledLocally = unreadForThread(threadId).map((notice) => notice.notice_id);
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
      for (const noticeId of settled) releaseClaimsFor(noticeId);
    })
    .catch(() => undefined);
}
