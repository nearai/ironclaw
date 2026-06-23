import { React } from "../../../lib/html.js";
import { openEventStream } from "../../../lib/api.js";

// v2 SSE emits `WebChatV2EventFrame` JSON, tagged with a typed
// event name (`event: accepted`, `event: final_reply`, etc.) so
// each frame routes to its `addEventListener("<name>", …)` handler.
// `onmessage` would only catch frames without an `event:` field,
// which the Rust handler never emits — so the SPA must register a
// listener for every event name it cares about. The names below
// mirror `WebChatV2Event::event_name()` in
// `crates/ironclaw_webui_v2/src/schema.rs`.
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
export function useSSE({ threadId, onEvent, enabled }) {
  const [status, setStatus] = React.useState("idle");
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
      setStatus("idle");
      return;
    }
    // New thread → drop the prior thread's cursor before the first
    // connect so we don't try to resume one thread's projection from
    // another thread's id.
    lastEventIdRef.current = null;

    let es = null;
    let reconnectTimer = null;
    let reconnectAttempts = 0;
    let terminalError = false;
    const handledEvents = new WeakSet();
    const maxReconnectDelay = 30_000;

    function connect() {
      if (terminalError) return;
      if (document.visibilityState === "hidden") {
        setStatus("paused");
        return;
      }
      setStatus(reconnectAttempts > 0 ? "reconnecting" : "connecting");

      es = openEventStream({
        threadId,
        afterCursor: lastEventIdRef.current || undefined,
      });

      es.onopen = () => {
        reconnectAttempts = 0;
        setStatus("connected");
      };

      es.onerror = (event) => {
        if (dispatchFrame(event, "error") === "terminal") return;
        if (es) es.close();
        es = null;
        setStatus("disconnected");
        if (terminalError) return;
        reconnectAttempts++;
        const delay = Math.min(1000 * 2 ** reconnectAttempts, maxReconnectDelay);
        reconnectTimer = setTimeout(connect, delay);
      };

      function dispatchFrame(event, fallbackType) {
        if (!event || handledEvents.has(event)) return null;
        let frame = null;
        try {
          frame = JSON.parse(event.data);
        } catch (_) {
          return null;
        }
        if (!frame || typeof frame !== "object") return null;
        handledEvents.add(event);
        if (event.lastEventId) {
          lastEventIdRef.current = event.lastEventId;
        }
        const frameType = frame.type || fallbackType;
        const stopAfterEvent = frameType === "error" && frame.retryable === false;
        if (stopAfterEvent) {
          terminalError = true;
          if (reconnectTimer) {
            clearTimeout(reconnectTimer);
            reconnectTimer = null;
          }
        }
        onEventRef.current?.({
          // The frame's own `type` field is the canonical source;
          // `event.type` (from the SSE `event:` line) is the
          // fallback for forwards-compatibility if Rust adds an
          // event without setting `type` in the body.
          type: frameType,
          frame,
          lastEventId: event.lastEventId || null,
        });
        if (stopAfterEvent && es) {
          es.close();
          es = null;
          setStatus("disconnected");
        }
        return stopAfterEvent ? "terminal" : "handled";
      }

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
      if (es) {
        es.close();
        es = null;
      }
      setStatus("paused");
    }

    function handleVisibilityChange() {
      if (document.visibilityState === "hidden") {
        disconnectForHiddenTab();
      } else if (!es) {
        connect();
      }
    }

    connect();
    document.addEventListener("visibilitychange", handleVisibilityChange);

    return () => {
      document.removeEventListener("visibilitychange", handleVisibilityChange);
      if (reconnectTimer) clearTimeout(reconnectTimer);
      if (es) es.close();
    };
  }, [enabled, threadId]);

  return { status };
}
