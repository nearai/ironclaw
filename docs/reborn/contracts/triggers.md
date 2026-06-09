# Reborn Contract — Trigger System

**Status:** Contract-freeze draft
**Date:** 2026-05-29
**Depends on:** [`conversation-binding.md`](conversation-binding.md), [`turn-persistence.md`](turn-persistence.md), [`turn-runner.md`](turn-runner.md), [`turns-agent-loop.md`](turns-agent-loop.md)

---

## 1. Purpose

The trigger system owns scheduled trigger intake, trigger records, source-provider evaluation, and conversion of a due trigger into a synthetic inbound turn.

It does **not** own a parallel agent loop, product adapter lifecycles, or outbound delivery targets. A trigger fire is routed into the normal Reborn turn pipeline and then persists through the same turn, run, and recovery machinery as any other inbound submission.

---

## 2. Ownership

| Component | Owns | Does not own |
| --- | --- | --- |
| `TriggerRecord` / `TriggerRepository` | Trigger definitions, schedule state, eligibility state, run summary fields, PostgreSQL/libSQL persistence | Turn execution, reply delivery, product payload parsing |
| `TriggerSourceProvider` | Determining whether a stored trigger should fire and computing the canonical fire slot | Turn submission, binding internals, delivery resolution |
| `TriggerFire` / `TriggerFireIdentity` | Normalized fire output and deterministic identity for a scheduled slot | Notification targets, reply routing policy, ad hoc retries |
| `TriggerPollerWorker` | Polling eligible triggers and submitting due fires | Alternate execution loops, hidden queues, outbound send logic |
| `trigger_create` / `trigger_list` / `trigger_remove` | First-party trigger management capabilities | Legacy tool-only management paths |

The trigger system is owned by `ironclaw_triggers` in implementation terms, but this contract freezes the behavior before code lands.

---

## 3. Trigger record model

`TriggerRecord` is the durable trigger definition and poller bookkeeping record. All identifiers are newtypes and all enums are wire-stable.

| Field | Meaning |
| --- | --- |
| `trigger_id` | Stable trigger identity |
| `tenant_id` | Owning tenant |
| `creator_user_id` | User who created the trigger |
| `agent_id` | Captured agent scope at create time |
| `project_id` | Captured project scope at create time |
| `name` | Human-readable label |
| `source` | Trigger source kind |
| `schedule` | V1 schedule definition |
| `prompt` | Materialized instruction content |
| `state` | Lifecycle state for the trigger definition |
| `next_run_at` | Next eligible fire time |
| `last_run_at` | Last time a fire was submitted |
| `last_fired_slot` | Last canonical fire slot submitted for this trigger |
| `last_status` | Synchronous submission outcome |
| `active_fire_slot` | Optional claimed slot whose submitted turn has not reached a terminal outcome |
| `active_run_ref` | Optional accepted/submitted turn reference used to check or clear the active fire |
| `created_at` | Creation timestamp |

### 3.1 Source kinds

V1 source kind is schedule-only.

- `Schedule` is the only V1 source kind.
- Webhook, regex, and internal system-event sources are fast-follow and must not be accepted by the V1 contract.

### 3.2 Schedule shape and cadence

V1 schedule shape is cron-backed schedule intake only.

- Schedules that can fire more often than once per minute must be rejected.
- Second-level cron fields, sub-minute intervals, and any equivalent cadence below one minute are invalid in V1.
- The create path must reject invalid schedules before persistence, not at poll time.

### 3.3 Trigger state

`TriggerRecord.state` is the trigger-definition state, not the turn-run state.
It is the source of truth for fire eligibility.

- `Scheduled` means the trigger may be polled and fired.
- `Paused` means the trigger is retained but must not fire.
- `Completed` is reserved for future finite schedules and must not be treated as a V1 cron-state requirement.
- V1 does not expose a separate `enabled` field. Durable backends may add
  denormalized indexes derived from `state == Scheduled`, but those indexes must
  never become independent fire gates.

---

## 4. Trigger fire model

Trigger source providers emit a normalized `TriggerFire`.

```text
TriggerFireIdentity {
    tenant_id,
    trigger_id,
    fire_slot,
    route_thread_id,
    external_event_id,
}

TriggerFire {
    identity,
    creator_user_id,
    agent_id,
    project_id,
    prompt,
}
```

### 4.1 Identity derivation

`TriggerFireIdentity` is the deterministic boundary between trigger evaluation and inbound turn submission.

- `fire_slot` is the provider's canonical dedupe coordinate for the scheduled fire.
- `route_thread_id` and `external_event_id` are both derived from the same
  tenant-scoped fire identity, but with separate domain labels.
- The same `tenant_id`, `trigger_id`, and `fire_slot` must always yield the same identity.
- A different slot must yield a different identity.
- A different tenant must yield a different identity even if imported data reuses
  a `trigger_id`.
- V1 does not add a separate trigger-fire idempotency ledger; the conversation layer owns inbound idempotency storage.

The canonical derivation input is a length-prefixed sequence of the canonical
UTF-8 bytes for `tenant_id`, `trigger_id`, and `fire_slot`, prefixed by the
literal version label `ironclaw.trigger-fire.v1`. Implementations must not use
raw string concatenation. `route_thread_id` uses the domain label
`route-thread`; `external_event_id` uses the domain label `external-event`.
Each output is encoded from a collision-resistant digest over
`version_label || domain_label || length_prefixed_components`.

### 4.2 Provider boundary

`TriggerSourceProvider` decides whether a persisted trigger should fire, computes the canonical fire slot, and emits `TriggerFire`.

- The provider boundary is source evaluation only.
- It does not submit turns directly.
- It does not resolve delivery targets.
- It does not own binding creation or turn-run recovery.

V1 has one provider: a schedule provider.

- The schedule provider is cron-backed.
- Webhook, regex, and system-event providers are fast-follow and must emit the same `TriggerFire` shape when they are later added.

---

## 5. Polling and concurrency

`TriggerPollerWorker` is the background evaluator that scans eligible triggers and submits fires through trusted inbound.

- The worker may poll globally on a configured interval and batch due triggers
  across tenants. Global due queries are host-owned background work only, not a
  user-scoped request surface.
- Every returned `TriggerRecord.tenant_id` is authority-bearing state. Trigger
  workers must mint trusted inbound requests from the record's `tenant_id`,
  `creator_user_id`, `agent_id`, and `project_id`; they must not use an ambient
  or default tenant/user scope for a fire.
- Claim, update, and remove operations must mutate the same tenant-scoped record
  that was returned or claimed. A worker must not retarget a fire to another
  tenant, actor, route, or scope.
- The worker must enforce `max_concurrent_fires_per_trigger = 1` in V1 through
  an atomic repository claim/lease operation that covers read, eligibility
  check, active-fire check, and claim write.
- If a previous fire for the same trigger is still active, the current tick for that trigger is skipped.
- A skipped tick does not create a second fire, does not create a second thread, and does not fork a parallel trigger loop.
- Active means the previous fire has not yet reached a terminal turn outcome.

`last_status` is not the active-fire sentinel. Active means either
`active_fire_slot` or `active_run_ref` is set; `last_status` never marks a
trigger active. PR 12 defines the backend-agnostic `claim_due_fire`
request/response contract and in-memory default behavior; the request/response
atomically covers due-row read, trigger-state check, active-fire check, and
claim write, and PR 13 owns the durable PostgreSQL/libSQL transaction/CAS
implementations plus concurrency proof.

Claim eligibility checks the trigger state before active-fire metadata. A
`Paused` or `Completed` trigger with stale active-fire metadata is not due; it
must not be surfaced as an active scheduled fire.

The skip policy is per-trigger, not global. Other triggers may continue to fire on the same tick.

### 5.1 Trusted poller scope

The global due/active repository queries are intentionally host-owned poller
plumbing, not capability APIs.

- `list_due_triggers` and `list_active_triggers` are the raw repository
  queries used by the trusted poller path.
- Trigger-owned poller code must keep worker-local call sites explicit about the
  trusted poller transition without adding a user-facing capability surface.
- Product adapters, first-party capability code, and other untrusted callers
  must not treat the global list methods as a user-facing surface.
- The poller may continue to use the raw repository methods internally, but the
  contract treats them as implementation plumbing, not a capability contract.

---

## 6. Trusted inbound and turn execution

A trigger fire is synthetic inbound, not a parallel agent loop.

- The fire must enter the normal Reborn inbound/turn pipeline.
- The trusted submitter implementation is conversation-owned and exposed to host composition through `trusted_trigger_fire_submitter(...) -> Arc<dyn TrustedTriggerFireSubmitter>`. This public factory is wiring only; trusted authority lives in the sealed `TrustedTriggerSubmitRequest` minted by the trigger worker. The raw `TrustedInboundTurnRequest` constructor and concrete submitter type stay private inside `ironclaw_conversations`; host/composition code only wires the trait object into the poller while the conversation crate converts the worker-carried canonical binding into the private trusted turn request.
- Binding resolution for trigger fires must use the trusted-scope path from `conversation-binding.md`.
- Product adapters, first-party capabilities, and product workflow code must not construct the conversation-owned trusted trigger submitter or submit `TrustedTriggerSubmitRequest`. PR18.5a enforces this with a private trusted request and architecture tests over adapter/product paths.
- The host mints the trusted trigger ingress request from `TriggerRecord` state:
  `tenant_id`, `creator_user_id`, `agent_id`, and `project_id` are host state,
  not product payload data.
- Before a trusted trigger fire is submitted, host composition must scan the
  materialized trigger prompt for prompt-injection patterns and reject high- or
  critical-severity findings as permanent materialization failures. The
  conversation submitter repeats this scan at the final trusted submission
  boundary. The prompt must not be silently rewritten before turn submission.
- Before trigger delivery launches, host composition must also verify at fire
  time that `creator_user_id` is still authorized for the target
  `tenant_id`/`agent_id`/`project_id`; revoked or unauthorized creators must
  produce a permanent authorization failure instead of submitting a turn.
  Backend unavailability while checking that policy must be retryable. The
  fire-time authorization request is a host-owned shape over
  `tenant_id`/`creator_user_id`/`agent_id`/`project_id`/`trigger_id`/`fire_slot`;
  trigger-domain crates must not own the access policy. Until that request
  is backed by the real agent/project access source of truth, a normal
  runtime must fail closed instead of enabling the trigger poller with the
  tenant-scope placeholder.
- Local-dev `run` and `serve` may satisfy that contract by seeding active
  access rows from trusted operator configuration at boot. `run` reconciles the
  configured CLI owner for its tenant/agent/no-project scope because the generic
  `run` path does not yet wire `[identity].default_project` into trigger create
  scope. `serve` reconciles the env-bearer WebUI user and, when local-dev SSO is
  enabled, existing admitted SSO users at boot plus each admitted SSO identity
  at login. Both paths wire the same store as the fire-time access checker.
  This local-dev access table is bootstrap authorization state only; it is not
  the production agent/project membership source of truth and must not be used
  to justify enabling trigger polling in a production or multi-tenant runtime.
  Bootstrap-owned active rows no longer present in the trusted local admission
  set are marked inactive, while manually inactive rows are not silently
  reactivated. The seeded row is exact
  `tenant_id`/`creator_user_id`/`agent_id`/`project_id` access; a missing
  project is not a wildcard.
- The trusted inbound request is a host-owned synthetic inbound shape around the ordinary inbound fields. It carries only ingress identity and turn scope data needed to create the canonical turn, and it has no adapter-supplied requested-scope hints before binding resolution.
- It must not encode delivery targets, notification targets, or any other outbound routing policy.

Host-trusted trigger ingress request fields are:

- `source`: `TriggerFire`;
- `adapter_kind`: host-trusted trigger ingress marker, not a product adapter kind;
- `adapter_installation_id`: host-trusted trigger installation marker;
- `external_actor_ref`: canonical actor route for the trigger creator authority;
- `external_conversation_ref`: synthetic trigger conversation key plus the
  deterministic route thread digest for this tenant-scoped fire slot;
- `external_event_id`: deterministic replay key derived from the same
  tenant-scoped fire identity;
- `route_kind`: direct;
- `actor`: `TurnActor` for `creator_user_id`;
- `content_ref`: materialized trigger prompt;
- `trusted_inbound_binding`: the canonical trigger-to-conversation binding
  fields used for both prompt recording and trusted turn submission.

The trigger-owned materialization seam keeps `ironclaw_triggers` free of
conversation and product-workflow dependencies: `TriggerPromptMaterializer`
accepts a `TriggerFire` and returns a `TriggerMaterializedPrompt` bundle
containing the opaque `TriggerInboundContentRef` plus the canonical trusted
inbound binding. Composition owns the concrete adapter that writes or resolves
that content ref and computes the binding once for the trusted inbound path.
The sealed `TrustedTriggerSubmitRequest` carries that paired materialized
prompt forward; conversations must not re-derive the binding identity from the
raw fire when submitting the trusted turn.

`TrustedTriggerFireSubmitOutcome` is success-only: accepted or replayed.
Submit failures travel through `Err(TriggerError)` and are classified once by
the trigger worker.

The sealed marker/installation/actor/conversation tuple must resolve to the same
`SourceBindingRef` on every retry of the same tenant-scoped fire identity. Replay
must happen before any new binding creation, so retried fires reuse the original
accepted message and turn submission.

The turn pipeline remains the source of truth for admission, active-lock handling, runner lease handling, approvals, blocking, recovery, and completion.

---

## 7. Run status

`TriggerRunStatus` is synchronous in V1.

- `Ok` means the fire was accepted and submitted into the normal turn pipeline,
  or replayed an already accepted/submitted fire for the same slot.
- `Error` means the fire could not be submitted.
- `ApprovalBlocked` and `TimedOut` are fast-follow statuses and must not appear in the V1 persisted status model unless later lifecycle-observer work is added and ratified.

In V1, `last_status` reflects submit outcome only. It is separate from the
active-fire claim and does not become an in-flight sentinel.

V1 also persists bounded per-trigger run-history rows for product-surface inspection:

- each row is scoped by `(tenant_id, trigger_id, fire_slot)` and records the
  deterministic trigger route thread id, optional submitted `TurnRunId`,
  status, `submitted_at`, and optional `completed_at`;
- `Running` means the fire was claimed or submitted and no terminal cleanup has
  completed for that `fire_slot`;
- `Ok` means active-run cleanup observed a completed terminal turn and cleared
  the exact active fire;
- `Error` means poller-owned claim or submit processing failed before an active
  run could complete, or observed a failed, cancelled, or recovery-required
  terminal turn;
- list APIs return newest rows first and clamp caller limits to the repository
  maximum. A zero limit returns no rows. User-facing list paths must use the
  batched repository query when loading histories for multiple triggers;
- durable repositories retain only the newest 500 run-history rows per trigger.

Run-history rows are observational. They must not be used as the idempotency
ledger for fire replay; deterministic fire identity and the trusted conversation
binding remain the replay source of truth.

Replay of an already accepted/submitted slot returns the original accepted
message and turn submission. If that submitted turn later reaches a terminal
failure, V1 does not mint a second turn for the same `fire_slot`; retry-on-run-
failure requires a later lifecycle-observer contract and a distinct retry
identity policy.

Slot bookkeeping is tied to acceptance, not merely polling:

- accepted or replayed fires write `last_run_at`, `last_fired_slot`,
  `last_status = Ok`, `next_run_at`, `active_fire_slot`, and `active_run_ref`
  in that order; `active_fire_slot` is written before turn submission and
  `active_run_ref` is populated only after the accepted/replayed submit result
  returns a `TurnRunId`;
- retryable submit failures write `last_status = Error`, clear
  `active_fire_slot` and `active_run_ref`, leave `last_fired_slot` and
  `last_run_at` unchanged, and keep `next_run_at` at or before the failed
  fire_slot so the poller can retry it on the next tick;
- permanent validation or authorization failures write `last_status = Error`,
  clear `active_fire_slot` and `active_run_ref`, leave `last_fired_slot` and
  `last_run_at` unchanged, and advance `next_run_at` beyond the failed
  fire_slot;
- permanent failures on a schedule with no future fire slot mark the trigger
  `Completed`, write `last_status = Error`, clear active-fire fields, and leave
  `next_run_at` at the failed fire slot. The `Completed` state, not a sentinel
  timestamp, removes the trigger from future due queries.

Turn terminal lookup and clearing are a narrow seam layered above fire-claim
and submit-result bookkeeping:

- `ironclaw_turns::active_run_ref_state` classifies
  `active_run_ref` through `get_run_state` and `TurnStatus::is_terminal`;
- `ironclaw_triggers::ClearActiveFireRequest` plus
  `TriggerRepository::clear_active_fire` clears only the exact matching
  `(tenant_id, trigger_id, active_fire_slot, active_run_ref)` after the caller
  has observed a terminal turn outcome.

The poller treats per-record due-fire processing and active-run terminal lookup
errors as structured tick report outcomes so one bad record does not block other
eligible triggers in the same tick. Batch-level repository list failures remain
fail-fast because the worker cannot know which records were safely observed.

Approval waits are owned by the normal turn pipeline. While a submitted trigger
turn is waiting for approval, the trigger remains active through
`active_run_ref` back-pressure. Later lifecycle/notification work must define
durable approval expiry, stale approval rejection, reminder throttling, and
user/admin notification paths without making the trigger poller deliver outbound
messages directly.

---

## 8. Capability surface

The trigger system must expose `trigger_create`, `trigger_list`, and `trigger_remove` as first-party Reborn capabilities.

- `trigger_create` validates the schedule, captures caller scope, pairs the
  caller as the host-trusted synthetic trigger actor used by the poller, and
  persists the trigger. This pairing is composition-owned trigger management
  wiring; trigger repositories remain storage-only, and the poller must still
  fail closed for records whose creator actor was not paired.
- `trigger_create` pairs the creator before persisting the trigger record. This
  intentionally fails closed before storage if the actor pairing cannot be
  established, instead of storing a trigger that the poller cannot fire. A
  pairing that remains after a later trigger-record write failure is reusable
  creator-scoped authorization state, not a trigger-specific fire gate.
- Durable local-dev composition must share the same trigger conversation
  services between `trigger_create` pairing and trigger-poller fire submission.
  The shared service preserves the conversation store's mutation lock across
  both paths and avoids racing optimistic durable-state writes.
- `trigger_list` is caller-scoped and surfaces the current schedule state plus
  `last_status` and a bounded `recent_runs` projection. Omitted `run_limit`
  defaults to 25 recent runs per trigger; callers that do not need embedded run
  history pass `run_limit = 0`.
- `trigger_remove` is caller-scoped delete.
- Local-dev builds compiled with `libsql` store trigger records in the
  local-dev libSQL database (`reborn-local-dev.db`) through the same
  `TriggerRepository` contract used by production libSQL. Local-dev builds
  without `libsql` keep the existing in-memory repository behavior.

Exact wiring of the capability registry and handler dependencies may land in later implementation PRs, but the capability names and semantics are frozen here.

Capability follow-ups before launch:

- Trigger count quotas must be enforced through an atomic repository/database
  policy when they are added. A handler-only precheck is not sufficient because
  concurrent creates can race past the cap.
- Durable backend hydration must keep malformed stored trigger rows observable
  as repository errors. Optimizing schedule hydration to avoid cron re-parsing
  is allowed only if the replacement keeps an explicit malformed-row validation
  boundary.
- PostgreSQL scoped-list NULL handling is performance tuning, not a V1
  correctness gate. The schema owns a composite scoped-list index; add
  NULL-specific partial indexes only with `EXPLAIN` or benchmark evidence.

---

## 9. Delivery

Trigger delivery is fast-follow.

- Trigger ingress identity must not include delivery targets.
- Trigger record identity must not include delivery targets.
- Trigger fire identity must not include delivery targets.
- When delivery is added, it must flow through the delivery-resolution/outbound contract track, not through trigger ingress identity.

V1 acceptance does not require external delivery. A valid V1 trigger fire is one that submits a cron-backed synthetic inbound turn and persists through the normal Reborn turn path.

---

## 10. Verification

- Unit tests should cover schedule validation, identity stability, and status serialization.
- Caller-level tests should drive the poller through trusted inbound and into the normal turn pipeline.
- PostgreSQL/libSQL parity is required for trigger persistence.
- `trigger_create` caller-level tests must prove sub-minute and second-level
  schedules are rejected before persistence.
- `trigger_create` caller-level tests must prove accepted finite schedules with
  no future slot at dispatch time are rejected before persistence.
- Trusted inbound caller-level tests must prove duplicate scheduled-slot retries
  replay the original accepted message and turn submission before binding
  creation.
- Poller caller-level tests must prove the worker skips a due fire while another
  fire for the same trigger is active.
- Persistence tests must prove atomic active-fire claim behavior for both
  PostgreSQL and libSQL, including concurrent claim attempts for the same
  trigger and slot.
- Unit tests must prove trigger fire identity derivation is collision-safe for
  delimiter-like or prefix-overlapping component values.
