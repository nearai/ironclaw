# Reborn Contract — Turn Persistence and Active Locks

**Status:** Contract-freeze draft  
**Date:** 2026-05-05  
**Depends on:** [`turns-agent-loop.md`](turns-agent-loop.md), [`host-api.md`](host-api.md), [`events-projections.md`](events-projections.md), [`runtime-profiles.md`](runtime-profiles.md)

---

## 1. Purpose

Turn persistence owns durable control-plane state for host-layer turn coordination:

- accepted turn metadata and canonical binding references;
- executable turn-run lifecycle state;
- one-active-run-per-canonical-thread locks;
- runner lease/checkpoint metadata;
- durable turn-admission reservations for active accepted runs;
- idempotency outcomes for adapter-facing mutations;
- redacted lifecycle cursors needed for replay/recovery.

It does **not** own canonical transcript/message storage. Transcript and thread-message history remain in the transcript/thread storage boundary.

---

## 2. Logical records

The `ironclaw_turns` contract models persistence with these record families:

| Record | Ownership |
| --- | --- |
| `turns` | One accepted inbound message: scope, actor, accepted-message ref, source/reply binding refs, created timestamp. |
| `turn_runs` | Executable state for one run: current source/reply binding refs, status, resolved run-profile snapshot, latest checkpoint/gate refs, runner lease fields, event cursor. |
| `turn_active_locks` | One lock per canonical scoped thread while a run is active or resumable. |
| `turn_checkpoints` | Dedicated checkpoint/gate records written when a running run blocks. |
| `turn_admission_reservations` | Reservation evidence tying each accepted run to tenant/actor/project/agent total and class buckets until terminal release. |
| `turn_idempotency_keys` | Prior sanitized outcomes for scoped submit/resume/cancel idempotency keys. |

The initial PostgreSQL/libSQL adapter slice stores each logical record family in its own table with indexed metadata columns plus a serialized contract payload. Mutations hold a backend transaction/write lock across snapshot load, in-memory contract mutation, and snapshot replacement so active-lock and idempotency semantics remain atomic. Backends must preserve the same semantics as the in-memory contract tests while later slices add incremental row-level updates, targeted read paths, and service-graph wiring.

---

## 3. Active-lock rules

- Active-lock key is the canonical `TurnScope`: tenant, agent, optional project, and thread.
- The key excludes `TurnActor.user_id`, channel IDs, source binding refs, and reply binding refs.
- A lock stores the current owning `TurnRunId`, explicit `TurnStatus`, monotonically increasing `TurnLockVersion`, `acquired_at`, and `updated_at`.
- Queued, running, cancel-requested, blocked, and recovery-required runs keep the lock.
- Terminal runs release the lock exactly once.
- Runner claim/resume/block/cancel-request transitions update the lock status/version while keeping ownership with the same run.

---

## 4. Idempotency rules

Adapter-facing mutations persist sanitized idempotency outcomes:

- `submit_turn` success records the accepted turn/run IDs and accepted response kind.
- `submit_turn` same-thread busy is transient: it does not create a turn/run, does not acquire admission, and is not cached as a submit idempotency replay.
- Capacity/policy admission rejections are replayable and do not create turn/run or reservation records.
- `resume_turn` and `cancel_run` record scoped run-operation outcomes.
- Idempotency records include a redacted replay envelope with response-critical fields such as status, event cursor, admission reason/capacity metadata, retry metadata, and cancellation `already_terminal` state.

A duplicate idempotency key must replay prior accepted submit and admission-rejection outcomes instead of re-running admission, lock acquisition, or state transitions. A duplicate same-thread busy submit with the same key may succeed later after the thread unlocks; legacy persisted `SubmitThreadBusy` replay rows are ignored on snapshot/DB load.

---

## 5. Turn-admission reservation rules

- Admission reservation is not a predicate: all configured tenant, actor-user, project, and agent total/class buckets must be checked and inserted atomically with turn/run creation.
- Each accepted V1 run records unlimited and limited canonical bucket reservations for telemetry and future limit changes.
- Submit admission policy checks that can reject unauthorized/profile-invalid requests run before returning same-thread busy metadata; same-thread busy is still checked before capacity reservation and never consumes admission slots.
- Capacity denial returns one deterministic safe `AdmissionRejected` payload with axis kind, total/class bucket, admission class when applicable, limit, active count, and optional retry hint. It must not expose foreign bucket IDs or raw provider internals.
- Missing limits mean unlimited. A non-AllowAll provider that is unavailable fails closed with `AdmissionRejectionReason::Unavailable` and creates no run/reservation.
- Queued, running, blocked, cancel-requested, and recovery-required runs keep reservations. Resume reuses the existing reservation.
- Terminal transitions (`Completed`, `Failed`, `Cancelled`, and future terminal states) release reservations exactly once. Released reservation evidence is retained only while the corresponding terminal run remains within the bounded terminal-record retention window; active capacity accounting must not scan unbounded released history.
- Limit changes do not evict existing runs; new admissions are denied until active reservations drop below the configured limit.
- Snapshot/DB loaders must synthesize unreleased reservation evidence for legacy non-terminal runs that predate persisted reservation rows so active capacity is not bypassed after migration/restart.

---

## 6. Runner lease and checkpoint rules

- Claiming a queued run atomically moves it to `Running`, stores runner ID/lease token, increments `claim_count`, records `last_heartbeat_at`, records `lease_expires_at`, and updates active-lock metadata.
- Heartbeats only renew metadata for matching, unexpired runner ID/lease token; successful heartbeats refresh `last_heartbeat_at` and extend `lease_expires_at`.
- Expired `Running` and `CancelRequested` leases transition to `RecoveryRequired`, clear current runner ownership, emit a redacted recovery event, and keep the active lock so uncertain side-effecting work is not auto-retried.
- Blocking a running run requires a matching, unexpired lease, writes a checkpoint record, stores the latest checkpoint/gate refs on the run, clears current lease ownership, and keeps the active lock.
- Terminal runner outcomes require the matching, unexpired runner ID/lease token and release the active lock only if the run still owns it.

---

## 7. Redaction boundary

Turn persistence stores metadata and references only. It must not persist raw prompts, assistant content, tool input, secrets, host paths, or backend error details in turn/run/checkpoint/idempotency records.
