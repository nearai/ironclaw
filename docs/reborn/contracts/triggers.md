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
| `enabled` | Optional denormalized query flag derived from `state`; not an independent fire gate |
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
- If an implementation stores `enabled` for index/query convenience, it must be
  derived from `state == Scheduled` and must never override `state`.

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

- The worker may poll on a configured interval and batch due triggers.
- The worker must enforce `max_concurrent_fires_per_trigger = 1` in V1 through
  an atomic repository claim/lease operation that covers read, eligibility
  check, active-fire check, and claim write.
- If a previous fire for the same trigger is still active, the current tick for that trigger is skipped.
- A skipped tick does not create a second fire, does not create a second thread, and does not fork a parallel trigger loop.
- Active means the previous fire has not yet reached a terminal turn outcome.

`last_status` is not the active-fire sentinel. Active-fire state is tracked by
the separate `active_fire_slot` / `active_run_ref` claim fields and cleared only
after the referenced turn reaches a terminal outcome. PostgreSQL and libSQL
backends must provide equivalent atomic claim semantics; in-memory tests are
not sufficient evidence for this invariant.

The skip policy is per-trigger, not global. Other triggers may continue to fire on the same tick.

---

## 6. Trusted inbound and turn execution

A trigger fire is synthetic inbound, not a parallel agent loop.

- The fire must enter the normal Reborn inbound/turn pipeline.
- Planned facade: `InboundTurnService::handle_inbound_turn_with_trusted_scope(...)`.
  This method does not exist in the current implemented conversation slice; PR 8
  in the implementation plan owns adding it and its caller-level tests.
- Binding resolution for trigger fires must use the trusted-scope path from `conversation-binding.md`.
- The host-trusted ingress marker and witness used for trigger submission must be type-sealed and unconstructible by product adapters.
- The host mints the trusted trigger ingress request from `TriggerRecord` state:
  `tenant_id`, `creator_user_id`, `agent_id`, and `project_id` are host state,
  not product payload data.
- The synthetic inbound request may carry only ingress identity and turn scope data needed to create the canonical turn.
- It must not encode delivery targets, notification targets, or any other outbound routing policy.

Synthetic trigger `InboundTurnRequest` fields are:

- `adapter_kind`: sealed host-trusted ingress marker, not a product adapter kind;
- `adapter_installation_id`: sealed host-trusted trigger installation marker;
- `external_actor_ref`: canonical actor route for the trigger creator authority;
- `external_conversation_ref`: synthetic dedicated-thread route for this fire,
  derived from `tenant_id + trigger_id + fire_slot`;
- `external_event_id`: deterministic replay key derived from the same
  tenant-scoped fire identity;
- `route_kind`: direct;
- `actor`: `TurnActor` for `creator_user_id`;
- `content_ref`: materialized trigger prompt.

The planned trusted facade takes a typed trusted request that bundles the
ordinary `InboundTurnRequest` fields with host-owned `tenant_id`,
`creator_user_id`, `agent_id`, and `project_id` authority. `creator_user_id` is
converted into the canonical `TurnActor` by the host-owned trusted path; product
adapters cannot supply or override it.

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

Replay of an already accepted/submitted slot returns the original accepted
message and turn submission. If that submitted turn later reaches a terminal
failure, V1 does not mint a second turn for the same `fire_slot`; retry-on-run-
failure requires a later lifecycle-observer contract and a distinct retry
identity policy.

Slot bookkeeping is tied to acceptance, not merely polling:

- accepted or replayed fires advance `last_fired_slot`, set `last_status = Ok`,
  set the active-fire claim to the accepted/submitted turn reference, and
  compute the next future slot;
- retryable submit failures leave `last_fired_slot` unchanged, set
  `last_status = Error`, leave the active-fire claim unset, and leave the slot
  retryable. `next_run_at` must remain at or before the failed slot's scheduled
  time so the poller can retry it on the next tick;
- permanent validation or authorization failures set `last_status = Error` and
  must not silently create a different route, actor, or scope.

---

## 8. Capability surface

The trigger system must expose `trigger_create`, `trigger_list`, and `trigger_remove` as first-party Reborn capabilities.

- `trigger_create` validates the schedule, captures caller scope, and persists the trigger.
- `trigger_list` is caller-scoped and surfaces the current schedule state plus `last_status`.
- `trigger_remove` is caller-scoped delete.

Exact wiring of the capability registry and handler dependencies may land in later implementation PRs, but the capability names and semantics are frozen here.

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
