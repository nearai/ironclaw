import type { ApiClient } from "@/app";

type StreamEventIter = Awaited<ReturnType<ApiClient["ironclaw"]["threads"]["streamEvents"]>>;
type StreamEvent = StreamEventIter extends AsyncIterable<infer T> ? T : never;

export type { StreamEvent };

export const V2_EVENT_NAMES = [
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
] as const;

export type IronclawSseStatus =
  | "idle"
  | "connecting"
  | "connected"
  | "reconnecting"
  | "paused"
  | "disconnected";

export interface IronclawSseEnvelope {
  event: StreamEvent;
  lastEventId: string | null;
}

export interface OpenIronclawEventSourceOptions {
  threadId: string;
  afterCursor?: string;
  onEvent: (envelope: IronclawSseEnvelope) => void;
  onOpen?: () => void;
  onError?: (status: IronclawSseStatus) => void;
}

export interface IronclawEventSourceHandle {
  close: () => void;
}

export function openIronclawEventSource({
  threadId,
  afterCursor,
  onEvent,
  onOpen,
  onError,
}: OpenIronclawEventSourceOptions): IronclawEventSourceHandle {
  const url = new URL(
    `/api/ironclaw/threads/${encodeURIComponent(threadId)}/events`,
    window.location.origin,
  );
  if (afterCursor) {
    url.searchParams.set("afterCursor", afterCursor);
  }

  const es = new EventSource(url.toString());

  es.onopen = () => {
    onOpen?.();
  };

  es.onerror = () => {
    onError?.(es.readyState === EventSource.CLOSED ? "disconnected" : "reconnecting");
  };

  const dispatchFrame = (event: MessageEvent, fallbackType: string) => {
    let frame: Record<string, unknown> | null = null;
    try {
      frame = JSON.parse(event.data);
    } catch {
      return;
    }
    if (!frame || typeof frame !== "object") return;

    onEvent({
      event: {
        type: (frame.type as string) || fallbackType,
        ...frame,
      } as StreamEvent,
      lastEventId: event.lastEventId || null,
    });
  };

  const handlers = new Map<string, (event: Event) => void>();
  for (const name of V2_EVENT_NAMES) {
    const handler = (event: Event) => dispatchFrame(event as MessageEvent, name);
    handlers.set(name, handler);
    es.addEventListener(name, handler);
  }

  es.onmessage = (event) => {
    dispatchFrame(event as MessageEvent, "message");
  };

  return {
    close: () => {
      for (const [name, handler] of handlers) {
        es.removeEventListener(name, handler);
      }
      es.onopen = null;
      es.onmessage = null;
      es.onerror = null;
      es.close();
    },
  };
}
