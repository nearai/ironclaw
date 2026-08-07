import {
  ActivityKind,
  MAX_INSPECTOR_ACTIVITY_ENTRIES,
  activityKindFromWire,
} from "./activity-kind";

export const INSPECTOR_RUN_HISTORY_KEY = "ironclaw:inspector-run-history";
export { MAX_INSPECTOR_ACTIVITY_ENTRIES } from "./activity-kind";
const MAX_INSPECTOR_RUNS_PER_THREAD = 32;

export interface BoundedDiagnosticText {
  content: string;
  original_bytes: number;
  truncated: boolean;
}

export interface InspectorActivityEvent {
  occurred_at: string;
  kind: ActivityKind;
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

interface InspectorActivitySortKey {
  sequenceAnchor: number | null;
  local: boolean;
  arrival: number;
}

function browserSessionStorage(): Storage | null {
  if (typeof window === "undefined") return null;
  try {
    return window.sessionStorage;
  } catch (_) {
    return null;
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
  const kind = activityKindFromWire(event.kind);
  if (typeof event.occurred_at !== "string" || !kind) return null;
  const summary = event.summary;
  return {
    occurred_at: event.occurred_at,
    kind,
    iteration: typeof event.iteration === "number" ? event.iteration : null,
    activity_id: typeof event.activity_id === "string" ? event.activity_id : null,
    model_call_id: typeof event.model_call_id === "string" ? event.model_call_id : null,
    summary: summary && typeof summary.content === "string" ? summary : null,
  };
}

function correlationKey(event: InspectorActivityEvent): string | null {
  if (event.model_call_id) return `model:${event.model_call_id}`;
  if (event.activity_id) return `tool:${event.activity_id}`;
  if (
    event.kind === ActivityKind.TurnStarted
    || event.kind === ActivityKind.FinalResponseCompleted
  ) return "turn";
  return null;
}

function isTerminalActivity(kind: ActivityKind): boolean {
  return kind === ActivityKind.ModelCallCompleted
    || kind === ActivityKind.ModelCallFailed
    || kind === ActivityKind.ToolCompleted
    || kind === ActivityKind.ToolFailed
    || kind === ActivityKind.FinalResponseCompleted;
}

function isStartedActivity(kind: ActivityKind): boolean {
  return kind === ActivityKind.ModelCallStarted
    || kind === ActivityKind.ToolStarted
    || kind === ActivityKind.TurnStarted;
}

function stableLifecycleKey(event: InspectorActivityEvent): string | null {
  if (event.model_call_id) return `${event.kind}:model:${event.model_call_id}`;
  if (event.activity_id) return `${event.kind}:tool:${event.activity_id}`;
  if (
    event.kind === ActivityKind.TurnStarted
    || event.kind === ActivityKind.FinalResponseCompleted
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
  const sortKeys = new Map<string, InspectorActivitySortKey>();
  const lifecycleRows = new Map<string, string>();
  let lastSeenSequence: number | null = null;
  let arrival = 0;
  const add = (key: string, sequence: number | null, rawEvent: unknown) => {
    const event = asActivityEvent(rawEvent);
    if (!event || rows.has(key)) return;
    const lifecycleKey = stableLifecycleKey(event);
    const existingKey = lifecycleKey ? lifecycleRows.get(lifecycleKey) : null;
    if (existingKey) {
      if (!existingKey.startsWith("local:") || key.startsWith("local:")) return;
      rows.delete(existingKey);
      sortKeys.delete(existingKey);
    }
    arrival += 1;
    rows.set(key, { ...event, key, sequence, pending: false });
    sortKeys.set(key, {
      sequenceAnchor: sequence ?? lastSeenSequence,
      local: sequence === null,
      arrival,
    });
    if (lifecycleKey) lifecycleRows.set(lifecycleKey, key);
  };

  const observeSequence = (sequence: number) => {
    if (lastSeenSequence === null || sequence > lastSeenSequence) {
      lastSeenSequence = sequence;
    }
  };

  if (Array.isArray(value?.activity)) {
    for (const rawEntry of value.activity) {
      const entry = rawEntry as InspectorActivityEntry;
      if (!Number.isSafeInteger(entry?.sequence)) continue;
      const sequence = entry.sequence as number;
      observeSequence(sequence);
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
    if (sequence !== null) observeSequence(sequence);
    add(localId ? `local:${localId}` : `${streamId}:${sequence}`, sequence, update.data);
  }

  const ordered = [...rows.values()].sort((left, right) => {
    const leftOrder = sortKeys.get(left.key);
    const rightOrder = sortKeys.get(right.key);
    if (!leftOrder || !rightOrder) return left.key.localeCompare(right.key);
    if (leftOrder.sequenceAnchor !== rightOrder.sequenceAnchor) {
      if (leftOrder.sequenceAnchor === null) return -1;
      if (rightOrder.sequenceAnchor === null) return 1;
      return leftOrder.sequenceAnchor < rightOrder.sequenceAnchor ? -1 : 1;
    }
    if (leftOrder.local !== rightOrder.local) return leftOrder.local ? 1 : -1;
    if (leftOrder.arrival !== rightOrder.arrival) {
      return leftOrder.arrival < rightOrder.arrival ? -1 : 1;
    }
    return left.key.localeCompare(right.key);
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
