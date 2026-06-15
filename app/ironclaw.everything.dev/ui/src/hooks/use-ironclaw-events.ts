import { useCallback, useEffect, useRef, useState } from "react";
import type {
  IronclawEventSourceHandle,
  IronclawSseEnvelope,
  IronclawSseStatus,
} from "@/lib/ironclaw-sse";
import { openIronclawEventSource } from "@/lib/ironclaw-sse";

export type { IronclawSseEnvelope, IronclawSseStatus };

const RECONNECT_BASE_MS = 1000;
const RECONNECT_MAX_MS = 30000;

export function useIronclawEvents({
  threadId,
  enabled,
  onEvent,
}: {
  threadId: string | null;
  enabled: boolean;
  onEvent: (envelope: IronclawSseEnvelope) => void;
}): { status: IronclawSseStatus; lastEventIdRef: React.MutableRefObject<string | null> } {
  const [status, setStatus] = useState<IronclawSseStatus>("idle");
  const onEventRef = useRef(onEvent);
  onEventRef.current = onEvent;
  const lastEventIdRef = useRef<string | null>(null);
  const sourceRef = useRef<IronclawEventSourceHandle | null>(null);
  const reconnectTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const reconnectAttemptsRef = useRef(0);

  const connect = useCallback(() => {
    if (!threadId) return;
    setStatus(reconnectAttemptsRef.current > 0 ? "reconnecting" : "connecting");

    const handle = openIronclawEventSource({
      threadId,
      afterCursor: lastEventIdRef.current || undefined,
      onEvent: (envelope) => {
        if (envelope.lastEventId) {
          lastEventIdRef.current = envelope.lastEventId;
        }
        onEventRef.current(envelope);
      },
      onOpen: () => {
        reconnectAttemptsRef.current = 0;
        setStatus("connected");
      },
      onError: (sseStatus) => {
        if (sseStatus === "disconnected") {
          setStatus("disconnected");
          reconnectAttemptsRef.current++;
          const delay = Math.min(
            RECONNECT_BASE_MS * 2 ** reconnectAttemptsRef.current,
            RECONNECT_MAX_MS,
          );
          reconnectTimerRef.current = setTimeout(connect, delay);
        }
      },
    });

    sourceRef.current = handle;
  }, [threadId]);

  useEffect(() => {
    if (!enabled || !threadId) {
      setStatus("idle");
      return;
    }
    lastEventIdRef.current = null;

    connect();

    return () => {
      if (reconnectTimerRef.current) {
        clearTimeout(reconnectTimerRef.current);
        reconnectTimerRef.current = null;
      }
      if (sourceRef.current) {
        sourceRef.current.close();
        sourceRef.current = null;
      }
    };
  }, [enabled, threadId, connect]);

  return { status, lastEventIdRef };
}
