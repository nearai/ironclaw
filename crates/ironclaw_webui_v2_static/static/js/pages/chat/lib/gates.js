// v2 gate normalization. Source shapes come from
// `WebChatV2Event::Gate { prompt: GatePromptView }` and
// `WebChatV2Event::AuthRequired { prompt: AuthPromptView }`. The
// browser must hold `run_id` + `gate_ref` so a follow-up
// `resolve_gate` call can fill them into the v2 path params.
export function gateFromEvent(eventType, prompt) {
  if (!prompt) return null;

  if (eventType === "gate") {
    return {
      kind: "gate",
      runId: prompt.turn_run_id,
      gateRef: prompt.gate_ref,
      headline: prompt.headline,
      body: prompt.body,
    };
  }

  if (eventType === "auth_required") {
    return {
      kind: "auth_required",
      // challenge_kind is populated by the Rust projection layer when an
      // auth-flow record exists for this gate (issue #4112). Falls back to
      // "manual_token" so the existing AuthTokenCard renders for legacy rows
      // and for cases where no flow record is wired.
      challengeKind: prompt.challenge_kind || "manual_token",
      runId: prompt.turn_run_id,
      // AuthPromptView carries `auth_request_ref`, but v2's resolve
      // path is `/runs/{run_id}/gates/{gate_ref}/resolve` — auth
      // prompts therefore round-trip through the same gate_ref slot.
      gateRef: prompt.auth_request_ref,
      provider: prompt.provider || "unknown",
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
