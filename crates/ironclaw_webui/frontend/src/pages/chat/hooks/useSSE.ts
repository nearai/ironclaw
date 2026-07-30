import React from "react";
import { EventSourcePlus } from "event-source-plus";
import { clientActionId, eventStreamRequest } from "../../../lib/api";
import {
  CONNECTION_STATUS,
  type ConnectionStatus,
} from "../lib/connection-status";

const ACTIVE_STREAM_STALL_DEADLINE_MS = 30_000;
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
    let disposed = false;
    let terminalErrorReceived = false;
    let connectedOnce = false;
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

    function activityIsExpected() {
      return activityExpectedRef.current === true;
    }

    function scheduleActivityWatchdog() {
      clearActivityWatchdog();
      if (
        disposed ||
        terminalErrorReceived ||
        !controller ||
        !activityIsExpected()
      ) {
        return;
      }
      activityWatchdog = setTimeout(() => {
        activityWatchdog = null;
        if (
          disposed ||
          terminalErrorReceived ||
          !controller ||
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
      connectedOnce = true;
      setStatus(CONNECTION_STATUS.CONNECTED);
      scheduleActivityWatchdog();
    }

    function connect() {
      if (disposed || terminalErrorReceived) return;
      if (document.visibilityState === "hidden") {
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
          setStatus(
            connectedOnce
              ? CONNECTION_STATUS.RECONNECTING
              : CONNECTION_STATUS.CONNECTING,
          );
        },
        onRequestError() {
          if (disposed || terminalErrorReceived) return;
          setStatus(CONNECTION_STATUS.RECONNECTING);
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
            setStatus(CONNECTION_STATUS.RECONNECTING);
            return;
          }
          terminalErrorReceived = true;
          clearActivityWatchdog();
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
            setStatus(CONNECTION_STATUS.RECONNECTING);
            controller?.reconnect();
          }
        },
      });
    }

    function disconnectForHiddenTab() {
      if (disposed || terminalErrorReceived) return;
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
      setStatus(CONNECTION_STATUS.RECONNECTING);
      controller?.reconnect();
    }

    syncActivityWatchdogRef.current = () => {
      if (controller) {
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
  }, [activityExpected]);

  return { status };
}
