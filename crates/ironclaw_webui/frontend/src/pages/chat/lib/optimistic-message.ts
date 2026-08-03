// Build the optimistic user message rendered immediately on send, before the
// server confirms it on the timeline. The pending-ref record and the
// in-state render message are the same shape; this is the single source so
// they never drift (e.g. an attachment card showing live but not after the
// confirmed row lands).
// `extra` is spread FIRST so the canonical fields below always win: a caller
// passing side metadata (retry payloads) can never silently reshape the
// message identity this function exists to be the single source of.
export function buildOptimisticMessage({ id, content, attachments = [], extra = {} }) {
  return {
    ...extra,
    id,
    role: "user",
    content,
    attachments,
    timestamp: new Date().toISOString(),
    isOptimistic: true,
  };
}
