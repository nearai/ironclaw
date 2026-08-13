// Wire vocabulary for the owner-scoped run-completion stream and its HTTP
// operations (2026-08-13 design §7.6–§7.8). Mirrors
// `ironclaw_product_contracts::run_completions`; every mutation rides
// authenticated HTTP, never the read-only session socket.

export const RUN_COMPLETION_NOTICE_SCHEMA = "webui.run_completion.v1";
export const RUN_COMPLETION_GRANT_SCHEMA = "webui.run_completion_grant.v1";
export const RUN_COMPLETION_CLEAR_SCHEMA = "webui.run_completion_clear.v1";

export type RunCompletionNotice = {
  schema: string;
  sequence: string;
  notice_id: string;
  run_id: string;
  thread_id: string;
  thread_tag: string;
  completed_at: string;
  read: boolean;
  unread_count_for_thread: number;
};

export type RunCompletionGrant = {
  schema: string;
  sequence: string;
  notice_id: string;
  grant_id: string;
  browser_instance_id: string;
  state_revision: number;
  surface: "no_surface_watching_thread" | "in_app" | "local_os";
  expires_at: string;
};

export type RunCompletionClear = {
  schema: string;
  sequence: string;
  notice_id: string;
  thread_id: string;
  thread_tag: string;
  read_at: string;
};

export type RunCompletionStreamEvent =
  | { type: "notice"; notice: RunCompletionNotice }
  | { type: "grant"; grant: RunCompletionGrant }
  | { type: "clear"; clear: RunCompletionClear };

export type RunCompletionIntentKind =
  | "reply_observed"
  | "watching_thread"
  | "in_app"
  | "local_os"
  | "unavailable";

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === "object" && !Array.isArray(value);
}

function stringField(record: Record<string, unknown>, key: string): string | null {
  const value = record[key];
  return typeof value === "string" && value.length > 0 ? value : null;
}

// The session socket wraps run-completion events as
// `{ type: "run_completion", event: { type: "notice" | ..., ...fields } }`.
export function parseRunCompletionEvent(
  body: Record<string, unknown>,
): RunCompletionStreamEvent | null {
  if (body.type !== "run_completion") return null;
  const event = body.event;
  if (!isRecord(event)) return null;
  switch (event.type) {
    case "notice": {
      const notice = noticeFromWire(event);
      return notice ? { type: "notice", notice } : null;
    }
    case "grant": {
      if (
        !stringField(event, "notice_id") ||
        !stringField(event, "grant_id") ||
        !stringField(event, "browser_instance_id") ||
        typeof event.state_revision !== "number" ||
        typeof event.surface !== "string"
      ) {
        return null;
      }
      return { type: "grant", grant: event as unknown as RunCompletionGrant };
    }
    case "clear": {
      if (!stringField(event, "notice_id") || !stringField(event, "thread_id")) {
        return null;
      }
      return { type: "clear", clear: event as unknown as RunCompletionClear };
    }
    default:
      // Newer server vocabulary: ignore rather than fail the subscription.
      return null;
  }
}

export function noticeFromWire(value: unknown): RunCompletionNotice | null {
  if (!isRecord(value)) return null;
  if (
    !stringField(value, "notice_id") ||
    !stringField(value, "run_id") ||
    !stringField(value, "thread_id") ||
    !stringField(value, "sequence")
  ) {
    return null;
  }
  return {
    schema: stringField(value, "schema") ?? RUN_COMPLETION_NOTICE_SCHEMA,
    sequence: String(value.sequence),
    notice_id: String(value.notice_id),
    run_id: String(value.run_id),
    thread_id: String(value.thread_id),
    thread_tag: stringField(value, "thread_tag") ?? "",
    completed_at: stringField(value, "completed_at") ?? "",
    read: value.read === true,
    unread_count_for_thread:
      typeof value.unread_count_for_thread === "number"
        ? value.unread_count_for_thread
        : 1,
  };
}

/** Numeric compare for opaque decimal sequence strings (u64 range). */
export function compareSequences(a: string, b: string): number {
  if (a.length !== b.length) return a.length - b.length;
  return a < b ? -1 : a > b ? 1 : 0;
}
