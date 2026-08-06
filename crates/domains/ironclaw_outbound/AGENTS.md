# ironclaw_outbound — working rules

Orientation (what this crate is, surface, deps, tests) lives in
[`README.md`](./README.md); the family boundary in
[`../AGENTS.md`](../AGENTS.md). This file is the canonical crate-local rules —
consolidated 2026-08-05 from the former `CLAUDE.md` guardrails (now a pointer)
per `docs/reborn/guidance-conventions.md` rule 1.
`docs/reborn/contracts/events-projections.md` is the source of truth for
outbound egress/subscription policy.

## Rules

- Own outbound egress policy, delivery-status metadata, and projection
  subscription cursor checkpoints only.
- Do not send transport messages, validate concrete Slack/Telegram/Web
  payloads, or mutate canonical transcript/projection state. Delivery failure
  records are separate from canonical transcript/projection state and must
  not mark turns/runs failed.
- Persist metadata/refs/cursors only: no raw prompts, message bodies, tool
  inputs/outputs, secrets, host paths, or backend error details (the store
  tests pin that backend error detail never leaks).
- External push targets are candidates only; the outbound policy service must
  call the reply-target validator before every delivery attempt.
- `OutboundResolutionEngine` is the read-only candidate selector above
  `OutboundPolicyService`; it resolves typed delivery requests into
  `CommunicationDeliveryCandidate` or `NoDelivery`, but never validates
  targets, records attempts, or mutates inbound/approval/auth state.
- Authorization-revoked delivery attempts record sanitized failure status
  (`DeliveryFailureKind::AuthorizationRevoked`) and must not return a
  sendable target.
- **Claim/seal split.** Trust-bearing types (`ThreadProjectionAccessGrant`,
  `ValidatedReplyTargetBinding`) are sealed: only `OutboundPolicyService`
  mints them via `pub(crate)` constructors. Policy and validator implementors
  return the corresponding untrusted `Claim` types
  (`ThreadProjectionAccessClaim`, `ReplyTargetBindingClaim`) and never
  construct a grant/binding directly. New trust-bearing types follow the same
  split, keep fields non-public, and must not derive `Deserialize`; public
  envelope types that carry them (e.g. `OutboundDeliveryDecision`) also must
  not derive `Deserialize`.
- Validator errors are classified at the service boundary with an exhaustive
  `match`: `AccessDenied` records `AuthorizationRevoked` (permanent);
  `Backend`/`Serialization` record `TransientValidatorError` (retryable);
  caller-bug errors (`InvalidRequest`, `SubscriptionScopeMismatch`,
  `DeliveryNotFound`) propagate and must not produce a phantom attempt row.
- Delivery candidates carry their tenant/agent/project/thread identity, and
  `OutboundPolicyService::prepare_delivery_attempt` must reject any
  scope/candidate identity mismatch before validator I/O or store writes.
- Failed `OutboundDeliveryAttempt` rows are the structured audit record for
  outbound denials/transient validator failures. Keep failure kinds sanitized
  and queryable.
- Rate limiting for validator calls and attempt-row creation belongs at the
  caller/orchestrator boundary before invoking `OutboundPolicyService`; this
  crate must not add bypass paths or return sendable targets when upstream
  validation/rate-limit policy denies.
- Prefer service-level tests when policy gates subscription, delivery,
  persistence, or authorization side effects.

## Validation

- Fast local check: `cargo test -p ironclaw_outbound`
- Contract suites: `outbound_policy_service_contract`,
  `outbound_state_store_contract`
- Backend parity without live Postgres:
  `IRONCLAW_SKIP_POSTGRES_TESTS=1 cargo test -p ironclaw_outbound --all-features`
- Lint: `cargo clippy -p ironclaw_outbound --all-targets --all-features -- -D warnings`
- Boundary check after dependency/API changes:
  `cargo test -p ironclaw_architecture_tests reborn_crate_dependency_boundaries_hold`
