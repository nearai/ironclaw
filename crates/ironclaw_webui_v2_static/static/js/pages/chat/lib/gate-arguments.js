// Join an approval gate to the tool-activity card it gates, strictly by
// `invocationId`, and surface that activity's arguments on the gate so the
// approval card shows what is being approved.
//
// This replaces an earlier render-time scan that, when the exact invocation
// had no arguments yet, fell back to "the latest parameterized activity for
// this run". That fallback mis-attributed arguments under concurrency (two
// tools gated in the same run would show each other's arguments). The gate
// and its activity now share an `invocationId`, so the join is exact: no
// match means no enrichment.
export function enrichApprovalGateWithActivityArguments(gate, messages) {
  if (!gate || gate.kind !== "gate" || !gate.invocationId) return gate;
  const activity = findActivityForInvocation(messages, gate.invocationId);
  const argumentsText =
    displayText(activity?.toolParameters) || displayText(activity?.toolDetail) || null;
  if (!argumentsText) return gate;

  const approvalDetails = Array.isArray(gate.approvalDetails) ? gate.approvalDetails : [];
  if (approvalDetails.some((detail) => isArgumentsDetail(detail?.label))) {
    return gate.parameters ? gate : { ...gate, parameters: argumentsText };
  }
  return {
    ...gate,
    approvalDetails: [
      ...approvalDetails,
      { label: "Arguments", value: argumentsText },
    ],
    parameters: gate.parameters || argumentsText,
  };
}

function findActivityForInvocation(messages, invocationId) {
  for (const message of messages || []) {
    if (message?.role === "tool_activity" && message.invocationId === invocationId) {
      return message;
    }
    const nested = (message?.toolCalls || []).find(
      (tool) => tool?.invocationId === invocationId,
    );
    if (nested) return nested;
  }
  return null;
}

function displayText(value) {
  return typeof value === "string" && value.trim() ? value.trim() : null;
}

function isArgumentsDetail(label) {
  const normalized = typeof label === "string" ? label.trim().toLowerCase() : "";
  return normalized === "arguments" || normalized === "parameters";
}
