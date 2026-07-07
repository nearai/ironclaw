import { isConnectionLostStatus } from "./connection-status.js";

export const CONNECTION_LOST_RUN_FAILURE_MESSAGE =
  "Connection to the server was lost. Please reconnect and try again.";

function normalizeText(value) {
  return typeof value === "string" ? value.trim() : "";
}

function normalizeLowerText(value) {
  return normalizeText(value).toLowerCase();
}

function hasConnectionLostContext({ connectionStatus, connectionInterrupted }) {
  return connectionInterrupted === true || isConnectionLostStatus(connectionStatus);
}

function isDriverUnavailableFailure({ failureCategory, failureSummary }) {
  const category = normalizeLowerText(failureCategory);
  if (category === "driver_unavailable") return true;

  const summary = normalizeLowerText(failureSummary);
  return (
    (summary.includes("execution driver") &&
      summary.includes("temporarily unavailable")) ||
    summary.includes("driver unavailable")
  );
}

function shouldPreferConnectionLostRunFailure(input) {
  return hasConnectionLostContext(input) && isDriverUnavailableFailure(input);
}

export function failureMessageForRunStatus({
  status,
  failureCategory,
  failureSummary,
  connectionStatus,
  connectionInterrupted,
}) {
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

export function rewriteConnectionLostRunFailures(messages, { runId } = {}) {
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
