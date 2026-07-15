# Gap B — Dead failure-category arms in Reborn failure-explanation path

Date: 2026-06-24
Branch: `fix/reborn-dead-failure-categories`

## Summary

The Reborn failure-explanation / safe-summary path has two independent
category→message mappers. Several match arms are DEAD: the value the arm
matches is never produced by the code that feeds the mapper, so execution
falls through to the generic fallback (`"The run failed before producing a
reply."` / `"The run failed for an unknown reason."`). Users see a useless
generic message even when a specific category-aware message exists.

This was verified end-to-end with codegraph + producer tracing (file:line
evidence below), not assumed from the arm names.

## The two mappers and their producers

### Mapper A — `reborn_failure_summary_for_category`
`crates/ironclaw_reborn_composition/src/failure_summary.rs:5`

Fed by `TurnLifecycleEvent.sanitized_reason`
(`crates/ironclaw_reborn_composition/src/projection/turn_events.rs:799-806`,
`failure_category_for_turn_event`). `sanitized_reason` is set from
`SanitizedFailure::category()` in
`crates/ironclaw_turns/src/lifecycle.rs:450-458`
(`sanitized_reason_for_state`) and the mirror in
`crates/ironclaw_loop_host/src/turn_event_publisher.rs:76-84`.

Exhaustive set of values `sanitized_reason` can take when status is
`Failed`/`RecoveryRequired`:
- `LoopFailureKind::as_str()` (`crates/ironclaw_turns/src/loop_exit.rs:449-464`):
  `model_error`, `context_build_failed`, `capability_protocol_error`,
  `iteration_limit`, `invalid_model_output`, `checkpoint_rejected`,
  `checkpoint_unavailable`, `transcript_write_failed`, `driver_bug`,
  `interrupted_unexpectedly`, `no_progress_detected`, `policy_denied`,
  `compaction_unavailable`
- `LoopExitViolationKind::failure_category()` (`loop_exit.rs:692-703`):
  `interrupted_unexpectedly`, `driver_protocol_violation`
- `RebornTurnRunExecutor` (`crates/ironclaw_runner/src/turn_run_executor.rs`,
  `turn_runner.rs`): `driver_not_found`, `host_creation_failed`,
  `route_snapshot_persistence_failed`, `driver_invalid_request`,
  `driver_unavailable`, `driver_failed`, `model_credits_exhausted`,
  `model_credentials_unavailable`, `unknown_failure`, `exit_application_failed`
- `TurnRunScheduler` (`crates/ironclaw_runner/src/turn_scheduler.rs:655,668`):
  **`scheduler_executor_panic`**, **`scheduler_heartbeat_failed`**
- store paths (`crates/ironclaw_turns/src/memory/mod.rs:2284,2990`):
  `interrupted_unexpectedly`, `lease_expired`

LIVE/DEAD verdict for Mapper A arms:

| Arm | Verdict | Evidence |
|---|---|---|
| `driver_not_found` | LIVE | turn_run_executor.rs:105 |
| `driver_unavailable` | LIVE | turn_run_executor.rs:118 |
| `driver_failed` | LIVE | turn_runner.rs:55,59 |
| `driver_invalid_request` | LIVE | turn_run_executor.rs:115 |
| `driver_panic` | **DEAD** | producer emits `scheduler_executor_panic` (turn_scheduler.rs:655); `driver_panic` only in test fixtures |
| `host_creation_failed` | LIVE | turn_run_executor.rs:108 |
| `route_snapshot_persistence_failed` | LIVE | turn_run_executor.rs:111 |
| `heartbeat_failed` | **DEAD** | producer emits `scheduler_heartbeat_failed` (turn_scheduler.rs:668) |
| `exit_application_failed` | LIVE | turn_run_executor.rs:283 |
| `lease_expired` | LIVE | memory/mod.rs:2990 |
| `interrupted_unexpectedly` | LIVE | memory/mod.rs:2284, loop_exit.rs:694 |
| `no_progress_detected` | LIVE | loop_exit.rs:461 |
| `iteration_limit` | LIVE | loop_exit.rs:454 |
| `unknown_failure` | LIVE | turn_runner.rs:31 |

### Mapper B — `runtime_failure_summary_for_category`
`crates/ironclaw_reborn_composition/src/projection.rs:1115`

Fed by `RunStatusProjection.error_kind` via `run_failure_category`
(projection.rs:1092) → `SanitizedFailure::new(category).ok()`. `error_kind`
is set in `apply_run_event`
(`crates/ironclaw_event_projections/src/runtime_projection.rs:209-253`) from
`RuntimeEvent.error_kind`, and is only read at `Failed`/`Killed` status.

The error_kind values that reach Failed/Killed status come from:
- `DispatchError::event_kind()`
  (`crates/ironclaw_dispatcher/src/dispatch.rs:409-422`): `unknown_capability`,
  `unknown_provider`, `runtime_mismatch`, `missing_runtime_backend`,
  `unsupported_runtime`, `auth_required`, `backend`, ... `unknown`
- `LoopFailureKind::as_str()` (loop_exit.rs:449): the loop-failure strings above
- process failures: `obligations.rs:1027` passes `"unknown"` fallback;
  `wrappers.rs:124` passes `"Unknown"` → sanitizes to `"Unclassified"` →
  filtered out by `SanitizedFailure::new().ok()`

LIVE/DEAD verdict for Mapper B arms:

| Arm | Verdict | Evidence |
|---|---|---|
| `model_failed` | **DEAD** | `ModelFailed` event keeps status `Running`, never Failed/Killed (runtime_projection.rs:396-398). No producer emits the literal `model_failed` as error_kind at Failed/Killed. |
| `dispatch_failed` | **DEAD** | `DispatchFailed` → Failed, but error_kind = `DispatchError::event_kind()` codes (e.g. `missing_runtime_backend`), never `dispatch_failed` (dispatch.rs:409-422) |
| `process_failed` | **DEAD** | `ProcessFailed` → Failed, but error_kind = process-record string / `unknown` fallback, never `process_failed` (obligations.rs:1027) |
| `process_killed` | **DEAD** | `RuntimeEvent::process_killed` sets `error_kind: None` (runtime_event.rs:589-590) |
| `hook_failed` | **DEAD** | `HookFailed` never transitions to Failed/Killed and sets `error_kind: None` (runtime_event.rs) |
| `process_killed` | (same row) | — |
| `unknown` | LIVE | `obligations.rs:1027` fallback `"unknown"` |
| `unclassified` | **DEAD** | sanitizer fallback is `Unclassified` (capital U) which fails `SanitizedFailure::new()`; lowercase `unclassified` never produced |

## User-visible symptom (incident logs)

`logs.1782330623559.log` / `logs.1782331100360.log` show the produced
runtime values are `failure_category=lease_expired` and
`error_kind="auth_required"` / `"authorization"` — i.e. the real values are
NOT any of the dead arms; the dead arms are unreachable. `auth_required`
falls through Mapper B's generic `_` arm, confirming the real values bypass
the specific arms.

## Decision

Per-arm, choosing the correct AND simplest option:

### Mapper A: WIRE (rename arms to the real produced strings)
The producer emits a stable, specific value (`scheduler_heartbeat_failed`,
`scheduler_executor_panic`). Renaming the dead arms to those exact strings
makes them live with NO invented data — users get a specific message for the
two scheduler-terminal failures. The existing copy is reused (heartbeat copy
stays; panic gets a clear "stopped unexpectedly" message).

- `"heartbeat_failed"` → `"scheduler_heartbeat_failed"`
- `"driver_panic"` → `"scheduler_executor_panic"` (keep the same message text;
  it already reads "stopped unexpectedly")

### Mapper B: DELETE the dead arms
The arms (`model_failed`, `dispatch_failed`, `process_failed`,
`process_killed`, `hook_failed`, `unclassified`) match a namespace
(milestone-kind names) the producer never emits as error_kind. Wiring would
require inventing a coarsening map from the finer-grained producer namespace
(`DispatchError` codes, `LoopFailureKind` codes) — data the producer does not
supply in this form. That is exactly the DELETE criterion. The real produced
values already fall through the generic `_` arm; that behavior is unchanged
and pre-existing. Keep `unknown` (LIVE).

The stale comment at projection.rs:1093-1097 (which claims `model_failed`,
`dispatch_failed`, `process_killed` are runtime-replay categories) is also
corrected to reflect reality.

Nothing else depends on these arms (codegraph impact: only the mapper
function and the file). No enum/type plumbing is removed — both mappers take
`&str`, so there is no cross-crate type to clean up.

## Tests (drive the caller, not just the helper)

- Mapper A: assert `reborn_failure_summary_for_category(Some("scheduler_heartbeat_failed"))`
  and `..("scheduler_executor_panic")` return the specific copy (was generic).
  Add a regression test that the *old* dead strings are no longer specially
  cased (fall through to generic), documenting the rename.
- Mapper B (driven end-to-end through the runtime projection via real
  `RuntimeEvent`s): assert `unknown` → specific copy, and that real produced
  values hit the generic fallback for BOTH producer families — `model_error`
  via `RuntimeEvent::loop_failed` (a `LoopFailureKind` code) and
  `missing_runtime_backend` via `RuntimeEvent::dispatch_failed` (a
  `DispatchError` code) — documenting that Mapper B intentionally only
  specially-cases `unknown`.
- Driver-level: the failure_explanation projection tests
  (`crates/ironclaw_reborn_composition/src/projection/tests/failure_explanation.rs`)
  already exercise `failure_details_for_turn_event`; update the `driver_panic`
  fixture to the real `scheduler_executor_panic` string and assert the
  specific summary so the caller path is covered.

## Quality gate

`cargo fmt --all`; `cargo clippy` + `cargo test` on
`ironclaw_reborn_composition` (the only touched crate).
