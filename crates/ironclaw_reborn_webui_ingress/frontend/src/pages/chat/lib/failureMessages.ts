import { isConnectionLostStatus } from "./connection-status";
import {
  createErrorChatMessage,
  isErrorChatMessage,
  RUN_FAILURE_ID_PREFIX,
} from "./message-types";

export const CONNECTION_LOST_RUN_FAILURE_MESSAGE =
  "Connection to the server was lost. Please reconnect and try again.";
const REQUEST_FAILURE_FALLBACK_MESSAGE =
  "The request failed before it could be sent.";

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

const SENSITIVE_ERROR_MESSAGE_PATTERNS = [
  /\b(?:authorization|bearer|api[_ -]?key|access[_ -]?token|refresh[_ -]?token|secret|password)\b\s*[:=]\s*\S+/i,
  /\b(?:sk-[A-Za-z0-9_-]{8,}|sk-proj-[A-Za-z0-9_-]{8,}|ghp_[A-Za-z0-9_]{8,}|github_pat_[A-Za-z0-9_]{8,}|xox[abprs]-[A-Za-z0-9-]{8,}|AKIA[0-9A-Z]{8,})\b/,
  /\bapi[_ -]?key\b.{0,80}\b[A-Za-z0-9_-]{24,}\b/i,
];

function messageContainsSensitiveCredential(message: string): boolean {
  return SENSITIVE_ERROR_MESSAGE_PATTERNS.some((pattern) =>
    pattern.test(message),
  );
}

export function failureMessageForRequestError(error: unknown): string {
  const message =
    typeof (error as { message?: unknown })?.message === "string"
      ? (error as { message: string }).message.trim()
      : "";
  if (!message) return REQUEST_FAILURE_FALLBACK_MESSAGE;
  return messageContainsSensitiveCredential(message)
    ? REQUEST_FAILURE_FALLBACK_MESSAGE
    : message;
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
  const targetId = `${RUN_FAILURE_ID_PREFIX}${runId}`;
  const next = messages.map((message) => {
    if (!isErrorChatMessage(message)) return message;
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
  const messageId = `${RUN_FAILURE_ID_PREFIX}${runId || "connection-lost"}`;
  const nextMessage = createErrorChatMessage({
    id: messageId,
    content: CONNECTION_LOST_RUN_FAILURE_MESSAGE,
    timestamp: timestamp || new Date().toISOString(),
    failureStatus: "failed",
    failureCategory: "connection_lost",
    failureSummary: CONNECTION_LOST_RUN_FAILURE_MESSAGE,
  });
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
