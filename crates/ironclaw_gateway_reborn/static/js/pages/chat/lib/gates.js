export function normalizeHistoryGate(gate) {
  if (!gate) return null;
  const gateName = gate.gateName || gate.gate_name;
  const resumeKind = gate.resume_kind || gate.resumeKind || {};

  return {
    kind: gate.kind || (gateName === "approval" ? "legacy" : "gate"),
    requestId: gate.requestId || gate.request_id,
    threadId: gate.threadId || gate.thread_id,
    gateName,
    toolName: gate.toolName || gate.tool_name,
    description: gate.description,
    parameters: gate.parameters,
    extensionName: gate.extensionName || gate.extension_name,
    allowAlways: gate.allowAlways ?? resumeKind?.Approval?.allow_always ?? true,
  };
}
