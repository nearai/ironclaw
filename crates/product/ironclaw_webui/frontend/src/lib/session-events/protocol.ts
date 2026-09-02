// Wire vocabulary for the webui.session_event.v1 session socket.
//
// The socket is an event transport, not a command bus: the complete client
// vocabulary is subscribe / unsubscribe / ping. Server frames all carry the
// schema tag so unknown vocabularies can be ignored by older clients.

export const SESSION_EVENT_SCHEMA = "webui.session_event.v1";

// Mirrors the closed `ProductStreamSelector` vocabulary in
// `ironclaw_product_contracts::surface`: a selector the server cannot admit
// must not be representable here.
export type SessionSelector =
  | { kind: "thread"; thread_id: string }
  | { kind: "run_completions" };

export type SessionServerFrame = {
  schema?: string;
  type?: string;
  subscription_id?: string;
  generation?: number;
  cursor?: string | null;
  event?: Record<string, unknown>;
  error?: string;
  kind?: string;
  retryable?: boolean;
  last_cursor?: string | null;
  reason?: string;
  violation?: string;
};

export function subscribeFrame(
  subscriptionId: string,
  selector: SessionSelector,
  afterCursor: string | null,
): string {
  return JSON.stringify({
    type: "subscribe",
    subscription_id: subscriptionId,
    selector,
    after_cursor: afterCursor,
  });
}

export function unsubscribeFrame(subscriptionId: string): string {
  return JSON.stringify({ type: "unsubscribe", subscription_id: subscriptionId });
}

export function pingFrame(): string {
  return JSON.stringify({ type: "ping" });
}

export function parseServerFrame(raw: unknown): SessionServerFrame | null {
  if (typeof raw !== "string") return null;
  try {
    const frame = JSON.parse(raw);
    if (!frame || typeof frame !== "object") return null;
    if (frame.schema !== SESSION_EVENT_SCHEMA) return null;
    return frame as SessionServerFrame;
  } catch (_) {
    return null;
  }
}
