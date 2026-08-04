import React from "react";
import { EventSourcePlus } from "event-source-plus";
import { clientActionId, eventStreamRequest } from "../../../lib/api";
import {
  CONNECTION_STATUS,
  type ConnectionStatus,
} from "../lib/connection-status";

const ACTIVE_STREAM_STALL_DEADLINE_MS = 30_000;
// Per-chunk reconnects (e.g. a proxy closing the SSE body between streamed
// frames) fire `onRequest` and would otherwise flip the badge to "Reconnecting"
// for every chunk. A short grace window keeps the status silently CONNECTED
// for routine in-flight reconnects that settle quickly; a reconnect that
// actually drags on past the deadline surfaces "Reconnecting" as before.
// Genuine loss paths (request error, retryable HTTP, watchdog stall, offline,
// replay rebase) bypass the grace and set RECONNECTING immediately.
const RECONNECT_GRACE_MS = 1_000;
const SSE_CONNECTION_ID = clientActionId();
let nextConnectionGeneration = 0;

function connectionGeneration() {
  nextConnectionGeneration =
    nextConnectionGeneration >= Number.MAX_SAFE_INTEGER
      ? 1
      : nextConnectionGeneration + 1;
  return nextConnectionGeneration;
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
    let reconnectGraceTimer = null;
    let disposed = false;
    let terminalErrorReceived = false;
    let connectedOnce = false;
    let streamOpen = false;
    const request = eventStreamRequest({
      threadId,
      connectionId: SSE_CONNECTION_ID,
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

    function clearReconnectGrace() {
      if (reconnectGraceTimer) {
        clearTimeout(reconnectGraceTimer);
        reconnectGraceTimer = null;
      }
    }

    function activityIsExpected() {
      return activityExpectedRef.current === true;
    }

    // `graceful: true` is for routine in-flight reconnects (the `onRequest`
    // hook firing because the previous stream body ended). Those reconnects
    // are expected and usually settle in well under a second, so we hide the
    // "Reconnecting" badge behind a grace timer to avoid per-chunk flicker.
    // Loss paths that already represent a real interruption call this with
    // `graceful: false` (the default) to set RECONNECTING immediately.
    function markReconnecting({ graceful = false } = {}) {
      if (disposed || terminalErrorReceived) return;
      if (graceful && connectedOnce) {
        clearReconnectGrace();
        reconnectGraceTimer = setTimeout(() => {
          reconnectGraceTimer = null;
          if (disposed || terminalErrorReceived) return;
          setStatus(CONNECTION_STATUS.RECONNECTING);
        }, RECONNECT_GRACE_MS);
        return;
      }
      clearReconnectGrace();
      setStatus(CONNECTION_STATUS.RECONNECTING);
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
        markReconnecting();
        controller.reconnect();
      }, ACTIVE_STREAM_STALL_DEADLINE_MS);
    }

    function markConnected() {
      if (disposed || terminalErrorReceived) return;
      clearReconnectGrace();
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
          options.query = {
            ...options.query,
            connection_generation: connectionGeneration(),
          };
          if (!connectedOnce) {
            setStatus(CONNECTION_STATUS.CONNECTING);
            return;
          }
          // Routine in-flight reconnect (e.g. proxy closed the stream body
          // between chunks). Stay silently CONNECTED under the grace window
          // instead of flashing "Reconnecting" for every chunk.
          markReconnecting({ graceful: true });
        },
        onRequestError() {
          if (disposed || terminalErrorReceived) return;
          markReconnecting();
        },
        onResponse({ response }) {
          if (disposed || terminalErrorReceived) return;
          if (
            response.ok &&
            response.headers.get("content-type")?.includes("text/event-stream")
          ) {
            markConnected();
          }
        },
        onResponseError({ response }) {
          if (disposed || terminalErrorReceived) return;
          if (isRetryableResponseStatus(response.status)) {
            markReconnecting();
            return;
          }
          terminalErrorReceived = true;
          streamOpen = false;
          clearActivityWatchdog();
          clearReconnectGrace();
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
            clearReconnectGrace();
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
            markReconnecting();
            controller?.reconnect();
          }
        },
      });
      if (terminalErrorReceived) {
        streamOpen = false;
        controller?.abort("non-retryable stream response");
        return;
      }
      streamOpen = true;
      scheduleActivityWatchdog();
    }

    function disconnectForHiddenTab() {
      if (disposed || terminalErrorReceived) return;
      streamOpen = false;
      clearActivityWatchdog();
      clearReconnectGrace();
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
        streamOpen = true;
        setStatus(CONNECTION_STATUS.CONNECTING);
        controller.reconnect();
        scheduleActivityWatchdog();
      }
    }

    function handleNetworkOffline() {
      if (disposed || terminalErrorReceived) return;
      markReconnecting();
    }

    function handleNetworkOnline() {
      if (disposed || terminalErrorReceived) return;
      markReconnecting();
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
      clearReconnectGrace();
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
