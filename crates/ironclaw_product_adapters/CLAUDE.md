# ironclaw_product_adapters guardrails

Owns the product-surface adapter boundary for IronClaw Reborn (issue #3269).

- This crate defines the **ProductAdapter** contract: typed inbound/outbound
  envelopes, capability descriptors, host-mediated protocol authentication
  evidence, constrained protocol HTTP egress, and the projection-derived
  outbound payload shape. It does **not** implement any specific protocol
  (Telegram, Slack, Web, CLI). Concrete adapters live in their own crates and
  components.
- Stay above the kernel/dispatcher layer. Do **not** depend on or re-export
  raw `CapabilityHost`, `RuntimeDispatcher`, runtime lanes, process spawning,
  raw network clients, raw secrets, raw filesystem mounts, or
  `ironclaw_turns::runner` trusted transition APIs. Boundary tests in
  `tests/product_adapter_contract.rs` enforce this.
- Adapters do not resolve canonical user/thread ids and do not call
  `TurnCoordinator` directly. The product workflow facade (`ProductWorkflow`)
  is the only path adapters use into the inbound pipeline; the workflow itself
  binds external refs to canonical scope and submits via `TurnCoordinator`.
- Inbound envelopes carry structured external refs only: actor, conversation,
  event, attachment descriptors, optional reply-target hints. Raw protocol
  payloads are normalized by the adapter; raw bytes/secrets/host paths must
  not appear in the envelope or in errors.
- Outbound envelopes are projection-derived. Adapters consume `FinalReplyView`
  / `ProgressUpdateView` / `GatePromptView` / `AuthPromptView` /
  `ProjectionSnapshot` / `ProjectionUpdate` and protocol-translate them. They
  never own canonical thread/run state.
- `ProtocolAuthEvidence::Verified` is **sealed**. Only the host can mint a
  `Verified` evidence by going through `ProtocolAuthEvidence::host_verified`,
  which requires a `HostAuthSeal` value that adapters cannot construct. WASM
  components and adapter implementations may only declare auth requirements
  and inspect evidence; they cannot fabricate verification.
- `ProtocolHttpEgress` is the only egress path adapters may use. The host
  resolves credential handles at request time, scans responses for leaks,
  and reports delivery status via `OutboundDeliverySink`.
- Delivery failures are best-effort and recorded as `DeliveryStatus`. They do
  not mutate canonical transcript/projection state.
- Approval/auth gate UX is deferred to #3094. `GatePromptView` /
  `AuthPromptView` are present so adapters can render placeholder prompts in
  fake contract tests, but production resolution flows live in the
  interaction services.

Tests:

- Unit tests in `src/**/mod tests {}` cover each DTO's validation/redaction.
- Boundary tests in `tests/product_adapter_contract.rs` ensure crate
  dependencies stay within the allowlist and that no canonical-state
  shortcut paths exist.
