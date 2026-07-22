# Reborn Contract — TurnRunner Execution Model

**Status:** Contract-freeze draft  
**Date:** 2026-05-06  
**Last reconciled:** 2026-07-22 (#6455)
**Depends on:** [`turn-persistence.md`](turn-persistence.md), [`turns-agent-loop.md`](turns-agent-loop.md), [`loop-exit.md`](loop-exit.md), [`runtime-profiles.md`](runtime-profiles.md)

---

## 1. Purpose

`TurnRunner` is the trusted worker-side control plane for executable turn runs. It claims queued runs, maintains leases while model/tool work is active, records safe checkpoint/block/terminal transitions, and moves abandoned work through checkpoint-aware, bounded recovery instead of blindly retrying uncertain side effects.

Product adapters must continue to use `TurnCoordinator`. Runner transition APIs are trusted-worker APIs and remain under `ironclaw_turns::runner`. Driver-facing loop exits remain distinct from trusted runner outcomes; see [`loop-exit.md`](loop-exit.md).

---

## 2. Claim and lease rules

- `submit_turn` creates a queued `TurnRunId` and active-thread lock, but no model/tool side effects may run before a runner claim succeeds.
- `claim_next_run` atomically moves one matching `Queued` run to `Running`.
- A successful claim stores `runner_id`, `lease_token`, `last_heartbeat_at`, `lease_expires_at`, increments `claim_count`, updates the active lock, and emits `RunnerClaimed`.
- `heartbeat` requires the matching `runner_id` and `lease_token`, only refreshes actively `Running` work, and rejects leases whose `lease_expires_at` has already passed. Once cancellation is requested, heartbeats no longer extend the lease; the runner must complete cancellation before the existing lease expires or the reconciler terminalizes the run as `Cancelled`. On success, heartbeat refreshes durable `last_heartbeat_at` and extends durable `lease_expires_at`; adapters may touch active-lock freshness and emit/coalesce `RunnerHeartbeat` lifecycle events, but consumers must use lease metadata as the liveness source of truth.
- Pull-based claims are authoritative. Wake notifications are optimization hints only.
- After `TurnCoordinator` durably accepts a submitted run or requeues a resumed/retried run, it may emit a redacted queued-run wake hint containing only the canonical scope, `TurnRunId`, queued status, and event cursor. Wake delivery is best-effort, is not a source of truth, must not fail the durable adapter call, and duplicate hints must be harmless.

---

## 3. Expired lease recovery

- A reconciler scans runner-owned `Running` and `CancelRequested` leases using durable `lease_expires_at` metadata.
- An expired `CancelRequested` lease becomes terminal `Cancelled`, clears runner ownership, and releases the canonical-thread active lock.
- An expired `Running` lease with any loop checkpoint becomes terminal `Failed(lease_expired)`. The latest resumable checkpoint is attached when one exists, making the normal explicit retry path available; a run with only a non-resumable checkpoint is not retried from scratch.
- An expired checkpointless `Running` lease is safe to re-drive because it has not crossed the first loop checkpoint. Recovery clears runner ownership and requeues it as `Queued`, while retaining the canonical-thread active lock and preserving `claim_count`.
- Checkpointless re-drive is bounded by `max_crash_recovery_reclaims`. Once `claim_count` reaches the bound, recovery produces terminal `Failed(crash_retry_exhausted)` and releases the active lock.
- A requeued run is claimable through the normal `claim_next_run` path. A duplicate/new submit for the same canonical thread remains `ThreadBusy` while that active run is queued or running.
- Recovery emits the lifecycle event for the resulting state. Because recovered runs leave `Running`/`CancelRequested`, later reconciliation scans do not repeat the same transition.
- `RecoveryRequired` remains readable as legacy status vocabulary, but current lease recovery does not produce it.

---

## 4. Existing checkpoint and terminal rules

- `block_run` requires the current, unexpired lease, persists a checkpoint/gate ref, clears runner ownership, keeps the active lock, and emits `Blocked`.
- `complete_run`, runner-side `cancel_run`, and `fail_run` require the matching, unexpired lease and release the active lock exactly once at terminal state.
- Failure and recovery/cancel reasons are stable sanitized categories only; raw prompts, tool input, host paths, backend errors, and secrets stay out of turn state and lifecycle events.

---

## 5. Loop exit validation

Agent-loop drivers return `LoopExit` claims. `TurnRunner` validates those claims before applying a trusted outcome:

- valid completed exits require host-verified durable reply/result refs and map to `TurnRunnerOutcome::Completed`;
- valid blocked exits require host-verified checkpoint + gate refs and map to `TurnRunnerOutcome::Blocked`;
- valid cancelled exits require observed host cancellation/interrupt and map to `TurnRunnerOutcome::Cancelled`; a missing final checkpoint is allowed for host-initiated cancellation because the host can preempt the driver before checkpointing; runner-side application then consults durable run state in one transition-port operation, terminalizing only recorded `CancelRequested` runs and mapping observed interrupts that race ahead of recorded cancellation to a sanitized terminal failure instead of terminal cancellation;
- valid failed exits require host-verified evidence that the failure is safe to terminalize, then map stable sanitized failure kinds or sanitized safe summaries to `TurnRunnerOutcome::Failed`; failed outcomes may include host-verified explanation refs and a retry checkpoint id admitted by the checkpoint policy;
- invalid exits map to a sanitized terminal failure. The loop-exit policy still names its unsafe internal mapping `RecoveryRequired`, but the current persistence transition terminalizes that mapping as `Failed` rather than persisting a new `RecoveryRequired` state;
- runner-side loop-exit application must call trusted transition-port methods, not mutate durable run state directly.

## 6. Deferred work

The current slices define the core lease/recovery state machine, initial PostgreSQL/libSQL persistence adapters, pure `LoopExit` validation/mapping types, trusted `LoopExitApplier` policy derivation from host-owned evidence, host-runtime production scheduler wiring, and failed-run retry persistence plus runner resume execution through `RetryTurnRequest`/`RetryTurnResponse`. Durable exit-id replay storage, transcript draft validation, side-effect boundary checkpoint cadence inside the loop, and safe explicit fork UX remain follow-up slices.

Broken-tool detection and automatic rebuild are not turn-runner responsibilities. Their disposition is recorded separately in [`../tier-b-self-repair-reconciliation.md`](../tier-b-self-repair-reconciliation.md).

## 7. Verification

The caller-level and persistence-contract evidence for this behavior is:

- scheduler heartbeats, heartbeat failure handling, cancellation, and periodic lease reconciliation: `crates/ironclaw_runner/tests/turn_scheduler_contract.rs`;
- checkpointless requeue and bounded exhaustion across a durable-store crash: `crates/ironclaw_turns/tests/row_store_crash_consistency.rs`;
- checkpoint-preserving failed-run retry: `crates/ironclaw_turns/tests/retry_failed_turn_store_contract.rs`;
- terminal state, lifecycle event, and active-lock release: `crates/ironclaw_turns/tests/turn_coordinator_contract.rs`;
- a production-shaped scheduler/tool-dispatch wedge: `tests/integration/lease_wedge.rs`.

Run the focused evidence with:

```bash
cargo test -p ironclaw_runner --test turn_scheduler_contract
cargo test -p ironclaw_turns --test row_store_crash_consistency lease_expiry_
cargo test -p ironclaw_turns --test retry_failed_turn_store_contract lease_recovery_preserves_retryability
cargo test -p ironclaw_turns --test turn_coordinator_contract expired_running_lease_fails_and_releases_thread_lock
cargo test -p ironclaw_turns --test turn_coordinator_contract expired_cancel_requested_lease_cancels_and_releases_thread_lock
cargo test --test reborn_integration_lease_wedge
```
