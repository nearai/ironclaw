export function normalizeLogEntry(entry) {
  return {
    id: String(entry?.id ?? `${entry?.timestamp}:${entry?.target}:${entry?.message}`),
    timestamp: entry?.timestamp || "",
    level: String(entry?.level || "info").toLowerCase(),
    target: entry?.target || "",
    message: entry?.message || "",
  };
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
