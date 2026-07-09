export const CHAT_MESSAGE_ROLES = Object.freeze({
  USER: "user",
  ASSISTANT: "assistant",
  SYSTEM: "system",
  ERROR: "error",
  TOOL_ACTIVITY: "tool_activity",
  THINKING: "thinking",
  IMAGE: "image",
} as const);

export type ChatMessageRole =
  (typeof CHAT_MESSAGE_ROLES)[keyof typeof CHAT_MESSAGE_ROLES];

export const RUN_FAILURE_ID_PREFIX = "err-";
export const REQUEST_FAILURE_ID_PREFIX = "err-request-";
export const STREAM_FAILURE_ID_PREFIX = "err-stream-";
export const UNKNOWN_RUN_FAILURE_ID = `${RUN_FAILURE_ID_PREFIX}unknown`;

export type ChatMessage = {
  id?: string;
  role?: ChatMessageRole;
  content?: unknown;
  timestamp?: string;
  [key: string]: unknown;
};

export type ErrorChatMessage = {
  id: string;
  role: typeof CHAT_MESSAGE_ROLES.ERROR;
  content: string;
  timestamp: string;
  failureStatus?: string | null;
  failureCategory?: string | null;
  failureSummary?: string | null;
  [key: string]: unknown;
};

export type ErrorChatMessageInput = {
  id: string;
  content: string;
  timestamp: string;
  failureStatus?: string | null;
  failureCategory?: string | null;
  failureSummary?: string | null;
  [key: string]: unknown;
};

export function createErrorChatMessage(
  input: ErrorChatMessageInput,
): ErrorChatMessage {
  return {
    ...input,
    role: CHAT_MESSAGE_ROLES.ERROR,
  };
}

export function isErrorChatMessage(
  message: unknown,
): message is ErrorChatMessage {
  return (
    typeof message === "object" &&
    message !== null &&
    (message as ChatMessage).role === CHAT_MESSAGE_ROLES.ERROR
  );
}

export function safeMessageIdToken(value: unknown): string {
  return String(value || "unknown").replace(/[^a-z0-9_-]+/gi, "-");
}

export function requestFailureIdForMessage(messageId: unknown): string {
  return `${REQUEST_FAILURE_ID_PREFIX}${safeMessageIdToken(messageId)}`;
}

export function isRunFailureMessageId(value: unknown): boolean {
  const id = typeof value === "string" ? value : "";
  return (
    id.startsWith(RUN_FAILURE_ID_PREFIX) &&
    !id.startsWith(REQUEST_FAILURE_ID_PREFIX) &&
    !id.startsWith(STREAM_FAILURE_ID_PREFIX)
  );
}
