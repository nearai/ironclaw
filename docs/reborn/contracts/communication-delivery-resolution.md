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
| `ChannelAdapter` implementations and transport glue | Rendering after outbound policy approves a candidate; host-provided transport execution | Communication policy selection, durable delivery state |

The resolver must stay host-owned and deterministic. Channel adapters can
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
6. The resolution rule (§6) is fixed and deterministic.
7. If the selected target is unavailable, revoked, unauthorized, or otherwise
   invalid, the system fails closed and does not silently fall back to another
   channel.
8. Implicit fallback is not part of the resolution rule. A future fallback
   must be modeled as an explicit ordered policy rule with tests.

---

## 4. Resolution Input

> **Rewritten 2026-08-04 (Task 17).** §§4-6 previously described a
> preference-slot resolution model deleted by this branch's channel-delivery
> work (Tasks 9-13): trigger-specific origin variants, a precedence chain over
> four per-purpose preference fields, and a "P0 rule order" search. The text
> below describes the landed model. See
> `crates/ironclaw_outbound/src/delivery_resolution.rs` and
> `resolution_engine.rs` for the live shape.

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
    ModelDelivery,
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

pub enum RunNotificationEventKind {
    FinalReplyReady,
    ProgressUpdate,
    ApprovalNeeded,
    AuthRequired,
    RunBlocked,
    DeliveryStatus,
    /// An explicit model-initiated delivery (`builtin.outbound_deliver`).
    /// Carries its own `CommunicationDeliveryKind::ModelDelivery` so attempts
    /// stay distinguishable in the durable audit trail and per-run accounting.
    ModelDelivery,
}

pub enum RunNotificationOrigin {
    LiveSourceRoute { source_route: SourceRouteContext },
    /// One host-sealed target for this run: a model-chosen catalog target for
    /// `ModelDelivery`, or one entry of a background run's
    /// notification-channel-set fan-out for `ApprovalNeeded` / `AuthRequired`
    /// / `RunBlocked` / `DeliveryStatus` when no live source route exists.
    /// Revalidated at egress.
    RunScopedTarget { target: ReplyTargetBindingRef },
    SystemEvent { reason: SystemEventReasonCode },
}
```

`SourceRouteContext` carries the validated reply target for a live inbound
conversation. `RunScopedTarget` is the one surviving per-target origin, and it
resolves verbatim: no preference lookup runs at all (§5, §6). Every
background-run notification and every explicit model delivery resolves
through this arm. There is no dedicated trigger-communication context, no
per-purpose preference-target field, and no precedence chain between them —
the caller that builds the `RunNotificationContext` (the background-run
notifier, or the `builtin.outbound_deliver` tool handler) has already decided
the one target this notification uses; the resolver's job is only to read it
back out, unmodified, as the candidate.

`ModelDelivery` can never legitimately carry `LiveSourceRoute`: that origin
means "reply where the inbound message came from," which is definitionally
the run's own conversation, and an explicit delivery targeting the run's own
conversation is denied before resolution ever runs (the tool's same-origin
check). `ModelDelivery` always carries the model's chosen `RunScopedTarget`.

`SystemEventReasonCode` is a stable, redacted enum/code. Human-readable backend
details, raw tool input, prompt material, OAuth state, approval payloads, and
transport errors do not enter the resolution request. If a product surface needs
display text, it receives a separately redacted display payload after the target
has been selected and validated.

---

## 5. Notification Targets

*(Renamed from "Preference Fields." The four per-purpose preference slots this
section used to describe — `final_reply_target`, `progress_target`,
`approval_prompt_target`, `auth_prompt_target` — are retired. Nothing writes a
per-purpose target any more: delivery is a model-called tool, never a stored
implicit route.)*

User communication defaults are owned by an
`ironclaw_outbound::CommunicationPreferenceRepository` backed by a dedicated
typed tenant/user database table (`CommunicationPreferenceRecord`). They are
not stored in the generic JSON settings store and are not profile/tone
preferences.

The record is keyed by scope (`DeliveryDefaultScope`, effectively
`(tenant_id, user_id)`) and carries:

- `notification_targets: Vec<OutboundDeliveryTargetId>` — an explicit,
  user-configured **set** of 0..8 catalog targets (`NOTIFICATION_TARGETS_CAP`)
  that receive gate prompts, auth prompts, and failure notices for a
  background/routine run with no live source route. Empty means "notification
  channels are the web app only" — there is no dedicated in-app pseudo-target
  to configure instead.
- `legacy_notification_target: Option<ReplyTargetBindingRef>` — read-migration
  input only (serialized under the historical wire name `final_reply_target`
  so a pre-migration row still deserializes). Nothing writes it.
  `CommunicationPreferenceRecord::effective_notification_target_ids` folds it
  into the notification set only when `notification_targets` is empty.
- `default_modality: Option<CommunicationModality>`.

There are no `progress_target`, `approval_prompt_target`, or
`auth_prompt_target` fields any more, and no precedence chain between them:
`OutboundResolutionEngine` (§6) reads no stored preference at all. A run's
final reply, progress update, approval prompt, auth prompt, and
delivery-status notice each resolve from the live source route or an explicit
`RunScopedTarget` the caller already picked — never from a stored per-purpose
slot.

Stored notification targets are candidates only. The outbound service must
revalidate tenant ownership, target capability, delivery kind, and modality
before recording a delivery attempt.

Product-facing reads/writes are descriptor-backed ProductSurface capabilities:
a `get_notification_channels` view projects the caller's effective
notification-target set (folding the legacy slot per above), and
`builtin.notification_channels_set` (model-callable, approval-gated,
full-replace CAS write) and the WebUI notification-channels panel's product command are the only two write surfaces — both enter the same validated `set_notification_channels` service path (owner-scoped id validation, dedup, cap). There is no
`get_outbound_preferences` / `set_outbound_preferences` facade any more — both
were deleted along with the preference slots they served.
`outbound_delivery_targets` remains the descriptor-backed view for the
caller-scoped target inventory. Writes remain side-effecting and must move
through the capability path.

---

## 6. Resolution Rule

The candidate is a direct read of the caller-supplied intent, not a search
through stored fallbacks. `OutboundResolutionEngine::resolve` is a single
match over `CommunicationDeliveryIntent`:

1. **`RequestedOutbound` returns the caller's explicit target verbatim.** The
   candidate's target is `requested_target`; its kind derives from
   `requested_kind` (`ProductMessage` → `FinalReply`, `DeliveryStatus` →
   `DeliveryStatus`). No other rule is consulted.
2. **`RunNotification` reads the target straight out of `origin`:**
   - `LiveSourceRoute` → the source route's reply target. Used whenever the
     run descended from a real inbound product message and this event
     supports replying where the message came from (a live conversation's
     final reply, progress update, gate prompt, or auth prompt).
   - `RunScopedTarget` → the sealed target verbatim, for every event kind. A
     background run's gate/auth/failure notices, and every explicit
     `ModelDelivery`, arrive this way — the caller already resolved the exact
     target (a notification-channel-set entry, or the model's chosen catalog
     target) before asking the resolver.
   - `SystemEvent` → no candidate; the reason code is recorded as delivery
     metadata only, and no external send is attempted.

There is no implicit fallback chain and no per-purpose preference lookup:
every rule above is a direct field read, not a search. If the caller building
the request has no target to offer for a given event, it constructs
`SystemEvent` rather than asking the resolver to invent one.

The resolver does not keep searching after it reads a target out this way. If
the returned candidate later fails validation (unavailable, revoked,
unauthorized), the result is failure, not an automatic hop to some other
channel — the caller decides whether and how to retry with a different
target.

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
Channel adapter / host transport
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
