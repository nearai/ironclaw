// Thread event subscription over the app-wide session event stream.
//
// One logical subscription per open thread rides the page's single session
// stream; the server renders every frame body through one shared codec, so
// `useChatEvents` consumes the same tagged WebChatV2Event JSON regardless of
// how it travelled. There is no fallback transport: a stream that cannot
// connect reports `reconnecting` and keeps retrying with capped backoff, and
// durable cursors guarantee nothing is lost while disconnected.
//
// The session client is dynamically imported so its bytes stay out of the
// eager /chat closure.

import React from "react";
import {
  CONNECTION_STATUS,
  type ConnectionStatus,
} from "../lib/connection-status";

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

function transportStatusToConnectionStatus(status: string): ConnectionStatus {
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
}: UseThreadEventsInput): { status: ConnectionStatus } {
  const [status, setStatus] = React.useState<ConnectionStatus>(CONNECTION_STATUS.IDLE);
  const onEventRef = React.useRef(onEvent);
  onEventRef.current = onEvent;

  React.useEffect(() => {
    if (!enabled || !threadId) {
      setStatus(CONNECTION_STATUS.IDLE);
      return undefined;
    }
    let disposed = false;
    let subscription: { unsubscribe: () => void } | null = null;
    setStatus(CONNECTION_STATUS.CONNECTING);
    void import("../../../lib/session-events/client").then(({ sessionEventClient }) => {
      if (disposed) return;
      subscription = sessionEventClient().subscribe(
        { kind: "thread", thread_id: threadId },
        {
          onEvent: ({ cursor, body }) => {
            const type = typeof body.type === "string" ? body.type : "message";
            onEventRef.current({
              // The frame body is the same tagged WebChatV2Event JSON the
              // legacy SSE `data:` line carried; useChatEvents reads it
              // unchanged. Transport errors arrive as subscription_error
              // frames, never as a `stream_error` body.
              type,
              frame: body,
              lastEventId: cursor,
            });
          },
          onError: (error) => {
            if (!error.retryable) {
              // Non-retryable selector failure (revoked access, foreign
              // thread): stop delivering and surface the terminal error.
              setStatus(CONNECTION_STATUS.DISCONNECTED);
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
            // Retryable errors resubscribe inside the client from the last
            // safe cursor; the projection replays durable state, so no
            // consumer action is needed.
          },
          onStatus: (transportStatus) => {
            setStatus(transportStatusToConnectionStatus(transportStatus));
          },
        },
        { idPrefix: `chat-${threadId.slice(0, 24)}` },
      );
    });
    return () => {
      disposed = true;
      subscription?.unsubscribe();
    };
  }, [enabled, threadId]);

  return { status };
}
