export function normalizeLogEntry(entry) {
  return {
    id: String(entry?.id ?? `${entry?.timestamp}:${entry?.target}:${entry?.message}`),
    timestamp: entry?.timestamp || "",
    level: String(entry?.level || "info").toLowerCase(),
    target: entry?.target || "",
    message: entry?.message || "",
    threadId: entry?.thread_id || null,
    runId: entry?.run_id || null,
    turnId: entry?.turn_id || null,
    toolCallId: entry?.tool_call_id || null,
    toolName: entry?.tool_name || null,
    source: entry?.source || null,
  };
}

// Builds a basename-RELATIVE logs path (e.g. "/logs?thread_id=..."). Callers
// pass the result to a react-router `<Link to>` / `navigate()`, which prepends
// the router basename ("/v2") on its own — so this must never include "/v2",
// otherwise the resolved href doubles to "/v2/v2/logs".
export function buildScopedLogsPath(
  { threadId, runId, turnId, toolCallId, toolName, source } = {},
) {
  const params = new URLSearchParams();
  if (threadId) params.set("thread_id", threadId);
  if (runId) params.set("run_id", runId);
  if (turnId) params.set("turn_id", turnId);
  if (toolCallId) params.set("tool_call_id", toolCallId);
  if (toolName) params.set("tool_name", toolName);
  if (source) params.set("source", source);
  const suffix = params.toString();
  return `/logs${suffix ? `?${suffix}` : ""}`;
}

export function normalizeOperatorLogsResponse(response) {
  const payload =
    response?.logs && typeof response.logs === "object" ? response.logs : response || {};
  const entries = Array.isArray(payload.entries) ? payload.entries : [];
  return {
    source: payload.source || "",
    entries: entries.map(normalizeLogEntry),
    nextCursor: payload.next_cursor || null,
    tailSupported: Boolean(payload.tail_supported),
    followSupported: Boolean(payload.follow_supported),
  };
}
