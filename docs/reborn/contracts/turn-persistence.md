# Reborn Contract — Agent-Turn Process Projection

**Status:** Implemented
**Date:** 2026-07-25
**Depends on:** [`turns-agent-loop.md`](turns-agent-loop.md), [`host-api.md`](host-api.md), [`events-projections.md`](events-projections.md), [`runtime-profiles.md`](runtime-profiles.md)

---

## 1. Purpose

Agent-turn coordination projects these domain concepts from the durable process
journal:

- accepted turn metadata and canonical binding references;
- executable process lifecycle state;
- one-active-process-per-canonical-thread concurrency;
- process lease/checkpoint metadata;
- durable turn-admission reservations for active accepted runs;
- idempotency outcomes for adapter-facing mutations;
- redacted lifecycle cursors needed for replay/recovery.

It does **not** own canonical transcript/message storage. Transcript and thread-message history remain in the transcript/thread storage boundary.

---

## 2. Logical records

`ironclaw_processes::ProcessJournalStore` is authoritative. The
`ironclaw_turns` contract exposes these projected record families:

| Record | Ownership |
| --- | --- |
| `turns` | One accepted inbound message: scope, actor, accepted-message ref, source/reply binding refs, created timestamp. |
| `turn_runs` | Agent-turn view of one process snapshot: bindings, status, resolved run profile, checkpoint/gate refs, lease fields, and journal cursor. |
| `turn_active_locks` | Agent-turn view of process concurrency ownership for a canonical scoped thread. |
| `turn_checkpoints` | Agent-turn view of process suspension/checkpoint records. |
| `turn_admission_reservations` | Reservation evidence tying each accepted run to tenant/actor/project/agent total and class buckets until terminal release. |
| `turn_idempotency_keys` | Prior sanitized outcomes for scoped submit/resume/cancel idempotency keys. |

Agent-turn metadata is stored as a bounded process metadata payload.
`AgentTurnProcessRuntime` maps coordinator operations to process submission,
control, journal, and tree ports, then reconstructs these turn views. It does
not persist a parallel turn snapshot.

---

## 3. Active-lock rules

- Active-lock key is the canonical `TurnScope`: tenant, agent, optional project, and thread.
- The key excludes `TurnActor.user_id`, channel IDs, source binding refs, and reply binding refs.
- A lock stores the current owning `TurnRunId`, explicit `TurnStatus`, monotonically increasing `TurnLockVersion`, `acquired_at`, and `updated_at`.
- Queued, running, cancel-requested, and blocked runs keep the lock.
- Terminal runs, including legacy `RecoveryRequired` records, release the lock exactly once.
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
- Physical adapters may split high-churn runner lease metadata from lower-churn turn snapshots/tables, as long as all read, recovery, and terminal transition APIs expose one logical run state. Liveness decisions must use durable lease metadata, not require one lifecycle event per heartbeat.
- Expired `Running` and `CancelRequested` leases clear current runner ownership and emit a redacted recovery event only when recovery resolves them to `Cancelled`, `Queued`, or `Failed`. A safe checkpoint still inside its full lease-TTL grace window remains unchanged, including its expired ownership metadata, until a later sweep resolves it. Recovery otherwise converges directly to a settled state rather than parking work in a distinct recovery status: cancellation to terminal `Cancelled`; a run with no checkpoint or one parked at a checkpoint that replays no side effect (`BeforeModel`, `BeforeBlock`) back to `Queued`; and a run parked at a side-effecting or unrecognized checkpoint, or one that has exhausted the bounded reclaim budget, to terminal `Failed`. Uncertain side-effecting work is still never auto-retried, and the active lock is released exactly when the resulting status is terminal. The full transition table is in `turn-runner.md` §3.
- Blocking a running run requires a matching, unexpired lease, writes a checkpoint record, stores the latest checkpoint/gate refs on the run, clears current lease ownership, and keeps the active lock.
- Loop-driver resume payloads are staged only in host memory. The subsequent
  process checkpoint command atomically persists the opaque ref, schema
  metadata, and bounded payload in one journal row.
- Process checkpoint records are scoped by the stable run/process identity and
  turn resource scope. Reads with a matching ref but foreign scope or run
  return no state, preserving tenant/thread/run isolation.
- Checkpoint payload bytes are bounded and debug-redacted. Lifecycle event,
  public turn/run, transport, and idempotency projections expose only metadata
  and refs, never raw checkpoint payload bytes.
- Terminal runner outcomes require the matching, unexpired runner ID/lease token and release the active lock only if the run still owns it.

---

## 7. Redaction boundary

Turn persistence stores metadata and references only. It must not persist raw prompts, assistant content, tool input, secrets, host paths, or backend error details in turn/run/checkpoint/idempotency records.
