export function addPending(store, key, record) {
  const existing = store.get(key) || [];
  store.set(key, [...existing, record]);
}

export function removePending(store, key, pendingId) {
  const next = (store.get(key) || []).filter((r) => r.id !== pendingId);
  if (next.length > 0) store.set(key, next);
  else store.delete(key);
}

export function recordAcceptedMessageRef(store, key, pendingId, acceptedMessageRef) {
  const timelineMessageId = timelineMessageIdFromAcceptedRef(acceptedMessageRef);
  if (!timelineMessageId) return null;

  updatePending(store, key, pendingId, { timelineMessageId });
  return timelineMessageId;
}

function updatePending(store, key, pendingId, patch) {
  const existing = store.get(key) || [];
  const next = existing.map((record) =>
    record.id === pendingId ? { ...record, ...patch } : record,
  );
  if (next.length > 0) store.set(key, next);
}

function timelineMessageIdFromAcceptedRef(ref) {
  if (typeof ref !== "string") return null;
  return ref.startsWith("msg:") ? ref.slice("msg:".length) : null;
}
