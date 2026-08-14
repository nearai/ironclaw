# Reborn Contract — TurnRunner Execution Model

**Status:** Contract-freeze draft  
**Date:** 2026-05-06  
**Depends on:** [`turn-persistence.md`](turn-persistence.md), [`turns-agent-loop.md`](turns-agent-loop.md), [`loop-exit.md`](loop-exit.md), [`runtime-profiles.md`](runtime-profiles.md)

---

## 1. Purpose

`TurnRunner` is the trusted worker-side control plane for executable turn runs. It claims queued runs, maintains leases while model/tool work is active, records safe checkpoint/block/terminal transitions, and moves abandoned work to explicit recovery instead of blindly retrying side effects.

Channel adapters must continue to use `TurnCoordinator`. Runner transition APIs are trusted-worker APIs and remain under `ironclaw_turns::runner`. Driver-facing loop exits remain distinct from trusted runner outcomes; see [`loop-exit.md`](loop-exit.md).

---

## 2. Claim and lease rules

- `submit_turn` creates a queued `TurnRunId` and active-thread lock, but no model/tool side effects may run before a runner claim succeeds.
- `claim_next_processes` atomically moves matching queued agent-turn processes to `Running`.
- A successful claim stores `runner_id`, `lease_token`, `last_heartbeat_at`, `lease_expires_at`, increments `claim_count`, updates the active lock, and emits `RunnerClaimed`.
- `heartbeat` requires the matching `runner_id` and `lease_token`, only refreshes actively `Running` work, and rejects leases whose `lease_expires_at` has already passed. Once cancellation is requested, heartbeats no longer extend the lease; the runner must complete cancellation before the existing lease expires or the reconciler moves the run to recovery. On success, heartbeat refreshes durable `last_heartbeat_at` and extends durable `lease_expires_at`; adapters may touch active-lock freshness and emit/coalesce `RunnerHeartbeat` lifecycle events, but consumers must use lease metadata as the liveness source of truth.
- Pull-based claims are authoritative. Wake notifications are optimization hints only.
- After `TurnCoordinator` durably accepts a submitted run or requeues a resumed/retried run, it may emit a redacted queued-run wake hint containing only the canonical scope, `TurnRunId`, queued status, and event cursor. Wake delivery is best-effort, is not a source of truth, must not fail the durable adapter call, and duplicate hints must be harmless.

---

## 3. Expired lease recovery

- A reconciler scans runner-owned `Running` and `CancelRequested` leases using durable `lease_expires_at` metadata. A lease is expired once `lease_expires_at` is at or before the sweep instant.
- Recovery converges an expired lease directly to a settled state rather than parking it in a distinct recovery status. A branch that resolves to `Cancelled`, `Queued`, or `Failed` clears current runner ownership and emits the matching redacted lifecycle event; a safe checkpoint still inside its grace window remains unchanged, including its expired ownership metadata, until a later sweep resolves it. The canonical-thread active lock is released exactly when the resulting status is terminal. The transition table:
  - `CancelRequested` with an expired lease → terminal `Cancelled`, immediately, with no grace window. Cancellation re-enters no committed work.
  - `Running` with **no** checkpoint, and `claim_count` below `max_crash_recovery_reclaims` → `Queued` (`Resumed`), immediately. There is nothing committed to replay, so the reclaim is safe at once.
  - `Running` parked at a checkpoint that replays no side effect (`BeforeModel`, `BeforeBlock`), and `claim_count` below `max_crash_recovery_reclaims` → `Queued` (`Resumed`), but only after a **grace window of one full lease TTL past expiry**. Inside that window the reconciler leaves the process untouched — an expired lease and a heartbeat-starved but still-live worker look identical, and a live worker would have renewed inside one TTL. A later sweep picks it up. The requeue carries `claim_count` forward as the durable `crash_reclaim_count`, so checkpointed and checkpointless reclaims share one bounded budget. Two further fences bound the zombie case: the supervisor never starts a replacement executor in-process while the reclaimed run's prior executor is still running, and transcript writes are lease-fenced at the write seam, so a worker that outlived every timing bound still cannot land output on the reclaimed run.
  - `Running` parked at a side-effecting checkpoint (`BeforeSideEffect`), or at a checkpoint whose kind this build does not recognize → terminal `Failed` with sanitized category `lease_expired`. An unknown kind fails closed as side-effecting. Side-effect checkpoints are never re-executed.
  - Any expired `Running` lease whose `claim_count` has reached `max_crash_recovery_reclaims` → terminal `Failed`, with category `lease_expired` when a checkpoint exists and `crash_retry_exhausted` when none does. The budget bounds crash-loop reclaims; it is not an unbounded retry.
- The checkpoint kind is carried on the durable process snapshot, not only on the checkpoint row, so a sweep can make this judgement without reading checkpoint payloads, and it survives a store reopen.
- Requeued runs re-enter the normal process-claim path; failed and cancelled runs are terminal and are never re-claimed. The system must not auto-retry uncertain side-effecting work.
- A duplicate/new submit for the same canonical thread remains `ThreadBusy` while a recovered run still holds the active lock.
- Expired-lease recovery never produces `RecoveryRequired`; the transition table above is applied directly. `RecoveryRequired` remains an explicit runner-side outcome (loop-exit validation) and a legacy import/migration status — it is terminal, releases the active lock, and explicit cancellation of it is terminal `Cancelled`.

### 3.1 Checkpointless pre-model failure re-drive

- A trusted scheduler may request `RedriveIfCheckpointless` only for a verified transient failure raised while draining accepted input or constructing the capability surface, context, or prompt before the first `BeforeModel` checkpoint. Other runner failures remain terminal.
- The store is authoritative for eligibility. Cancellation takes precedence; any durable loop checkpoint prevents scratch re-drive; and `claim_count` must remain below `max_crash_recovery_reclaims`.
- An eligible failure atomically retires the current runner lease, returns the same run to `Queued`, preserves the canonical turn/run IDs and `accepted_message_ref`, retains the active-thread lock, clears the transient failure from live state, and emits the queued lifecycle classification. The next claim reconstructs work from the already accepted message rather than accepting or persisting a duplicate input.
- `claim_count` is durable across requeue and process restart. When the bound is reached, or when a checkpoint exists, the run becomes terminal `Failed` with the original sanitized failure category and safe detail. The scheduler must retain the exact active lease identity for a same-run re-drive so shutdown relinquishes only the currently claimed attempt.
- `runner_failure_recovery_covers_terminal_checkpoint_cancel_and_bounded_redrive_states`, `checkpointless_failure_redrive_is_bounded_and_durable_on_libsql`, and `send_user_message_uses_caller_supplied_skill_context_source` enforce the full transition table, durable bounds/reopen semantics, backend behavior, and scheduler-to-composition wiring.

---

## 4. Existing checkpoint and terminal rules

- `block_run` requires the current, unexpired lease, persists a checkpoint/gate ref, clears runner ownership, keeps the active lock, and emits `Blocked`.
- `complete_run`, runner-side `cancel_run`, and `fail_run` require the matching, unexpired lease and release the active lock exactly once at terminal state.
- Failure and recovery/cancel reasons are stable sanitized categories only; raw prompts, tool input, host paths, backend errors, and secrets stay out of turn state and lifecycle events.
- A pre-model `CheckpointRejected` is deterministic for the proposed state and
  is never repaired by relabeling, blindly retried, or followed by model/tool
  work. If private checkpoint payload staging succeeded but checkpoint metadata
  validation failed, the payload remains non-authoritative. The runner
  terminalizes exactly once through the synchronously durable turn-state
  row/`Failed` event channel, persists a bounded host-authored explanation, and
  prohibits retry of that failed run. This is the explicit host-authored
  exception to model-final-word handling; product projection must not invoke a
  failure-explainer model or manufacture an assistant transcript message.

The canonical caller regressions are
`executor_checkpoint_rejection_maps_to_host_authored_terminal_explanation`
(`crates/loop/ironclaw_turn_runner/src/planned_driver.rs`) and
`retry_rejects_checkpoint_rejection_without_creating_a_process`
(`crates/kernel/ironclaw_turns/src/process_projection/tests.rs`); the latter
selector is also mapped into `scripts/reborn-e2e-rust.sh`.

---

## 5. Loop exit validation

Agent-loop drivers return `LoopExit` claims. `TurnRunner` validates those claims before applying a trusted outcome:

- valid completed exits require host-verified durable reply/result refs and map to `TurnRunnerOutcome::Completed`;
- valid blocked exits require host-verified checkpoint + gate refs and map to `TurnRunnerOutcome::Blocked`;
- valid cancelled exits require observed host cancellation/interrupt and map to `TurnRunnerOutcome::Cancelled`; a missing final checkpoint is allowed for host-initiated cancellation because the host can preempt the driver before checkpointing; runner-side application then consults durable run state in one transition-port operation, terminalizing only recorded `CancelRequested` runs and mapping observed interrupts that race ahead of recorded cancellation to recovery instead of terminal cancellation;
- valid failed exits require host-verified evidence that the failure is safe to terminalize, then map stable sanitized failure kinds or sanitized safe summaries to `TurnRunnerOutcome::Failed`; failed outcomes may include host-verified explanation refs and a retry checkpoint id admitted by the checkpoint policy;
- invalid exits map either to sanitized terminal failure or runner/system-derived `RecoveryRequired` depending on side-effect safety evidence;
- runner-side loop-exit application must call trusted transition-port methods, not mutate durable run state directly.

## 6. Deferred work

The current slices define the core lease/recovery state machine, initial PostgreSQL/libSQL persistence adapters, pure `LoopExit` validation/mapping types, trusted `LoopExitApplier` policy derivation from host-owned evidence, host-runtime production scheduler wiring, and failed-run retry persistence plus runner resume execution through `RetryTurnRequest`/`RetryTurnResponse`. Durable exit-id replay storage, transcript draft validation, side-effect boundary checkpoint cadence inside the loop, and safe explicit fork UX remain follow-up slices.
