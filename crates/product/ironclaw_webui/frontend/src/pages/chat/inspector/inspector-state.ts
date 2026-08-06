export const INSPECTOR_TABS = ["prompt", "activity", "stats"] as const;
export type InspectorTab = (typeof INSPECTOR_TABS)[number];

export const INSPECTOR_HEALTH = {
  IDLE: "idle",
  LOADING: "loading",
  CONNECTING: "connecting",
  CONNECTED: "connected",
  RECONNECTING: "reconnecting",
  DISCONNECTED: "disconnected",
  FORBIDDEN: "forbidden",
  UNAVAILABLE: "unavailable",
} as const;

export type InspectorHealth =
  (typeof INSPECTOR_HEALTH)[keyof typeof INSPECTOR_HEALTH];

export const INSPECTOR_PREFERENCES_KEY = "ironclaw:inspector-preferences";
export const INSPECTOR_RUN_HISTORY_KEY = "ironclaw:inspector-run-history";
export const MAX_INSPECTOR_ACTIVITY_ENTRIES = 1_000;
const MAX_INSPECTOR_RUNS_PER_THREAD = 32;

export interface InspectorPreferences {
  open: boolean;
  activeTab: InspectorTab;
}

export interface BoundedDiagnosticText {
  content: string;
  original_bytes: number;
  truncated: boolean;
}

export interface InspectorActivityEvent {
  occurred_at: string;
  kind: string;
  iteration: number | null;
  activity_id: string | null;
  model_call_id: string | null;
  summary: BoundedDiagnosticText | null;
}

export interface InspectorActivityRow extends InspectorActivityEvent {
  key: string;
  sequence: number | null;
  pending: boolean;
}

interface InspectorActivityEntry {
  sequence?: unknown;
  event?: unknown;
}

interface InspectorActivityUpdate {
  stream_id?: unknown;
  sequence?: unknown;
  local_id?: unknown;
  update?: unknown;
}

const DEFAULT_PREFERENCES: InspectorPreferences = {
  open: true,
  activeTab: "prompt",
};

function browserSessionStorage(): Storage | null {
  if (typeof window === "undefined") return null;
  try {
    return window.sessionStorage;
  } catch (_) {
    return null;
  }
}

export function inspectorDebugEnabled(search = ""): boolean {
  try {
    return new URLSearchParams(search).get("debug") === "true";
  } catch (_) {
    return false;
  }
}

export function inspectorViewportMode(width: number): "mobile" | "overlay" | "sidebar" {
  if (!Number.isFinite(width) || width < 640) return "mobile";
  return width < 1280 ? "overlay" : "sidebar";
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

export function readInspectorPreferences(
  storage: Pick<Storage, "getItem"> | null = browserSessionStorage(),
): InspectorPreferences {
  try {
    const raw = storage?.getItem(INSPECTOR_PREFERENCES_KEY);
    if (!raw) return { ...DEFAULT_PREFERENCES };
    const parsed = JSON.parse(raw);
    const activeTab = INSPECTOR_TABS.includes(parsed?.activeTab)
      ? parsed.activeTab
      : DEFAULT_PREFERENCES.activeTab;
    return {
      open: typeof parsed?.open === "boolean" ? parsed.open : DEFAULT_PREFERENCES.open,
      activeTab,
    };
  } catch (_) {
    return { ...DEFAULT_PREFERENCES };
  }
}

export function writeInspectorPreferences(
  preferences: InspectorPreferences,
  storage: Pick<Storage, "setItem"> | null = browserSessionStorage(),
): void {
  try {
    storage?.setItem(INSPECTOR_PREFERENCES_KEY, JSON.stringify(preferences));
  } catch (_) {
    // Debug UI preferences are best effort and must never affect chat.
  }
}

function validRunHistory(value: unknown): Record<string, string[]> {
  if (!value || typeof value !== "object" || Array.isArray(value)) return {};
  const result: Record<string, string[]> = {};
  for (const [threadId, runIds] of Object.entries(value)) {
    if (!threadId || !Array.isArray(runIds)) continue;
    const valid = runIds.filter(
      (runId, index): runId is string =>
        typeof runId === "string" && Boolean(runId) && runIds.indexOf(runId) === index,
    );
    if (valid.length > 0) result[threadId] = valid.slice(-MAX_INSPECTOR_RUNS_PER_THREAD);
  }
  return result;
}

export function rememberInspectorRun(
  threadId: string | null,
  runId: string | null,
  storage: Pick<Storage, "getItem" | "setItem"> | null = browserSessionStorage(),
): string[] {
  if (!threadId) return [];
  try {
    const history = validRunHistory(JSON.parse(storage?.getItem(INSPECTOR_RUN_HISTORY_KEY) || "{}"));
    const current = history[threadId] || [];
    if (runId) history[threadId] = [...current.filter((value) => value !== runId), runId]
      .slice(-MAX_INSPECTOR_RUNS_PER_THREAD);
    storage?.setItem(INSPECTOR_RUN_HISTORY_KEY, JSON.stringify(history));
    return history[threadId] || [];
  } catch (_) {
    return runId ? [runId] : [];
  }
}

function asActivityEvent(value: unknown): InspectorActivityEvent | null {
  if (!value || typeof value !== "object") return null;
  const event = value as Partial<InspectorActivityEvent>;
  if (typeof event.occurred_at !== "string" || typeof event.kind !== "string") return null;
  const summary = event.summary;
  return {
    occurred_at: event.occurred_at,
    kind: event.kind,
    iteration: typeof event.iteration === "number" ? event.iteration : null,
    activity_id: typeof event.activity_id === "string" ? event.activity_id : null,
    model_call_id: typeof event.model_call_id === "string" ? event.model_call_id : null,
    summary: summary && typeof summary.content === "string" ? summary : null,
  };
}

function correlationKey(event: InspectorActivityEvent): string | null {
  if (event.model_call_id) return `model:${event.model_call_id}`;
  if (event.activity_id) return `tool:${event.activity_id}`;
  if (event.kind === "turn_started" || event.kind === "final_response_completed") return "turn";
  return null;
}

function isTerminalActivity(kind: string): boolean {
  return kind === "model_call_completed"
    || kind === "model_call_failed"
    || kind === "tool_completed"
    || kind === "tool_failed"
    || kind === "final_response_completed";
}

function isStartedActivity(kind: string): boolean {
  return kind === "model_call_started" || kind === "tool_started" || kind === "turn_started";
}

function stableLifecycleKey(event: InspectorActivityEvent): string | null {
  if (event.model_call_id) return `${event.kind}:model:${event.model_call_id}`;
  if (event.activity_id) return `${event.kind}:tool:${event.activity_id}`;
  if (
    event.kind === "turn_started"
    || event.kind === "prompt_prepared"
    || event.kind === "final_response_completed"
  ) {
    return event.kind;
  }
  return null;
}

export function reduceInspectorActivity(
  snapshot: unknown,
  updates: InspectorActivityUpdate[],
): InspectorActivityRow[] {
  const value = snapshot && typeof snapshot === "object"
    ? snapshot as { stream_id?: unknown; activity?: unknown }
    : null;
  const snapshotStream = typeof value?.stream_id === "string" ? value.stream_id : "snapshot";
  const rows = new Map<string, InspectorActivityRow>();
  const lifecycleRows = new Map<string, string>();
  const add = (key: string, sequence: number | null, rawEvent: unknown) => {
    const event = asActivityEvent(rawEvent);
    if (!event || rows.has(key)) return;
    const lifecycleKey = stableLifecycleKey(event);
    const existingKey = lifecycleKey ? lifecycleRows.get(lifecycleKey) : null;
    if (existingKey) {
      if (!existingKey.startsWith("local:") || key.startsWith("local:")) return;
      rows.delete(existingKey);
    }
    rows.set(key, { ...event, key, sequence, pending: false });
    if (lifecycleKey) lifecycleRows.set(lifecycleKey, key);
  };

  if (Array.isArray(value?.activity)) {
    for (const rawEntry of value.activity) {
      const entry = rawEntry as InspectorActivityEntry;
      if (!Number.isSafeInteger(entry?.sequence)) continue;
      const sequence = entry.sequence as number;
      add(`${snapshotStream}:${sequence}`, sequence, entry.event);
    }
  }
  for (const envelope of updates) {
    const update = envelope?.update as { type?: unknown; data?: unknown } | undefined;
    if (update?.type !== "activity") continue;
    const streamId = typeof envelope.stream_id === "string" ? envelope.stream_id : snapshotStream;
    const sequence = Number.isSafeInteger(envelope.sequence) ? envelope.sequence as number : null;
    const localId = typeof envelope.local_id === "string" ? envelope.local_id : null;
    if (sequence === null && !localId) continue;
    add(localId ? `local:${localId}` : `${streamId}:${sequence}`, sequence, update.data);
  }

  const ordered = [...rows.values()].sort((left, right) => {
    if (left.sequence !== null && right.sequence !== null) return left.sequence - right.sequence;
    const time = Date.parse(left.occurred_at) - Date.parse(right.occurred_at);
    return Number.isNaN(time) ? left.key.localeCompare(right.key) : time;
  }).slice(-MAX_INSPECTOR_ACTIVITY_ENTRIES);
  const completed = new Set<string>();
  for (const row of ordered) {
    const key = correlationKey(row);
    if (key && isTerminalActivity(row.kind)) completed.add(key);
  }
  return ordered.map((row) => ({
    ...row,
    pending: isStartedActivity(row.kind) && !completed.has(correlationKey(row) || ""),
  }));
}

interface ParsedCursor {
  streamId: string;
  sequence: number;
}

function parseCursor(value: string | null | undefined): ParsedCursor | null {
  if (!value) return null;
  const separator = value.lastIndexOf(":");
  if (separator <= 0) return null;
  const streamId = value.slice(0, separator);
  const sequence = Number(value.slice(separator + 1));
  if (!streamId || !Number.isSafeInteger(sequence) || sequence < 0) return null;
  return { streamId, sequence };
}

export function shouldAcceptInspectorCursor(
  previous: string | null,
  candidate: string | null,
): boolean {
  const next = parseCursor(candidate);
  if (!next) return false;
  const current = parseCursor(previous);
  if (!current || current.streamId !== next.streamId) return true;
  return next.sequence > current.sequence;
}

export function healthForInspectorStatus(status: number): InspectorHealth {
  if (status === 401 || status === 403) return INSPECTOR_HEALTH.FORBIDDEN;
  if (status === 404 || status === 405 || status === 501) {
    return INSPECTOR_HEALTH.UNAVAILABLE;
  }
  if (status === 408 || status === 425 || status === 429 || status >= 500) {
    return INSPECTOR_HEALTH.RECONNECTING;
  }
  return INSPECTOR_HEALTH.DISCONNECTED;
}
