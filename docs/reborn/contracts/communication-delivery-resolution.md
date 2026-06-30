# Reborn Contract - Communication Delivery Resolution

**Status:** Contract draft
**Date:** 2026-05-29
**Depends on:** [`events-projections.md`](events-projections.md), [`conversation-binding.md`](conversation-binding.md), [`approvals.md`](approvals.md), [`auth-product.md`](auth-product.md), [`run-state.md`](run-state.md)

---

## 1. Purpose

Communication delivery resolution decides which outbound target should be tried
for a user-visible communication event after Reborn has already determined that
delivery should be attempted.

Candidate selection is part of the `ironclaw_outbound::OutboundPolicyService`
contract. The selection step returns a **candidate only**; the same outbound
service boundary then validates the target, records a delivery attempt, and
hands a validated target to the product outbound path. Transport traffic remains
outside the selection step.

This contract keeps three concepts separate:

- ingress identity: how a message, trigger, or event entered the system;
- execution authority: which tenant/user/thread scope is running;
- communication destination: where the final reply, progress update, approval
  prompt, auth prompt, or delivery-status notice should be attempted.

Delivery resolution is not required for trigger event execution itself. Trigger
polling, trusted ingress, turn submission, and run persistence can proceed
without an outbound target. Delivery resolution is only needed when the system
intends to send the external trigger result or another run notification to a
product surface.

---

## 2. Ownership

| Component | Owns | Does not own |
| --- | --- | --- |
| `ironclaw_outbound::OutboundPolicyService` | Candidate selection, target revalidation, delivery-attempt recording, pre-send gating | Trigger execution, auth/approval state, transport send |
| `ironclaw_outbound::OutboundResolutionEngine` | Optional internal helper for candidate selection inside `OutboundPolicyService` | Public caller boundary, target validation authority, delivery-attempt records |
| `ironclaw_conversations` | Ingress identity, source-route binding, reply-target binding semantics | Outbound policy selection, product-specific reply behavior |
| `ironclaw_event_projections` / `ironclaw_event_streams` | Durable event facts, projection rebuilds, notification/fan-out surfaces | Final outbound target choice, transport send, send authority |
| `ProductAdapter` implementations and transport glue | Rendering after outbound policy approves a candidate; host-provided transport execution | Communication policy selection, durable delivery state |

The resolver must stay host-owned and deterministic. Product adapters can
describe capabilities, but they do not get to define the resolver's policy
language or inject product-specific behavior into the contract.

---

## 3. Contract Invariants

1. Outbound candidate selection returns a `CommunicationDeliveryCandidate` only.
2. The candidate is not authority. It still must pass
   `OutboundPolicyService` validation before any send.
3. The resolver must never collapse ingress identity, execution authority, and
   communication destination into one field or string.
4. Trigger event execution does not depend on delivery resolution. Only the
   external delivery of a trigger result uses this contract.
5. The resolver must not encode product-specific behavior such as "Web UI can
   show approval cards" or "Telegram cannot do gate prompts". Capabilities are
   evaluated later at the outbound policy boundary.
6. P0 rule order is fixed and deterministic.
7. If the selected target is unavailable, revoked, unauthorized, or otherwise
   invalid, the system fails closed and does not silently fall back to another
   channel.
8. Implicit fallback is not part of P0. A future fallback must be modeled as an
   explicit ordered policy rule with tests.

---

## 4. Resolution Input

The outbound service uses one typed resolution envelope so callers cannot smuggle
unrelated auth, approval, or transport fields into the request while the
implementation still has one public outbound API surface.

```rust
pub struct CommunicationDeliveryResolutionRequest {
    scope: TurnScope,
    actor: TurnActor,
    modality: CommunicationModality,
    intent: CommunicationDeliveryIntent,
}

pub enum CommunicationDeliveryKind {
    FinalReply,
    ProgressUpdate,
    DeliveryStatus,
    ApprovalPrompt,
    AuthPrompt,
}

pub enum CommunicationDeliveryIntent {
    RequestedOutbound(RequestedOutboundContext),
    RunNotification(RunNotificationContext),
}

pub struct RequestedOutboundContext {
    requested_target: ReplyTargetBindingRef,
    requested_kind: RequestedOutboundKind,
}

pub enum RequestedOutboundKind {
    ProductMessage,
    DeliveryStatus,
}
```

Requested outbound is explicit command intent. Run notification is lifecycle
policy for final replies, progress updates, approval prompts, auth prompts, and
delivery-status notices.

`RequestedOutboundContext.requested_target` is a typed reply-target binding candidate,
not a raw channel, adapter string, product-specific conversation id, or
transport address. `RequestedOutboundKind` is intentionally narrower than the
run-notification delivery kinds and excludes approval/auth prompt payloads.
The requested outbound target is still only a candidate and must pass
`OutboundPolicyService` validation for the current scope, actor, derived
delivery kind, and modality before any send. `CommunicationDeliveryResolutionRequest`
derives its delivery kind from `intent`; callers must not provide a separate
top-level kind that could contradict the request branch.

```rust
pub struct RunNotificationContext {
    event_kind: RunNotificationEventKind,
    origin: RunNotificationOrigin,
}

pub enum RunNotificationOrigin {
    LiveSourceRoute { source_route: SourceRouteContext },
    Triggered { trigger: TriggerCommunicationContext },
    TriggeredFromSourceRoute {
        trigger: TriggerCommunicationContext,
        source_route: SourceRouteContext,
    },
    SystemEvent { reason: SystemEventReasonCode },
}
```

`SourceRouteContext` carries the validated reply target for a live inbound
conversation. `TriggerCommunicationContext` identifies the trigger fire without
turning that trigger into a communication destination. The trigger reference is
an outbound-local correlation value; the canonical trigger identity belongs to
the future `ironclaw_triggers::TriggerId` in PR 9.

`SystemEventReasonCode` is a stable, redacted enum/code. Human-readable backend
details, raw tool input, prompt material, OAuth state, approval payloads, and
transport errors do not enter the resolution request. If a product surface needs
display text, it receives a separately redacted display payload after the target
has been selected and validated.

---

## 5. Preference Fields

User communication preferences are owned by an
`ironclaw_outbound::CommunicationPreferenceRepository` backed by a dedicated
typed tenant/user database table. They are not stored in the generic JSON
settings store and are not profile/tone preferences.

The V1 preference row is keyed by `(tenant_id, user_id)` and may contain these
optional `ReplyTargetBindingRef` candidate fields:

- `final_reply_target`: default target for ordinary run results when no live
  source route applies;
- `progress_target`: target for progress updates when the live source route is
  absent or cannot validate for progress;
- `approval_prompt_target`: exact-owner target for approval prompts;
- `auth_prompt_target`: exact-owner target for auth prompts;
- `default_modality`: preferred modality when a caller does not specify a more
  specific supported modality.

Preference fields are candidates only. The outbound service must revalidate
tenant ownership, exact owner where required, target capability, delivery kind,
and modality before recording a delivery attempt.

---

## 6. P0 Rule Order

The first matching rule yields the only candidate.

1. **Authority-bearing prompts use exact-owner prompt targets.** Approval and
   auth prompts use `approval_prompt_target` / `auth_prompt_target`
   preferences only when outbound validation confirms the target supports the
   requested prompt kind and belongs to the exact owner. Shared/group widening
   is forbidden for these payloads. `RequestedOutbound` cannot choose an
   independent approval/auth prompt target; if present for these delivery
   kinds, it may only narrow to the same exact-owner prompt target after
   validation.
2. **Explicit requested outbound wins for non-authority delivery.** If a caller
   explicitly requests an outbound target for a non-approval and non-auth
   delivery kind, the resolver returns that typed target as the candidate.
3. **Live source route wins for ordinary run notifications.** If the run
   descended from a real inbound product message, final replies and supported
   progress/status updates prefer the live source route's reply target.
4. **Triggered-from-source-route uses the live source route.** If the run
   descended from both a trigger and a live source route, ordinary final
   replies, progress updates, and delivery-status notices prefer the live
   source route over the trigger creator's preferred target.
5. **Triggered preferred target wins for ordinary trigger results without a
   live source route.** If the run descended from a trigger and has no live
   source route, final replies prefer the creator user's configured
   `final_reply_target`.
6. **System events have no implicit external target in P0.** `SystemEvent`
   origins require `RequestedOutbound` for external delivery. Without an
   explicit requested target, the event is recorded as delivery metadata only
   and no external send is attempted.

Delivery-status notifications follow the same origin rule as the delivery they
describe. Progress updates use the source route when that route validates for
progress; otherwise they use `progress_target` when configured. Unsupported
progress, approval, or auth delivery is recorded as delivery metadata only; it
must not resume work or change approval/auth state.

The resolver does not keep searching after a target fails validation. If the
first eligible candidate is unavailable or revoked, the result is failure, not
an automatic hop to some other channel.

---

## 7. Validation Boundary

Validation and delivery-attempt recording remain in `ironclaw_outbound`.

The flow is:

```text
OutboundPolicyService candidate-selection step
  -> returns CommunicationDeliveryCandidate
OutboundPolicyService
  -> validates target and capability scope
  -> records delivery attempt
  -> returns validated target or rejection
Product adapter / host transport
  -> renders through adapter and sends through host-owned transport only after validation
```

The validator owns the final answer for whether the candidate still belongs to
the current tenant/user/scope and whether the target supports the requested
modality and notification kind.

---

## 8. Trigger Delivery Boundary

Trigger loops are not blocked on outbound delivery resolution. A trigger can
fire, execute, and persist its run even if no external delivery path is
available yet.

When a trigger result must be delivered externally, the resolver treats it as a
communication event, not as trigger authority. Trigger identity stays in the
trigger domain; outbound destination choice stays in `ironclaw_outbound`.

---

## 9. Non-Goals

This contract does not define:

- transport-specific rendering;
- product UI behavior;
- subscription fan-out policy;
- auth-flow creation or callback handling;
- approval resolution or lease semantics;
- trigger scheduling, polling, or execution orchestration.

Those responsibilities stay with their owning contracts and services.
