export const CONVERSATION_EVENT_NAMES = [
  "snapshot",
  "messages_changed",
  "message_added",
  "run_finished",
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
  "keep_alive",
  "error",
] as const;

export type ConversationEventName = (typeof CONVERSATION_EVENT_NAMES)[number];

export type IronclawSseStatus =
  | "idle"
  | "connecting"
  | "connected"
  | "reconnecting"
  | "paused"
  | "disconnected";

export interface IronclawSseEnvelope {
  event: {
    type: ConversationEventName | string;
    [key: string]: unknown;
  };
  lastEventId: string | null;
}

export interface OpenIronclawEventSourceOptions {
  threadId: string;
  afterCursor?: string;
  onEvent: (envelope: IronclawSseEnvelope) => void;
  onSnapshot?: (data: unknown) => void;
  onUpdate?: (data: unknown) => void;
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
  onSnapshot,
  onUpdate,
  onOpen,
  onError,
}: OpenIronclawEventSourceOptions): IronclawEventSourceHandle {
  const url = new URL(
    `/api/conversation/threads/${encodeURIComponent(threadId)}/events`,
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

    const type = (frame.type as string) || fallbackType;
    const envelope: IronclawSseEnvelope = {
      event: { type, ...frame },
      lastEventId: event.lastEventId || null,
    };

    if (type === "snapshot") {
      onSnapshot?.(frame);
    } else if (type === "messages_changed") {
      onUpdate?.(frame);
    }

    onEvent(envelope);
  };

  const handlers = new Map<string, (event: Event) => void>();
  for (const name of CONVERSATION_EVENT_NAMES) {
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
