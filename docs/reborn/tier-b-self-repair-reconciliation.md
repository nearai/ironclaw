# Tier B Self-Repair Reconciliation

Issues: #6455, parent #6369
Date: 2026-07-22

## Decision

The v1 self-repair module combined two independent features:

1. recovery of jobs that stopped making progress; and
2. detection and automatic rebuilding of repeatedly failing dynamic tools.

Reborn already satisfies the stuck-run part through its durable runner lease,
heartbeat, checkpoint, retry, and lifecycle contracts. This issue therefore
does not add a second scheduler, background worker, recovery state machine, or
configuration default. The broken-tool rebuild part is not satisfied by lease
recovery and remains separate future work.

The historical comparison point is
`5b307e2c920e2b6e1e6f219a8776103d1906257f:src/agent/self_repair.rs`, the final
v1 tree before the legacy monolith was removed.

## Stuck-run mapping

| v1 responsibility | Reborn owner and behavior | Evidence | Disposition |
| --- | --- | --- | --- |
| Detect an in-progress job that stopped making progress | `TurnRunScheduler` heartbeats active executors; the durable turn store identifies expired `Running` and `CancelRequested` leases from `lease_expires_at`; the scheduler periodically invokes `recover_expired_leases`. | `crates/ironclaw_runner/src/turn_scheduler.rs`; `scheduler_heartbeats_long_running_executor_until_completion`; `wedged_tool_call_is_reaped_by_lease_expiry_not_left_running_forever` | Satisfied |
| Recover work that is safe to run again | A checkpointless expired run is cleared of lease ownership and requeued as `Queued`; normal claim processing re-drives it. Work that crossed a loop checkpoint is not blindly restarted. | `crates/ironclaw_turns/src/filesystem_store/turn_state_engine/transitions.rs`; `lease_expiry_requeues_checkpointless_run_as_redrivable` | Satisfied |
| Bound repeated recovery attempts | Every claim increments durable `claim_count`; checkpointless recovery stops at `max_crash_recovery_reclaims` (default `5`) and terminal-fails with `crash_retry_exhausted`. | `crates/ironclaw_turns/src/filesystem_store/turn_state_engine/limits.rs`; `lease_expiry_crash_retry_bound_fails_with_crash_retry_exhausted`; `expired_lease_reconciler_fails_running_run_at_crash_retry_bound` | Satisfied |
| Preserve safe progress instead of restarting uncertain work | A run with any loop checkpoint becomes `Failed(lease_expired)`; the latest resumable checkpoint is attached when available and can be used by the explicit failed-run retry path. | `assert_lease_recovery_preserves_retryability` in `crates/ironclaw_turns/tests/retry_failed_turn_store_contract.rs` | Satisfied |
| Stop and report terminal recovery outcomes | An expired recorded `CancelRequested` lease becomes `Cancelled`; exhausted pre-checkpoint recovery becomes `Failed(crash_retry_exhausted)`; checkpointed expiry becomes `Failed(lease_expired)`. An observed interrupt that races ahead of persisted cancellation becomes sanitized terminal `Failed`, not `Cancelled`. Terminal transitions release the thread lock and publish sanitized lifecycle state. | `expired_running_lease_fails_and_releases_thread_lock`; `expired_cancel_requested_lease_cancels_and_releases_thread_lock`; `tests/integration/lease_wedge.rs` | Satisfied |
| Avoid repeated repair notifications for the same stuck item | Reconciliation only selects `Running`/`CancelRequested`; each result leaves those states, so later scans cannot repeat the same recovery transition. Requeued work can expire again only after a new claim, and that cycle is bounded. | `recover_expired_leases` transition code and the bounded reclaim tests above | Satisfied |

No additional caller-level regression test is needed for #6455: the scheduler
contract covers heartbeat and reconciliation, the row-store contract covers
requeue and exhaustion, the retry contract covers checkpoints, and the root
integration test wedges the real tool-dispatch caller path.

## Broken-tool auto-rebuild split

Lease recovery answers whether a turn run is still owned by a live runner. It
does not establish that a tool artifact is defective, decide whether source is
trusted, compile replacement code, install an artifact, or activate it.

The v1 behavior that counted repeated tool failures and invoked
`SoftwareBuilder`/`ToolRegistry` to rebuild non-built-in tools is therefore
**not** covered by Reborn lease recovery and is intentionally out of scope for
#6455. It should be tracked in a separate future issue, for example:

> Reconcile v1 broken-tool detection and auto-rebuild with the Reborn extension lifecycle

That follow-up must first define failure attribution, trusted source and build
isolation, approval policy, bounded attempts, artifact verification, activation
rollback, and operator-visible evidence. It must not be implemented as an
implicit side effect of turn lease recovery.

## Compatibility and rollback

- Runtime behavior and defaults are unchanged.
- Persistence schemas and serialized status vocabulary are unchanged;
  `RecoveryRequired` remains readable for legacy compatibility.
- The corrected contract describes behavior already shipped and tested under
  #6284.
- Rollback is a documentation revert only.

## Validation

```bash
cargo test -p ironclaw_runner --test turn_scheduler_contract
cargo test -p ironclaw_turns --test row_store_crash_consistency lease_expiry_
cargo test -p ironclaw_turns --test retry_failed_turn_store_contract lease_recovery_preserves_retryability
cargo test -p ironclaw_turns --test turn_coordinator_contract expired_running_lease_fails_and_releases_thread_lock
cargo test -p ironclaw_turns --test turn_coordinator_contract expired_cancel_requested_lease_cancels_and_releases_thread_lock
cargo test --test reborn_integration_lease_wedge
```
