import { authScope } from "./auth-scope.js";

const STORAGE_PREFIX = "ironclaw:v2-notifications:";
const MAX_SEEN_IDS = 250;
const MAX_MESSAGES = 30;

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

export function ensureNotificationBaseline(messageIds = [], scope) {
  ensureScope(scope);
  if (!state.initialized) {
    state.initialized = true;
    for (const id of messageIds) {
      if (id) state.seenIds.add(id);
    }
    writePersisted(scope);
    emit(scope);
  }
  return snapshot(scope);
}

export function markNotificationIdsSeen(messageIds = [], scope) {
  ensureScope(scope);
  state.initialized = true;
  for (const id of messageIds) {
    if (id) state.seenIds.add(id);
  }
  writePersisted(scope);
  emit(scope);
  return snapshot(scope);
}

export function automationRunNotificationId(automation, run) {
  const automationId = automation?.automation_id || "unknown";
  const runKey =
    run?.run_id ||
    run?.thread_id ||
    run?.timestamp_source ||
    run?.submitted_at ||
    run?.fired_at ||
    run?.fire_slot;
  if (!runKey) return null;
  return `automation:${automationId}:run:${runKey}`;
}

export function automationRunNotifications(automations = [], t = (key) => key) {
  const tx = typeof t === "function" ? t : (key) => key;
  const messages = [];

  for (const automation of Array.isArray(automations) ? automations : []) {
    for (const run of Array.isArray(automation?.recent_runs) ? automation.recent_runs : []) {
      const id = automationRunNotificationId(automation, run);
      if (!id) continue;
      const timestamp = Number.isFinite(run.timestamp) ? run.timestamp : 0;
      messages.push({
        id,
        type: "automation",
        icon: "calendar",
        title: tx("notifications.automation.title"),
        body: automation.display_name || automation.name || tx("automations.untitled"),
        detail: run.status_label || null,
        timeLabel: run.fired_label || run.submitted_label || "",
        timestamp,
        href: run.chat_path || null,
      });
    }
  }

  return messages
    .sort((a, b) => b.timestamp - a.timestamp)
    .slice(0, MAX_MESSAGES);
}
