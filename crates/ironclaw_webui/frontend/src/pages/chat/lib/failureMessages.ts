import { isConnectionLostStatus } from "./connection-status";
import {
  createErrorChatMessage,
  isErrorChatMessage,
  RUN_FAILURE_ID_PREFIX,
} from "./message-types";

export const CONNECTION_LOST_RUN_FAILURE_KEY = "chat.failure.connectionLost";
const REQUEST_FAILURE_FALLBACK_KEY = "chat.failure.request";

type Translate = (
  key: string,
  params?: Record<string, string | number>,
) => string;

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
}: RunFailureMessageInput, t: Translate): string {
  if (
    shouldPreferConnectionLostRunFailure({
      failureCategory,
      failureSummary,
      connectionStatus,
      connectionInterrupted,
    })
  ) {
    return t(CONNECTION_LOST_RUN_FAILURE_KEY);
  }
  if (typeof failureSummary === "string" && failureSummary.trim()) {
    return failureSummary.trim();
  }
  if (typeof failureCategory === "string" && failureCategory.trim()) {
    return t("chat.failure.runCategory", {
      detail: failureCategory.trim().replaceAll("_", " "),
    });
  }
  return status === "recovery_required"
    ? t("chat.failure.recoveryRequired")
    : t("chat.failure.run");
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

function isClientGeneratedRequestMessage(
  error: unknown,
  message: string,
): boolean {
  if (!message) return true;
  if (messageContainsSensitiveCredential(message)) return true;
  if (typeof error !== "object" || error === null) return false;

  const requestError = error as {
    name?: unknown;
    body?: unknown;
    payload?: unknown;
    statusText?: unknown;
  };
  if (requestError.name === "TypeError" || requestError.name === "AbortError") {
    return true;
  }
  if (requestError.name !== "ApiError") return false;

  if (requestError.payload && typeof requestError.payload === "object") {
    return true;
  }
  const body = normalizeText(requestError.body);
  if (body && !body.startsWith("{") && !body.startsWith("[")) {
    return false;
  }
  const statusText = normalizeText(requestError.statusText);
  return !body || message === statusText || message === "Request failed";
}

export function failureMessageForRequestError(
  error: unknown,
  t: Translate,
): string {
  const message =
    typeof (error as { message?: unknown })?.message === "string"
      ? (error as { message: string }).message.trim()
      : "";
  return isClientGeneratedRequestMessage(error, message)
    ? t(REQUEST_FAILURE_FALLBACK_KEY)
    : message;
}

export function failureMessageForStreamError(
  { error, kind, retryable }: StreamFailureMessageInput,
  t: Translate,
): string {
  const detail = humanizeFailureToken(kind || error || "stream_error");
  return retryable
    ? t("chat.failure.streamRetryable", { detail })
    : t("chat.failure.stream", { detail });
}

function humanizeFailureToken(token: unknown): string {
  return String(token)
    .replace(/[_-]+/g, " ")
    .trim()
    .replace(/^\w/, (char) => char.toUpperCase());
}

export function rewriteConnectionLostRunFailures(
  messages: any,
  {
    runId,
    t,
  }: { runId?: string | null; t: Translate },
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
    }, t);
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
    t,
  }: {
    runId?: string | null;
    timestamp?: string | null;
    t: Translate;
  },
) {
  if (!Array.isArray(messages)) return messages;
  const messageId = `${RUN_FAILURE_ID_PREFIX}${runId || "connection-lost"}`;
  const content = t(CONNECTION_LOST_RUN_FAILURE_KEY);
  const nextMessage = createErrorChatMessage({
    id: messageId,
    content,
    timestamp: timestamp || new Date().toISOString(),
    failureStatus: "failed",
    failureCategory: "connection_lost",
    failureSummary: content,
  });
  const existing = messages.findIndex((message) => message?.id === messageId);
  if (existing < 0) return [...messages, nextMessage];
  if (messages[existing]?.content === content) {
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
