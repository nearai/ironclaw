import React from "react";
import { EventSourcePlus } from "event-source-plus";
import { clientActionId, eventStreamRequest } from "../../../lib/api";
import {
  CONNECTION_STATUS,
  type ConnectionStatus,
} from "../lib/connection-status";

const ACTIVE_STREAM_STALL_DEADLINE_MS = 30_000;
const SSE_CONNECTION_STORAGE_KEY = "ironclaw:v2-sse-connection";
const SSE_RETRY_BASE_MS = 1_000;
const SSE_RETRY_MAX_MS = 30_000;
const SSE_RETRY_JITTER_RATIO = 0.2;
const SSE_RETRY_RESET_AFTER_MS = 15_000;
const MAX_TIMER_DELAY_MS = 2_147_483_647;

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

function localRetryDelayMs(attempt) {
  const exponential = Math.min(
    SSE_RETRY_BASE_MS * 2 ** Math.min(attempt, 30),
    SSE_RETRY_MAX_MS,
  );
  const jitter = 1 - SSE_RETRY_JITTER_RATIO + Math.random() * 2 * SSE_RETRY_JITTER_RATIO;
  return Math.min(Math.round(exponential * jitter), SSE_RETRY_MAX_MS);
}

function responseRetryAfterMs(response) {
  const raw = response?.headers?.get?.("retry-after")?.trim();
  if (!raw) return 0;
  const seconds = Number(raw);
  if (Number.isFinite(seconds) && seconds >= 0) {
    return Math.min(Math.ceil(seconds * 1_000), MAX_TIMER_DELAY_MS);
  }
  const deadline = Date.parse(raw);
  if (!Number.isFinite(deadline)) return 0;
  return Math.min(Math.max(0, deadline - Date.now()), MAX_TIMER_DELAY_MS);
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
    let retryTimer = null;
    let retryAttempt = 0;
    let disposed = false;
    let terminalErrorReceived = false;
    let connectedOnce = false;
    let streamOpen = false;
    let streamOpenedAt = null;
    const request = eventStreamRequest({
      threadId,
    });
    const stream = new EventSourcePlus(request.url, {
      credentials: "same-origin",
      headers: request.headers,
      maxRetryInterval: 30_000,
      // IronClaw owns reconnect timing below. `event-source-plus` still owns
      // fetch, framing, cancellation, and Last-Event-ID, but its 0.1.x retry
      // clock starts at 2ms and resets on HTTP headers rather than a proven
      // live SSE frame. Letting both layers retry creates request storms.
      retryStrategy: "on-error",
    });

    function clearActivityWatchdog() {
      if (activityWatchdog) {
        clearTimeout(activityWatchdog);
        activityWatchdog = null;
      }
    }

    function markTransportUnavailable() {
      streamOpen = false;
      streamOpenedAt = null;
      clearActivityWatchdog();
    }

    function cancelScheduledRetry() {
      if (retryTimer) {
        clearTimeout(retryTimer);
        retryTimer = null;
      }
    }

    function resetRetryBackoff() {
      retryAttempt = 0;
      cancelScheduledRetry();
    }

    function scheduleReconnect(reason, response = null) {
      if (disposed || terminalErrorReceived) return;
      markTransportUnavailable();
      setStatus(CONNECTION_STATUS.RECONNECTING);
      if (retryTimer) return;

      // Abort the package-owned request before its catch path can start an
      // automatic retry. Every retry source then converges on this one timer.
      controller?.abort(`retry scheduled: ${reason}`);
      if (document.visibilityState === "hidden" || isBrowserOffline()) return;

      const delay = Math.max(
        localRetryDelayMs(retryAttempt),
        responseRetryAfterMs(response),
      );
      retryAttempt = Math.min(retryAttempt + 1, 30);
      retryTimer = setTimeout(() => {
        retryTimer = null;
        if (
          disposed ||
          terminalErrorReceived ||
          document.visibilityState === "hidden" ||
          isBrowserOffline()
        ) {
          return;
        }
        controller?.reconnect();
      }, delay);
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
        scheduleReconnect("activity watchdog");
      }, ACTIVE_STREAM_STALL_DEADLINE_MS);
    }

    function markConnected() {
      if (disposed || terminalErrorReceived) return;
      if (!streamOpen) streamOpenedAt = Date.now();
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
          scheduleReconnect("request error");
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
          if (isRetryableResponseStatus(response.status)) {
            scheduleReconnect("retryable stream response", response);
            return;
          }
          markTransportUnavailable();
          terminalErrorReceived = true;
          cancelScheduledRetry();
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
          if (type !== "error") {
            markConnected();
            // The server emits one keep_alive immediately after admission.
            // Require a frame after a stable interval before resetting, or a
            // stream that opens, pings once, and dies can still loop at 1s.
            if (
              streamOpenedAt !== null &&
              Date.now() - streamOpenedAt >= SSE_RETRY_RESET_AFTER_MS
            ) {
              resetRetryBackoff();
            }
          }
          onEventRef.current?.({
            type,
            frame,
            lastEventId: message.id || null,
          });
          scheduleActivityWatchdog();
          if (type === "error" && frame.retryable === false) {
            terminalErrorReceived = true;
            markTransportUnavailable();
            cancelScheduledRetry();
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
            scheduleReconnect("projection replay unavailable");
          }
        },
      });
      controller.onAbort?.((event) => {
        if (event.type === "end-of-stream") {
          scheduleReconnect("stream ended");
        }
      });
      if (terminalErrorReceived) {
        streamOpen = false;
        controller?.abort("non-retryable stream response");
        return;
      }
    }

    function disconnectForHiddenTab() {
      if (disposed || terminalErrorReceived) return;
      markTransportUnavailable();
      cancelScheduledRetry();
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
        setStatus(CONNECTION_STATUS.CONNECTING);
        scheduleReconnect("document visible");
      }
    }

    function handleNetworkOffline() {
      if (disposed || terminalErrorReceived) return;
      markTransportUnavailable();
      cancelScheduledRetry();
      setStatus(CONNECTION_STATUS.RECONNECTING);
    }

    function handleNetworkOnline() {
      if (disposed || terminalErrorReceived) return;
      scheduleReconnect("network online");
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
      cancelScheduledRetry();
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
