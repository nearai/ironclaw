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
