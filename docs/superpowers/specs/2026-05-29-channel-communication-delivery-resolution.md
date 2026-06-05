# Channel / Communication Delivery Resolution - Design

**Date:** 2026-05-29
**Status:** Design draft
**Target architecture:** IronClaw Reborn (`crates/ironclaw_*`)
**Related specs:** companion trigger-loop draft in nearai/ironclaw#3874,
`docs/reborn/contracts/events-projections.md`,
`docs/reborn/contracts/approvals.md`,
`docs/reborn/contracts/conversation-binding.md`

## 1. Purpose

Define the layer that decides where user-visible communication should go after a
Reborn run emits an event.

This layer answers: given the run scope, actor, event kind, origin, and
modality, which outbound target should IronClaw try? The answer is a
**candidate only**. `ironclaw_outbound` still owns reply-target validation and
delivery-attempt records before any transport sends.

This design keeps three concepts separate:

- **Ingress identity:** how a message or trigger entered IronClaw.
- **Execution authority:** which tenant/user/agent/project scope runs the turn.
- **Communication destination:** where final replies, progress, or approval
  notifications should be delivered.

## 2. Existing Owners

| Area | Owner | Notes |
| --- | --- | --- |
| Inbound binding and replay | `ironclaw_conversations` | Owns `InboundTurnRequest`, `ExternalConversationRef`, `external_event_id`, reply-target binding semantics. |
| Run execution events | agent loop / run-state / event-stream crates | Emit final reply, progress, blocked approval/auth/resource states. |
| Approval resolution | `ironclaw_approvals` | Resolves durable pending approvals into scoped leases. It does not prompt users or route UI. |
| Outbound policy and validation | `ironclaw_outbound` | Owns notification policy state, `ReplyTargetBindingRef` validation, delivery attempts, sanitized failure records. |
| Product transport rendering | product adapters / channels | Web UI, Telegram, etc. render/send after outbound policy authorizes a target. |

Current implementation caveat: `ironclaw_outbound` is presently a policy and
delivery-attempt preflight boundary, not the concrete sender. P0 delivery
resolution applies only to Reborn product-adapter outbound paths such as
`ProductAdapter::render_outbound` and adapter-declared outbound capabilities.
Legacy channel/helper send paths are out of scope for this design until they are
ported behind the Reborn product-adapter outbound boundary. Slack has not yet
been ported to Reborn in this branch, so Slack-specific requested outbound is
not a P0 acceptance criterion.

## 3. Proposed Layer

Add `ironclaw_outbound::OutboundResolutionEngine` inside the outbound policy
boundary. Every Reborn product-adapter outbound decision enters this engine
before transport rendering. P0 is a **small deterministic rule engine**, not a
user-authored or general-purpose rules platform: it applies an ordered set of
host-owned policy rules to already-canonical run events, explicit outbound
requests, and preferences, then hands the selected candidate to existing
outbound validation.

```text
Run event
  FinalReplyReady | ProgressUpdate | ApprovalNeeded | AuthRequired | RunBlocked
        |
        v
ironclaw_outbound::OutboundResolutionEngine
  entrypoints:
    resolve_requested_outbound(RequestedOutboundResolutionRequest)
    resolve_run_notification(RunNotificationResolutionRequest)
  reads: tenant/user communication preferences
  applies: ordered P0 communication rules
        |
        v
CommunicationDeliveryCandidate
  target: ReplyTargetBindingRef
  kind: final_reply | progress | approval_needed | auth_required | delivery_status
        |
        v
ironclaw_outbound::OutboundPolicyService
  revalidates target
  records delivery attempt
  returns sealed ValidatedReplyTargetBinding or rejection
        |
        v
Product outbound adapter / transport
```

`ironclaw_outbound` owns this engine because the existing Reborn event /
projection contract already assigns outbound notification policy state,
reply-target validation, delivery attempts, and sanitized failure records to
that crate. The resolution engine must not live in `ironclaw_triggers`,
`ironclaw_conversations`, or `ironclaw_approvals`. Reborn composition wires the
engine and store dependencies, but does not own the policy semantics.

The resolution engine is read-only with respect to ingress and authority. It
must not mutate `ProductInboundEnvelope`, `ExternalConversationRef`, inbound
dedupe state, `AcceptedMessageRef`, `SourceBindingRef`,
`ReplyTargetBindingRef`, pending-gate records, auth-flow records, approval
records, leases, or turn submission behavior.

The engine is not a replacement for `ThreadNotificationPolicy` or
`OutboundPolicyService`. `ThreadNotificationPolicy` remains the existing
thread-scoped policy model for configured push targets and may be extended when
the current target-kind model is insufficient. `OutboundResolutionEngine` is the
decision layer above that policy model: it decides which candidate should be
attempted for a particular outbound intent, then delegates validation and
delivery-attempt recording to `OutboundPolicyService`.

The engine exposes typed entrypoints instead of one catch-all request:

- `resolve_requested_outbound` handles explicit product-adapter send intent and
  requires a requested target.
- `resolve_run_notification` handles run-lifecycle notifications and receives
  only the canonical origin facts needed for policy resolution.

Both entrypoints return `CommunicationDeliveryCandidate`, and every candidate
must pass through `OutboundPolicyService::prepare_delivery_attempt` before any
product adapter renders or sends externally.

### P0 Rule Engine

P0 rule order is locked:

1. **Explicit requested outbound wins.** When a Reborn product-adapter
   capability path is explicitly trying to send a product message to a requested
   outbound target — for example "send a test message on Telegram" through the
   Telegram v2 adapter — communication resolution follows that requested
   target. The target is still only a candidate and must pass outbound
   validation before any transport sends.
2. **Live inbound loops reply to their source route.** When the run descends
   from a real inbound product/channel message, final replies prefer the same
   source route's `reply_target_binding_ref`.
3. **Triggered loops use preferred outbound.** When the run descends from a
   trigger and has no live source route, final replies prefer the creator
   user's configured outbound target.

If the selected target is unavailable, unauthorized, unbound, or lacks required
capabilities, P0 fails closed: outbound records a sanitized delivery failure and
does not silently fall back to another channel. A future policy version may add
an explicit `FallbackToDefault` rule, but that fallback must be modeled as an
ordered policy rule with tests and must still revalidate the fallback target.

## 4. Resolution Inputs

The engine needs enough context to distinguish requested outbound from
run-lifecycle notification, live chat from triggered jobs, text from voice, and
final replies from approval/auth prompts. Use typed request shapes rather than a
single broad tagged enum.

### Requested Outbound Request

```rust
RequestedOutboundResolutionRequest {
    scope: TurnScope,
    actor: TurnActor,
    requested: RequestedOutboundContext,
    modality: CommunicationModality,
}
```

Requested outbound is command intent: the caller is asking to send a product
message to a specific target. This request is invalid without a requested target.

### Run Notification Request

```rust
RunNotificationResolutionRequest {
    scope: TurnScope,
    actor: TurnActor,
    event_kind: RunNotificationEventKind,
    modality: CommunicationModality,
    origin: RunNotificationOrigin,
}
```

Run notification is lifecycle policy: the run emitted a final reply, progress
update, approval/auth prompt, blocked state, or delivery-status update, and the
engine decides where notification should be attempted.

### Run Notification Event Kind

```rust
enum RunNotificationEventKind {
    FinalReplyReady,
    ProgressUpdate,
    ApprovalNeeded,
    AuthRequired,
    RunBlocked,
    DeliveryStatus,
}
```

### Run Notification Origin

```rust
enum RunNotificationOrigin {
    LiveSourceRoute {
        source_route: SourceRouteContext,
    },
    Triggered {
        trigger: TriggerCommunicationContext,
    },
    TriggeredFromSourceRoute {
        trigger: TriggerCommunicationContext,
        source_route: SourceRouteContext,
    },
    SystemEvent {
        reason: String,
    },
}
```

This shape is a contract invariant: requested outbound uses
`RequestedOutboundResolutionRequest` and requires an explicit requested target;
live user-message notifications require source-route context; trigger-origin
notifications require trigger context; future message-regex trigger
notifications can normalize to `TriggeredFromSourceRoute` because they are
caused by a real product message but execute a stored trigger rule. Callers must
not pass an under-specified request.

The request intentionally does not carry `PendingGate`, pending auth IDs,
approval request IDs, credential names, OAuth callback data, or raw product
payloads. Those identifiers stay with their owning auth, approval, gateway, or
product-workflow paths.

### Requested Outbound Context

Use only when a capability/tool path is explicitly asking to send a message to a
specific outbound target:

```rust
RequestedOutboundContext {
    requested_target: ReplyTargetBindingRef,
    requested_kind: RequestedOutboundKind,
}

enum RequestedOutboundKind {
    ProductMessage,
    DeliveryStatus,
}
```

`requested_target` is a candidate, not send authority. It must still pass
`ironclaw_outbound` validation before transport send. This context must not
carry raw message payloads, credentials, channel tokens, OAuth state, or backend
error details.

### Modality

```rust
enum CommunicationModality {
    Text,
    Voice,
    Image,
    Mixed,
    Unknown,
}
```

### Source Route Context

Use only when the event descends from a real product/channel message:

```rust
SourceRouteContext {
    adapter_kind,
    adapter_installation_id,
    external_actor_ref,
    external_conversation_ref,
    reply_target_binding_ref,
}
```

The `reply_target_binding_ref` is an outbound candidate, not authority. It still
requires `ironclaw_outbound` revalidation before send.

### Trigger Context

Use when the event descends from a trigger fire:

```rust
TriggerCommunicationContext {
    trigger_id,
    trigger_source_kind,
    fire_slot,
}
```

Trigger context must not be encoded into `ExternalConversationRef`, `TurnActor`,
or `adapter_kind` as a communication destination.

## 5. Preferences and Overrides

Users need changeable persisted defaults for normal communication and approvals.
`ironclaw_outbound` owns this configuration because it owns notification policy,
reply-target validation, delivery attempts, and sanitized failure records. Store
preferences by tenant and user:

```rust
CommunicationPreference {
    tenant_id,
    user_id,
    final_replies_target: Option<ReplyTargetBindingRef>,
    progress_target: Option<ReplyTargetBindingRef>,
    approval_prompt_target: Option<ReplyTargetBindingRef>,
    auth_prompt_target: Option<ReplyTargetBindingRef>,
    default_modality: Option<CommunicationModality>,
    updated_at,
    updated_by,
}
```

This record is durable user configuration, not an inbound binding and not a
grant of send authority. The stored `ReplyTargetBindingRef` values are
candidates that must be revalidated at send time. `updated_by` records the user
or tenant admin that last changed the preference.

`CommunicationPreference` is not a replacement for the existing
`ThreadNotificationPolicy`. Preferences are user-level defaults used by
`OutboundResolutionEngine` when a run has no live source route or explicit
requested target. `ThreadNotificationPolicy` remains the thread-scoped policy
shape for configured push targets. Implementation may either derive direct
candidates from preferences for P0, or lower preferences into a
`ThreadNotificationPolicy` for a concrete thread when durable thread policy is
needed; either path must still revalidate through `OutboundPolicyService`.

Trigger records do not carry delivery configuration in the first trigger slice.
Triggered jobs use the creator user's `final_replies_target` by default. A
future per-trigger override may be added only as an override consumed by the
outbound resolution engine:

```rust
enum TriggerDeliveryOverride {
    DefaultUserNotification,
    ExplicitReplyTarget(ReplyTargetBindingRef),
    LocalThreadOnly,
}
```

Rules:

- Per-user defaults are changeable by the owning user or tenant admin.
- Per-trigger delivery overrides, when added, are changeable by the trigger
  creator or tenant admin.
- Trigger delivery overrides affect final replies and delivery status only.
- Trigger delivery overrides must not grant approval authority or bypass the
  approval resolver.
- Stored targets are candidates only; every send revalidates through
  `ironclaw_outbound`.

## 6. Resolution Rules

Resolution is an ordered rule engine. The first matching rule yields a single
candidate; validation and send preparation happen afterward through
`OutboundPolicyService`.

### Explicit Requested Outbound

`resolve_requested_outbound` honors the requested target:

```text
RequestedOutbound:
  requested.requested_target
```

If validation rejects the requested target, P0 fails closed and records a
sanitized delivery failure. It does not fall back to source-route or user-default
delivery. A future `FallbackToDefault` rule may be added as an explicit policy
extension.

### Live User Message

For `RunNotificationOrigin::LiveSourceRoute`, prefer replying to the source route
when the source route has a reply target:

```text
FinalReplyReady:
  source_route.reply_target_binding_ref
  else preference.final_replies_target

ProgressUpdate:
  source_route.reply_target_binding_ref if channel supports progress
  else preference.progress_target

ApprovalNeeded:
  preference.approval_prompt_target if target supports gate prompts
  else normal run-state/event path only

AuthRequired:
  source_route.reply_target_binding_ref if target supports auth prompts
  else preference.auth_prompt_target if target supports auth prompts
  else normal auth-flow path only
```

### Triggered Job

For `RunNotificationOrigin::Triggered`, there is no live origin conversation
to reply to:

```text
FinalReplyReady:
  preference.final_replies_target
  else LocalThreadOnly

ProgressUpdate:
  preference.progress_target only if enabled

ApprovalNeeded:
  preference.approval_prompt_target only if target supports gate prompts
  else record ApprovalBlocked / RunBlocked through normal run-state/event path

AuthRequired:
  preference.auth_prompt_target only if target supports auth prompts
  else record auth-required state through normal auth-flow path
```

### Webhook Trigger

Webhook trigger delivery normalizes to `RunNotificationOrigin::Triggered` and
follows triggered-job rules unless the trigger definition eventually carries an
explicit delivery override. The webhook request itself must not be treated as a
trusted reply destination.

### Message Regex Trigger

Message-regex triggers observe an already-normalized inbound product message.
They normalize to `RunNotificationOrigin::TriggeredFromSourceRoute`. They may
default to the originating conversation only if a future trigger delivery
override says so and the source route's reply target revalidates.

## 7. Target Capabilities

The resolution engine must not hard-code product-specific behavior such as "Web
UI supports approval cards" or "Telegram does not support gate prompts." Target
capability knowledge belongs at the product-adapter / outbound validation
boundary.

```rust
DeliveryTargetCapabilities {
    final_replies: bool,
    progress: bool,
    gate_prompts: bool,
    auth_prompts: bool,
    modalities: Vec<CommunicationModality>,
}
```

`ReplyTargetBindingValidator` or a sibling outbound capability port returns the
capabilities for the candidate `ReplyTargetBindingRef` after validating that the
target still belongs to the requested tenant/user/scope. The engine may use
capabilities to suppress unsupported push kinds, but it must not infer support
from channel names.

### Modality

Modality is a policy input:

- Text input can reply as text by default.
- Voice input may prefer text or voice according to user preference and target
  capabilities.
- Unknown or unsupported modality falls back to text final reply if allowed.
- Modality must not change approval authority.

## 8. Approval Delivery

Approval notification and approval resolution are different contracts.
The intended end state still supports product-channel approvals: if a user's
default approval target is Telegram and that validated target supports gate
prompts plus approval-resolution inbound events, the user can receive the prompt
in Telegram, approve there, and the existing approval resolver resumes the exact
parked invocation.

On approval-needed:

```text
CapabilityHost returns RequireApproval
  -> ApprovalRequestStore saves Pending request
  -> RunState marks BlockedApproval
  -> EventStream publishes approval_needed
  -> ironclaw_outbound::OutboundResolutionEngine may notify approval_prompt_target
  -> Product adapter renders GatePrompt if validated capabilities allow it
  -> Product inbound receives ApprovalResolution
  -> ApprovalResolver handles approve/deny
  -> CapabilityHost resumes exact invocation using fingerprinted lease
```

The outbound resolution engine may choose where to notify the user, but it does not
approve, deny, mint leases, or resume runtime work.

Security rules:

- Approval resolution remains tenant/user/agent scoped.
- Approval resume validates the exact invocation fingerprint.
- Notifications use redacted display metadata only.
- Raw tool input, secrets, host paths, and backend error details must not be
  sent in approval notifications.
- If the validated target capabilities do not include gate prompts, do not
  emulate approval with normal chat text.
- If the target cannot produce a scoped `ApprovalResolution` inbound event,
  notify-only delivery is allowed, but approval must still happen through a
  supported approval surface before the invocation resumes.
- Product examples: current Telegram v2 drops `GatePrompt` / `AuthPrompt`, while
  Web UI can surface approval state through the normal pending-gate/run-history
  path. These examples document current product behavior; the engine consumes
  validated capabilities, not product names.

## 9. Auth Prompt Delivery

Auth prompt delivery and auth flow resolution are different contracts.
Auth-required state remains owned by the auth flow / auth interaction path. The
outbound resolution engine may select where an already-created, redacted
`AuthPrompt` notification should be attempted, but it must not create, complete,
cancel, replay, or retry auth flows.

The intended end state still supports product-channel auth interaction: if the
validated default target supports auth prompts plus auth-resolution inbound
events, the user can start or complete the auth interaction from that product
surface and the existing auth interaction path continues the loop.

Security rules:

- Auth callback handling, credential exchange, token storage, and secret
  material stay outside communication delivery resolution.
- The engine must not accept credential names, OAuth state, callback payloads,
  pending-auth IDs, or pending-gate records as inputs.
- If the validated target capabilities do not include auth prompts, do not
  emulate auth with normal chat text.
- If the target cannot produce a scoped `AuthResolution` inbound event, delivery
  is notify-only; credential exchange and resume still happen through a supported
  auth surface.
- Product examples such as Web UI surfacing auth state and Telegram v2 dropping
  `AuthPrompt` are adapter behavior. The engine consumes validated
  capabilities, not product names.

## 10. Outbound Validation and Failure Semantics

Every resolved target is an outbound candidate:

```text
CommunicationDeliveryCandidate
  -> OutboundPushTargetRequest
  -> OutboundPolicyService::prepare_delivery_attempt
  -> ReplyTargetBindingValidator
```

If reply-target authorization is revoked or the target is unbound, outbound
records a sanitized `authorization_revoked` delivery failure and returns no
sendable target. The engine must not silently fall back to another channel
after revocation. Users can change their defaults explicitly. Future fallback
behavior, if added, must be an explicit ordered rule rather than an implicit
error handler.

Delivery failure records are support/audit metadata. They do not mutate
canonical transcript state and do not mark the run failed.

## 11. Inbound Non-Impact Contract

Communication delivery resolution must not change inbound acceptance, replay, or
resume semantics.

- Inbound user-message binding stays in product workflow and
  `ironclaw_conversations`. The engine consumes canonical scope, actor, and
  route context after inbound acceptance; it does not parse product payloads or
  submit turns.
- Approval and auth resume stay in their existing gateway / bridge / control
  plane paths. A delivery target can notify a user that action is needed, but it
  cannot approve, deny, provide credentials, clear pending gates, or resume
  runtime work.
- Product outbound payload shapes and adapter behavior stay unchanged. The
  engine selects a target candidate; product adapters still render
  `FinalReply`, `Progress`, `GatePrompt`, `AuthPrompt`, projection updates, and
  unsupported-payload deferrals according to their existing capabilities.
- Delivery failures remain delivery metadata only. They do not mutate canonical
  transcript, projection, auth, approval, pending-gate, or turn state.

## 12. Security Invariants

- Do not encode Web UI, Telegram, or any communication destination into
  `ExternalConversationRef`, `TurnActor`, `adapter_kind`, or trigger ingress
  identity.
- Do not let trigger delivery override approval authority.
- Do not let communication delivery override auth flow authority.
- Do not use tenant/user defaults as proof that a target is still authorized.
- Do not infer delivery, approval, or auth capabilities from channel names.
- Revalidate `ReplyTargetBindingRef` before every external push.
- Do not silently fallback from an explicit requested outbound target to another
  channel in P0.
- Keep ingress, execution, and outbound delivery identities separate.
- Keep preference lookup scoped by tenant and user. Cross-tenant lookup must
  fail as unknown, not as unauthorized-with-details.
- Store refs, settings, status, and sanitized failure kinds only. Do not store
  raw prompts, tool inputs, secrets, or channel credentials in communication
  preference records.

## 13. Example Flows

### Live Web UI Chat

```text
User sends text in Web UI
  -> source_route.reply_target_binding_ref exists
  -> FinalReplyReady resolves to source route
  -> outbound revalidates
  -> Web UI renders final reply
```

### Explicit Product-Adapter Test Message

```text
Agent/tool asks to send a test Telegram message through the Reborn product adapter
  -> resolve_requested_outbound carries requested Telegram target
  -> engine selects requested target
  -> outbound revalidates
  -> Telegram adapter renders/sends only if authorized
```

If the Telegram binding was revoked, outbound records `authorization_revoked`
and does not silently send the test message to Web UI or another product. Slack
follows this rule only after a Reborn Slack product adapter exists.

### Cron Trigger With Telegram Default

```text
Trigger fires with RunNotificationOrigin::Triggered
  -> FinalReplyReady
  -> engine uses user's final_replies_target = Telegram reply target
  -> outbound revalidates
  -> Telegram adapter renders FinalReply
```

If Telegram binding was revoked, outbound records `authorization_revoked` and
does not send or auto-fallback to Web UI.

### Trigger Needs Approval

```text
Triggered run hits approval-gated tool
  -> ApprovalNeeded / BlockedApproval
  -> engine checks approval_prompt_target capabilities
  -> Web UI target can surface gate prompt when validator reports gate_prompts
  -> Telegram target is skipped until its validated capabilities include gate_prompts
  -> ApprovalResolver remains the only approve/deny authority
```

### Auth Prompt Notification

```text
Auth flow records auth-required state
  -> EventStream publishes auth_required
  -> engine checks auth_prompt_target capabilities
  -> Product adapter may render AuthPrompt if validated capabilities allow it
  -> Auth interaction service remains the only auth completion authority
```

## 14. Contract Updates Needed

- `events-projections.md`: define where communication resolution plugs into
  event/projection/outbound flow.
- `approvals.md`: state that approval notification is separate from approval
  resolution and leases.
- Auth product / runtime workflow contracts: state that auth prompt notification
  is separate from auth flow creation, callback handling, credential exchange,
  and token storage.
- `conversation-binding.md`: keep reply-target binding semantics distinct from
  synthetic trigger ingress identity.
- Trigger-loop spec: reference this document for default notification and
  approval delivery resolution.
- Tests: when this moves from design to implementation, add caller-level
  regression coverage for product inbound acceptance/replay, chat approval/gate
  resolution, and auth token/cancel flows. Resolver unit tests alone are not
  enough to prove inbound safety.
