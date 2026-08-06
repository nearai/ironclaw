import React from "react";
import { EventSourcePlus } from "event-source-plus";
import { clientActionId, eventStreamRequest } from "../../../lib/api";
import {
  CONNECTION_STATUS,
  type ConnectionStatus,
} from "../lib/connection-status";

const ACTIVE_STREAM_STALL_DEADLINE_MS = 30_000;
const SSE_CONNECTION_STORAGE_KEY = "ironclaw:v2-sse-connection";

function newConnectionState() {
  return { connectionId: clientActionId(), generation: 0 };
}

function isDocumentReload() {
  try {
    const navigation =
      globalThis.performance?.getEntriesByType?.("navigation")[0];
    return Boolean(
      navigation && "type" in navigation && navigation.type === "reload",
    );
  } catch (_) {
    return false;
  }
}

function loadConnectionState() {
  // sessionStorage can be copied into a newly opened or duplicated tab. Only
  // an actual reload may reuse the predecessor document's stream identity;
  // every fresh top-level navigation must get an independent server slot.
  if (!isDocumentReload()) return newConnectionState();
  try {
    const raw = globalThis.sessionStorage?.getItem(SSE_CONNECTION_STORAGE_KEY);
    if (!raw) return newConnectionState();
    const candidate = JSON.parse(raw);
    const validConnectionId =
      typeof candidate?.connectionId === "string" &&
      /^[A-Za-z0-9_-]{1,64}$/.test(candidate.connectionId);
    const validGeneration =
      typeof candidate?.generation === "number" &&
      Number.isSafeInteger(candidate.generation) &&
      candidate.generation >= 0;
    if (validConnectionId && validGeneration) {
      return {
        connectionId: candidate.connectionId,
        generation: candidate.generation,
      };
    }
  } catch (_) {
    // Storage may be unavailable or contain stale data. A fresh identity still
    // gives this document a usable stream; the server's max lifetime bounds
    // any proxy-held stream that cannot be superseded.
  }
  return newConnectionState();
}

const sseConnectionState = loadConnectionState();

function nextConnectionState() {
  if (sseConnectionState.generation >= Number.MAX_SAFE_INTEGER) {
    sseConnectionState.connectionId = clientActionId();
    sseConnectionState.generation = 0;
  }
  sseConnectionState.generation += 1;
  try {
    globalThis.sessionStorage?.setItem(
      SSE_CONNECTION_STORAGE_KEY,
      JSON.stringify(sseConnectionState),
    );
  } catch (_) {
    // Best effort. In-memory state still orders this document's reconnects.
  }
  return { ...sseConnectionState };
}

function isBrowserOffline() {
  return typeof navigator !== "undefined" && navigator.onLine === false;
}

function isRetryableResponseStatus(status) {
  return (
    status === 204 ||
    status === 408 ||
    status === 425 ||
    status === 429 ||
    status >= 500
  );
}

export function useSSE({
  threadId,
  onEvent,
  enabled,
  activityExpected = false,
}) {
  const [status, setStatus] = React.useState<ConnectionStatus>(
    CONNECTION_STATUS.IDLE,
  );
  const onEventRef = React.useRef(onEvent);
  onEventRef.current = onEvent;
  const activityExpectedRef = React.useRef(activityExpected);
  activityExpectedRef.current = activityExpected;
  const syncActivityWatchdogRef = React.useRef(() => {});
  React.useEffect(() => {
    if (!enabled || !threadId) {
      setStatus(CONNECTION_STATUS.IDLE);
      return;
    }
    let controller = null;
    let activityWatchdog = null;
    let disposed = false;
    let terminalErrorReceived = false;
    let connectedOnce = false;
    let streamOpen = false;
    const request = eventStreamRequest({
      threadId,
    });
    const stream = new EventSourcePlus(request.url, {
      credentials: "same-origin",
      headers: request.headers,
      maxRetryInterval: 30_000,
      retryStrategy: "always",
    });

    function clearActivityWatchdog() {
      if (activityWatchdog) {
        clearTimeout(activityWatchdog);
        activityWatchdog = null;
      }
    }

    function markTransportUnavailable() {
      streamOpen = false;
      clearActivityWatchdog();
    }

    function activityIsExpected() {
      return activityExpectedRef.current === true;
    }

    function scheduleActivityWatchdog() {
      clearActivityWatchdog();
      if (
        disposed ||
        terminalErrorReceived ||
        !streamOpen ||
        !activityIsExpected()
      ) {
        return;
      }
      activityWatchdog = setTimeout(() => {
        activityWatchdog = null;
        if (
          disposed ||
          terminalErrorReceived ||
          !streamOpen ||
          !activityIsExpected()
        ) {
          return;
        }
        setStatus(CONNECTION_STATUS.RECONNECTING);
        controller.reconnect();
      }, ACTIVE_STREAM_STALL_DEADLINE_MS);
    }

    function markConnected() {
      if (disposed || terminalErrorReceived) return;
      streamOpen = true;
      connectedOnce = true;
      setStatus(CONNECTION_STATUS.CONNECTED);
    }

    function connect() {
      if (disposed || terminalErrorReceived) return;
      if (document.visibilityState === "hidden") {
        streamOpen = false;
        setStatus(CONNECTION_STATUS.PAUSED);
        return;
      }
      setStatus(
        isBrowserOffline()
          ? CONNECTION_STATUS.RECONNECTING
          : CONNECTION_STATUS.CONNECTING,
      );
      controller = stream.listen({
        onRequest({ options }) {
          if (disposed || terminalErrorReceived) return;
          markTransportUnavailable();
          const connectionState = nextConnectionState();
          options.query = {
            ...options.query,
            connection_id: connectionState.connectionId,
            connection_generation: connectionState.generation,
          };
          setStatus(
            connectedOnce
              ? CONNECTION_STATUS.RECONNECTING
              : CONNECTION_STATUS.CONNECTING,
          );
        },
        onRequestError() {
          if (disposed || terminalErrorReceived) return;
          markTransportUnavailable();
          setStatus(CONNECTION_STATUS.RECONNECTING);
        },
        onResponse({ response }) {
          if (disposed || terminalErrorReceived) return;
          if (
            response.ok &&
            response.headers.get("content-type")?.includes("text/event-stream")
          ) {
            markConnected();
            scheduleActivityWatchdog();
          }
        },
        onResponseError({ response }) {
          if (disposed || terminalErrorReceived) return;
          markTransportUnavailable();
          if (isRetryableResponseStatus(response.status)) {
            setStatus(CONNECTION_STATUS.RECONNECTING);
            return;
          }
          terminalErrorReceived = true;
          controller?.abort("non-retryable stream response");
          setStatus(CONNECTION_STATUS.DISCONNECTED);
        },
        onMessage(message) {
          if (disposed || terminalErrorReceived) return;
          let frame = null;
          try {
            frame = JSON.parse(message.data);
          } catch (_) {
            return;
          }
          if (!frame || typeof frame !== "object") return;
          const rawType = frame.type || message.event || "message";
          const type = rawType === "stream_error" ? "error" : rawType;
          if (type !== "error") markConnected();
          onEventRef.current?.({
            type,
            frame,
            lastEventId: message.id || null,
          });
          scheduleActivityWatchdog();
          if (type === "error" && frame.retryable === false) {
            terminalErrorReceived = true;
            streamOpen = false;
            clearActivityWatchdog();
            controller?.abort("non-retryable stream event");
            setStatus(CONNECTION_STATUS.DISCONNECTED);
            return;
          }
          if (
            type === "error" &&
            frame.kind === "replay_unavailable" &&
            frame.retryable === true
          ) {
            stream.lastEventId = undefined;
            markTransportUnavailable();
            setStatus(CONNECTION_STATUS.RECONNECTING);
            controller?.reconnect();
          }
        },
      });
      if (terminalErrorReceived) {
        streamOpen = false;
        controller?.abort("non-retryable stream response");
        return;
      }
    }

    function disconnectForHiddenTab() {
      if (disposed || terminalErrorReceived) return;
      streamOpen = false;
      clearActivityWatchdog();
      controller?.abort("document hidden");
      setStatus(CONNECTION_STATUS.PAUSED);
    }

    function handleVisibilityChange() {
      if (disposed || terminalErrorReceived) return;
      if (document.visibilityState === "hidden") {
        disconnectForHiddenTab();
      } else if (!controller) {
        connect();
      } else {
        markTransportUnavailable();
        setStatus(CONNECTION_STATUS.CONNECTING);
        controller.reconnect();
      }
    }

    function handleNetworkOffline() {
      if (disposed || terminalErrorReceived) return;
      setStatus(CONNECTION_STATUS.RECONNECTING);
    }

    function handleNetworkOnline() {
      if (disposed || terminalErrorReceived) return;
      markTransportUnavailable();
      setStatus(CONNECTION_STATUS.RECONNECTING);
      controller?.reconnect();
    }

    syncActivityWatchdogRef.current = () => {
      if (streamOpen) {
        scheduleActivityWatchdog();
      } else {
        clearActivityWatchdog();
      }
    };
    connect();
    document.addEventListener("visibilitychange", handleVisibilityChange);
    window.addEventListener("offline", handleNetworkOffline);
    window.addEventListener("online", handleNetworkOnline);

    return () => {
      disposed = true;
      streamOpen = false;
      document.removeEventListener("visibilitychange", handleVisibilityChange);
      window.removeEventListener("offline", handleNetworkOffline);
      window.removeEventListener("online", handleNetworkOnline);
      clearActivityWatchdog();
      syncActivityWatchdogRef.current = () => {};
      controller?.abort("component disposed");
      controller = null;
    };
  }, [enabled, threadId]);

  React.useEffect(() => {
    // A send can begin after an idle stream has already gone half-open,
    // so no accepted/running frame is available to arm the watchdog. Sync it
    // when processing state changes as well as when frames arrive.
    syncActivityWatchdogRef.current();
  }, [activityExpected, enabled, threadId]);

  return { status };
}
