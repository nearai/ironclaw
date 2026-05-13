# ironclaw_outbound guardrails

- Own outbound egress policy, delivery-status metadata, and projection subscription cursor checkpoints only.
- Do not send transport messages, validate concrete Slack/Telegram/Web payloads, or mutate canonical transcript/projection state.
- Persist metadata/refs/cursors only: no raw prompts, message bodies, tool inputs/outputs, secrets, host paths, or backend error details.
- External push targets are candidates only; the outbound policy service must call the reply-target validator before every delivery attempt.
- Authorization-revoked delivery attempts record sanitized failure status and must not return a sendable target.
- Delivery failure records are separate from canonical transcript/projection state and must not mark turns/runs failed.
- Trust-bearing types (`ThreadProjectionAccessGrant`, `ValidatedReplyTargetBinding`) are sealed: only `OutboundPolicyService` mints them via `pub(crate)` constructors. Policy and validator implementors return the corresponding untrusted `Claim` types (`ThreadProjectionAccessClaim`, `ReplyTargetBindingClaim`) and never construct a grant/binding directly. New trust-bearing types added to this crate follow the same claim/seal split, keep fields non-public, and must not derive `Deserialize`; public envelope types that carry them (for example `OutboundDeliveryDecision`) also must not derive `Deserialize`.
- Validator errors are classified at the service boundary: `AccessDenied` records `DeliveryFailureKind::AuthorizationRevoked` (permanent); `Backend`/`Serialization` record `DeliveryFailureKind::TransientValidatorError` (retryable); caller-bug errors (`InvalidRequest`, `SubscriptionScopeMismatch`, `DeliveryNotFound`) propagate to the caller and must not produce a phantom attempt row.
- Failed `OutboundDeliveryAttempt` rows are the structured audit record for outbound denials/transient validator failures. Keep failure kinds sanitized and queryable; do not log raw payloads, prompts, target bodies, or backend error details.
- Rate limiting for validator calls and attempt-row creation belongs at the caller/orchestrator boundary before invoking `OutboundPolicyService`; this crate must not add bypass paths or return sendable targets when upstream validation/rate-limit policy denies.
