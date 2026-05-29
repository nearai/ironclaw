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

## 3. Proposed Layer

Add a communication resolution step inside the outbound policy boundary between
run events and outbound delivery. This is a **policy resolver**, not a general
rules engine: it selects candidate delivery targets from already-canonical run
events and preferences, then hands those candidates to existing outbound
validation.

```text
Run event
  FinalReplyReady | ProgressUpdate | ApprovalNeeded | AuthRequired | RunBlocked
        |
        v
ironclaw_outbound::CommunicationPolicyResolver
  input: CommunicationResolutionRequest
  reads: tenant/user communication preferences
  applies: source-route and user-default delivery policy
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

`ironclaw_outbound` owns this resolver because the existing Reborn event /
projection contract already assigns outbound notification policy state,
reply-target validation, delivery attempts, and sanitized failure records to
that crate. The resolver must not live in `ironclaw_triggers`,
`ironclaw_conversations`, or `ironclaw_approvals`. Reborn composition wires the
resolver and store dependencies, but does not own the policy semantics.

The resolver is read-only with respect to ingress and authority. It must not
mutate `ProductInboundEnvelope`, `ExternalConversationRef`, inbound dedupe state,
`AcceptedMessageRef`, `SourceBindingRef`, `ReplyTargetBindingRef`, pending-gate
records, auth-flow records, approval records, leases, or turn submission
behavior.

## 4. Resolver Input

The resolver needs enough context to distinguish live chat from triggered jobs,
text from voice, and final replies from approvals. Use one explicit request type
with a tagged context enum rather than independently optional fields:

```rust
CommunicationResolutionRequest {
    scope: TurnScope,
    actor: TurnActor,
    event_kind: CommunicationEventKind,
    modality: CommunicationModality,
    context: CommunicationContext,
}
```

### Event Kind

```rust
enum CommunicationEventKind {
    FinalReplyReady,
    ProgressUpdate,
    ApprovalNeeded,
    AuthRequired,
    RunBlocked,
    DeliveryStatus,
}
```

### Context

```rust
enum CommunicationContext {
    LiveUserMessage {
        source_route: SourceRouteContext,
    },
    TriggeredJob {
        trigger: TriggerCommunicationContext,
    },
    WebhookTrigger {
        trigger: TriggerCommunicationContext,
    },
    MessageRegexTrigger {
        trigger: TriggerCommunicationContext,
        source_route: SourceRouteContext,
    },
    SystemEvent {
        reason: String,
    },
}
```

This shape is a contract invariant: live user messages require source-route
context; trigger-origin events require trigger context; message-regex triggers
carry both because they are caused by a real product message but execute a
stored trigger rule. Callers must not pass an under-specified request.

The request intentionally does not carry `PendingGate`, pending auth IDs,
approval request IDs, credential names, OAuth callback data, or raw product
payloads. Those identifiers stay with their owning auth, approval, gateway, or
product-workflow paths.

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
    default_modality: Option<CommunicationModality>,
    updated_at,
    updated_by,
}
```

This record is durable user configuration, not an inbound binding and not a
grant of send authority. The stored `ReplyTargetBindingRef` values are
candidates that must be revalidated at send time. `updated_by` records the user
or tenant admin that last changed the preference.

Trigger records do not carry delivery configuration in the first trigger slice.
Triggered jobs use the creator user's `final_replies_target` by default. A
future per-trigger override may be added only as an override consumed by the
outbound resolver:

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

### Live User Message

For `CommunicationContext::LiveUserMessage`, prefer replying to the source route
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
  else normal auth-flow path only
```

### Triggered Job

For `CommunicationContext::TriggeredJob`, there is no live origin conversation
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
  preference.approval_prompt_target only if target supports auth prompts
  else record auth-required state through normal auth-flow path
```

### Webhook Trigger

Webhook trigger delivery follows triggered-job rules unless the trigger
definition eventually carries an explicit delivery override. The webhook request
itself must not be treated as a trusted reply destination.

### Message Regex Trigger

Message-regex triggers observe an already-normalized inbound product message.
They may default to the originating conversation only if a future trigger
delivery override says so and the source route's reply target revalidates.

## 7. Target Capabilities

The resolver must not hard-code product-specific behavior such as "Web UI
supports approval cards" or "Telegram does not support gate prompts." Target
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
target still belongs to the requested tenant/user/scope. The resolver may use
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
  -> ironclaw_outbound::CommunicationPolicyResolver may notify approval_prompt_target
  -> Product adapter renders GatePrompt if validated capabilities allow it
  -> Product inbound receives ApprovalResolution
  -> ApprovalResolver handles approve/deny
  -> CapabilityHost resumes exact invocation using fingerprinted lease
```

The communication resolver may choose where to notify the user, but it does not
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
  path. These examples document current product behavior; the resolver consumes
  validated capabilities, not product names.

## 9. Auth Prompt Delivery

Auth prompt delivery and auth flow resolution are different contracts.
Auth-required state remains owned by the auth flow / auth interaction path. The
communication resolver may select where an already-created, redacted
`AuthPrompt` notification should be attempted, but it must not create, complete,
cancel, replay, or retry auth flows.

The intended end state still supports product-channel auth interaction: if the
validated default target supports auth prompts plus auth-resolution inbound
events, the user can start or complete the auth interaction from that product
surface and the existing auth interaction path continues the loop.

Security rules:

- Auth callback handling, credential exchange, token storage, and secret
  material stay outside communication delivery resolution.
- The resolver must not accept credential names, OAuth state, callback payloads,
  pending-auth IDs, or pending-gate records as inputs.
- If the validated target capabilities do not include auth prompts, do not
  emulate auth with normal chat text.
- If the target cannot produce a scoped `AuthResolution` inbound event, delivery
  is notify-only; credential exchange and resume still happen through a supported
  auth surface.
- Product examples such as Web UI surfacing auth state and Telegram v2 dropping
  `AuthPrompt` are adapter behavior. The resolver consumes validated
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
sendable target. The resolver must not silently fall back to another channel
after revocation. Users can change their defaults explicitly.

Delivery failure records are support/audit metadata. They do not mutate
canonical transcript state and do not mark the run failed.

## 11. Inbound Non-Impact Contract

Communication delivery resolution must not change inbound acceptance, replay, or
resume semantics.

- Inbound user-message binding stays in product workflow and
  `ironclaw_conversations`. The resolver consumes canonical scope, actor, and
  route context after inbound acceptance; it does not parse product payloads or
  submit turns.
- Approval and auth resume stay in their existing gateway / bridge / control
  plane paths. A delivery target can notify a user that action is needed, but it
  cannot approve, deny, provide credentials, clear pending gates, or resume
  runtime work.
- Product outbound payload shapes and adapter behavior stay unchanged. The
  resolver selects a target candidate; product adapters still render
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

### Cron Trigger With Telegram Default

```text
Trigger fires with CommunicationContext::TriggeredJob
  -> FinalReplyReady
  -> outbound resolver uses user's final_replies_target = Telegram reply target
  -> outbound revalidates
  -> Telegram adapter renders FinalReply
```

If Telegram binding was revoked, outbound records `authorization_revoked` and
does not send or auto-fallback to Web UI.

### Trigger Needs Approval

```text
Triggered run hits approval-gated tool
  -> ApprovalNeeded / BlockedApproval
  -> outbound resolver checks approval_prompt_target capabilities
  -> Web UI target can surface gate prompt when validator reports gate_prompts
  -> Telegram target is skipped until its validated capabilities include gate_prompts
  -> ApprovalResolver remains the only approve/deny authority
```

### Auth Prompt Notification

```text
Auth flow records auth-required state
  -> EventStream publishes auth_required
  -> outbound resolver checks auth prompt target capabilities
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
