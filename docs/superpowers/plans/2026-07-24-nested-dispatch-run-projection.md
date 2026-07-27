# Nested Dispatch Run Projection Fix Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prevent a failed capability dispatch inside a successful loop run from being projected to WebChat as a failed turn run.

**Architecture:** The dispatcher already knows the enclosing loop `RunId`; copy that identity into `RuntimeEvent.parent_invocation_id` for its requested/selected/succeeded/failed events. The runtime projection must keep those parented dispatcher events in the capability-activity read model while excluding them from the run-status read model. Unparented direct dispatch and process events retain their current run-projection behavior.

**Tech Stack:** Rust, Tokio contract tests, Reborn durable runtime events, event projections, product projection stream.

## Global Constraints

- A nested capability failure remains visible as a failed `CapabilityActivity`; only the false failed `RunStatus` is removed.
- A loop/turn may become terminally failed only from its authoritative loop/turn lifecycle event, not from a child capability dispatch.
- Standalone unparented dispatcher events retain their existing run-status projection behavior.
- Dispatcher events remain metadata-only and must not add raw input, output, provider errors, host paths, or secrets.
- Do not change frontend error-message handling in this task; fix the incorrect backend projection at its source.
- Add regression tests that fail before production code changes, observe the expected RED failure, and then pass after the minimal implementation.
- Do not add `.unwrap()` or `.expect()` to production code.
- Preserve the existing event-sink best-effort behavior and dispatcher success/failure outcomes.
- Update the relevant event and projection contracts, naming the tests that enforce the new behavior.

---

### Task 1: Preserve nested dispatch lineage and project only the parent run

**Files:**
- Modify: `crates/ironclaw_dispatcher/src/lib.rs`
- Test: `crates/ironclaw_dispatcher/tests/event_dispatch_contract.rs`
- Modify: `crates/ironclaw_event_projections/src/runtime_projection.rs`
- Test: `crates/ironclaw_event_projections/tests/nested_dispatch_projection_contract.rs`
- Test: `crates/ironclaw_host_runtime/tests/host_runtime_services_contract.rs`
- Modify: `crates/ironclaw_reborn_composition/src/projection/tests.rs`
- Test: `crates/ironclaw_reborn_composition/src/projection/tests/nested_dispatch_stream.rs`
- Modify: `docs/reborn/contracts/events.md`
- Modify: `docs/reborn/contracts/events-projections.md`
- Modify: `scripts/reborn-e2e-rust.sh`

**Interfaces:**
- Consumes: `InvocationOrigin::{LoopRun,ScheduledLoopRun}(RunId)`, `RunId::as_uuid`, `InvocationId::from_uuid`, and `RuntimeEvent.parent_invocation_id`.
- Produces: dispatcher runtime events whose `parent_invocation_id` is the enclosing loop invocation for non-process loop capability calls; runtime snapshots containing one authoritative parent run plus a separately failed child capability activity.

- [ ] **Step 1: Add failing dispatcher lineage tests**

Extend `crates/ironclaw_dispatcher/tests/event_dispatch_contract.rs` with caller-level success and failure cases that construct an authorized request with `run_id: Some(run_id)`, drive `RuntimeDispatcher::dispatch_json`, and assert every emitted dispatch lifecycle event carries:

```rust
Some(InvocationId::from_uuid(run_id.as_uuid()))
```

The success case must cover `DispatchRequested`, `RuntimeSelected`, and `DispatchSucceeded`. The failure case must cover `DispatchRequested`, `RuntimeSelected`, and `DispatchFailed`.

- [ ] **Step 2: Run the dispatcher tests to verify RED**

Run:

```bash
cargo test -p ironclaw_dispatcher --test event_dispatch_contract dispatcher_marks_loop_dispatch_events_with_parent_run -- --nocapture
```

Expected: FAIL because current dispatcher events have `parent_invocation_id == None`.

- [ ] **Step 3: Add failing projection contract tests**

In `crates/ironclaw_event_projections/tests/nested_dispatch_projection_contract.rs`, add snapshot and cursor-resume cases:

1. A parent `ModelStarted` event scoped to the parent invocation.
2. Child `DispatchRequested`, `RuntimeSelected`, and `DispatchFailed` events scoped to a distinct child invocation and carrying `parent_invocation_id = Some(parent_invocation_id)`.
3. A parent `AssistantReplyFinalized` and/or `LoopCompleted` event.

Assert:

```rust
assert_eq!(snapshot.runs.len(), 1);
assert_eq!(snapshot.runs[0].invocation_id, parent_invocation_id);
assert_eq!(snapshot.runs[0].status, RunProjectionStatus::Completed);
assert_eq!(snapshot.capability_activities.len(), 1);
assert_eq!(
    snapshot.capability_activities[0].status,
    CapabilityActivityStatus::Failed
);
assert_eq!(
    snapshot.capability_activities[0].run_id,
    Some(parent_invocation_id)
);
```

In `crates/ironclaw_reborn_composition/src/projection/tests/nested_dispatch_stream.rs`, reproduce the same durable event sequence through the real `build_reborn_projection_services(...).product_event_stream().drain(...)` caller path for both snapshots and cursor resumes. Assert the product projection contains the completed parent `RunStatus` and failed child `CapabilityActivity`, and contains no failed `RunStatus` for the child invocation.

- [ ] **Step 4: Run the projection tests to verify RED**

Run:

```bash
cargo test -p ironclaw_event_projections --test nested_dispatch_projection_contract -- --nocapture
cargo test -p ironclaw_reborn_composition --lib projection::tests::nested_dispatch_stream -- --nocapture
```

Expected: FAIL because the child `DispatchFailed` is currently inserted into `RuntimeProjectionState.runs` and serialized as a failed product run status.

- [ ] **Step 5: Propagate the parent identity from the dispatcher**

In `RuntimeDispatcher::dispatch_json`, derive the existing optional loop `RunId` as today and map it to:

```rust
let parent_invocation_id =
    run_id.map(|run_id| InvocationId::from_uuid(run_id.as_uuid()));
```

Attach that value to every dispatcher-created `RuntimeEvent` for the same invocation—requested, runtime selected, succeeded, and failed—without changing the adapter request or dispatch result. Thread the parent identity through `emit_dispatch_failure` so all failure exits preserve the same lineage.

- [ ] **Step 6: Exclude parented dispatcher events from run status**

In `apply_run_event`, return early for parented dispatcher lifecycle events:

```rust
let nested_dispatch = event.parent_invocation_id.is_some()
    && matches!(
        event.kind,
        RuntimeEventKind::DispatchRequested
            | RuntimeEventKind::RuntimeSelected
            | RuntimeEventKind::DispatchSucceeded
            | RuntimeEventKind::DispatchFailed
    );
```

Keep the existing exclusion for `CapabilityActivityRequested`, `CapabilityActivitySucceeded`, and `CapabilityActivityFailed`. Do not exclude unparented dispatcher events or process lifecycle events.

- [ ] **Step 7: Run focused tests to verify GREEN**

Run:

```bash
cargo test -p ironclaw_dispatcher --test event_dispatch_contract dispatcher_marks_loop_dispatch_events_with_parent_run -- --nocapture
cargo test -p ironclaw_event_projections --test nested_dispatch_projection_contract -- --nocapture
cargo test -p ironclaw_reborn_composition --lib projection::tests::nested_dispatch_stream -- --nocapture
```

Expected: PASS. The child capability remains failed, while only the completed parent appears as a run.

- [ ] **Step 8: Update contracts**

Update `docs/reborn/contracts/events.md` to state that dispatcher events for loop-origin capability calls carry the enclosing run as `parent_invocation_id`.

Update `docs/reborn/contracts/events-projections.md` to state that parented dispatcher lifecycle events feed capability activity only and never create run-status rows. Name:

```text
crates/ironclaw_event_projections/tests/nested_dispatch_projection_contract.rs::runtime_snapshot_keeps_nested_dispatch_failure_out_of_run_status
crates/ironclaw_event_projections/tests/nested_dispatch_projection_contract.rs::runtime_resume_keeps_late_nested_dispatch_failure_out_of_run_status
crates/ironclaw_reborn_composition/src/projection/tests/nested_dispatch_stream.rs::product_event_stream_snapshot_keeps_nested_dispatch_failure_out_of_run_status
crates/ironclaw_reborn_composition/src/projection/tests/nested_dispatch_stream.rs::product_event_stream_cursor_resume_keeps_late_nested_failure_out_of_run_status
```

- [ ] **Step 9: Run task validation**

Run:

```bash
cargo fmt --all -- --check
cargo test -p ironclaw_dispatcher
cargo test -p ironclaw_event_projections
cargo test -p ironclaw_reborn_composition
cargo clippy -p ironclaw_dispatcher --all-targets --all-features -- -D warnings
cargo clippy -p ironclaw_event_projections --all-targets --all-features -- -D warnings
cargo clippy -p ironclaw_reborn_composition --all-targets --all-features -- -D warnings
cargo test -p ironclaw_architecture
bash scripts/reborn-e2e-rust.sh
scripts/pre-commit-safety.sh
```

Run the repository-required workspace-wide zero-warning check before committing:

```bash
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

- [ ] **Step 10: Self-review and commit**

Search changed production Rust for prohibited panic calls and sibling projection mistakes:

```bash
rg -n '\\.(unwrap|expect)\\(' crates/ironclaw_dispatcher/src/lib.rs crates/ironclaw_event_projections/src/runtime_projection.rs
rg -n 'DispatchFailed.*RunProjectionStatus::Failed|apply_run_event' crates
git diff --check
git status --short
```

Commit only the scoped code, tests, contracts, and this implementation plan:

```bash
git add \
  crates/ironclaw_dispatcher/src/lib.rs \
  crates/ironclaw_dispatcher/tests/event_dispatch_contract.rs \
  crates/ironclaw_event_projections/src/runtime_projection.rs \
  crates/ironclaw_event_projections/tests/nested_dispatch_projection_contract.rs \
  crates/ironclaw_host_runtime/tests/host_runtime_services_contract.rs \
  crates/ironclaw_reborn_composition/src/projection/tests.rs \
  crates/ironclaw_reborn_composition/src/projection/tests/nested_dispatch_stream.rs \
  docs/reborn/contracts/events.md \
  docs/reborn/contracts/events-projections.md \
  docs/superpowers/plans/2026-07-24-nested-dispatch-run-projection.md \
  scripts/reborn-e2e-rust.sh
git commit -m "fix(reborn): keep tool failures out of run status"
```
