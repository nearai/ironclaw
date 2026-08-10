export const INSPECTOR_DEBUG_ENABLED_KEY = "ironclaw_debug";

function browserSessionStorage(): Storage | null {
  if (typeof window === "undefined") return null;
  try {
    return window.sessionStorage;
  } catch (_) {
    return null;
  }
}

function inspectorDebugQueryValue(search: string): boolean | null {
  try {
    const params = new URLSearchParams(search);
    if (!params.has("debug")) return null;
    return params.get("debug") === "true";
  } catch (_) {
    return null;
  }
}

export function inspectorDebugEnabled(
  search = "",
  storage: Pick<Storage, "getItem"> | null = browserSessionStorage(),
): boolean {
  const queryValue = inspectorDebugQueryValue(search);
  if (queryValue !== null) return queryValue;
  try {
    return storage?.getItem(INSPECTOR_DEBUG_ENABLED_KEY) === "true";
  } catch (_) {
    return false;
  }
}

export function persistInspectorDebugPreference(
  search: string,
  storage: Pick<Storage, "setItem"> | null = browserSessionStorage(),
): void {
  const queryValue = inspectorDebugQueryValue(search);
  if (queryValue === null) return;
  try {
    storage?.setItem(INSPECTOR_DEBUG_ENABLED_KEY, String(queryValue));
  } catch (_) {
    // Debug UI preferences are best effort and must never affect chat.
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
