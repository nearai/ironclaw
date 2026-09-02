// Transport-independent thread event subscription.
//
// Chooses between the app-wide session WebSocket (when the deployment
// advertises `features.session_events` and the client has not degraded) and
// the route-local compatibility SSE hook. Both paths hand `useChatEvents`
// byte-identical frame bodies — the server renders them through one shared
// codec — so the consumer cannot tell transports apart, which is exactly the
// rollout/rollback property the design requires (§7.5, §16).
//
// The session client itself is dynamically imported so its bytes stay out of
// the eager /chat closure.

import React from "react";
import { isSessionEventsAdvertised } from "../../../lib/session-events/transport-flag";
import {
  CONNECTION_STATUS,
  type ConnectionStatus,
} from "../lib/connection-status";
import { useSSE } from "./useSSE";

type ThreadEventEnvelope = {
  type: string;
  frame: Record<string, unknown>;
  lastEventId: string | null;
};

type UseThreadEventsInput = {
  threadId?: string | null;
  onEvent: (envelope: ThreadEventEnvelope) => void;
  enabled: boolean;
  activityExpected?: boolean;
};

function socketStatusToConnectionStatus(status: string): ConnectionStatus {
  switch (status) {
    case "open":
      return CONNECTION_STATUS.CONNECTED;
    case "connecting":
      return CONNECTION_STATUS.CONNECTING;
    case "reconnecting":
      return CONNECTION_STATUS.RECONNECTING;
    default:
      return CONNECTION_STATUS.DISCONNECTED;
  }
}

export function useThreadEvents({
  threadId,
  onEvent,
  enabled,
  activityExpected = false,
}: UseThreadEventsInput): { status: ConnectionStatus } {
  // Transport choice is sampled per mount and re-evaluated when the session
  // client degrades; a degradation mid-stream flips this thread to SSE,
  // which resumes from durable state (route-local SSE always starts at the
  // projection origin, same as a thread remount).
  const [socketDegraded, setSocketDegraded] = React.useState(false);
  const useSocket = enabled && isSessionEventsAdvertised() && !socketDegraded;
  const [socketStatus, setSocketStatus] = React.useState<ConnectionStatus>(
    CONNECTION_STATUS.IDLE,
  );
  const onEventRef = React.useRef(onEvent);
  onEventRef.current = onEvent;

  const sseState = useSSE({
    threadId,
    onEvent,
    enabled: Boolean(enabled && threadId) && !useSocket,
    activityExpected,
  });

  React.useEffect(() => {
    if (!useSocket || !threadId) {
      setSocketStatus(CONNECTION_STATUS.IDLE);
      return undefined;
    }
    let disposed = false;
    let subscription: { unsubscribe: () => void } | null = null;
    setSocketStatus(CONNECTION_STATUS.CONNECTING);
    void import("../../../lib/session-events/client").then(
      ({ sessionEventClient }) => {
        if (disposed) return;
        const client = sessionEventClient();
        if (client.isDegraded()) {
          setSocketDegraded(true);
          return;
        }
        subscription = client.subscribe(
          { kind: "thread", thread_id: threadId },
          {
            onEvent: ({ cursor, body }) => {
              const type = typeof body.type === "string" ? body.type : "message";
              onEventRef.current({
                // The session frame body is the same tagged WebChatV2Event
                // JSON the SSE `data:` line carries; useChatEvents reads it
                // unchanged. `stream_error` never appears here — transport
                // errors arrive as subscription_error frames instead.
                type,
                frame: body,
                lastEventId: cursor,
              });
            },
            onError: (error) => {
              if (!error.retryable) {
                // Non-retryable selector failure (revoked access, foreign
                // thread): stop delivering, exactly like the SSE terminal
                // error path.
                setSocketStatus(CONNECTION_STATUS.DISCONNECTED);
                onEventRef.current({
                  type: "error",
                  frame: {
                    error: error.error,
                    kind: error.kind,
                    retryable: error.retryable,
                  },
                  lastEventId: null,
                });
              }
              // Retryable errors rebase inside the client (it resubscribes
              // from the last safe cursor); the projection replays durable
              // state so no consumer action is needed.
            },
            onStatus: (status) => {
              if (status === "degraded") {
                setSocketDegraded(true);
                return;
              }
              setSocketStatus(socketStatusToConnectionStatus(status));
            },
          },
          { idPrefix: `chat-${threadId.slice(0, 24)}` },
        );
      },
    );
    return () => {
      disposed = true;
      subscription?.unsubscribe();
    };
  }, [useSocket, threadId]);

  return { status: useSocket ? socketStatus : sseState.status };
}
