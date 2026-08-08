import { authScope } from "../../../lib/auth-scope";
import { shouldAcceptInspectorCursor } from "./inspector-state";

const INSPECTOR_STREAM_SESSION_KEY_PREFIX = "ironclaw:inspector-stream-session";

/**
 * Per-caller storage key for browser-observed stream state.
 *
 * Resume cursors and the observed-update counters describe one operator's
 * runs, and a single tab supports bearer session changes without an explicit
 * sign-out. Namespacing by the resolved identity keeps a later caller from
 * resuming — or reporting — the previous caller's stream position.
 */
export function inspectorStreamSessionKey(): string {
  return `${INSPECTOR_STREAM_SESSION_KEY_PREFIX}:${authScope()}`;
}

const MAX_INSPECTOR_STREAM_SCOPES = 32;

export interface InspectorStreamMetrics {
  reconnectCount: number;
  receivedUpdateCount: number;
  lastUpdateAt: string | null;
}

interface InspectorStreamSession extends InspectorStreamMetrics {
  cursors: Record<string, string>;
  scopeOrder: string[];
}

function browserSessionStorage(): Storage | null {
  if (typeof window === "undefined") return null;
  try {
    return window.sessionStorage;
  } catch (_) {
    return null;
  }
}

function emptySession(): InspectorStreamSession {
  return {
    reconnectCount: 0,
    receivedUpdateCount: 0,
    lastUpdateAt: null,
    cursors: {},
    scopeOrder: [],
  };
}

function safeCount(value: unknown): number {
  return typeof value === "number"
    && Number.isSafeInteger(value)
    && value >= 0
    ? value
    : 0;
}

function loadSession(storage: Pick<Storage, "getItem"> | null): InspectorStreamSession {
  try {
    const parsed = JSON.parse(storage?.getItem(inspectorStreamSessionKey()) || "null");
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) return emptySession();
    const rawCursors = parsed.cursors && typeof parsed.cursors === "object"
      && !Array.isArray(parsed.cursors)
      ? parsed.cursors as Record<string, unknown>
      : {};
    const requestedOrder: string[] = Array.isArray(parsed.scopeOrder)
      ? (parsed.scopeOrder as unknown[]).filter((scope: unknown): scope is string => {
        if (typeof scope !== "string" || !scope) return false;
        return typeof rawCursors[scope] === "string";
      })
      : [];
    const scopeOrder = [...new Set(requestedOrder)].slice(-MAX_INSPECTOR_STREAM_SCOPES);
    return {
      reconnectCount: safeCount(parsed.reconnectCount),
      receivedUpdateCount: safeCount(parsed.receivedUpdateCount),
      lastUpdateAt: typeof parsed.lastUpdateAt === "string" ? parsed.lastUpdateAt : null,
      cursors: Object.fromEntries(
        scopeOrder.map((scope) => [scope, rawCursors[scope] as string]),
      ),
      scopeOrder,
    };
  } catch (_) {
    return emptySession();
  }
}

function saveSession(
  session: InspectorStreamSession,
  storage: Pick<Storage, "setItem"> | null,
): void {
  try {
    storage?.setItem(inspectorStreamSessionKey(), JSON.stringify(session));
  } catch (_) {
    // Browser-observed diagnostics are best effort and never affect chat.
  }
}

function metrics(session: InspectorStreamSession): InspectorStreamMetrics {
  return {
    reconnectCount: session.reconnectCount,
    receivedUpdateCount: session.receivedUpdateCount,
    lastUpdateAt: session.lastUpdateAt,
  };
}

function increment(value: number): number {
  return value >= Number.MAX_SAFE_INTEGER ? Number.MAX_SAFE_INTEGER : value + 1;
}

function retainCursor(
  session: InspectorStreamSession,
  scope: string,
  cursor: string,
): void {
  session.cursors[scope] = cursor;
  session.scopeOrder = [...session.scopeOrder.filter((value) => value !== scope), scope]
    .slice(-MAX_INSPECTOR_STREAM_SCOPES);
  const retainedScopes = new Set(session.scopeOrder);
  session.cursors = Object.fromEntries(
    Object.entries(session.cursors).filter(([key]) => retainedScopes.has(key)),
  );
}

export function readInspectorStreamMetrics(
  storage: Pick<Storage, "getItem"> | null = browserSessionStorage(),
): InspectorStreamMetrics {
  return metrics(loadSession(storage));
}

export function readInspectorStreamCursor(
  scope: string,
  storage: Pick<Storage, "getItem"> | null = browserSessionStorage(),
): string | null {
  return loadSession(storage).cursors[scope] || null;
}

export function recordInspectorReconnect(
  storage: Pick<Storage, "getItem" | "setItem"> | null = browserSessionStorage(),
): InspectorStreamMetrics {
  const session = loadSession(storage);
  session.reconnectCount = increment(session.reconnectCount);
  saveSession(session, storage);
  return metrics(session);
}

export function recordInspectorDiagnosticUpdate(
  scope: string,
  cursor: string | null,
  observedAt: string,
  storage: Pick<Storage, "getItem" | "setItem"> | null = browserSessionStorage(),
): { accepted: boolean; metrics: InspectorStreamMetrics } {
  const session = loadSession(storage);
  if (!scope || !shouldAcceptInspectorCursor(session.cursors[scope] || null, cursor)) {
    return { accepted: false, metrics: metrics(session) };
  }
  session.receivedUpdateCount = increment(session.receivedUpdateCount);
  session.lastUpdateAt = observedAt;
  retainCursor(session, scope, cursor as string);
  saveSession(session, storage);
  return { accepted: true, metrics: metrics(session) };
}

export function rememberInspectorStreamCursor(
  scope: string,
  cursor: string | null,
  storage: Pick<Storage, "getItem" | "setItem"> | null = browserSessionStorage(),
): void {
  const session = loadSession(storage);
  if (!scope || !shouldAcceptInspectorCursor(session.cursors[scope] || null, cursor)) return;
  retainCursor(session, scope, cursor as string);
  saveSession(session, storage);
}
