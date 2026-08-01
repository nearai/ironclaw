// v2 gate normalization. Source shapes come from
// `WebChatV2Event::Gate { prompt: GatePromptView }` and
// `WebChatV2Event::AuthRequired { prompt: AuthPromptView }`. The
// browser must hold `run_id` + `gate_ref` so a follow-up
// `resolve_gate` call can fill them into the v2 path params.

// Challenge kinds describe interaction modality, not provider:
// `manual_token` is a pasted credential, `oauth_url` is a browser relay, and
// `pairing` presents a host-issued code/deep link/QR flow.

// Optional `ChannelConnectionPromptContext` carried on channel authentication
// gates. Present on the live `auth_required` prompt as `prompt.connection`
// and on the projection gate as `gate.auth_context.connection`. Normalized to
// camelCase so the pairing card can render `{ channel, strategy, instructions,
// inputPlaceholder, submitLabel, errorMessage }` straight off the gate.
function connectionFromContext(connection) {
  if (!connection || typeof connection !== "object") return null;
  const channel = String(connection.channel || "").trim();
  if (!channel) return null;
  return {
    channel,
    strategy: typeof connection.strategy === "string" ? connection.strategy : null,
    instructions:
      typeof connection.instructions === "string" ? connection.instructions : null,
    inputPlaceholder:
      typeof connection.input_placeholder === "string"
        ? connection.input_placeholder
        : null,
    submitLabel:
      typeof connection.submit_label === "string" ? connection.submit_label : null,
    errorMessage:
      typeof connection.error_message === "string" ? connection.error_message : null,
  };
}

export function gateFromEvent(eventType, prompt) {
  if (!prompt) return null;

  if (eventType === "gate") {
    const approvalContext = prompt.approval_context || null;
    const gate = {
      kind: "gate",
      gateKind: "approval",
      runId: prompt.turn_run_id,
      gateRef: prompt.gate_ref,
      invocationId: prompt.invocation_id || null,
      headline: prompt.headline,
      body: prompt.body,
      allowAlways: prompt.allow_always === true,
    };
    return gateWithApprovalContext(gate, approvalContext, prompt.body);
  }
  if (eventType === "auth_required") {
    return {
      kind: "auth_required",
      gateKind: "auth",
      // Legacy auth_required prompts predate challenge_kind and are paste-a-secret
      // prompts. Explicit unknown/other challenge kinds still route to the neutral
      // auth card in chat.tsx.
      challengeKind:
        prompt.challenge_kind ||
        (prompt.provider ||
        prompt.account_label ||
        prompt.authorization_url ||
        prompt.expires_at
          ? "other"
          : "manual_token"),
      // Channel-pairing gates ride the same `manual_token` rail but carry the
      // connection requirement so the frontend renders the pairing card.
      connection: connectionFromContext(prompt.connection),
      runId: prompt.turn_run_id,
      // AuthPromptView carries `auth_request_ref`, but v2's resolve
      // path is `/runs/{run_id}/gates/{gate_ref}/resolve` — auth
      // prompts therefore round-trip through the same gate_ref slot.
      gateRef: prompt.auth_request_ref,
      invocationId: prompt.invocation_id || null,
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

export function gateFromProjectionGate(gate) {
  if (!gate?.run_id || !gate.gate_ref) return null;
  const gateKind = gate.gate_kind || "generic";
  const base = {
    gateKind,
    runId: gate.run_id,
    gateRef: gate.gate_ref,
    invocationId: gate.invocation_id || null,
    headline: gate.headline,
    body: gate.body || "",
    allowAlways: gate.allow_always === true,
  };
  if (gateKind === "auth") {
    const authContext = gate.auth_context || {};
    return {
      ...base,
      kind: "auth_required",
      challengeKind: authContext.challenge_kind || "other",
      provider: authContext.provider || null,
      accountLabel: authContext.account_label || "",
      authorizationUrl: authContext.authorization_url || null,
      expiresAt: authContext.expires_at || null,
      // Present only for channel-pairing gates (see connectionFromContext).
      connection: connectionFromContext(authContext.connection),
    };
  }
  return {
    ...base,
    kind: "gate",
  };
}

// A "channel connection" gate is a host-issued pairing gate that also carries
// the manifest connection context. Both the chat composer affordance
// (`activeThreadHasChannelConnectionGate`) and the pairing-card selector in
// chat.tsx derive from this ONE predicate, so a gate can never be treated as a
// channel-connect by the composer and a token-paste by the card.
//
// Backend invariant (crates/ironclaw_product_workflow/src/auth_prompt.rs):
// `connection` is populated only when `challenge_kind == pairing`, never on
// `manual_token`/`oauth_url`. Requiring the pairing kind here keeps the
// frontend consistent even if that invariant is ever relaxed upstream.
export function channelConnectionFromGate(gate) {
  if (!gate || gate.kind !== "auth_required") return null;
  if (gate.challengeKind !== "pairing") return null;
  return gate.connection || null;
}

function gateWithApprovalContext(gate, approvalContext, fallbackDescription) {
  if (!approvalContext) return gate;
  const approvalDetails = approvalDetailsFromContext(approvalContext);
  return {
    ...gate,
    toolName: approvalContext.tool_name || null,
    description: approvalContext.reason || fallbackDescription,
    actionLabel: approvalContext.action?.label || null,
    destination: approvalContext.destination || null,
    approvalScope: approvalContext.scope || null,
    approvalDetails,
    parameters: approvalDetails.length
      ? approvalDetails.map((detail) => `${detail.label}: ${detail.value}`).join("\n")
      : null,
  };
}

function approvalDetailsFromContext(context) {
  const details = [];
  if (context.action?.label) {
    details.push({ label: "Action", labelKey: "approval.detail.action", value: context.action.label });
  }
  if (context.destination?.label) {
    details.push({ label: "Destination", labelKey: "approval.detail.destination", value: context.destination.label });
  }
  if (context.scope?.label) {
    details.push({ label: "Scope", labelKey: "approval.detail.scope", value: context.scope.label });
  }
  for (const detail of context.details || []) {
    if (!detail?.label || detail.value == null) continue;
    details.push({ label: detail.label, value: String(detail.value) });
  }
  return details;
}
