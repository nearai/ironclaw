import { authScope } from "./auth-scope.js";

const STORAGE_PREFIX = "ironclaw:v2-notifications:";
const MAX_SEEN_IDS = 250;
const MAX_MESSAGES = 30;
const APPROVAL_STATES = new Set([
  "needs_attention",
  "awaitingapproval",
  "awaiting_approval",
]);

const subscribers = new Set();
let loadedScope = null;
let state = {
  initialized: false,
  seenIds: new Set(),
};

function notificationScope(scope) {
  return scope || authScope();
}

function storageKey(scope) {
  return `${STORAGE_PREFIX}${notificationScope(scope)}`;
}

function readPersisted(scope) {
  try {
    if (typeof window === "undefined" || !window.localStorage) {
      return { initialized: false, seenIds: [] };
    }
    const raw = window.localStorage.getItem(storageKey(scope));
    if (!raw) return { initialized: false, seenIds: [] };
    const parsed = JSON.parse(raw);
    return {
      initialized: parsed?.initialized === true,
      seenIds: Array.isArray(parsed?.seen_ids)
        ? parsed.seen_ids.filter((id) => typeof id === "string")
        : [],
    };
  } catch (_) {
    return { initialized: false, seenIds: [] };
  }
}

function writePersisted(scope) {
  try {
    if (typeof window === "undefined" || !window.localStorage) return;
    window.localStorage.setItem(
      storageKey(scope),
      JSON.stringify({
        initialized: state.initialized,
        seen_ids: [...state.seenIds].slice(-MAX_SEEN_IDS),
      }),
    );
  } catch (_) {
    // Best-effort only; unread state should never block the header.
  }
}

function trimSeenIds() {
  if (state.seenIds.size <= MAX_SEEN_IDS) return;
  state.seenIds = new Set([...state.seenIds].slice(-MAX_SEEN_IDS));
}

function ensureScope(scope) {
  const nextScope = notificationScope(scope);
  if (nextScope === loadedScope) return;
  const persisted = readPersisted(nextScope);
  state = {
    initialized: persisted.initialized,
    seenIds: new Set(persisted.seenIds),
  };
  loadedScope = nextScope;
}

function snapshot(scope) {
  ensureScope(scope);
  return {
    initialized: state.initialized,
    seenIds: new Set(state.seenIds),
  };
}

function emit(scope) {
  const nextScope = notificationScope(scope);
  const next = snapshot(nextScope);
  for (const listener of subscribers) {
    try {
      listener(next, nextScope);
    } catch (_) {
      // Ignore subscriber errors; this is UI convenience state.
    }
  }
}

export function getNotificationState(scope) {
  return snapshot(scope);
}

export function subscribeNotifications(listener) {
  subscribers.add(listener);
  return () => {
    subscribers.delete(listener);
  };
}

export function markNotificationIdsSeen(messageIds = [], scope) {
  ensureScope(scope);
  state.initialized = true;
  for (const id of messageIds) {
    if (id) state.seenIds.add(id);
  }
  trimSeenIds();
  writePersisted(scope);
  emit(scope);
  return snapshot(scope);
}

export function isApprovalThread(thread, state) {
  const summaryState = String(thread?.state || "").toLowerCase();
  const localState = String(state || "").toLowerCase();
  return APPROVAL_STATES.has(summaryState) || APPROVAL_STATES.has(localState);
}

export function approvalThreadNotificationId(thread) {
  const threadId = thread?.id || thread?.thread_id;
  if (!threadId) return null;
  const freshness =
    thread?.approval_request_id ||
    thread?.approval_id ||
    thread?.gate_ref ||
    thread?.run_id ||
    thread?.turn_run_id ||
    thread?.updated_at ||
    thread?.created_at ||
    thread?.last_activity ||
    thread?.last_activity_at ||
    "pending";
  return `approval:${threadId}:${encodeURIComponent(String(freshness))}`;
}

function threadTimestamp(thread) {
  const value =
    thread?.updated_at ||
    thread?.created_at ||
    thread?.last_activity ||
    thread?.last_activity_at;
  const timestamp = value ? Date.parse(value) : NaN;
  return Number.isFinite(timestamp) ? timestamp : 0;
}

export function approvalThreadNotifications(
  threads = [],
  threadStates = new Map(),
  t = (key) => key,
) {
  const tx = typeof t === "function" ? t : (key) => key;
  const messages = [];

  for (const thread of Array.isArray(threads) ? threads : []) {
    const threadId = thread?.id || thread?.thread_id;
    const state = threadStates instanceof Map ? threadStates.get(threadId) : null;
    if (!isApprovalThread(thread, state)) continue;
    const id = approvalThreadNotificationId(thread);
    if (!id || !threadId) continue;
    const timestamp = threadTimestamp(thread);
    messages.push({
      id,
      type: "approval",
      icon: "shield",
      title: tx("notifications.approval.title"),
      body: thread.title || tx("notifications.approval.untitled"),
      detail: tx("notifications.approval.detail"),
      timeLabel: timestamp ? new Date(timestamp).toLocaleString([], {
        month: "short",
        day: "numeric",
        hour: "2-digit",
        minute: "2-digit",
      }) : "",
      timestamp,
      href: `/chat/${encodeURIComponent(threadId)}`,
    });
  }

  return messages
    .sort((a, b) => b.timestamp - a.timestamp)
    .slice(0, MAX_MESSAGES);
}
