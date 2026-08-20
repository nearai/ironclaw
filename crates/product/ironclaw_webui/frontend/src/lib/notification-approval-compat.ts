// @ts-nocheck
// Temporary compatibility path for rolling deployments whose backend does not
// expose the durable notification inbox API yet.

import { authScope } from "./auth-scope";

const STORAGE_PREFIX = "ironclaw:v2-notifications:";
const MAX_SEEN_IDS = 250;
const MAX_MESSAGES = 30;
const APPROVAL_STATES = new Set([
  "needs_attention",
  "awaitingapproval",
  "awaiting_approval",
]);

let loadedScope = null;
let state = { initialized: false, seenIds: new Set() };

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
    // Best-effort only; compatibility state must never break the header.
  }
}

export function getNotificationState(scope) {
  ensureScope(scope);
  return {
    initialized: state.initialized,
    seenIds: new Set(state.seenIds),
  };
}

export function markNotificationIdsSeen(messageIds = [], scope) {
  ensureScope(scope);
  state.initialized = true;
  for (const id of messageIds) {
    if (id) state.seenIds.add(id);
  }
  if (state.seenIds.size > MAX_SEEN_IDS) {
    state.seenIds = new Set([...state.seenIds].slice(-MAX_SEEN_IDS));
  }
  writePersisted(scope);
  return getNotificationState(scope);
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

function isApprovalThread(thread, localState) {
  return APPROVAL_STATES.has(String(thread?.state || "").toLowerCase()) ||
    APPROVAL_STATES.has(String(localState || "").toLowerCase());
}

function threadTimestamp(thread) {
  const value =
    thread?.updated_at ||
    thread?.created_at ||
    thread?.last_activity ||
    thread?.last_activity_at;
  const parsed = value ? Date.parse(value) : NaN;
  return Number.isFinite(parsed) ? parsed : 0;
}

export function approvalThreadNotifications(
  threads = [],
  threadStates = new Map(),
  t = (key) => key,
) {
  const tx = typeof t === "function" ? t : (key) => key;
  return (Array.isArray(threads) ? threads : [])
    .flatMap((thread) => {
      const threadId = thread?.id || thread?.thread_id;
      const localState = threadStates instanceof Map ? threadStates.get(threadId) : null;
      if (!threadId || !isApprovalThread(thread, localState)) return [];
      const id = approvalThreadNotificationId(thread);
      if (!id) return [];
      const timestamp = threadTimestamp(thread);
      return [{
        id,
        type: "approval",
        icon: "shield",
        title: tx("notifications.approval.title"),
        body: thread.title || tx("notifications.approval.untitled"),
        detail: tx("notifications.approval.detail"),
        timeLabel: timestamp
          ? new Date(timestamp).toLocaleString([], {
              month: "short",
              day: "numeric",
              hour: "2-digit",
              minute: "2-digit",
            })
          : "",
        timestamp,
        href: `/chat/${encodeURIComponent(threadId)}`,
        threadId,
        read: false,
      }];
    })
    .sort((left, right) => right.timestamp - left.timestamp)
    .slice(0, MAX_MESSAGES);
}
