const STORAGE_PREFIX = "ironclaw:chat:text-before-activity:";
const MAX_RUNS_PER_THREAD = 100;

function storage() {
  try {
    return globalThis.sessionStorage || null;
  } catch (_) {
    return null;
  }
}

function storageKey(threadId) {
  return threadId ? `${STORAGE_PREFIX}${threadId}` : null;
}

export function rememberTextBeforeActivity(threadId, runId) {
  if (!threadId || !runId) return;
  const store = storage();
  const key = storageKey(threadId);
  if (!store || !key) return;
  const remembered = rememberedRuns(store, key);
  remembered.delete(runId);
  remembered.add(runId);
  const values = Array.from(remembered).slice(-MAX_RUNS_PER_THREAD);
  try {
    store.setItem(key, JSON.stringify(values));
  } catch (_) {
    // Best-effort UI hint only. Timeline remains the source of truth.
  }
}

export function hasRememberedTextBeforeActivity(threadId, runId) {
  if (!threadId || !runId) return false;
  const store = storage();
  const key = storageKey(threadId);
  if (!store || !key) return false;
  return rememberedRuns(store, key).has(runId);
}

function rememberedRuns(store, key) {
  try {
    const parsed = JSON.parse(store.getItem(key) || "[]");
    return new Set(Array.isArray(parsed) ? parsed.filter(isNonEmptyString) : []);
  } catch (_) {
    return new Set();
  }
}

function isNonEmptyString(value) {
  return typeof value === "string" && value.length > 0;
}
