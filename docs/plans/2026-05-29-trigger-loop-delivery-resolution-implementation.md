# Trigger Loop and Delivery Resolution — Implementation Plan

**Date:** 2026-05-29
**Status:** PR 1–18 merged on `reborn-integration`. Post-PR-18 delivery plan
added below (see "Post-PR-18 Work Plan"). Reborn ownership audit (2026-06-02)
found no boundary breaches.
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
14. Host-trusted trigger ingress authority is sealed by
    `TrustedTriggerSubmitRequest` minting in `ironclaw_triggers` and private
    trusted inbound construction inside `ironclaw_conversations`. Product
    adapters must not receive constructors, call the trusted trigger submitter
    factory, or model trusted trigger ingress in product payload DTOs.

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
  PR 7 + PR 18 + named adapter readiness
      ├─> PR 18.5 / PR 19 prerequisite: harden trusted trigger ingress
      ├─> PR 18.5 / PR 19 prerequisite: fire-time creator authorization
      └─> PR 19 Trigger Delivery Integration Fast-Follow
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
- `active_run_ref` is persisted and round-tripped as the submitted Reborn
  `TurnRunId`; it is not an auth-layer `TurnRunRef` or a trigger-local opaque
  wrapper, and PR 10 does not interpret it as a claim/clear decision
- due-trigger query with limit
- scoped list/remove behavior
- backend-specific tests

Reborn storage boundary: `ironclaw_triggers` may own the trigger repository
backend because it owns trigger schema, row decoding, due-query semantics, and
trigger-scoped persistence tests. It must not own generic database accessors,
database URL/path/env parsing, production substrate selection, or shared
connection bootstrap. Composition/bootstrap opens `Arc<libsql::Database>` or a
PostgreSQL pool, then passes the already-constructed handle into the trigger
repository constructor. This mirrors Reborn's substrate boundary: storage crates
own domain persistence adapters; composition owns backend selection and handle
construction.

Because Reborn has moved several tenant-scoped stores away from raw database
handles toward scoped filesystem storage, PR 10 must keep tenant boundaries
explicit in the repository contract. Scoped create/list/remove remain
tenant-scoped. The global due query is allowed only for the trusted host poller,
and returned records must carry tenant/user/agent/project authority forward to
later trusted-ingress materialization.

Expected size: less than 1000 lines.

### PR 11 — Trigger Persistence, Backend 2 and Parity

Add the second required backend and parity coverage:

- migrations/schema for the second backend
- shared parity tests across both backends
- parity for active-fire fields and retryable `next_run_at` behavior
- any schema compatibility fixes from PR 10

PR 11 must preserve the same boundary as PR 10: add the second backend-specific
repository implementation and parity tests in the trigger storage layer, but do
not introduce a trigger-owned generic DB bootstrap or connection-string parser.
Backend construction remains composition-owned.

Expected size: less than 1000 lines.

### PR 12 — Atomic Fire Claim API

Add the backend-agnostic repository claim/lease API that makes
`max_concurrent_fires_per_trigger = 1` enforceable across concurrent pollers:

- `claim_due_fire` request/response types and trait method, plus the in-memory
  default behavior used by tests and non-durable harnesses.
- claim operation contract atomically covers due-row read, trigger state
  check, active-fire check, and claim write; durable PostgreSQL/libSQL
  transaction or CAS implementations land in PR 13.
- explicit submit-result update methods for accepted, replayed, retryable
  failed, and permanent failed outcomes.
- write-order contract for accepted/replayed fires:
  `last_run_at`, `last_fired_slot`, `last_status = Ok`, `next_run_at`,
  `active_fire_slot`, `active_run_ref`.
- `active_fire_slot` is written before turn submission; `active_run_ref` is
  populated only after the accepted/replayed submit result returns a
  `TurnRunId`.
- retryable failed writes `last_status = Error`, clears active fields, leaves
  `last_fired_slot` and `last_run_at` unchanged, and keeps `next_run_at` at or
  before the failed fire slot.
- permanent failed writes `last_status = Error`, clears active fields, leaves
  `last_fired_slot` and `last_run_at` unchanged, and advances `next_run_at`
  beyond the failed fire slot.
- active-fire claim never uses `last_status` as the in-flight sentinel.
- turn terminal lookup and clearing remain on the later PR 14+ seam; PR 12 and
  PR 13 do not consult turn state yet.

Expected size: less than 600 lines.

### PR 13 — Atomic Claim Backend Implementations

Implement the durable backend versions of the PR 12 claim API and prove the
concurrency invariant:

- PostgreSQL implementation with transaction/row-lock or compare-and-swap
  semantics.
- libSQL implementation with equivalent transaction/compare-and-swap semantics.
- in-memory implementation only for tests; it is not proof of the durable
  invariant.
- backend parity tests for concurrent claim attempts, accepted/replayed write
  order, retryable failure bookkeeping, and permanent failure bookkeeping.
- durable claim implementations must preserve PR 12 state-first eligibility:
  `Paused` or `Completed` rows return not-due even if stale active-fire metadata
  is still present.
- replace the PR 12 durable-backend sentinel defaults deliberately. Decide in
  PR 13 whether the trait keeps explicit temporary backend errors during rollout
  or moves to compile-time enforcement once PostgreSQL/libSQL implement every
  method.
- duplicate replay for the same fire identity returns the original accepted
  message and turn submission; terminal run failure does not mint a second V1
  turn for the same fire slot.

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
- treat per-record due-fire processing failures and active-run lookup failures
  as structured tick report outcomes so later due records can still be handled;
  keep batch-level repository list failures fail-fast
- for permanent failures with no future schedule slot, mark the trigger
  `Completed` rather than writing a sentinel `next_run_at`

Keep post-run async statuses fast-follow.

Expected size: less than 1000 lines.

### PR 16 — Trigger Poller Caller-Level Harness

Add the heavier caller-level tests separately from the worker core:

- repository + provider + inbound service + turn coordinator test path
- integration and E2E-style harnesses should intercept external infrastructure
  and endpoints only. Use real domain classes and composition-owned ports for
  trigger repositories, source providers, materialization, turn submission, and
  turn-state lookup whenever those implementations exist, so tests exercise the
  full in-process path instead of replacing internal behavior with mocks.
- one new canonical thread per fire
- trusted scope reaches binding resolution
- same scheduled slot replays instead of double-submitting
- active-run back-pressure behavior
- proof that a second due fire is skipped while one fire for the same trigger
  is active
- trusted-poller authority guard coverage: global poller-only repository
  queries such as `list_active_triggers` must remain unreachable from
  user/API/capability paths. Add an architecture or caller-level test that
  proves only the trusted poller path can exercise the cross-tenant active
  scan, and carry this into PR 18 if the final token is constructed by
  composition.
- active-cleanup fairness coverage: when the earliest active rows are
  long-running, nonterminal, or claim-only, later terminal active rows must
  still be reached eventually instead of being starved by the same ordered
  first page on every tick. Treat this as a PR 16 caller-level harness
  requirement, not a best-effort unit test: the harness should run multiple
  ticks with more active rows than the cleanup page size and prove terminal
  rows outside the first page are eventually cleared.
- if the fairness test exposes starvation, add cursor/rotation, widened cleanup
  scanning, or an equivalent repository/worker policy. If this touches
  in-memory `list_active_triggers`, consider replacing sort-then-truncate with
  a bounded selection approach in the same pass; otherwise keep that as a
  low-priority test-helper optimization.
- concurrent poller claim attempts cannot both submit the same trigger/slot
- claim-only active-fire recovery must require a composition-owned lease or
  age signal before retrying, so a freshly claimed slot is never misclassified
  as abandoned and submitted twice
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

PR 17 follow-ups that must not be lost:

- Per-scope trigger quotas are deferred from the first capability slice. Do not
  implement them as a race-prone handler-only `list then create` check. The
  next quota implementation should be repository/database-owned and atomic
  across durable backends, with any host-runtime error mapping layered on top of
  that contract.
- Durable backend hydration should continue to reject malformed stored trigger
  rows. The cron re-parse-on-read optimization is valid future performance work
  only if it preserves that malformed-row behavior, for example by introducing
  an explicit stored-schedule hydration constructor plus backend hydration
  validation tests.
- `trigger_create` currently parses/validates the cron expression, then computes
  `next_run_at` through a second parse. Collapse that to a single parse in a
  later performance pass without weakening schedule validation or malformed-row
  hydration guarantees.
- PostgreSQL NULL-scope planner tuning is deferred. The durable schemas include
  the scoped-list composite index; add a NULL-specific partial index only after
  `EXPLAIN` or production-like benchmark evidence shows Postgres is not using
  the composite index for `agent_id/project_id IS NOT DISTINCT FROM NULL`.

Expected size: less than 1000 lines.

### PR 18 — Trigger Worker Config and Lifecycle

Wire the trigger poller into Reborn composition:

- a dedicated `TriggerPollerWorkerConfig` or equivalent composition-owned type
  for poll interval, fires per tick, and per-trigger active-run cap. Do not
  reuse `RebornRuntimeInput::PollSettings`, which is request-completion polling.
- preserve PR 16's explicit worker-local trusted poller scan call sites when
  exposing the real background poller lifecycle. Product adapters, first-party
  capabilities, and tenant-scoped APIs must not receive access to cross-tenant
  poller scans; keep tenant-scoped `list_triggers(tenant_id)` as the only
  user/API listing path.
- background task bundle or worker-supervisor type that owns both turn-runner
  and trigger-poller handles, cancellation, await/shutdown ordering, and panic
  or early-exit reporting.
- decide whether the PR 15 `worker.rs` file is split into
  `worker::{config,ports,report,mod}` or receives a tracked architecture
  exemption. The current large-file shape is acceptable for landing the core
  slice only if the follow-up is explicit before lifecycle and harness code add
  more responsibilities.
- background trigger poller lifecycle should apply bounded startup and per-tick
  wake jitter to reduce replica stampedes, but it must not jitter trigger
  schedule calculation, fire identity, `fire_slot`, or `next_run_at`.
- consider bounded active-run lookup concurrency at the lifecycle/config layer
  once caller-level fairness coverage exists. Keep the default conservative
  until the concrete turn-state backend and shutdown/cancellation behavior are
  wired.
- do not parallelize active-fire cleanup and the due-trigger query with a raw
  `tokio::join!` unless the design preserves cleanup-before-due semantics:
  clearing a terminal active fire before `list_due_triggers` can make that same
  trigger eligible in the current tick.
- lifecycle/notification wiring should define how trigger submit failures are
  surfaced to users or admins. Permanent failures should produce a durable,
  throttled notification; retryable failures should avoid per-tick spam and use
  thresholded or summarized reporting.
- preserve the PR 15 bounded tick-report failure categories when lifecycle,
  logging, and notification wiring are added. Do not reintroduce persisted,
  broadly logged, or user-visible raw `TriggerSourceProvider`,
  `TriggerPromptMaterializer`, backend, or submitter error strings; map typed
  categories to sanitized summaries at the lifecycle/notification boundary.
- approval waits continue to belong to the turn pipeline. Composition should
  define durable approval TTL/reminder behavior, fail-closed expiry, and stale
  approval rejection while preserving trigger `active_run_ref` back-pressure.
- readiness semantics for whether a disabled trigger poller is allowed and
  whether a failed trigger worker marks Reborn runtime readiness degraded.
- PR18 review follow-up status:
  - host-trusted trigger ingress is hardened with sealed trigger-worker request
    minting, private conversation-owned trusted inbound construction, and
    architecture tests that restrict trusted constructor/factory call sites.
  - trigger poller startup is opt-in by default; runtimes must explicitly pass
    enabled trigger poller settings before the background worker starts.
  - runtime shutdown cancels the trigger poller and waits only for a bounded
    shutdown interval before aborting a stalled in-flight tick.
  - trusted trigger prompt recording happens after trusted inbound turn
    acceptance/replay, so failed submissions do not durably inject visible
    prompt content.
  - trusted trigger prompt submission now applies a composition-owned
    prompt-injection scan before turn submission and rejects high/critical
    findings as permanent materialization failures without silently rewriting
    the scheduled prompt.
  - active-run lookup is batched for each cleanup page so composition snapshots
    turn state once per active page rather than once per active trigger record.
  - PR18.5 / PR19 prerequisite: keep host-trusted trigger ingress as a
    compile-time sealed trigger submission path, not a reusable generic trusted
    ingress facade. The prerequisite is not just another dependency-boundary
    assertion; it needs a concrete API shape where product adapter crates cannot
    mint trigger authority. Expected work:
    - keep trusted trigger authority on the worker-minted sealed trigger
      request, not on a reusable authority-token facade;
    - keep `TrustedInboundTurnRequest` raw construction private inside
      `ironclaw_conversations`;
    - expose only the narrow trigger-fire submission operation needed by
      composition, not a reusable generic trusted-inbound token;
    - update architecture tests so adapter/product crates are forbidden from
      introducing a generic trusted ingress facade and forbidden from calling
      the trusted trigger constructor/factory;
    - add a negative or architecture test proving a product adapter path cannot
      construct host-trusted trigger ingress;
    - preserve existing PR18 poller behavior and trusted inbound replay tests.
    If PR19 starts wiring external delivery or any user-visible trigger launch
    path, this must be completed before that delivery path ships.
  - PR18.5 / PR19 prerequisite: add fire-time creator authorization wired to
    the real agent/project access source of truth. This must not be an
    allow-all placeholder port. Expected work:
    - define a composition-owned trigger fire authorization port whose request
      includes `tenant_id`, `creator_user_id`, `agent_id`, `project_id`,
      `trigger_id`, and `fire_slot`;
    - wire the port before trusted inbound turn submission and before prompt
      thread recording;
    - implement the port against the real access-control source for the target
      agent/project, or keep trigger external delivery disabled until that
      source exists;
    - classify denied/revoked access as a permanent authorization failure so
      the trigger claim is cleared and the failed slot is advanced according to
      the trigger contract;
    - classify temporary authz backend unavailability as retryable without
      marking the fire active;
    - add caller-level tests for authorized creator, revoked creator,
      project-specific denial, and retryable authz backend failure;
    - ensure retry/replay does not submit a turn after a denied fire-time authz
      check.
    If PR19 makes trigger results externally deliverable, this must be
    completed before delivery is enabled.
  - still open for the next lifecycle/recovery slice: define active-run lookup
    behavior when turn retention prunes the referenced `TurnRunId`, including
    whether terminal tombstones or a narrower durable lookup are required before
    missing runs can unblock stale active-fire metadata.
  - still open for production lifecycle wiring: switch lifecycle jitter to a
    real per-process random or seeded PRNG source if deployed replicas need
    stronger startup/tick de-correlation than the current bounded wall-clock
    fallback.
- architecture tests for `ironclaw_triggers` dependency edges
- current architecture map update
- `FEATURE_PARITY.md` update with a distinct Reborn trigger-loop note rather
  than relying on legacy cron rows

Expected size: less than 1000 lines; split into config and lifecycle if needed.

### PR 19 — Trigger Delivery Integration Fast-Follow

Only after delivery-resolution PRs are merged and a concrete adapter path is
ready, connect trigger-origin final reply delivery:

- Before enabling external trigger delivery, close the two PR18 trusted-poller
  security fast-follows: harden trusted trigger ingress into a compile-time host
  facade or equivalent sealed factory, and enforce fire-time creator
  authorization against the real agent/project access source of truth.
- name the first real adapter path and readiness gate. Do not use the WebUI
  projection path as a stand-in unless it is explicitly routed through
  `ironclaw_outbound`.
- construct `RunNotificationOrigin::Triggered`.
- construct `RunNotificationOrigin::TriggeredFromSourceRoute` when a trigger
  run also has a live source route, and verify live source route precedence.
- resolve with `OutboundResolutionEngine`.
- validate with `OutboundPolicyService`.
- send only through Reborn product-adapter outbound paths that are ready.

Carry-forward finding from the default outbound preference service slice:
Phase 2 composes a WebUI-usable `OutboundPreferencesProductFacade` backed by
`RebornLocalRuntimeServices::outbound_preferences`, but Slack host-beta final
reply delivery still constructs its own outbound state store inside
`slack_host_beta.rs`. Before PR 19 claims Slack end-to-end delivery, the Slack
integration must bridge this split by exposing Slack delivery targets through
the composition-owned target inventory and making Slack final-reply delivery
read the same communication preference repository used by the default outbound
preference API. Do not encode WebUI rendering assumptions in this bridge; the
API must remain usable by any authenticated product surface.

Carry-forward API and repository follow-ups from GitHub PR #4537 review:

- Expand `RebornOutboundDeliveryModality` in a dedicated product API follow-up
  before surfacing non-text outbound defaults. The outbound repository already
  supports `Text`, `Voice`, `Image`, `Mixed`, and `Unknown`, while the current
  WebUI/product response DTO intentionally exposes only `Text`. Do not add a
  partial mapper in composition until the public enum and serde contract are
  deliberately expanded.
- Add a product-safe stale-target representation before treating missing
  inventory matches as first-class UI state. The preferred low-churn shape is a
  sibling status field that can distinguish `none_configured` from
  `unavailable` while preserving the existing `final_reply_target:
  Option<RebornOutboundDeliveryTargetSummary>` field. A tagged target-state DTO
  is cleaner but should be evaluated as a larger API contract change.
- Add an atomic update operation to `CommunicationPreferenceRepository` before
  depending on concurrent preference updates for production behavior. The fix
  belongs in the repository/store contract so in-memory and filesystem-backed
  stores re-read and re-apply the update under their own lock/CAS semantics. Do
  not paper over this with a composition-local facade mutex; that would only
  serialize one process and one facade instance.
- Decide whether local-dev builds with the `postgres` feature should move
  outbound preferences, and possibly the broader non-libSQL local-dev store
  graph, to filesystem-backed persistence. Do not switch only this one
  preference store casually: the current non-`libsql` local-dev graph still uses
  in-memory run, thread, checkpoint, and event stores, while production
  PostgreSQL already routes durable state through `PostgresRootFilesystem`.

If concrete Reborn product egress is not ready, leave this as fast-follow and
ship trigger V1 as local persisted threads only.

Expected size: less than 1000 lines.

## Post-PR-18 Work Plan

### Status Snapshot (2026-06-02)

PR 1–18 merged. Trigger backend complete: `ironclaw_triggers` domain crate,
dual-backend persistence, atomic fire claim, poller worker, trusted inbound
facade, `trigger_*` first-party capabilities, and worker lifecycle composition
wiring. A Reborn ownership audit found no boundary breaches.

Two gaps block shipping user-creatable cron jobs:

1. **Poller is not enabled in a shipped runtime without real fire-time creator
   authorization.** `TriggerPollerSettings` defaults `enabled: false`
   (`runtime_input.rs`). Even if config/env enables the poller, composition now
   rejects runtime build unless the placeholder tenant-scope authorizer is
   explicitly allowed by test-support code. Cron triggers can only fire from Rust
   test code today. This is the hard blocker between "backend exists" and "cron
   actually fires."
2. **Some security hardening is deferred.** Trusted trigger submission is now
   type-sealed through `TrustedTriggerSubmitRequest` and converted inside
   `ironclaw_conversations`; product/adapter crates cannot mint host-trusted
   inbound turns directly. Fire-time creator authorization is still a
   tenant-ID-equality placeholder
   (`TenantScopedTrustedTriggerFireAuthorizer`), not wired to a real agent/project
   access source. Both are plan-mandated before any user-visible trigger launch
   path or external delivery ships (see PR 18 follow-up status and PR 19).

### Goal

Deliver user-creatable cron jobs that actually fire end-to-end: a user creates a
cron trigger, the host poller fires it on schedule, a synthetic inbound turn runs
through trusted ingress, and a dedicated thread is persisted. External delivery
of the trigger result stays fast-follow (PR 19).

**Cron is the first of several trigger sources, not the end state.** Today the
domain models only one source: `TriggerSchedule::Cron`, `TriggerSourceKind::Schedule`,
and the single `ScheduleTriggerSourceProvider` impl
(`crates/ironclaw_triggers/src/lib.rs`). `TriggerSourceProvider` is the pluggable
seam — future sources (webhook, inbound event, email-arrival, etc.) are new
enum variants plus new provider impls, with the poller, persistence, trusted
ingress, and capabilities all unchanged. The user-facing surface is therefore
designed source-agnostic from day one and labeled **Automations**: cron presents
as a "Schedule" trigger type; later sources add new trigger types under the same
list/create/manage UI and API. Build the API and UI around a generic
trigger-type selector, not a cron-only form.

### Work Items

Three independent tracks branch from the PR 18 baseline.

#### Track E — Firing Enablement (turns the poller on)

**PR 18.6 — Trigger Poller Config and Runtime Enablement**

- Add a `[trigger_poller]` section to `ironclaw_reborn_config` (`config_file.rs`):
  `enabled`, poll interval, fires per tick, and per-trigger active-run cap. Map
  onto the existing `TriggerPollerWorkerConfig` / `TriggerPollerSettings` shape —
  do not invent a parallel type.
- Honor an env fallback (`IRONCLAW_TRIGGER_POLLER_ENABLED`, plus optional
  interval/cap overrides) so smoke tests do not need a config file.
- Read the resolved settings in
  `ironclaw_reborn_cli::build_runtime_input_with_options` and call
  `.with_trigger_poller_settings(...)`.
- Document the flag in `.env.example`, off by default.
- Preserve the PR 18 opt-in-default rule: absent config/env ⇒ poller stays off.
- Deps: none (PR 18 merged). Can start immediately.
- Expected size: less than 400 lines.

**PR 18.7 — Full-Path Poller Integration Test**

- Rust integration test (`--features integration`): enable the poller, create a
  trigger via the `trigger_create` capability with `next_fire_at` in the past,
  await one poll tick, and assert a thread + run record persisted.
- Extend `tests/support/reborn/harness.rs` to start the poller (it currently
  registers the trigger tools but never starts the background worker).
- Use real domain classes and composition-owned ports per the PR 16 testing
  rule; intercept external infrastructure only.
- Deps: PR 18.6.
- Expected size: less than 500 lines.

**PR 18.8 — Python E2E Cron Trigger Scenario**

- New `tests/e2e/scenarios/test_cron_trigger_autofires.py`: create a cron
  trigger through chat send, wait one poller interval, and assert a completed
  run exists under the new `ironclaw_triggers` domain. Distinct from the v1
  routines scenario (`test_routine_full_job.py`), which exercises the legacy
  Mission/routine path, not `ironclaw_triggers`.
- Deps: PR 18.6 (e2e runs the real binary; the poller must be enableable there).
- Expected size: less than 300 lines.

#### Track S — Security Hardening (gates user-visible launch and delivery)

**PR 18.5a — Type-Seal Trusted Ingress Facade (Prereq A)**

- Seal trusted trigger submission so product/adapter crates cannot mint
  host-trusted inbound turns even by adding dependencies. Authority lives in
  `TrustedTriggerSubmitRequest`, constructed only by the trigger worker and
  converted inside `ironclaw_conversations`.
- Keep `TrustedInboundTurnRequest` raw construction private in
  `ironclaw_conversations` (already done) and expose only the narrow
  trigger-fire submission operation.
- Add a negative/structural test proving an adapter path cannot construct
  host-trusted ingress — not just another dependency-edge assertion.
- Preserve existing PR 18 poller behavior and trusted inbound replay tests.
- Deps: none. Parallel to Track E.
- Expected size: less than 400 lines.

**PR 18.5b — Fire-Time Creator Authorization (Prereq B)**

- Define a typed `TriggerFireAuthRequest` carrying `tenant_id`,
  `creator_user_id`, `agent_id`, `project_id`, `trigger_id`, and `fire_slot`
  (today the trait takes the whole `TriggerFire`).
- Implement the port against the real agent/project access-control source of
  truth (today `TenantScopedTrustedTriggerFireAuthorizer` checks tenant-ID equality
  only). If no real source exists, keep external delivery disabled; composition
  must fail closed when a normal runtime tries to enable the poller with the
  tenant-only placeholder.
- Add a `TriggerFireAuthError::Retryable` variant for backend unavailability;
  `Denied`/revoked stays permanent (clear claim, advance slot); retryable does
  not mark the fire active and keeps `next_run_at` retryable.
- The wiring position is already correct (authz runs before binding resolution
  and prompt recording in `trigger_poller_trusted_submit.rs`).
- Caller tests: authorized creator, revoked creator, project-specific denial,
  retryable authz backend failure, and replay-after-deny does not submit.
- Deps: none. Parallel to 18.5a and Track E. **Both 18.5a and 18.5b edit
  `trigger_poller_trusted_submit.rs` — coordinate merge order.**
- Expected size: less than 600 lines.

#### Track U — User-Facing "Automations" Surface

The user-facing label is **Automations**. The API and UI are trigger-source
agnostic: cron is the first selectable trigger type ("Schedule"), and future
source providers add new types under the same surface without reshaping it.

**PR 18.9 — Automations Management HTTP API (v2 surface)**

- Add a read-only automation list route to the `ironclaw_webui_v2` router.
  Match the existing WebUI v2 pattern:
  `ironclaw_webui_v2` handlers consume only
  `ironclaw_product_workflow::RebornServicesApi`; product-workflow owns the
  WebUI-facing automation facade/DTOs; `ironclaw_reborn_composition` wires the
  concrete facade to the Reborn host-runtime capability path.
- Do not put dispatcher, host-runtime, trigger repository, DB/storage, or
  product-adapter transport/rendering dependencies in `ironclaw_webui_v2`.
  Browser management ingress is moderated by host composition
  (auth/CORS/body/rate limits) plus `WebUiAuthenticatedCaller` and the
  product-workflow facade. Product adapters continue to moderate external
  product ingress such as Slack/Telegram events; they are not the browser
  management API boundary.
- Route reads through the existing shared trigger capability path
  (`builtin.trigger_list`) from composition. Do not bypass that path with direct
  trigger repository reads from WebUI/product-workflow.
- Do not expose browser create/delete HTTP routes in this slice. Automation
  mutations must go through the LLM/tool path so the same product workflow,
  trigger capability, trust, authorization, and audit boundaries that create
  automation changes today remain in charge.
- Avoid exposing "cron" as the primary user-facing label. API fields may carry
  the cron expression where needed, but browser copy and response labels should
  use **Schedule** / scheduled automation language.
- List responses surface the source type per record so the UI can render mixed
  source kinds later without a schema change; for V1 the only rendered type is
  Schedule. Do not show unsupported source types such as webhooks in the
  Automations tab.
- Stamp scope from session/auth context; recheck on list.
- Keep separate from the v1 `/api/routines/*` surface and the v1
  `static/js/surfaces/routines.js` UI.
- Deps: PR 18.5a (a user-visible launch path requires the sealed facade per the
  PR 18 follow-up rule).
- Expected size: less than 600 lines.

**PR 18.10 — Automations Web UI Panel**

- Add a read-only Automations panel in the webui_v2 static JS, pulling scheduled
  automation records from the new trigger domain via the PR 18.9 API. Distinct
  from the v1 routines panel (`static/js/surfaces/routines.js`).
- Do not add create/delete controls in the browser panel. For now, automation
  changes are initiated through the LLM/tool path; the UI only shows the
  projected state.
- The list view shows each automation's trigger type, summary, next fire, and
  state. Use user-facing **Schedule** language instead of leading with "cron",
  and do not render unsupported types such as webhooks.
- Deps: PR 18.9 (API) and PR 18.6 (enabled poller, so a created automation
  actually fires).
- Expected size: less than 800 lines.

#### Future Trigger Sources (post-cron, out of scope here)

Each new source is an independent slice on top of the surface above and needs no
poller/persistence/ingress/UI-shell changes:

- New `TriggerSourceKind` + `TriggerSchedule`/source-config variant.
- New `TriggerSourceProvider` impl (the seam already used by
  `ScheduleTriggerSourceProvider`).
- API/UI: flip the new trigger type from "coming soon" to enabled and add its
  source-specific config fields.
- Source-specific validation and tests.

Examples to scope later: webhook/inbound HTTP, system/inbound event, scheduled
one-shot (vs recurring cron). These belong in their own plans once cron
automations ship.

### Updated Dependency DAG

```text
PR 18 (merged)
  │
  ├─ Track E — firing enablement (no security gate; agent-created triggers)
  │    PR 18.6 Poller config + runtime/env wiring
  │      ├─> PR 18.7 Full-path integration test
  │      └─> PR 18.8 Python e2e cron scenario
  │
  ├─ Track S — security hardening (gates user-visible launch + delivery)
  │    PR 18.5a Type-seal trusted ingress facade ─┐
  │    PR 18.5b Fire-time creator authorization ──┘ (independent;
  │                                                  same file — merge order)
  │
  └─ Track U — user-facing "Automations" surface (source-agnostic; cron first)
       PR 18.9 Automations management HTTP API (v2)  [needs 18.5a]
         └─> PR 18.10 Automations web UI panel       [needs 18.9 + 18.6]

External trigger result delivery (unchanged):
  PR 19 Trigger Delivery Fast-Follow
      [needs 18.5a + 18.5b + PR 1, 3, 4, 5, 6, 7 + named ready adapter]
```

### Parallelization

- Tracks E, S, and the 18.5a head of U start immediately from the PR 18
  baseline — they touch disjoint crates (`reborn_config`/`reborn_cli` vs
  `trusted_ingress`/composition authz vs `webui_v2`).
- Within E: 18.6 → 18.7 and 18.6 → 18.8 (both need the poller enableable).
- Within S: 18.5a ∥ 18.5b — but both edit `trigger_poller_trusted_submit.rs`,
  so land one, rebase the other.
- 18.5b is required only for external delivery (PR 19), not for cron creation +
  firing. If the milestone is "cron job creation support," 18.5b can lag.
- U: 18.9 needs 18.5a; 18.10 needs 18.9 + 18.6.

### Milestone Gates

- **M1 — Cron fires (agent-created):** PR 18.6 (+ PR 18.7 for confidence). An
  agent creates a trigger via `trigger_create` in chat, the poller fires it, and
  a thread persists. Meets the plan's V1 acceptance. No Track S gate — the
  fire path is host-owned and already witness-sealed at runtime by PR 18; no
  user-visible *launch* surface is added here.
- **M2 — Validated cron e2e:** M1 + PR 18.8 (Python e2e proof against the real
  binary).
- **M3 — User-managed Automations (UI):** M2 + PR 18.5a (seal the facade before
  exposing a user-visible launch surface) + PR 18.9 + PR 18.10. **This is the
  "deliver cron job creation support" milestone** — cron ships as the first
  Automations trigger type, on a source-agnostic surface ready for future
  sources.
- **M4 — Externally delivered cron result:** M3 + PR 18.5b + the delivery track
  (PR 1, 3, 4, 5, 6, 7) + PR 19.

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
