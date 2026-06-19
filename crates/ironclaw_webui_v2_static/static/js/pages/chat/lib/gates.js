// v2 gate normalization. Source shapes come from
// `WebChatV2Event::Gate { prompt: GatePromptView }` and
// `WebChatV2Event::AuthRequired { prompt: AuthPromptView }`. The
// browser must hold `run_id` + `gate_ref` so a follow-up
// `resolve_gate` call can fill them into the v2 path params.
export function gateFromEvent(eventType, prompt) {
  if (!prompt) return null;

  if (eventType === "gate") {
    const approvalContext = prompt.approval_context || null;
    const approvalDetails = approvalContext
      ? approvalDetailsFromContext(approvalContext)
      : [];
    const gate = {
      kind: "gate",
      runId: prompt.turn_run_id,
      gateRef: prompt.gate_ref,
      invocationId: prompt.invocation_id || null,
      headline: prompt.headline,
      body: prompt.body,
      allowAlways: prompt.allow_always === true,
    };
    if (!approvalContext) return gate;
    return {
      ...gate,
      toolName: approvalContext.tool_name || null,
      description: approvalContext.reason || prompt.body,
      actionLabel: approvalContext.action?.label || null,
      destination: approvalContext.destination || null,
      approvalScope: approvalContext.scope || null,
      approvalDetails,
      parameters: approvalDetails.length
        ? approvalDetails.map((detail) => `${detail.label}: ${detail.value}`).join("\n")
        : null,
    };
  }
  if (eventType === "auth_required") {
    return {
      kind: "auth_required",
      // Legacy auth_required prompts predate challenge_kind and are manual
      // token prompts. Explicit unknown/other challenge kinds still route to
      // the neutral auth card in chat.js.
      challengeKind:
        prompt.challenge_kind ||
        (prompt.provider ||
        prompt.account_label ||
        prompt.authorization_url ||
        prompt.expires_at
          ? "other"
          : "manual_token"),
      runId: prompt.turn_run_id,
      // AuthPromptView carries `auth_request_ref`, but v2's resolve
      // path is `/runs/{run_id}/gates/{gate_ref}/resolve` — auth
      // prompts therefore round-trip through the same gate_ref slot.
      gateRef: prompt.auth_request_ref,
      // Falls back to null when unpopulated; components render a generic
      // label rather than a misleading provider name.
      provider: prompt.provider || null,
      // Falls back to empty string so auth-token-card subtitle is hidden
      // when not set; card falls back to provider label if non-null.
      accountLabel: prompt.account_label || "",
      // Only present for oauth_url challenges:
      authorizationUrl: prompt.authorization_url || null,
      expiresAt: prompt.expires_at || null,
      headline: prompt.headline,
      body: prompt.body,
    };
  }

  return null;
}
function approvalDetailsFromContext(context) {
  const details = [];
  if (context.action?.label) {
    details.push({ label: "Action", value: context.action.label });
  }
  if (context.destination?.label) {
    details.push({ label: "Destination", value: context.destination.label });
  }
  if (context.scope?.label) {
    details.push({ label: "Scope", value: context.scope.label });
  }
  for (const detail of context.details || []) {
    if (!detail?.label || detail.value == null) continue;
    details.push({ label: detail.label, value: String(detail.value) });
  }
  return details;
}
