import React from "react";
import { openEventStream } from "../../../lib/api";
import { authScope } from "../../../lib/auth-scope";
import {
  CONNECTION_STATUS,
  type ConnectionStatus,
} from "../lib/connection-status";

// v2 SSE emits `WebChatV2EventFrame` JSON, tagged with a typed
// event name (`event: accepted`, `event: final_reply`, etc.) so
// each frame routes to its `addEventListener("<name>", …)` handler.
// `onmessage` would only catch frames without an `event:` field,
// which the Rust handler never emits — so the SPA must register a
// listener for every event name it cares about. The names below
// mirror `WebChatV2Event::event_name()` in
// `crates/ironclaw_webui/src/webui_v2/schema.rs`.
const V2_EVENT_NAMES = [
  "accepted",
  "running",
  "capability_progress",
  "capability_activity",
  "capability_display_preview",
  "gate",
  "auth_required",
  "final_reply",
  "cancelled",
  "failed",
  "projection_snapshot",
  "projection_update",
  "keep_alive",
  "stream_error",
];

const EVENT_SOURCE_CLOSED = 2;
const EVENT_SOURCE_OPEN = 1;
const MAX_CACHED_CURSORS = 30;
const lastEventIdByThread = new Map<string, string>();

function cursorKey(threadId: string) {
  return `${authScope()}:${threadId}`;
}

function getLastEventId(threadId: string) {
  return lastEventIdByThread.get(cursorKey(threadId));
}

function setLastEventId(threadId: string, eventId: string) {
  const key = cursorKey(threadId);
  lastEventIdByThread.delete(key);
  lastEventIdByThread.set(key, eventId);
  while (lastEventIdByThread.size > MAX_CACHED_CURSORS) {
    const oldestKey = lastEventIdByThread.keys().next().value;
    if (oldestKey === undefined) break;
    lastEventIdByThread.delete(oldestKey);
  }
}

function deleteLastEventId(threadId: string) {
  lastEventIdByThread.delete(cursorKey(threadId));
}

function eventSourceReadyStateConstant(staticValue: unknown, fallback: number) {
  return typeof staticValue === "number" ? staticValue : fallback;
}

function isEventSourceClosed(source) {
  const closedState = typeof EventSource === "function"
    ? eventSourceReadyStateConstant(EventSource.CLOSED, EVENT_SOURCE_CLOSED)
    : EVENT_SOURCE_CLOSED;
  return source?.readyState === closedState;
}

function isEventSourceOpen(source) {
  const openState = typeof EventSource === "function"
    ? eventSourceReadyStateConstant(EventSource.OPEN, EVENT_SOURCE_OPEN)
    : EVENT_SOURCE_OPEN;
  return source?.readyState === openState;
}

function isBrowserOffline() {
  return typeof navigator !== "undefined" && navigator.onLine === false;
}

export function useSSE({ threadId, onEvent, enabled }) {
  const [status, setStatus] = React.useState<ConnectionStatus>(
    CONNECTION_STATUS.IDLE,
  );
  const onEventRef = React.useRef(onEvent);
  onEventRef.current = onEvent;
  React.useEffect(() => {
    if (!enabled || !threadId) {
      setStatus(CONNECTION_STATUS.IDLE);
      return;
    }
    let es = null;
    let reconnectTimer = null;
    let openWatchdog = null;
    let reconnectAttempts = 0;
    let disposed = false;
    let terminalErrorReceived = false;
    const maxReconnectDelay = 30_000;
    const reconnectOpenDeadline = 10_000;

    function clearOpenWatchdog() {
      if (openWatchdog) {
        clearTimeout(openWatchdog);
        openWatchdog = null;
      }
    }

    function markConnected(source) {
      if (disposed || terminalErrorReceived || es !== source) return;
      clearOpenWatchdog();
      reconnectAttempts = 0;
      setStatus(CONNECTION_STATUS.CONNECTED);
    }

    function scheduleOpenWatchdog(source) {
      if (openWatchdog) return;
      openWatchdog = setTimeout(() => {
        openWatchdog = null;
        if (disposed || terminalErrorReceived || es !== source) return;
        if (isEventSourceOpen(source)) {
          markConnected(source);
          return;
        }
        reconnectWithTimer(CONNECTION_STATUS.RECONNECTING);
      }, reconnectOpenDeadline);
    }

    function reconnectWithTimer(
      status: ConnectionStatus = CONNECTION_STATUS.DISCONNECTED,
    ) {
      if (disposed || terminalErrorReceived) return;
      if (es) {
        es.close();
        es = null;
      }
      if (reconnectTimer) {
        clearTimeout(reconnectTimer);
        reconnectTimer = null;
      }
      clearOpenWatchdog();
      setStatus(status);
      reconnectAttempts++;
      const delay = Math.min(1000 * 2 ** reconnectAttempts, maxReconnectDelay);
      reconnectTimer = setTimeout(connect, delay);
    }

    function connect() {
      reconnectTimer = null;
      if (disposed || terminalErrorReceived) return;
      if (document.visibilityState === "hidden") {
        setStatus(CONNECTION_STATUS.PAUSED);
        return;
      }
      setStatus(
        isBrowserOffline() || reconnectAttempts > 0
          ? CONNECTION_STATUS.RECONNECTING
          : CONNECTION_STATUS.CONNECTING,
      );

      es = openEventStream({
        threadId,
        afterCursor: getLastEventId(threadId) || undefined,
      });
      const source = es;

      // A replacement EventSource can remain in CONNECTING forever without
      // firing either callback when a proxy accepts the HTTP request but does
      // not establish the event stream. Bound reconnect attempts explicitly;
      // the initial connection remains browser-managed, while every recovery
      // attempt must prove it opened within this deadline.
      if (reconnectAttempts > 0) scheduleOpenWatchdog(source);

      source.onopen = () => {
        markConnected(source);
      };

      const dispatchFrame = (event, fallbackType) => {
        if (disposed || es !== source) return;
        let frame = null;
        try {
          frame = JSON.parse(event.data);
        } catch (_) {
          return;
        }
        if (!frame || typeof frame !== "object") return;
        if (event.lastEventId) {
          setLastEventId(threadId, event.lastEventId);
        }
        const rawType = frame.type || fallbackType;
        const type = rawType === "stream_error" ? "error" : rawType;
        // Some browsers resume an interrupted EventSource by delivering the
        // next frame without a second `open` callback. A normal frame proves
        // the transport recovered and must clear a stale reconnecting badge.
        // Classified stream errors keep their own terminal/retry state below.
        if (type !== "error") markConnected(source);
        onEventRef.current?.({
          // The frame's own `type` field is the canonical source;
          // `event.type` (from the SSE `event:` line) is the
          // fallback for forwards-compatibility if Rust adds an
          // event without setting `type` in the body.
          type,
          frame,
          lastEventId: event.lastEventId || null,
        });
        // The server has already classified this failure as permanent for
        // this subscription (for example, a thread that no longer exists).
        // EventSource reports the subsequent clean server close through
        // `onerror`; remember the terminal frame and close locally so that
        // callback cannot turn a non-retryable response into an infinite
        // reconnect loop.
        if (type === "error" && frame.retryable === false && es === source) {
          terminalErrorReceived = true;
          if (reconnectTimer) {
            clearTimeout(reconnectTimer);
            reconnectTimer = null;
          }
          es = null;
          source.close();
          setStatus(CONNECTION_STATUS.DISCONNECTED);
          return;
        }
        // A replay-unavailable response means retrying the same cursor can
        // never make progress. Replace this EventSource so the browser drops
        // its internal Last-Event-ID, and reconnect from the projection origin
        // where durable run/final-reply state can be rebuilt.
        if (
          type === "error" &&
          frame.kind === "replay_unavailable" &&
          frame.retryable === true &&
          es === source
        ) {
          deleteLastEventId(threadId);
          reconnectWithTimer(CONNECTION_STATUS.RECONNECTING);
        }
      };

      source.onerror = (event) => {
        if (disposed || terminalErrorReceived || es !== source) return;
        // Compatibility with servers that emitted application failures on
        // the reserved `event: error` channel. Those arrive as MessageEvents
        // with data; a native EventSource transport failure has no data.
        // Parsing the former here prevents one browser event from also
        // entering the transport reconnect state machine.
        if (typeof event?.data === "string") {
          dispatchFrame(event, "error");
          return;
        }
        // Preserve EventSource's native retry for transient failures. Closing
        // it immediately creates a fresh HTTP stream for every error, which can
        // race server-side slot release behind a proxy and strand the client in
        // a reconnect loop. The watchdog below remains the bounded fallback if
        // native recovery never opens or delivers another frame.
        if (!isEventSourceClosed(source)) {
          setStatus(CONNECTION_STATUS.RECONNECTING);
          scheduleOpenWatchdog(source);
          return;
        }
        reconnectWithTimer();
      };

      // Cover anything emitted without an `event:` field — defensive
      // only; the Rust handler always tags its frames today.
      es.onmessage = (event) => dispatchFrame(event, "message");

      // The Rust handler tags each frame with `event: <name>` so the
      // browser routes it through the named listener below.
      for (const name of V2_EVENT_NAMES) {
        es.addEventListener(name, (event) => dispatchFrame(event, name));
      }
    }

    function disconnectForHiddenTab() {
      if (disposed || terminalErrorReceived) return;
      if (reconnectTimer) {
        clearTimeout(reconnectTimer);
        reconnectTimer = null;
      }
      clearOpenWatchdog();
      if (es) {
        es.close();
        es = null;
      }
      setStatus(CONNECTION_STATUS.PAUSED);
    }

    function handleVisibilityChange() {
      if (disposed || terminalErrorReceived) return;
      if (document.visibilityState === "hidden") {
        disconnectForHiddenTab();
      } else if (!es) {
        connect();
      }
    }

    function handleNetworkOffline() {
      if (disposed || terminalErrorReceived) return;
      setStatus(CONNECTION_STATUS.RECONNECTING);
    }

    function handleNetworkOnline() {
      if (disposed || terminalErrorReceived) return;
      if (es && isEventSourceOpen(es)) {
        markConnected(es);
        return;
      }
      setStatus(CONNECTION_STATUS.RECONNECTING);
      if (es) {
        scheduleOpenWatchdog(es);
        return;
      }
      if (reconnectTimer) {
        clearTimeout(reconnectTimer);
        reconnectTimer = null;
      }
      connect();
    }

    connect();
    document.addEventListener("visibilitychange", handleVisibilityChange);
    window.addEventListener("offline", handleNetworkOffline);
    window.addEventListener("online", handleNetworkOnline);

    return () => {
      disposed = true;
      document.removeEventListener("visibilitychange", handleVisibilityChange);
      window.removeEventListener("offline", handleNetworkOffline);
      window.removeEventListener("online", handleNetworkOnline);
      if (reconnectTimer) clearTimeout(reconnectTimer);
      clearOpenWatchdog();
      const source = es;
      es = null;
      source?.close();
    };
  }, [enabled, threadId]);

  return { status };
}
