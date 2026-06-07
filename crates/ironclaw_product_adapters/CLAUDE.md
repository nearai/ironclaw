# ironclaw_product_adapters guardrails

Owns the product-surface adapter boundary for IronClaw Reborn (issue #3269).

- This crate defines the **ProductAdapter** contract: typed inbound/outbound
  parsed event DTOs, capability descriptors, host-mediated protocol
  authentication evidence, constrained protocol HTTP egress, delivery status
  reporting, and projection-derived outbound payloads. It does **not**
  implement any specific protocol (Telegram, Slack, Web, CLI). Concrete
  adapters live in their own crates/components.
- Stay above the kernel/dispatcher layer. Do **not** depend on or re-export
  raw `CapabilityHost`, `RuntimeDispatcher`, runtime lanes, process spawning,
  raw network clients, raw secrets, raw filesystem mounts, or
  `ironclaw_turns::runner` trusted transition APIs. Boundary tests in
  `tests/product_adapter_contract.rs` enforce this with cargo metadata and
  source scans.
- Adapters do not resolve canonical user/thread ids and do not call
  `TurnCoordinator` directly. The product workflow facade (`ProductWorkflow`)
  is the only path adapters use into the inbound pipeline; the workflow itself
  binds external refs to canonical scope, resolves projection subscriptions,
  stages attachments, and submits via `TurnCoordinator`.
- Adapters return `ParsedProductInbound` from `parse_inbound`. Trusted fields
  (`ProductAdapterId`, `AdapterInstallationId`, verified auth claim, and
  `received_at`) are host-stamped through `TrustedInboundContext` before a
  `ProductInboundEnvelope` reaches workflow code. Ignored authenticated events
  must be represented as `ProductInboundPayload::NoOp`, not dropped with an
  out-of-band `None` path.
- Inbound DTOs carry structured external refs only: actor, conversation,
  event, attachment descriptors, optional reply-target/action hints. Raw
  protocol payloads are normalized by the adapter; raw bytes/secrets/host
  paths must not appear in envelopes or externally surfaced errors. Validated
  DTOs must validate both constructors and serde deserialization.
- External refs separate stable identity from presentation/reply metadata:
  actor equality excludes display name; conversation equality/fingerprints
  exclude reply-target hints and use collision-resistant length-prefixed
  segments.
- Outbound envelopes are projection-derived and carry a resolved
  `ProductOutboundTarget` plus a single projection cursor source of truth.
  Projection snapshot/update payloads must contain renderable
  `ProductProjectionState`, not cursor-only metadata. Push-channel adapters
  report delivery via `OutboundDeliverySink`; synchronous surfaces use the
  explicit render outcome/response types.
- `ProtocolAuthEvidence::Verified` is **sealed** for production code. Public
  production APIs do not expose host-verification constructors; only
  test-support builds expose `ProtocolAuthEvidence::test_verified` for fakes.
  WASM components and adapter implementations may declare auth requirements
  and inspect evidence; they must not fabricate verification.
- `ProductAdapter::auth_requirement` and `ProductAdapter::declared_egress`
  are host-visible control-plane metadata. Host glue must build protocol-auth
  and egress policy from these typed declarations, not from side tables.
- `ProtocolHttpEgress` is the only network path adapters may use. Requests use
  validated method/path/header types, preserve duplicate request headers, and
  reject host/authorization/cookie-style headers owned by the host. Responses
  expose status/body only; raw response headers remain host-only so reflected
  credential material cannot cross back into adapters.
- Delivery failures are best-effort and recorded as `DeliveryStatus`. Use
  `FailedRetryable`, `FailedPermanent`, and `FailedUnauthorized` distinctly;
  sinks dedupe by `DeliveryAttemptId`.
- Approval/auth gate UX is deferred to #3094, but inbound placeholder payloads
  must carry typed resolution action/result data so future interaction flows do
  not need a breaking pseudo-message path.

Tests:

- Unit tests in `src/**/mod tests {}` cover each DTO's validation/redaction.
- Boundary tests in `tests/product_adapter_contract.rs` ensure crate
  dependencies stay within the allowlist and that no canonical-state shortcut
  paths exist.
- `tests/review_findings_contract.rs` pins the PR-review security/correctness
  regressions so future contract edits cannot reintroduce them.
