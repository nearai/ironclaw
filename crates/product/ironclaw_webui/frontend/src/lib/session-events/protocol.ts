// Wire vocabulary for the webui.session_event.v1 session event stream.
//
// The stream is an event transport, not a command bus: the client sends the
// subscription set once, in the request body, and nothing afterwards. Server
// frames all carry the schema tag so unknown vocabularies can be ignored.

export const SESSION_EVENT_SCHEMA = "webui.session_event.v1";

// Mirrors the closed `ProductStreamSelector` vocabulary in
// `ironclaw_product_contracts::surface`: a selector the server cannot admit
// must not be representable here.
export type SessionSelector =
  | { kind: "thread"; thread_id: string }
  | { kind: "run_completions" };

export type SessionSubscriptionRequest = {
  subscription_id: string;
  selector: SessionSelector;
  after_cursor: string | null;
};

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
};

/** The JSON body of `POST /api/webchat/v2/session/events`. */
export function sessionEventsRequestBody(
  subscriptions: SessionSubscriptionRequest[],
): string {
  return JSON.stringify({ subscriptions });
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
