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
      runId: prompt.turn_run_id,
      // AuthPromptView carries `auth_request_ref`, but v2's resolve
      // path is `/runs/{run_id}/gates/{gate_ref}/resolve` — auth
      // prompts therefore round-trip through the same gate_ref slot.
      gateRef: prompt.auth_request_ref,
      provider: prompt.provider || "github",
      accountLabel: prompt.account_label || "Manual token",
      headline: prompt.headline,
      body: prompt.body,
    };
  }

  return null;
}
