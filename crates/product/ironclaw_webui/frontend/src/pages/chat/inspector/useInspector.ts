import React from "react";
import { EventSourcePlus } from "event-source-plus";

import { clientActionId } from "../../../lib/api";
import { fetchInspectorSnapshot, inspectorEventStreamRequest } from "./inspector-api";
import { subscribeProductInspectorActivity } from "./product-activity";
import {
  INSPECTOR_HEALTH,
  healthForInspectorStatus,
  shouldAcceptInspectorCursor,
  type InspectorHealth,
} from "./inspector-state";

const MAX_RETRY_INTERVAL_MS = 30_000;
const MAX_RETAINED_UPDATES = 1_024;
const INSPECTOR_CONNECTION_STORAGE_KEY = "ironclaw:inspector-sse-connection";

interface InspectorConnectionState {
  connectionId: string;
  generation: number;
}

function newConnectionState(): InspectorConnectionState {
  return { connectionId: clientActionId(), generation: 0 };
}

function isDocumentReload(): boolean {
  try {
    const navigation = globalThis.performance?.getEntriesByType?.("navigation")[0];
    return Boolean(navigation && "type" in navigation && navigation.type === "reload");
  } catch (_) {
    return false;
  }
}

function loadConnectionState(): InspectorConnectionState {
  if (!isDocumentReload()) return newConnectionState();
  try {
    const raw = globalThis.sessionStorage?.getItem(INSPECTOR_CONNECTION_STORAGE_KEY);
    if (!raw) return newConnectionState();
    const candidate = JSON.parse(raw);
    if (
      typeof candidate?.connectionId === "string"
      && /^[A-Za-z0-9_-]{1,64}$/.test(candidate.connectionId)
      && typeof candidate?.generation === "number"
      && Number.isSafeInteger(candidate.generation)
      && candidate.generation >= 0
    ) {
      return candidate as InspectorConnectionState;
    }
  } catch (_) {
    // A fresh id remains safe when storage or navigation metadata is unavailable.
  }
  return newConnectionState();
}

const inspectorConnectionState = loadConnectionState();

function nextConnectionState(): InspectorConnectionState {
  if (inspectorConnectionState.generation >= Number.MAX_SAFE_INTEGER) {
    inspectorConnectionState.connectionId = clientActionId();
    inspectorConnectionState.generation = 0;
  }
  inspectorConnectionState.generation += 1;
  try {
    globalThis.sessionStorage?.setItem(
      INSPECTOR_CONNECTION_STORAGE_KEY,
      JSON.stringify(inspectorConnectionState),
    );
  } catch (_) {
    // Best effort. The in-memory generation still orders this document's requests.
  }
  return { ...inspectorConnectionState };
}

export interface DiagnosticUpdate {
  stream_id?: string;
  sequence?: number;
  local_id?: string;
  update?: unknown;
  [key: string]: unknown;
}

interface InspectorSnapshotResponse {
  snapshot?: {
    prompt?: unknown;
    activity?: unknown[];
    stats?: Record<string, unknown>;
    [key: string]: unknown;
  } | null;
}

interface InspectorState {
  snapshot: InspectorSnapshotResponse["snapshot"];
  updates: DiagnosticUpdate[];
  health: InspectorHealth;
  error: string | null;
  lastCursor: string | null;
}

function safeJson(value: string): Record<string, unknown> | null {
  try {
    const parsed = JSON.parse(value);
    return parsed && typeof parsed === "object" ? parsed : null;
  } catch (_) {
    return null;
  }
}

function errorStatus(error: unknown): number {
  const status = (error as { status?: unknown } | null)?.status;
  return typeof status === "number" ? status : 0;
}

export function useInspector({
  enabled,
  threadId,
  runId,
}: {
  enabled: boolean;
  threadId: string | null;
  runId: string | null;
}): InspectorState {
  const [snapshot, setSnapshot] = React.useState<InspectorState["snapshot"]>(null);
  const [updates, setUpdates] = React.useState<DiagnosticUpdate[]>([]);
  const [health, setHealth] = React.useState<InspectorHealth>(INSPECTOR_HEALTH.IDLE);
  const [error, setError] = React.useState<string | null>(null);
  const [snapshotGeneration, setSnapshotGeneration] = React.useState(0);
  const lastCursorRef = React.useRef<string | null>(null);
  const transportSequenceRef = React.useRef(0);

  React.useEffect(() => {
    lastCursorRef.current = null;
    setSnapshot(null);
    setUpdates([]);
    setError(null);
    transportSequenceRef.current = 0;
  }, [enabled, threadId, runId]);

  React.useEffect(() => {
    if (!enabled || !threadId || !runId) return undefined;
    return subscribeProductInspectorActivity(threadId, runId, (activity) => {
      setUpdates((current) => [...current, {
        local_id: activity.localId,
        update: {
          type: "activity",
          data: {
            occurred_at: activity.occurredAt,
            kind: activity.kind,
            iteration: null,
            activity_id: activity.activityId,
            model_call_id: null,
            summary: {
              content: activity.summary,
              original_bytes: activity.summary.length,
              truncated: false,
            },
          },
        },
      }].slice(-MAX_RETAINED_UPDATES));
    });
  }, [enabled, threadId, runId]);

  React.useEffect(() => {
    if (!enabled || !threadId || !runId) {
      setHealth(INSPECTOR_HEALTH.IDLE);
      return undefined;
    }

    const controller = new AbortController();
    setHealth(INSPECTOR_HEALTH.LOADING);
    fetchInspectorSnapshot({ threadId, runId, signal: controller.signal })
      .then((response) => {
        if (controller.signal.aborted) return;
        setSnapshot((response as InspectorSnapshotResponse)?.snapshot ?? null);
        setError(null);
        setHealth((current) =>
          current === INSPECTOR_HEALTH.LOADING ? INSPECTOR_HEALTH.CONNECTING : current,
        );
      })
      .catch((cause) => {
        if (controller.signal.aborted) return;
        const nextHealth = healthForInspectorStatus(errorStatus(cause));
        setHealth(nextHealth);
        setError(nextHealth === INSPECTOR_HEALTH.FORBIDDEN
          ? "This session is not authorized to inspect diagnostics."
          : "Diagnostics are currently unavailable.");
      });
    return () => controller.abort();
  }, [enabled, threadId, runId, snapshotGeneration]);

  React.useEffect(() => {
    if (!enabled || !threadId || !runId) return undefined;

    let disposed = false;
    let connectedOnce = false;
    let transportDisconnected = false;
    let controller: ReturnType<EventSourcePlus["listen"]> | null = null;
    const request = inspectorEventStreamRequest({ threadId, runId });
    const stream = new EventSourcePlus(request.url, {
      credentials: "same-origin",
      headers: request.headers,
      maxRetryInterval: MAX_RETRY_INTERVAL_MS,
      retryStrategy: "always",
    });

    function terminal(healthState: InspectorHealth, message: string): void {
      setHealth(healthState);
      setError(message);
      controller?.abort("terminal inspector response");
    }

    function appendTransportActivity(kind: "stream_disconnected" | "stream_resumed"): void {
      transportSequenceRef.current += 1;
      const summary = kind === "stream_disconnected"
        ? "Diagnostics stream disconnected"
        : "Diagnostics stream resumed";
      setUpdates((current) => [...current, {
        local_id: `transport-${transportSequenceRef.current}`,
        update: {
          type: "activity",
          data: {
            occurred_at: new Date().toISOString(),
            kind,
            iteration: null,
            activity_id: null,
            model_call_id: null,
            summary: {
              content: summary,
              original_bytes: summary.length,
              truncated: false,
            },
          },
        },
      }].slice(-MAX_RETAINED_UPDATES));
    }

    function noteDisconnected(): void {
      if (transportDisconnected) return;
      transportDisconnected = true;
      appendTransportActivity("stream_disconnected");
    }

    function connect(): void {
      if (disposed) return;
      setHealth(connectedOnce ? INSPECTOR_HEALTH.RECONNECTING : INSPECTOR_HEALTH.CONNECTING);
      controller = stream.listen({
        onRequest({ options }) {
          if (!disposed) {
            const connection = nextConnectionState();
            options.query = {
              ...options.query,
              connection_id: connection.connectionId,
              connection_generation: connection.generation,
            };
            setHealth(
              connectedOnce ? INSPECTOR_HEALTH.RECONNECTING : INSPECTOR_HEALTH.CONNECTING,
            );
          }
        },
        onRequestError() {
          if (!disposed) {
            noteDisconnected();
            setHealth(INSPECTOR_HEALTH.RECONNECTING);
          }
        },
        onResponse({ response }) {
          if (disposed) return;
          if (response.ok && response.headers.get("content-type")?.includes("text/event-stream")) {
            if (transportDisconnected) appendTransportActivity("stream_resumed");
            transportDisconnected = false;
            connectedOnce = true;
            setError(null);
            setHealth(INSPECTOR_HEALTH.CONNECTED);
          }
        },
        onResponseError({ response }) {
          if (disposed) return;
          const nextHealth = healthForInspectorStatus(response.status);
          if (nextHealth === INSPECTOR_HEALTH.FORBIDDEN) {
            terminal(nextHealth, "This session is not authorized to inspect diagnostics.");
          } else if (nextHealth === INSPECTOR_HEALTH.UNAVAILABLE) {
            terminal(nextHealth, "Diagnostics are not available on this deployment.");
          } else {
            setHealth(nextHealth);
          }
        },
        onMessage(message) {
          if (disposed) return;
          const payload = safeJson(message.data);
          if (!payload) return;
          if (message.event === "diagnostic_connected") {
            connectedOnce = true;
            transportDisconnected = false;
            setError(null);
            setHealth(INSPECTOR_HEALTH.CONNECTED);
            return;
          }
          if (message.event === "stream_error") {
            noteDisconnected();
            const nextHealth = payload.retryable === false
              ? INSPECTOR_HEALTH.DISCONNECTED
              : INSPECTOR_HEALTH.RECONNECTING;
            setHealth(nextHealth);
            if (payload.retryable === false) {
              terminal(nextHealth, "The diagnostics stream was closed.");
            }
            return;
          }
          const cursor = message.id || null;
          if (message.event === "diagnostic_rebase") {
            if (cursor) lastCursorRef.current = cursor;
            setUpdates([]);
            setSnapshotGeneration((generation) => generation + 1);
            return;
          }
          if (message.event !== "diagnostic_update") return;
          if (!shouldAcceptInspectorCursor(lastCursorRef.current, cursor)) return;
          lastCursorRef.current = cursor;
          setUpdates((current) => [...current, payload as DiagnosticUpdate].slice(-MAX_RETAINED_UPDATES));
          const update = payload.update as { type?: unknown } | undefined;
          if (
            update?.type === "prompt_updated"
            || update?.type === "model_call"
            || update?.type === "tool_execution_updated"
            || update?.type === "stats"
          ) {
            setSnapshotGeneration((generation) => generation + 1);
          }
          setHealth(INSPECTOR_HEALTH.CONNECTED);
        },
      });
    }

    function onVisibilityChange(): void {
      if (disposed) return;
      if (document.visibilityState === "hidden") {
        noteDisconnected();
        controller?.abort("inspector hidden");
        setHealth(INSPECTOR_HEALTH.IDLE);
      } else {
        setHealth(INSPECTOR_HEALTH.RECONNECTING);
        controller?.reconnect();
      }
    }

    if (document.visibilityState !== "hidden") connect();
    document.addEventListener("visibilitychange", onVisibilityChange);
    return () => {
      disposed = true;
      document.removeEventListener("visibilitychange", onVisibilityChange);
      controller?.abort("inspector disposed");
    };
  }, [enabled, threadId, runId]);

  return {
    snapshot,
    updates,
    health,
    error,
    lastCursor: lastCursorRef.current,
  };
}
