import { ActivityKind, MAX_INSPECTOR_ACTIVITY_ENTRIES } from "./activity-kind";
import { inspectorDebugEnabled } from "./inspector-shell";

export interface ProductInspectorActivity {
  localId: string;
  threadId: string;
  runId: string;
  occurredAt: string;
  kind: ActivityKind;
  activityId: string | null;
  summaryKey: string;
}

export interface PublishProductInspectorActivity {
  threadId: unknown;
  runId: unknown;
  kind: ActivityKind;
  activityId?: unknown;
  summaryKey: string;
  dedupeKey: string;
}

type ProductActivityListener = (activity: ProductInspectorActivity) => void;

const MAX_PRODUCT_ACTIVITY_RUNS = 32;
// Product events are provisional UI hints. The diagnostic stream is
// authoritative and replaces matching hints in reduceInspectorActivity.
// Keep the hint buffer at the final reducer's shared ceiling so the local
// source cannot introduce a larger competing retention contract.
const MAX_PRODUCT_ACTIVITY_PER_RUN = MAX_INSPECTOR_ACTIVITY_ENTRIES;
const retained = new Map<string, ProductInspectorActivity[]>();
const dedupe = new Map<string, Set<string>>();
const listeners = new Map<string, Set<ProductActivityListener>>();
let nextLocalId = 0;

function isInspectorActivityEnabled(): boolean {
  return typeof window !== "undefined"
    && inspectorDebugEnabled(window.location.search);
}

function scopeKey(threadId: string, runId: string): string {
  return `${threadId}\n${runId}`;
}

function touchScope(key: string): void {
  const entries = retained.get(key);
  const keys = dedupe.get(key);
  if (entries) {
    retained.delete(key);
    retained.set(key, entries);
  }
  if (keys) {
    dedupe.delete(key);
    dedupe.set(key, keys);
  }
  while (retained.size > MAX_PRODUCT_ACTIVITY_RUNS) {
    const oldest = retained.keys().next().value as string | undefined;
    if (!oldest) break;
    retained.delete(oldest);
    dedupe.delete(oldest);
  }
}

export function publishProductInspectorActivity(
  input: PublishProductInspectorActivity,
): void {
  if (!isInspectorActivityEnabled()) return;
  if (typeof input.threadId !== "string" || !input.threadId) return;
  if (typeof input.runId !== "string" || !input.runId) return;
  const key = scopeKey(input.threadId, input.runId);
  const known = dedupe.get(key) || new Set<string>();
  if (known.has(input.dedupeKey)) return;
  known.add(input.dedupeKey);
  dedupe.set(key, known);
  nextLocalId += 1;
  const activity: ProductInspectorActivity = {
    localId: `product-${nextLocalId}`,
    threadId: input.threadId,
    runId: input.runId,
    occurredAt: new Date().toISOString(),
    kind: input.kind,
    activityId: typeof input.activityId === "string" ? input.activityId : null,
    summaryKey: input.summaryKey,
  };
  const entries = [...(retained.get(key) || []), activity].slice(-MAX_PRODUCT_ACTIVITY_PER_RUN);
  retained.set(key, entries);
  touchScope(key);
  for (const listener of listeners.get(key) || []) listener(activity);
}

export function subscribeProductInspectorActivity(
  threadId: string,
  runId: string,
  listener: ProductActivityListener,
): () => void {
  const key = scopeKey(threadId, runId);
  const scopedListeners = listeners.get(key) || new Set<ProductActivityListener>();
  scopedListeners.add(listener);
  listeners.set(key, scopedListeners);
  for (const activity of retained.get(key) || []) listener(activity);
  return () => {
    const current = listeners.get(key);
    current?.delete(listener);
    if (current?.size === 0) listeners.delete(key);
  };
}
