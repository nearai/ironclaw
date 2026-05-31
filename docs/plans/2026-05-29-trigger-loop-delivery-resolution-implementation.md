# Trigger Loop and Delivery Resolution — Implementation Plan

**Date:** 2026-05-29
**Status:** Draft — reviewed by codebase subagents, pending owner decisions
**Branch:** reborn-integration
**Primary specs:**
- `docs/superpowers/specs/2026-05-21-trigger-loop-design.md`
- `docs/superpowers/specs/2026-05-29-channel-communication-delivery-resolution.md`

## Goal

Implement communication delivery resolution and scheduled trigger loops in
small, reviewable PRs. Each PR should target fewer than 1000 lines of code where
practical, merge before the next dependent slice, and preserve Reborn ownership
boundaries.

Delivery resolution and trigger trusted-ingress are separate early contract
tracks. Delivery resolution is required before user-visible trigger delivery,
but it is not required before the trigger loop can fire, run through the Reborn
turn path, and persist a thread. If product outbound is not ready when trigger
V1 lands, V1 acceptance is: cron trigger fires, submits a synthetic inbound turn
through trusted ingress, runs in a dedicated thread, and persists the result.

## Current Ground Truth

- Work targets `reborn-integration`, not `staging`.
- `ironclaw_outbound` owns outbound policy, reply-target validation, delivery
  attempts, and thread notification policy.
- `OutboundPolicyService::prepare_delivery_attempt` currently has contract-test
  coverage but no production orchestration caller.
- Existing outbound policy is thread-scoped. User-level delivery preferences do
  not exist yet; the existing `src/profile.rs::CommunicationPreferences` is
  unrelated profile/tone data.
- Product workflow is primarily inbound. Product adapters own transport
  rendering through `ProductAdapter::render_outbound`; host/composition glue
  must own the policy sequence before adapter render.
- `ironclaw_conversations::InboundTurnService` owns inbound replay, binding
  resolution, message acceptance, and turn submission. Trigger ingress must
  extend this service rather than duplicate the pipeline.
- `ConversationBindingService` already exposes
  `resolve_or_create_binding_with_trusted_scope`, but `InboundTurnService` does
  not yet expose a trusted inbound facade.
- `AdapterKind` and `ExternalConversationRef` are publicly constructible
  bounded values today, so a reserved string alone is not enough to seal
  host-internal trigger ingress from product adapters.
- `ironclaw_triggers` does not exist yet.
- Runtime composition currently owns one turn-runner worker handle; adding a
  trigger poller needs explicit multi-worker startup/shutdown ownership.
- Trigger delivery must not smuggle communication destinations through
  `ExternalConversationRef`, `TurnActor`, `adapter_kind`, or trigger ingress
  identity.

## Owner Decisions Before PR 1

These decisions are now locked for the first implementation pass; the two doc
PRs should record them as contract language.

1. PR 1 and PR 2 are parallel contract tracks, but PR 2 is the sole owner of
   trusted-ingress semantics and trigger-boundary changes in
   `conversation-binding.md`. PR 1 stays outbound/reply-target only. If we
   choose a single first PR, prioritize PR 2 because trusted ingress is the hard
   prerequisite for trigger execution.
2. Trigger fires need a host-internal ingress representation that product
   adapters cannot construct. The contract must define deterministic
   `adapter_kind`, `external_actor_ref`, `external_conversation_ref`, and
   `external_event_id` values, but not rely on a raw reserved string alone for
   trust.
3. Trusted trigger scope flows through a planned typed
   `handle_inbound_turn_with_trusted_scope` request bundling host-owned
   `tenant_id`, `creator_user_id`, `agent_id`, and `project_id` authority.
   Synthetic trigger requests should not carry normal untrusted requested scope
   hints as the authority source.
4. V1 `TriggerRunStatus` stays synchronous: `Ok` means submitted, `Error` means
   submission failed. `ApprovalBlocked` and `TimedOut` are fast-follow unless a
   later PR explicitly wires turn-lifecycle observation.
5. Communication preferences are database-backed from day one, using a
   dedicated typed tenant/user communication-preference table/repository rather
   than the generic JSON settings store.
6. Host/composition owns
   `CommunicationDeliveryCandidate -> prepare_delivery_attempt -> render_outbound`.
   Product adapters keep transport rendering ownership and do not perform
   outbound policy lookup.
7. Trigger prompt materialization uses a narrow port/helper. `ironclaw_triggers`
   asks for an inbound content ref and does not reach into composition, product
   adapters, or transcript internals directly.
8. V1 active-run back-pressure is required:
   `max_concurrent_fires_per_trigger = 1`. A trigger skips a tick while its
   previous fire is still active. This is enforced by an atomic repository
   claim/lease plus turn-state lookup, not by `last_status` or an in-memory
   poller counter.
9. `trigger_create`, `trigger_list`, and `trigger_remove` are required
   user-facing first-party capabilities, registered first in
   `ironclaw_host_runtime::first_party_tools` and then consumed by composition.
   The host-runtime package, handlers, and registry entries must stay in
   lockstep.
10. Trigger poll settings are composition-owned, and V1 schedules must reject
    sub-minute cadence. No trigger may fire more frequently than once per
    minute.
11. Trigger result delivery remains fast-follow until a concrete outbound
    adapter path is named and proven ready with target validation, envelope
    construction, delivery-attempt recording, and caller-level tests.
12. Requested outbound is not allowed to bypass authority-bearing prompt
    policy. Approval/auth prompt delivery must resolve through exact-owner
    prompt targets first; requested outbound may only apply to ordinary
    non-authority delivery or narrow to the same exact-owner prompt target.
13. Trigger fires bypass `ironclaw_product_workflow` ingress entirely. Product
    workflow remains adapter-facing; scheduled triggers enter only through the
    planned `ironclaw_conversations::InboundTurnService` trusted facade.
14. The sealed host-trusted ingress marker and witness are owned by
    `ironclaw_conversations::trusted_ingress`; host composition constructs them
    for trigger submission. Product adapters must not receive constructors or
    model them in product payload DTOs.

## Dependency DAG

Trigger event execution does **not** need delivery resolution. The trigger loop
can ship once trusted ingress, trigger storage, the poller, and composition
wiring are complete. Delivery resolution is only required for pushing the final
trigger result to an external product/channel.

```text
Contract tracks
  PR 1 Delivery Contract
  PR 2 Trigger Trusted-Ingress Contract

Delivery track
  PR 1 ─> PR 3 Outbound Resolver Domain Types
             ├─> PR 4 Communication Preferences Store
             └───────────────┐
  PR 4 ──────────────────────┴─> PR 5 Outbound Resolution Engine
                                     └─> PR 6 Outbound Validation Integration
                                          └─> PR 7 Product Outbound Orchestration

Trigger execution track
  PR 2 ─> PR 8 Trusted Inbound Facade ─> PR 9 ironclaw_triggers Crate Skeleton
                                           └─> PR 10 Trigger Persistence Model and Backend 1
                                                └─> PR 11 Trigger Persistence Backend 2 and Parity
                                                     └─> PR 12 Atomic Fire Claim API
                                                          └─> PR 13 Atomic Claim Backend Implementations
                                                               └─> PR 14 Materialization and Turn-State Seams
                                                                    └─> PR 15 Trigger Poller Core
                                                                         └─> PR 16 Poller Caller-Level Harness
                                                                              ├─> PR 17 trigger_* First-Party Capabilities
                                                                              └─> PR 18 Trigger Worker Config and Lifecycle

External trigger result delivery
  PR 7 + PR 18 + named adapter readiness ─> PR 19 Trigger Delivery Integration Fast-Follow
```

Parallelization notes:

- PR 1 and PR 2 are independent doc/contract tracks and can be prepared in
  parallel. If we want a single first PR, choose PR 2 first when optimizing for
  trigger execution, or PR 1 first when optimizing for outbound delivery.
- After PR 1 merges, PR 3 can proceed without waiting for PR 2.
- After PR 2 merges, PR 8 can proceed without waiting for delivery work.
- PR 4 and PR 8 can run in parallel because they touch different crates and
  solve different prerequisites.
- PR 5/6/7 are delivery-only. They do not block PR 9 through PR 18 unless the
  chosen milestone is "trigger result is pushed externally" rather than
  "trigger event fires and creates a persisted thread."
- PR 10 and PR 11 are serial if PR 11 is the parity backend, but they can be
  reversed if backend ownership prefers implementing libSQL first or PostgreSQL
  first.
- PR 12 and PR 13 must land after both persistence backends because atomic
  claim/lease semantics need PostgreSQL/libSQL parity before the poller depends
  on them.
- PR 17 and PR 18 can start from the same post-PR16 baseline, but they should
  merge carefully because both need repository/config wiring from composition.
- PR 19 should remain fast-follow until PR 7 is merged and a concrete outbound
  adapter path is declared ready.

Milestone gates:

- **Trigger event MVP:** PR 2, PR 8, PR 9, PR 10, PR 11, PR 12, PR 13, PR 14,
  PR 15, PR 16, and PR 18. PR 17 is required if the MVP includes user-facing `trigger_*`
  management rather than seeded/test-created triggers.
- **User-managed trigger MVP:** Trigger event MVP plus PR 17.
- **Externally delivered trigger result:** User-managed trigger MVP plus PR 1,
  PR 3, PR 4, PR 5, PR 6, PR 7, and PR 19.

## PR Sequence

### PR 1 — Delivery Contract

Promote the delivery-resolution design into Reborn contracts before code:

- Add or update the delivery-resolution contract under `docs/reborn/contracts/`.
- Update `events-projections.md` with where communication resolution plugs into
  event/projection/outbound flow.
- Update `approvals.md` to state that approval notification is separate from
  approval resolution and leases.
- Update auth/product runtime contracts to state that auth prompt notification
  is separate from auth-flow creation, callback handling, credential exchange,
  and token storage.
- Update `conversation-binding.md` only for reply-target binding semantics that
  are needed by outbound delivery; all trusted-ingress semantics belong to PR 2.
- Update `runtime-workflows.md` where needed so approval/auth prompt delivery
  ownership is reflected in the runtime interaction loop contracts.
- Define the typed resolution envelope, preference fields, and deterministic P0
  order before implementation so PR 3 and PR 5 do not reinterpret prompt
  authority, trigger/source-route precedence, or system-event behavior.

Expected size: docs only.

### PR 2 — Trigger Trusted-Ingress Contract

Ratify trigger-specific contract changes before code:

- Add host-trusted inbound ingress semantics to `conversation-binding.md`.
- Define planned `InboundTurnService::handle_inbound_turn_with_trusted_scope`
  and its typed trusted request.
- Define every synthetic `InboundTurnRequest` field used by trigger fires:
  `adapter_kind`, `external_actor_ref`, `external_conversation_ref`,
  `external_event_id`, route kind, actor, content ref, and scope flow.
- Specify that host-internal ingress values are type-sealed or otherwise
  unconstructible by product adapters, not merely conventional reserved strings.
- Update and promote `docs/reborn/contracts/triggers.md` as the only
  trigger-system contract source of truth, covering `TriggerRecord`,
  `TriggerSourceProvider`, `TriggerFireIdentity`, poller semantics,
  deterministic-slot idempotency, and scope rules.
- State that post-run `ApprovalBlocked` / `TimedOut` status updates are
  fast-follow in V1.
- State the V1 schedule granularity rule: cron and other schedule providers
  must reject schedules that can fire more frequently than once per minute.

Expected size: docs only.

### PR 3 — Outbound Resolver Domain Types

Add typed request/response shapes in `ironclaw_outbound`:

- `CommunicationDeliveryResolutionRequest`
- `CommunicationDeliveryIntent`
- `CommunicationDeliveryKind`
- `RunNotificationEventKind`
- `RunNotificationOrigin`
- `RequestedOutboundContext`
- `SourceRouteContext`
- `TriggerCommunicationContext`
- `CommunicationModality`
- `CommunicationDeliveryCandidate`
- delivery target capability types
- translation notes to existing `OutboundPushCandidate` /
  `PrepareOutboundDeliveryRequest`

`RequestedOutboundContext` must carry a typed `ReplyTargetBindingRef` candidate,
not a raw adapter/channel/conversation string. The top-level request must carry
the intent, and `CommunicationDeliveryResolutionRequest::delivery_kind()` is
derived from that intent so validation can reject shared/group widening for
authority-bearing prompt payloads without allowing contradictory input.

`SourceRouteContext` must also stay outbound-owned and binding-level only:
carry the canonical `ReplyTargetBindingRef` produced by
`ironclaw_conversations`, not raw adapter identity such as `AdapterKind`,
`AdapterInstallationId`, `ExternalActorRef`, or `ExternalConversationRef`.
`ironclaw_outbound` must not depend on `ironclaw_conversations`; composition or
later product outbound orchestration owns any translation between conversation
source-route records and outbound resolution inputs.

Include serde and unit tests. Do not wire product egress yet.

Expected size: less than 700 lines.

### PR 4 — Communication Preferences DB Model

Add user delivery preferences owned by `ironclaw_outbound` and persisted in a
dedicated typed database table/repository:

- final replies target
- progress target
- approval prompt target
- auth prompt target
- default modality
- tenant/user composite identity
- updated timestamp and updater identity

Stored `ReplyTargetBindingRef` values are candidates only and must be
revalidated at send time. Do not reuse the existing profile/TOML config path or
the generic DB-backed JSON settings store as the source of truth; those are
operator/user-settings shaped and not tenant/user typed delivery policy.
Imitate the DB store pattern where useful, but keep communication preferences a
typed outbound-owned repository.

The repository, DTOs, and backend/migration code live under the
`ironclaw_outbound` ownership boundary, not `src/db` generic settings. Backend
integration may reuse workspace migration conventions, but schema ownership and
tests belong with the outbound crate/module that owns communication policy.

Existing `ThreadNotificationPolicy` remains thread-scoped push policy for
projection subscriptions and legacy thread notifications. Tenant/user
communication preferences are a separate candidate-source layer. PR 5 must
define precedence explicitly: authority-bearing prompt preferences are
consulted first; explicit requested outbound and source-route rules follow the
delivery-resolution contract; thread policy can suppress push attempts for
ordinary progress/final reply notifications but must not grant authority or
retarget approval/auth prompts.

The repository fields should map directly to the delivery contract names:
`final_reply_target`, `progress_target`, `approval_prompt_target`,
`auth_prompt_target`, and `default_modality`.
`final_reply_target` is the canonical schema/DTO name; any plural
`final_replies_target` wording in older specs should be treated as legacy
terminology and normalized before implementation.

Expected size: less than 1000 lines. If PostgreSQL + libSQL parity pushes past
the line budget, split this into model/trait + first backend, then second
backend/parity before PR 5.

### PR 5 — Outbound Resolution Engine

Implement `OutboundResolutionEngine` as a deterministic, host-owned P0 rule
engine after database-backed preferences exist:

1. Authority-bearing approval/auth prompts use exact-owner prompt targets.
2. Explicit requested outbound wins only for non-authority delivery kinds.
3. Live inbound loops reply to their source route for ordinary notifications.
4. Triggered-from-source-route origins prefer the live source route.
5. Triggered loops without a live source route use the creator user's
   configured `final_reply_target`.
6. System-event origins require an explicit requested outbound target for
   external delivery; otherwise they record metadata only.

The engine returns a candidate only. It must not mutate inbound state, approval
state, auth state, pending gates, transcript state, or delivery attempts. If
the selected target is missing or revoked, P0 fails closed; no implicit fallback.

Expected size: less than 900 lines.

### PR 6 — Outbound Validation Integration

Connect resolved candidates to existing outbound validation without touching
adapter transport rendering:

- Convert `CommunicationDeliveryCandidate` into the existing
  `OutboundPushCandidate` / `PrepareOutboundDeliveryRequest` path.
- Ensure every candidate flows through
  `OutboundPolicyService::prepare_delivery_attempt`.
- Add caller-level tests for requested outbound, live source-route final reply,
  triggered default target, triggered-from-source-route precedence, system-event
  no-target behavior, prompt exact-owner enforcement, and revoked target
  failure.

Expected size: less than 1000 lines.

### PR 7 — Product Outbound Orchestration

Wire the host/composition-side outbound orchestration point for a named real
adapter path. Do not treat the current WebUI projection path as the outbound
orchestration path unless this PR explicitly refactors it to enter
`ironclaw_outbound`.

- Own the sequence
  `resolve candidate -> prepare delivery attempt -> adapter render_outbound`.
- Keep `ProductAdapter::render_outbound` as transport rendering, not policy
  lookup.
- Name the concrete first path being wired and keep adapter-specific behavior
  behind adapter capability/validation boundaries.
- Keep WebUI projection envelopes separate from product outbound delivery
  unless the PR deliberately routes that path through the same outbound
  policy service.

Expected size: less than 1000 lines; split if this touches both composition
and adapter call sites heavily.

### PR 8 — Trusted Inbound Facade

Implement the planned
`InboundTurnService::handle_inbound_turn_with_trusted_scope` facade in
`ironclaw_conversations` after PR 2:

- Add a typed trusted request shape that bundles the ordinary inbound request
  with host-owned `agent_id` and `project_id` authority. Adapter-supplied
  requested scope hints are cleared before trusted binding resolution.
- Add `ironclaw_conversations` sealed trusted-ingress marker/witness types, but
  do not expose production minting publicly in this PR. PR 8 seals and tests
  the facade locally; the later trigger worker/composition integration PR owns
  the host-owned construction shim for scheduled triggers. Product adapters
  cannot model, mint, or construct trusted ingress.
- Trigger fires call only this `ironclaw_conversations` facade. They must not
  pass through `ironclaw_product_workflow::InboundTurnService`, which remains
  adapter-facing.
- Keep replay lookup first, exactly like `handle_inbound_turn`, so duplicate
  scheduled-slot retries hit existing inbound idempotency.
- Route fresh binding resolution through
  `resolve_or_create_binding_with_trusted_scope`.
- Reuse the existing accept and submit path.
- Add a caller-level test double that fails if `resolve_or_create_binding` is
  called, proving the trusted method is actually used.
- Add replay coverage proving duplicate trusted inbound avoids double
  submission.

Expected size: less than 500 lines.

### PR 9 — `ironclaw_triggers` Crate Skeleton

Add the trigger crate with domain and in-memory behavior:

- workspace-member registration
- architecture-boundary test updates for the new crate
- `TriggerId`
- `TriggerRecord`
- `TriggerSchedule`
- `TriggerSourceKind`
- `TriggerState`
- `TriggerRunStatus`
- `TriggerFire`
- `TriggerFireIdentity`
- `TriggerSourceProvider`
- `TriggerRepository` trait
- cron validation and next-slot computation
- schedule validation rejecting sub-minute fire cadence
- in-memory repository for tests

`TriggerRecord` should use `state` as the single V1 fire gate and should not
carry a separate `enabled` field. Durable backends may add derived indexes in
PR 10, but those indexes must not become independent authority or eligibility
state.

`TriggerRepository::list_due_triggers` may be global because the poller is
host-owned background work, but every returned `TriggerRecord.tenant_id` is
authority-bearing. Later worker/claim code must mint trusted inbound requests
from each record's tenant/user/agent/project scope and must not use an ambient
tenant scope.

Include unit tests for schedule validation, serde, and deterministic fire
identity. Include tests proving expressions with sub-minute cadence are
rejected. The workspace already has `cron = "0.13"` available.
Identity derivation must use the contract's length-prefixed, domain-separated,
collision-resistant digest over `(tenant_id, trigger_id, fire_slot)`; do not
use raw string concatenation.

Expected size: less than 1000 lines.

### PR 10 — Trigger Persistence Model and Backend 1

Add the first durable `TriggerRepository` backend:

- repository trait methods for create/list/remove, due-trigger lookup, and
  submit-result bookkeeping, but not the atomic claim API yet
- migrations/schema for one chosen backend
- composite poller index on `(tenant_id, state, next_run_at)` or an equivalent
  backend-specific derived index; any denormalized scheduled/enabled index must
  be derived from `state == Scheduled`, not written as independent fire state
- `active_fire_slot` and `active_run_ref` persistence fields separate from
  `last_status`
- due-trigger query with limit
- scoped list/remove behavior
- backend-specific tests

Expected size: less than 1000 lines.

### PR 11 — Trigger Persistence, Backend 2 and Parity

Add the second required backend and parity coverage:

- migrations/schema for the second backend
- shared parity tests across both backends
- parity for active-fire fields and retryable `next_run_at` behavior
- any schema compatibility fixes from PR 10

Expected size: less than 1000 lines.

### PR 12 — Atomic Fire Claim API

Add the backend-agnostic repository claim/lease API that makes
`max_concurrent_fires_per_trigger = 1` enforceable across concurrent pollers:

- `claim_due_fire`-style request/response types and trait method.
- claim operation contract covers due-row read, trigger state check,
  active-fire check, and claim write in one database transaction or equivalent
  backend primitive.
- explicit submit-result update methods for accepted, replayed, retryable
  failed, and permanent failed outcomes.
- write-order contract for `last_run_at`, `last_fired_slot`, `last_status`,
  `next_run_at`, `active_fire_slot`, and `active_run_ref`.
- active-fire claim never uses `last_status` as the in-flight sentinel.

Expected size: less than 600 lines.

### PR 13 — Atomic Claim Backend Implementations

Implement the atomic claim API for both durable backends:

- PostgreSQL implementation with transaction/row-lock or compare-and-swap
  semantics.
- libSQL implementation with equivalent transaction/compare-and-swap semantics.
- in-memory implementation only for tests; it is not proof of the durable
  invariant.
- retryable submit failure keeps `last_fired_slot` unchanged, leaves active
  claim unset, and keeps `next_run_at` at or before the failed slot's scheduled
  time.
- duplicate replay for the same fire identity returns the original accepted
  message and turn submission; terminal run failure does not mint a second V1
  turn for the same fire slot.
- PostgreSQL/libSQL parity tests for concurrent claim attempts and retryable
  failure bookkeeping.

Expected size: less than 1000 lines; split by backend if implementation or
tests exceed the line budget.

### PR 14 — Trigger Materialization and Turn-State Seams

Add the narrow ports/helpers the poller needs before the worker implementation:

- `ironclaw_triggers` owns the prompt-materialization port trait and asks for an
  inbound content ref. Composition provides the adapter from trigger prompt data
  to the conversation/thread content-ref boundary.
- `ironclaw_triggers` owns the active-fire clear request type, but the concrete
  turn-state lookup adapter is supplied by composition over `ironclaw_turns` /
  turn-persistence APIs. It is not a trigger-local counter.
- V1 policy is exactly one active fire per trigger; later concurrency can be an
  explicit config expansion.
- tests for both seams at the owning crate boundary.

Expected size: less than 800 lines.

### PR 15 — Trigger Poller Core

Implement `TriggerPollerWorker` core logic:

- poll due schedule triggers
- cap fires per tick
- apply per-trigger active-run back-pressure by using the repository atomic
  fire-claim seam, not an in-memory `last_status` check
- construct deterministic synthetic `InboundTurnRequest`
- call `handle_inbound_turn_with_trusted_scope`
- persist synchronous submit status and next-run bookkeeping
- preserve replay safety across crash retry or dual poller attempts

Keep post-run async statuses fast-follow.

Expected size: less than 1000 lines.

### PR 16 — Trigger Poller Caller-Level Harness

Add the heavier caller-level tests separately from the worker core:

- repository + provider + inbound service + turn coordinator test path
- one new canonical thread per fire
- trusted scope reaches binding resolution
- same scheduled slot replays instead of double-submitting
- active-run back-pressure behavior
- proof that a second due fire is skipped while one fire for the same trigger
  is active
- concurrent poller claim attempts cannot both submit the same trigger/slot
- retryable submit failure leaves `next_run_at` retryable
- terminal run failure for an already accepted/submitted slot does not mint a
  second V1 turn for the same fire identity

Expected size: less than 1000 lines; split further if the harness grows.

### PR 17 — `trigger_*` First-Party Capabilities

Expose trigger management through the host-runtime first-party capability
registry first, not local-dev synthetic capabilities:

- `trigger_create`
- `trigger_list`
- `trigger_remove`
- package declarations in `ironclaw_host_runtime::first_party_tools`
- handler registration in `ironclaw_host_runtime::first_party_tools`
- `FirstPartyCapabilityRegistry` entries in `ironclaw_host_runtime`
- production composition wiring that injects the trigger repository dependency
- tests that capability IDs are present in package manifest, handlers, and
  registry

Scope must be stamped from invocation context and rechecked on list/remove.
Repository access must be injected through an explicit composition-owned seam;
do not assume `InvocationServices` already carries a trigger repository.

Expected size: less than 1000 lines.

### PR 18 — Trigger Worker Config and Lifecycle

Wire the trigger poller into Reborn composition:

- a dedicated `TriggerPollerWorkerConfig` or equivalent composition-owned type
  for poll interval, fires per tick, and per-trigger active-run cap. Do not
  reuse `RebornRuntimeInput::PollSettings`, which is request-completion polling.
- background task bundle or worker-supervisor type that owns both turn-runner
  and trigger-poller handles, cancellation, await/shutdown ordering, and panic
  or early-exit reporting.
- readiness semantics for whether a disabled trigger poller is allowed and
  whether a failed trigger worker marks Reborn runtime readiness degraded.
- architecture tests for `ironclaw_triggers` dependency edges
- current architecture map update
- `FEATURE_PARITY.md` update with a distinct Reborn trigger-loop note rather
  than relying on legacy cron rows

Expected size: less than 1000 lines; split into config and lifecycle if needed.

### PR 19 — Trigger Delivery Integration Fast-Follow

Only after delivery-resolution PRs are merged and a concrete adapter path is
ready, connect trigger-origin final reply delivery:

- name the first real adapter path and readiness gate. Do not use the WebUI
  projection path as a stand-in unless it is explicitly routed through
  `ironclaw_outbound`.
- construct `RunNotificationOrigin::Triggered`.
- construct `RunNotificationOrigin::TriggeredFromSourceRoute` when a trigger
  run also has a live source route, and verify live source route precedence.
- resolve with `OutboundResolutionEngine`.
- validate with `OutboundPolicyService`.
- send only through Reborn product-adapter outbound paths that are ready.

If concrete Reborn product egress is not ready, leave this as fast-follow and
ship trigger V1 as local persisted threads only.

Expected size: less than 1000 lines.

## Review Summary

Five codebase review agents checked the original plan against the current
Reborn code. Their main findings are incorporated above:

- Delivery and trusted ingress should be independent early contract tracks.
- Preference-backed trigger delivery cannot land before a real user delivery
  preference store exists.
- Product adapters should keep transport rendering ownership; host/composition
  should own outbound policy orchestration before render.
- Host-internal trigger ingress must be sealed, not just represented by a
  conventional string value.
- Trigger persistence, poller implementation, and poller integration tests need
  separate PR slices to respect the line budget.
- `trigger_*` belongs on the first-party capability registry path, not the
  local-dev synthetic wrapper.
- Trigger worker lifecycle needs an explicit multi-worker ownership model.
- Communication preferences should be DB-backed from day one as a typed
  tenant/user repository, not stored in legacy profile/TOML config or generic
  JSON settings.
- Trigger fires bypass product-workflow ingress and use only the conversations
  trusted inbound facade.
- Atomic fire claim APIs, durable backend implementations, and poller harnesses
  are separate slices to keep the line budget realistic.
- Trigger worker configuration must be distinct from request-completion
  `PollSettings`, and worker supervision needs explicit shutdown/readiness
  semantics.
