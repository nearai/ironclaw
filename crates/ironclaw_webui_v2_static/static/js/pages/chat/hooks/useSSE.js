import { React } from "../../../lib/html.js";
import { openEventStream } from "../../../lib/api.js";

// v2 SSE emits `WebChatV2EventFrame` JSON on the default `message`
// channel. Each frame carries `{ cursor, type, ...payload }` and
// the SSE `id:` is the JSON-serialized projection cursor so the
// browser can resume from the last delivered event via the
// standard `Last-Event-ID` reconnect header (handled automatically
// by the `EventSource` API).
export function useSSE({ threadId, onEvent, enabled }) {
  const [status, setStatus] = React.useState("idle");
  const onEventRef = React.useRef(onEvent);
  onEventRef.current = onEvent;

  React.useEffect(() => {
    if (!enabled || !threadId) {
      setStatus("idle");
      return;
    }

    let es = null;
    let reconnectTimer = null;
    let reconnectAttempts = 0;
    const maxReconnectDelay = 30_000;

    function connect() {
      if (document.visibilityState === "hidden") {
        setStatus("paused");
        return;
      }
      setStatus(reconnectAttempts > 0 ? "reconnecting" : "connecting");

      es = openEventStream({ threadId });

      es.onopen = () => {
        reconnectAttempts = 0;
        setStatus("connected");
      };

      es.onerror = () => {
        if (es) es.close();
        setStatus("disconnected");
        reconnectAttempts++;
        const delay = Math.min(1000 * 2 ** reconnectAttempts, maxReconnectDelay);
        reconnectTimer = setTimeout(connect, delay);
      };

      es.onmessage = (event) => {
        let frame = null;
        try {
          frame = JSON.parse(event.data);
        } catch (_) {
          return;
        }
        if (!frame || typeof frame !== "object") return;
        onEventRef.current?.({
          type: frame.type,
          frame,
          lastEventId: event.lastEventId || null,
        });
      };
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
