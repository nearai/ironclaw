export function inspectorDebugEnabled(search = ""): boolean {
  try {
    return new URLSearchParams(search).get("debug") === "true";
  } catch (_) {
    return false;
  }
}

export function latestInspectorRunId(activeRun: unknown, messages: unknown[]): string | null {
  const current = activeRun as { runId?: unknown } | null;
  if (typeof current?.runId === "string" && current.runId) return current.runId;
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    const message = messages[index] as { turnRunId?: unknown } | null;
    if (typeof message?.turnRunId === "string" && message.turnRunId) {
      return message.turnRunId;
    }
  }
  return null;
}
