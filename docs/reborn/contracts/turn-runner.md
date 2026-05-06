# Reborn Contract — TurnRunner Execution Model

**Status:** Contract-freeze draft  
**Date:** 2026-05-06  
**Depends on:** [`turn-persistence.md`](turn-persistence.md), [`turns-agent-loop.md`](turns-agent-loop.md), [`runtime-profiles.md`](runtime-profiles.md)

---

## 1. Purpose

`TurnRunner` is the trusted worker-side control plane for executable turn runs. It claims queued runs, maintains leases while model/tool work is active, records safe checkpoint/block/terminal transitions, and moves abandoned work to explicit recovery instead of blindly retrying side effects.

Product adapters must continue to use `TurnCoordinator`. Runner transition APIs are trusted-worker APIs and remain under `ironclaw_turns::runner`.

---

## 2. Claim and lease rules

- `submit_turn` creates a queued `TurnRunId` and active-thread lock, but no model/tool side effects may run before a runner claim succeeds.
- `claim_next_run` atomically moves one matching `Queued` run to `Running`.
- A successful claim stores `runner_id`, `lease_token`, `last_heartbeat_at`, `lease_expires_at`, increments `claim_count`, updates the active lock, and emits `RunnerClaimed`.
- `heartbeat` requires the matching `runner_id` and `lease_token` and rejects leases whose `lease_expires_at` has already passed; on success it refreshes `last_heartbeat_at`, extends `lease_expires_at`, touches the active lock, and emits `RunnerHeartbeat`.
- Pull-based claims are authoritative. Wake notifications are optimization hints only.

---

## 3. Expired lease recovery

- A reconciler scans runner-owned `Running` and `CancelRequested` leases using durable `lease_expires_at` metadata.
- Expired `Running` or `CancelRequested` leases transition to `RecoveryRequired`, clear current runner ownership, emit a redacted `RecoveryRequired` event with reason `lease_expired`, and keep the same canonical-thread active lock.
- `RecoveryRequired` runs are not returned by the normal `claim_next_run` path. The system must not auto-retry uncertain side-effecting work.
- A duplicate/new submit for the same canonical thread remains `ThreadBusy` while recovery is required.
- Explicit cancellation of `RecoveryRequired` is terminal `Cancelled` and releases the active lock so a new turn can be submitted.

---

## 4. Existing checkpoint and terminal rules

- `block_run` requires the current, unexpired lease, persists a checkpoint/gate ref, clears runner ownership, keeps the active lock, and emits `Blocked`.
- `complete_run`, runner-side `cancel_run`, and `fail_run` require the matching, unexpired lease and release the active lock exactly once at terminal state.
- Failure and recovery/cancel reasons are stable sanitized categories only; raw prompts, tool input, host paths, backend errors, and secrets stay out of turn state and lifecycle events.

---

## 5. Deferred work

The current `ironclaw_turns` slices define the core lease/recovery state machine and initial PostgreSQL/libSQL persistence adapters. AgentLoopHost/AgentLoopDriver integration, side-effect boundary checkpoint cadence inside the loop, production service-graph wiring, and safe explicit retry/fork UX remain follow-up slices.
