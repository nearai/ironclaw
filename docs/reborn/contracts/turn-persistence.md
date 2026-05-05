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
| `turn_idempotency_keys` | Prior sanitized outcomes for scoped submit/resume/cancel idempotency keys. |

Concrete PostgreSQL/libSQL tables are intentionally deferred until the DB adapter slice. Backends must preserve the same semantics when those adapters are added.

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
- `submit_turn` busy path records a `ThreadBusy` outcome without creating a new turn/run.
- Admission rejections are replayable and do not create turn/run records.
- `resume_turn` and `cancel_run` record scoped run-operation outcomes.
- Idempotency records include a redacted replay envelope with response-critical fields such as status, event cursor, active run ID, admission reason/retry metadata, and cancellation `already_terminal` state.

A duplicate idempotency key must replay the prior sanitized success/error outcome instead of re-running admission, lock acquisition, or state transitions.

---

## 5. Runner lease and checkpoint rules

- Claiming a queued run atomically moves it to `Running`, stores runner ID/lease token, increments `claim_count`, and updates heartbeat metadata.
- Heartbeats only renew metadata for matching runner ID/lease token.
- Blocking a running run writes a checkpoint record, stores the latest checkpoint/gate refs on the run, clears current lease ownership, and keeps the active lock.
- Terminal runner outcomes require the matching runner ID/lease token and release the active lock only if the run still owns it.

---

## 6. Redaction boundary

Turn persistence stores metadata and references only. It must not persist raw prompts, assistant content, tool input, secrets, host paths, or backend error details in turn/run/checkpoint/idempotency records.
