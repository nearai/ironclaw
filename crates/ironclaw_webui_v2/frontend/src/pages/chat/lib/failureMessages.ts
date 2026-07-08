export function failureMessageForRunStatus({
  status,
  failureCategory,
  failureSummary,
}) {
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

export function failureMessageForRequestError(error) {
  const message =
    typeof error?.message === "string" ? error.message.trim() : "";
  return message || "The request failed before it could be sent.";
}

export function failureMessageForStreamError({ error, kind, retryable } = {}) {
  const detail = humanizeFailureToken(kind || error || "stream_error");
  return retryable
    ? `The chat stream hit a retryable error: ${detail}.`
    : `The chat stream failed: ${detail}.`;
}

function humanizeFailureToken(token) {
  return String(token)
    .replace(/[_-]+/g, " ")
    .trim()
    .replace(/^\w/, (char) => char.toUpperCase());
}
