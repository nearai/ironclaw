import { React } from "../../../lib/html.js";
import { readStoredToken } from "../../../lib/api.js";

const EVENT_TYPES = [
  "response",
  "stream_chunk",
  "thinking",
  "tool_started",
  "tool_completed",
  "tool_result",
  "status",
  "gate_required",
  "gate_resolved",
  "approval_needed",
  "error",
  "image_generated",
  "suggestions",
  "turn_cost",
  "plan_update",
  "onboarding_state",
];

/**
 * @param {{ onEvent: (event: { type: string, data: any, lastEventId?: string }) => void, enabled: boolean }} options
 */
export function useSSE({ onEvent, enabled }) {
  const [status, setStatus] = React.useState("idle");
  const lastEventIdRef = React.useRef("");
  const onEventRef = React.useRef(onEvent);
  onEventRef.current = onEvent;

  React.useEffect(() => {
    if (!enabled) {
      setStatus("idle");
      return;
    }

    let es = null;
    let reconnectTimer = null;
    let reconnectAttempts = 0;
    const maxReconnectDelay = 30000;

    function connect() {
      if (document.visibilityState === "hidden") {
        setStatus("paused");
        return;
      }
      setStatus(reconnectAttempts > 0 ? "reconnecting" : "connecting");

      const token = readStoredToken();
      let url = token
        ? `/api/chat/stream?token=${encodeURIComponent(token)}`
        : "/api/chat/stream";
      if (lastEventIdRef.current) {
        url += `${url.includes("?") ? "&" : "?"}last_event_id=${encodeURIComponent(lastEventIdRef.current)}`;
      }

      es = new EventSource(url);

      es.onopen = () => {
        reconnectAttempts = 0;
        setStatus("connected");
      };

      es.onerror = () => {
        if (es) es.close();
        setStatus("disconnected");
        reconnectAttempts++;
        const delay = Math.min(1000 * Math.pow(2, reconnectAttempts), maxReconnectDelay);
        reconnectTimer = setTimeout(connect, delay);
      };

      EVENT_TYPES.forEach((type) => {
        es.addEventListener(type, (event) => {
          if (event.lastEventId) {
            lastEventIdRef.current = event.lastEventId;
          }
          let data = null;
          try {
            data = JSON.parse(event.data);
          } catch {
            data = event.data;
          }
          onEventRef.current?.({ type, data, lastEventId: event.lastEventId });
        });
      });
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
  }, [enabled]);

  return { status };
}
