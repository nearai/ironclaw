import React from "react";
import { openEventStream } from "../../../lib/api";
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
  "error",
];

const EVENT_SOURCE_CLOSED = 2;
const EVENT_SOURCE_OPEN = 1;

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
  // Last cursor we successfully received. EventSource sends
  // `Last-Event-ID` automatically while a single instance reconnects
  // internally, but a *fresh* EventSource (tab resume from hidden,
  // explicit reconnect after threadId change) loses that memory. We
  // pipe it through the v2 backend's `?after_cursor=` query fallback
  // so resumption survives those cases too.
  const lastEventIdRef = React.useRef(null);

  React.useEffect(() => {
    if (!enabled || !threadId) {
      setStatus(CONNECTION_STATUS.IDLE);
      return;
    }
    // New thread → drop the prior thread's cursor before the first
    // connect so we don't try to resume one thread's projection from
    // another thread's id.
    lastEventIdRef.current = null;

    let es = null;
    let reconnectTimer = null;
    let reconnectWatchdog = null;
    let reconnectAttempts = 0;
    const maxReconnectDelay = 30_000;
    const nativeReconnectWatchdogDelay = 10_000;

    function clearReconnectWatchdog() {
      if (reconnectWatchdog) {
        clearTimeout(reconnectWatchdog);
        reconnectWatchdog = null;
      }
    }

    function reconnectWithTimer() {
      if (es) {
        es.close();
        es = null;
      }
      clearReconnectWatchdog();
      setStatus(CONNECTION_STATUS.DISCONNECTED);
      reconnectAttempts++;
      const delay = Math.min(1000 * 2 ** reconnectAttempts, maxReconnectDelay);
      reconnectTimer = setTimeout(connect, delay);
    }

    function scheduleNativeReconnectWatchdog(source) {
      if (reconnectWatchdog) return;
      reconnectWatchdog = setTimeout(() => {
        reconnectWatchdog = null;
        if (es !== source || !source) {
          return;
        }
        if (isEventSourceOpen(source)) {
          reconnectAttempts = 0;
          setStatus(CONNECTION_STATUS.CONNECTED);
          return;
        }
        reconnectWithTimer();
      }, nativeReconnectWatchdogDelay);
    }

    function connect() {
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
        afterCursor: lastEventIdRef.current || undefined,
      });

      es.onopen = () => {
        clearReconnectWatchdog();
        reconnectAttempts = 0;
        setStatus(CONNECTION_STATUS.CONNECTED);
      };

      es.onerror = () => {
        if (!es) return;
        if (!isEventSourceClosed(es)) {
          setStatus(CONNECTION_STATUS.RECONNECTING);
          scheduleNativeReconnectWatchdog(es);
          return;
        }
        reconnectWithTimer();
      };

      const dispatchFrame = (event, fallbackType) => {
        let frame = null;
        try {
          frame = JSON.parse(event.data);
        } catch (_) {
          return;
        }
        if (!frame || typeof frame !== "object") return;
        if (event.lastEventId) {
          lastEventIdRef.current = event.lastEventId;
        }
        onEventRef.current?.({
          // The frame's own `type` field is the canonical source;
          // `event.type` (from the SSE `event:` line) is the
          // fallback for forwards-compatibility if Rust adds an
          // event without setting `type` in the body.
          type: frame.type || fallbackType,
          frame,
          lastEventId: event.lastEventId || null,
        });
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
      if (reconnectTimer) {
        clearTimeout(reconnectTimer);
        reconnectTimer = null;
      }
      clearReconnectWatchdog();
      if (es) {
        es.close();
        es = null;
      }
      setStatus(CONNECTION_STATUS.PAUSED);
    }

    function handleVisibilityChange() {
      if (document.visibilityState === "hidden") {
        disconnectForHiddenTab();
      } else if (!es) {
        connect();
      }
    }

    function handleNetworkOffline() {
      setStatus(CONNECTION_STATUS.RECONNECTING);
    }

    function handleNetworkOnline() {
      if (es && isEventSourceOpen(es)) {
        clearReconnectWatchdog();
        reconnectAttempts = 0;
        setStatus(CONNECTION_STATUS.CONNECTED);
        return;
      }
      setStatus(CONNECTION_STATUS.RECONNECTING);
      if (es) {
        scheduleNativeReconnectWatchdog(es);
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
      document.removeEventListener("visibilitychange", handleVisibilityChange);
      window.removeEventListener("offline", handleNetworkOffline);
      window.removeEventListener("online", handleNetworkOnline);
      if (reconnectTimer) clearTimeout(reconnectTimer);
      clearReconnectWatchdog();
      if (es) es.close();
    };
  }, [enabled, threadId]);

  return { status };
}
