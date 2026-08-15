# Capability Invocation Edge Writes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Persist fresh capability invocation state with one atomic journal command at its first suspend or terminal edge while retaining durable, lease-fenced cross-worker gate resume.

**Architecture:** `ProcessInvocationStore` keeps fresh `ProcessInvocationStart` values in a worker-local map. The process journal gains an atomic edge-submission command that creates a process directly as suspended, completed, or failed. Pending records use that command; records already persisted by a gate continue through the existing resume, claim, and terminal transition path.

**Tech Stack:** Rust, Tokio, async-trait, serde-tagged process journal commands, in-memory `RootFilesystem` test backend, Reborn integration harness.

---

## File map

- `crates/kernel/ironclaw_processes/src/journal.rs` — public atomic edge request/enum and submission-port method.
- `crates/kernel/ironclaw_processes/src/journal_store/command.rs` — persisted tagged command and row-load references.
- `crates/kernel/ironclaw_processes/src/journal_store/state.rs` — command application, shared submission validation, direct edge snapshot construction.
- `crates/kernel/ironclaw_processes/src/journal_store.rs` — journal-store port implementation.
- `crates/kernel/ironclaw_processes/src/lib.rs` — exports for the new request/enum.
- `crates/kernel/ironclaw_processes/src/invocation_state.rs` — worker-local pending map and pending-to-edge promotion.
- `crates/kernel/ironclaw_turns/src/process_projection/store_adapter.rs` — delegate the expanded submission contract.
- `crates/loop/ironclaw_turn_runner/src/steering_reconcile.rs` — update test/runtime adapters that implement the expanded process port, if the compiler identifies them.
- `crates/kernel/ironclaw_capabilities/tests/capability_host_process_integration.rs` — fresh non-gated one-write contract.
- `crates/kernel/ironclaw_capabilities/tests/capability_host_dispatcher_integration.rs` — real second-store approval resume contract.
- `crates/kernel/ironclaw_host_runtime/tests/host_runtime_contract.rs` — same-worker pending visibility and cross-worker non-visibility.
- `tests/integration/lease_wedge.rs` — durable `BeforeSideEffect` checkpoint evidence after simulated worker loss.

### Task 1: Atomic process edge submission

**Files:**
- Modify: `crates/kernel/ironclaw_processes/src/journal.rs`
- Modify: `crates/kernel/ironclaw_processes/src/journal_store/command.rs`
- Modify: `crates/kernel/ironclaw_processes/src/journal_store/state.rs`
- Modify: `crates/kernel/ironclaw_processes/src/journal_store/state_tests.rs`
- Modify: `crates/kernel/ironclaw_processes/src/journal_store.rs`
- Modify: `crates/kernel/ironclaw_processes/src/lib.rs`
- Modify: `crates/kernel/ironclaw_turns/src/process_projection/store_adapter.rs`

- [ ] **Step 1: Write failing state-machine tests for direct edges**

Add table-driven tests beside the existing submission tests in `journal_store/state_tests.rs`. Build one `SubmitProcessAtEdgeRequest` per edge and assert one journal entry is emitted with no lease:

```rust
for (edge, expected_status, expected_kind) in [
    (
        ProcessSubmissionEdge::Completed,
        ProcessLifecycleStatus::Completed,
        ProcessJournalKind::Completed,
    ),
    (
        ProcessSubmissionEdge::Failed {
            failure: SanitizedFailure::from_trusted_static("capability_failed"),
        },
        ProcessLifecycleStatus::Failed,
        ProcessJournalKind::Failed,
    ),
] {
    let mut state = ProcessJournalMaterializedState::default();
    let outcome = state
        .apply_command(StoredProcessCommand::SubmitAtEdge(Box::new(
            SubmitProcessAtEdgeRequest {
                submission: submission(ProcessId::new(), request_scope.clone()),
                edge,
            },
        )))
        .expect("edge submission applies");
    let StoredCommandOutcome::Submitted(snapshot, true) = outcome else {
        panic!("expected new edge submission");
    };
    assert_eq!(snapshot.status, expected_status);
    assert_eq!(snapshot.lease, None);
    assert_eq!(state.journal.len(), 1);
    assert_eq!(state.journal[0].kind, expected_kind);
}
```

Add a suspended case with `submission.checkpoint_ref = Some(...)` and `ProcessSubmissionEdge::Suspended { suspension }`; assert `Suspended`, the checkpoint reference, and the complete suspension payload. Add rejection tests for suspended-without-checkpoint and for an edge request carrying process input, lineage, dependency, or `exclusive_within_scope`; the edge API is intentionally limited to standalone bookkeeping processes.

- [ ] **Step 2: Run the new tests and confirm RED**

Run:

```bash
cargo test -p ironclaw_processes journal_store::state_tests -- --nocapture
```

Expected: compile failure because `ProcessSubmissionEdge`, `SubmitProcessAtEdgeRequest`, and `StoredProcessCommand::SubmitAtEdge` do not exist.

- [ ] **Step 3: Add the public edge request and port method**

In `journal.rs`, add:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProcessSubmissionEdge {
    Suspended { suspension: ProcessSuspension },
    Completed,
    Failed { failure: SanitizedFailure },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubmitProcessAtEdgeRequest {
    pub submission: SubmitProcessRequest,
    pub edge: ProcessSubmissionEdge,
}
```

Extend `ProcessSubmissionPort` with:

```rust
async fn submit_process_at_edge(
    &self,
    request: SubmitProcessAtEdgeRequest,
) -> Result<JournaledProcessSnapshot, Self::Error>;
```

Export both types from `lib.rs`. Before editing the exported trait, run language-server references for `ProcessSubmissionPort`; if rust-analyzer remains unavailable, use the already identified implementations and let the targeted compiler enumerate any remaining adapters.

- [ ] **Step 4: Add the persisted command and state transition**

Add `StoredProcessCommand::SubmitAtEdge(Box<SubmitProcessAtEdgeRequest>)` in `command.rs`, include the same scope/process load references as `Submit`, and route it in `state.rs` to `apply_submit_at_edge`.

Refactor submission validation and snapshot insertion into one private helper that accepts an initial snapshot shape. Normal `Submit` passes queued/submitted. `SubmitAtEdge` passes the requested edge and emits only the matching edge entry. Preserve the existing submission replay key and return `Submitted(existing, false)` for an identical replay. Reject an edge replay whose process identity exists with different immutable metadata.

For the edge path, construct these fields exactly:

```rust
let (status, kind, suspension, failure) = match request.edge {
    ProcessSubmissionEdge::Suspended { suspension } => (
        ProcessLifecycleStatus::Suspended,
        ProcessJournalKind::Suspended,
        Some(suspension),
        None,
    ),
    ProcessSubmissionEdge::Completed => (
        ProcessLifecycleStatus::Completed,
        ProcessJournalKind::Completed,
        None,
        None,
    ),
    ProcessSubmissionEdge::Failed { failure } => (
        ProcessLifecycleStatus::Failed,
        ProcessJournalKind::Failed,
        None,
        Some(failure),
    ),
};
```

The resulting snapshot has `lease: None`, `crash_reclaim_count: 0`, and one cursor shared by the snapshot and its only journal entry. A suspended edge requires `submission.checkpoint_ref.is_some()`; terminal edges clear `checkpoint_ref`.

- [ ] **Step 5: Implement and delegate the port method**

In `journal_store.rs`, execute `StoredProcessCommand::SubmitAtEdge` and require `StoredCommandOutcome::Submitted`. In `ironclaw_turns`' store adapter, delegate to the wrapped runtime and map the process error through the same conversion as `submit_process`. Add direct delegation to any compiler-reported process-runtime test adapters; do not add a default no-op.

- [ ] **Step 6: Verify GREEN and compatibility tests**

Run:

```bash
cargo test -p ironclaw_processes journal_store::state_tests -- --nocapture
cargo test -p ironclaw_processes process_journal_store_contract -- --nocapture
cargo test -p ironclaw_turns process_projection -- --nocapture
```

Expected: all selected tests pass. The serialized command round-trip test must include `submit_at_edge` so the new tagged variant is permanently readable.

- [ ] **Step 7: Commit the atomic journal primitive**

```bash
git add crates/kernel/ironclaw_processes crates/kernel/ironclaw_turns/src/process_projection/store_adapter.rs
git commit -m "feat(processes): add atomic process edge submission"
```

### Task 2: Worker-local fresh invocation state

**Files:**
- Modify: `crates/kernel/ironclaw_processes/src/invocation_state.rs`

- [ ] **Step 1: Replace the existing transition test with failing edge-write contracts**

Extend the existing `invocation_state.rs` tests with three observable contracts:

```rust
#[tokio::test]
async fn fresh_completion_writes_only_the_terminal_edge() {
    let (store, journal) = process_store();
    let invocation_id = InvocationId::new();
    let scope = scope(invocation_id);

    store.start(start(invocation_id, scope.clone())).await.unwrap();
    assert_eq!(store.get(&scope, invocation_id).await.unwrap().unwrap().status,
        ProcessInvocationStatus::Running);
    assert!(journal.read_process_journal_after(&scope, None, None, 16)
        .await.unwrap().entries.is_empty());

    store.complete(&scope, invocation_id).await.unwrap();
    let page = journal.read_process_journal_after(&scope, None, None, 16).await.unwrap();
    assert_eq!(page.entries.iter().map(|entry| entry.kind).collect::<Vec<_>>(),
        vec![ProcessJournalKind::Completed]);
}
```

Add `fresh_suspension_is_visible_after_store_reconstruction`: store A starts and blocks approval/auth, store B over the same journal loads the record. Add `failed_edge_write_retains_pending_record` using the existing one-shot filesystem fault seam or a small failing runtime adapter, and assert local `get` still returns `Running`.

- [ ] **Step 2: Run and confirm RED**

```bash
cargo test -p ironclaw_processes invocation_state::tests -- --nocapture
```

Expected: the first test observes `Submitted` and `Claimed` before completion.

- [ ] **Step 3: Add pending storage without holding a lock across I/O**

Change `ProcessInvocationStore` to own:

```rust
pending: std::sync::Mutex<HashMap<InvocationId, ProcessInvocationStart>>,
```

`start` checks durable state for an existing invocation, inserts under the mutex, and returns a derived `Running` record. Map a poisoned mutex to `ProcessInvocationError::Backend("pending invocation state lock poisoned".to_string())`. Never retain the mutex guard across `.await`.

Add focused helpers:

```rust
fn pending_record(start: &ProcessInvocationStart) -> ProcessInvocationRecord;
fn pending_start(
    &self,
    scope: &ResourceScope,
    invocation_id: InvocationId,
) -> Result<Option<ProcessInvocationStart>, ProcessInvocationError>;
fn remove_pending(
    &self,
    invocation_id: InvocationId,
) -> Result<(), ProcessInvocationError>;
```

`pending_start` must return `None` on scope mismatch so a caller cannot promote another scope's local record.

- [ ] **Step 4: Promote pending records at edges**

For `block_approval`, `block_auth`, `complete`, and `fail`, branch on `pending_start`:

```rust
if let Some(start) = self.pending_start(scope, invocation_id)? {
    let snapshot = self
        .processes
        .submit_process_at_edge(SubmitProcessAtEdgeRequest {
            submission: Self::submission(start, checkpoint_ref, metadata)?,
            edge,
        })
        .await
        .map_err(|error| map_process_error(error, invocation_id))?;
    self.remove_pending(invocation_id)?;
    return Self::record(snapshot)?
        .ok_or(ProcessInvocationError::UnknownInvocation { invocation_id });
}
```

Approval/auth edges reuse the exact existing metadata and `ProcessSuspension` payloads. Completed and failed edges reuse the exact existing metadata and sanitized failure construction. If no pending record exists, preserve the current durable transition code unchanged for gate resumes.

- [ ] **Step 5: Merge local pending records into readers**

`get` returns a durable snapshot first, then a matching pending record. `records_for_scope` loads durable records, adds local pending records for the exact scope only when their invocation ID is absent, and retains the current invocation-ID sort order.

- [ ] **Step 6: Verify GREEN**

```bash
cargo test -p ironclaw_processes invocation_state::tests -- --nocapture
cargo test -p ironclaw_processes
```

Expected: the fresh terminal page contains one entry; reconstructed stores read suspended/terminal state; existing durable resume tests still pass.

- [ ] **Step 7: Commit local buffering**

```bash
git add crates/kernel/ironclaw_processes/src/invocation_state.rs
git commit -m "feat(processes): persist invocation state at edges"
```

### Task 3: Capability-host end-to-end invocation contracts

**Files:**
- Modify: `crates/kernel/ironclaw_capabilities/tests/capability_host_process_integration.rs`
- Modify: `crates/kernel/ironclaw_capabilities/tests/capability_host_dispatcher_integration.rs`

- [ ] **Step 1: Add a failing non-gated journal-count test**

Build `ProcessServices::in_memory()`, construct a production `ProcessInvocationStore` from `process_services.process_runtime()`, invoke the real capability host through the recording runtime dispatcher, then read journal entries for the invocation scope:

```rust
let entries = process_runtime
    .read_process_journal_after(&scope, None, None, 16)
    .await
    .unwrap()
    .entries
    .into_iter()
    .filter(|entry| entry.process_kind == ProcessKind::CapabilityInvocationState)
    .collect::<Vec<_>>();
assert_eq!(entries.len(), 1);
assert_eq!(entries[0].kind, ProcessJournalKind::Completed);
```

Also reload through a second `ProcessInvocationStore` and assert the record is completed with the original capability, scope, and actor.

- [ ] **Step 2: Convert the existing approval integration to two workers**

In `capability_host_blocks_then_resumes_approved_dispatch_through_runtime_dispatcher`, replace the test-only map store with two production stores sharing one `ProcessRuntimePort`. Attach store A to `block_host` and store B to `resume_host`. Read the blocked record through store B before approval, then resume through store B and read the completed record through a newly constructed store C.

- [ ] **Step 3: Run both tests and confirm RED before Task 2 implementation, then GREEN after it**

```bash
cargo test -p ironclaw_capabilities --test capability_host_process_integration -- --nocapture
cargo test -p ironclaw_capabilities --test capability_host_dispatcher_integration capability_host_blocks_then_resumes_approved_dispatch_through_runtime_dispatcher -- --exact --nocapture
```

Expected after implementation: both pass; the adapter dispatch count remains one; the approval lease is consumed once.

- [ ] **Step 4: Run all capability-host contract tests**

```bash
cargo test -p ironclaw_capabilities
```

Expected: all tests pass, including auth resume, approval mismatch, obligation failure, spawn, and custom state-store doubles.

- [ ] **Step 5: Commit capability regression coverage**

```bash
git add crates/kernel/ironclaw_capabilities/tests/capability_host_process_integration.rs crates/kernel/ironclaw_capabilities/tests/capability_host_dispatcher_integration.rs
git commit -m "test(capabilities): prove invocation edge persistence"
```

### Task 4: Reader visibility contract

**Files:**
- Modify: `crates/kernel/ironclaw_host_runtime/tests/host_runtime_contract.rs`

- [ ] **Step 1: Make runtime-status coverage use the production store**

Update `default_runtime_status_reports_running_invocations_only` to use store A backed by an in-memory process runtime. After `start`, retain the existing same-worker `active_work` assertions. Construct store B over the same runtime and assert:

```rust
assert!(store_b
    .records_for_scope(&context.resource_scope)
    .await
    .unwrap()
    .is_empty());
```

This pins the deliberate loss of cross-worker visibility for fresh, unrecoverable inline calls without changing same-worker status/cancellation behavior.

- [ ] **Step 2: Verify reader tests**

```bash
cargo test -p ironclaw_host_runtime --test host_runtime_contract default_runtime_status_reports_running_invocations_only -- --exact --nocapture
cargo test -p ironclaw_host_runtime --test host_runtime_contract default_runtime_cancel_reports_running_invocations_as_unsupported -- --exact --nocapture
```

Expected: both pass.

- [ ] **Step 3: Commit reader coverage**

```bash
git add crates/kernel/ironclaw_host_runtime/tests/host_runtime_contract.rs
git commit -m "test(host-runtime): pin local invocation visibility"
```

### Task 5: Crash boundary integration evidence

**Files:**
- Modify: `tests/integration/lease_wedge.rs`

- [ ] **Step 1: Strengthen the existing wedged-tool test**

After the run reaches `TurnStatus::Failed`, assert the run still identifies the durable checkpoint written before dispatch:

```rust
assert!(
    state.checkpoint_id.is_some(),
    "a lease-expired possible side effect must retain its durable BeforeSideEffect checkpoint"
);
assert_eq!(gate.dispatch_count(), 1, "lease recovery must not redispatch the tool");
```

If `ParkingCapabilityGate` does not expose a count, extend its existing atomic state with a read-only test helper; do not add a production seam. The existing failed `lease_expired` assertion remains.

- [ ] **Step 2: Run and confirm the strengthened assertion detects the intended boundary**

Run before any needed helper change:

```bash
cargo test --test reborn_integration_lease_wedge wedged_tool_call_is_reaped_by_lease_expiry_not_left_running_forever -- --exact --nocapture
```

Expected: checkpoint assertion passes on current behavior; dispatch-count compilation fails until the test helper is added, or the existing recorder provides the count. This test is a retained safety contract, not a production behavior change.

- [ ] **Step 3: Run the complete lease-wedge target**

```bash
cargo test --test reborn_integration_lease_wedge -- --nocapture
```

Expected: both the fail-closed `BeforeSideEffect` case and resumable `BeforeModel` case pass.

- [ ] **Step 4: Commit crash-boundary coverage**

```bash
git add tests/integration/lease_wedge.rs tests/integration/support/doubles/parking_host_runtime.rs
git commit -m "test: retain side-effect checkpoint evidence on lease loss"
```

### Task 6: Architecture, formatting, and final verification

**Files:**
- Review all files modified above.

- [ ] **Step 1: Run the architecture review skill checklist**

Confirm the expanded submission method remains owned by `ironclaw_processes`, adds no dependency edge, and does not let product/composition bypass process validation. Confirm the atomic edge API rejects general executable process submissions so queued execution cannot be skipped accidentally.

- [ ] **Step 2: Format**

```bash
cargo fmt
```

Expected: success.

- [ ] **Step 3: Run focused crate suites**

```bash
cargo test -p ironclaw_processes
cargo test -p ironclaw_capabilities
cargo test -p ironclaw_host_runtime
cargo test -p ironclaw_turns
cargo test --test reborn_integration_lease_wedge
cargo test -p ironclaw_architecture_tests
```

Expected: all pass.

- [ ] **Step 4: Run warnings-as-errors checks**

```bash
cargo clippy -p ironclaw_processes --all-targets --all-features -- -D warnings
cargo clippy -p ironclaw_capabilities --all-targets --all-features -- -D warnings
cargo clippy -p ironclaw_host_runtime --all-targets --all-features -- -D warnings
cargo clippy -p ironclaw_turns --all-targets --all-features -- -D warnings
```

Expected: all pass with zero warnings.

- [ ] **Step 5: Run source safety scans**

Search changed production files for `.unwrap()`, `.expect()`, hardcoded temporary paths, and error mappings that discard causes. Confirm every occurrence is either absent or test-only. Search the process journal command readers for exhaustive handling of `SubmitAtEdge`.

- [ ] **Step 6: Verify docs placement**

```bash
python3 scripts/ci/docs_publication_boundary.py
```

Expected: `docs/ publication boundary: every page is published or fenced`.

- [ ] **Step 7: Final commit if formatting or adapter fixes remain**

```bash
git add crates tests docs/internal/superpowers
git commit -m "feat: collapse capability invocation state writes"
```

Skip this commit when the worktree is already clean after the task commits.
