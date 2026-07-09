import { isConnectionLostStatus } from "./connection-status.js";

export const CONNECTION_LOST_RUN_FAILURE_MESSAGE =
  "Connection to the server was lost. Please reconnect and try again.";

type RunFailureMessageInput = {
  status?: string | null;
  failureCategory?: string | null;
  failureSummary?: string | null;
  connectionStatus?: string | null;
  connectionInterrupted?: boolean | null;
};

function normalizeText(value: unknown): string {
  return typeof value === "string" ? value.trim() : "";
}

function normalizeLowerText(value: unknown): string {
  return normalizeText(value).toLowerCase();
}

function hasConnectionLostContext({
  connectionStatus,
  connectionInterrupted,
}: RunFailureMessageInput): boolean {
  return connectionInterrupted === true || isConnectionLostStatus(connectionStatus);
}

function isDriverUnavailableFailure({
  failureCategory,
}: RunFailureMessageInput): boolean {
  const category = normalizeLowerText(failureCategory);
  return category === "driver_unavailable";
}

function shouldPreferConnectionLostRunFailure(
  input: RunFailureMessageInput,
): boolean {
  return hasConnectionLostContext(input) && isDriverUnavailableFailure(input);
}

export function failureMessageForRunStatus({
  status,
  failureCategory,
  failureSummary,
  connectionStatus,
  connectionInterrupted,
}: RunFailureMessageInput = {}): string {
  if (
    shouldPreferConnectionLostRunFailure({
      failureCategory,
      failureSummary,
      connectionStatus,
      connectionInterrupted,
    })
  ) {
    return CONNECTION_LOST_RUN_FAILURE_MESSAGE;
  }
  if (typeof failureSummary === "string" && failureSummary.trim()) {
    return failureSummary.trim();
  }
  if (typeof failureCategory === "string" && failureCategory.trim()) {
    return `The run failed: ${failureCategory.trim().replaceAll("_", " ")}.`;
  }
  return status === "recovery_required"
    ? "The run is awaiting recovery — backend reported `recovery_required`."
    : "The run failed before producing a reply.";
}

type StreamFailureMessageInput = {
  error?: string | null;
  kind?: string | null;
  retryable?: boolean | null;
};

export function failureMessageForRequestError(error: any): string {
  const message =
    typeof error?.message === "string" ? error.message.trim() : "";
  return message || "The request failed before it could be sent.";
}

export function failureMessageForStreamError(
  { error, kind, retryable }: StreamFailureMessageInput = {},
): string {
  const detail = humanizeFailureToken(kind || error || "stream_error");
  return retryable
    ? `The chat stream hit a retryable error: ${detail}.`
    : `The chat stream failed: ${detail}.`;
}

function humanizeFailureToken(token: unknown): string {
  return String(token)
    .replace(/[_-]+/g, " ")
    .trim()
    .replace(/^\w/, (char) => char.toUpperCase());
}

export function rewriteConnectionLostRunFailures(
  messages: any,
  { runId }: { runId?: string | null } = {},
) {
  if (!Array.isArray(messages)) return messages;
  if (!runId) return messages;
  let changed = false;
  const targetId = `err-${runId}`;
  const next = messages.map((message) => {
    if (!message || message.role !== "error") return message;
    if (message.id !== targetId) return message;

    const content = failureMessageForRunStatus({
      status: message.failureStatus || "failed",
      failureCategory: message.failureCategory,
      failureSummary: message.failureSummary || message.content,
      connectionInterrupted: true,
    });
    if (content === message.content) return message;
    changed = true;
    return { ...message, content };
  });
  return changed ? next : messages;
}

export function upsertConnectionLostRunFailure(
  messages: any,
  {
    runId,
    timestamp,
  }: { runId?: string | null; timestamp?: string | null } = {},
) {
  if (!Array.isArray(messages)) return messages;
  const messageId = `err-${runId || "connection-lost"}`;
  const nextMessage = {
    id: messageId,
    role: "error",
    content: CONNECTION_LOST_RUN_FAILURE_MESSAGE,
    timestamp: timestamp || new Date().toISOString(),
    failureStatus: "failed",
    failureCategory: "connection_lost",
    failureSummary: CONNECTION_LOST_RUN_FAILURE_MESSAGE,
  };
  const existing = messages.findIndex((message) => message?.id === messageId);
  if (existing < 0) return [...messages, nextMessage];
  if (messages[existing]?.content === CONNECTION_LOST_RUN_FAILURE_MESSAGE) {
    return messages;
  }
  const next = [...messages];
  next[existing] = {
    ...messages[existing],
    ...nextMessage,
    timestamp: messages[existing].timestamp || nextMessage.timestamp,
  };
  return next;
}
