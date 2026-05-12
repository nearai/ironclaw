# WS-8 — End-to-End Integration Tests

**Workstream:** WS-8 (final — closes the skeleton)
**Crates touched:** `ironclaw_agent_loop` + `ironclaw_reborn`
**Depends on:** WS-7 (and therefore implicitly all of WS-0 through WS-6)
**Master doc:** [`../agent-loop-skeleton.md`](../agent-loop-skeleton.md) §3, §8, §10, §13

---

## 1. Scope

Land the cross-workstream integration suite that proves the framework actually composes into a working loop. Per-workstream briefs (WS-0 through WS-7) each verify their own surface in isolation; this brief verifies the *intersections* — the bugs that only surface when multiple strategies, the executor, and the driver adapter run together against a realistic host.

Three pieces:

1. **`test_support` module** in `ironclaw_agent_loop`, feature-gated so production builds don't pull it in. Houses the canonical `MockAgentLoopDriverHost`, scenario builders, state builders, and a checkpoint-capture recorder. Becomes the shared fixture for all current and future loop-family integration tests.
2. **Integration test suite** in `ironclaw_agent_loop/tests/` — four test files covering happy paths, safety nets, strategy interactions, and state lifecycle.
3. **Driver-side end-to-end** in `ironclaw_reborn/tests/` — exercises `PlannedDriver` through the real `AgentLoopDriver` surface, consuming the `test_support` fixtures via dev-dependencies with `features = ["test-support"]`.

The single command `cargo test --workspace --features ironclaw_agent_loop/test-support` is the green-light for the whole skeleton.

## 2. Files

### NEW
- `crates/ironclaw_agent_loop/src/test_support/mod.rs` — feature-gated. Contains:
  - `MockAgentLoopDriverHost` builder with scriptable model responses, capability outcomes, gates, errors, cancellation
  - `CheckpointRecorder` — captures the ordered sequence of `(CheckpointKind, iteration)` writes for assertion
  - `LoopExecutionStateBuilder` — ergonomic construction of bespoke states (e.g. preload `recent_call_signatures` for repetition tests)
  - `ScenarioScript` — small DSL for "model returns X on call N, capability Y returns Z, then Reply"
- `crates/ironclaw_agent_loop/tests/executor_happy_paths.rs`
- `crates/ironclaw_agent_loop/tests/safety_nets.rs`
- `crates/ironclaw_agent_loop/tests/strategy_interactions.rs`
- `crates/ironclaw_agent_loop/tests/state_lifecycle.rs`
- `crates/ironclaw_reborn/tests/planned_driver_e2e.rs`

### EXTEND
- `crates/ironclaw_agent_loop/Cargo.toml` — add `[features] test-support = []` (no extra deps; the module itself uses things already in `[dependencies]`)
- `crates/ironclaw_agent_loop/src/lib.rs` — add `#[cfg(any(test, feature = "test-support"))] pub mod test_support;`
- `crates/ironclaw_reborn/Cargo.toml` — add to `[dev-dependencies]`: `ironclaw_agent_loop = { workspace = true, features = ["test-support"] }`

### NOT TOUCHED
- Production code in `ironclaw_agent_loop` and `ironclaw_reborn` — this brief is tests + fixtures only
- `TextOnlyModelReplyDriver` — WS-8 may add a smoke test for it alongside the new path, but the existing tests stay untouched

## 3. `test_support` module

### 3.1 `MockAgentLoopDriverHost`

```rust
//! crates/ironclaw_agent_loop/src/test_support/mod.rs

#[cfg(any(test, feature = "test-support"))]
pub mod test_support {

use std::sync::{Arc, Mutex};
use ironclaw_turns::run_profile::{AgentLoopDriverHost, /* port traits */};

/// Scriptable host facade for integration tests.
///
/// Built via `MockAgentLoopDriverHostBuilder`. All host calls record into a
/// per-instance `CallLog` so tests can assert ordering of `build_prompt_bundle`,
/// `stream_model`, `invoke_capability_batch`, `finalize_assistant_message`,
/// `save_checkpoint`, `poll_inputs`, `ack_inputs`, etc.
pub struct MockAgentLoopDriverHost {
    run_context: ironclaw_turns::run_profile::LoopRunContext,
    script: Mutex<ScenarioScript>,
    call_log: Mutex<Vec<MockHostCall>>,
    checkpoints: Arc<CheckpointRecorder>,
    cancel_after_calls: Option<usize>,  // simulate cancellation
}

#[async_trait::async_trait]
impl AgentLoopDriverHost for MockAgentLoopDriverHost {
    fn run_context(&self) -> &ironclaw_turns::run_profile::LoopRunContext { &self.run_context }
    // … all required port methods, each recording to call_log and consulting script
}

pub struct MockAgentLoopDriverHostBuilder { /* … */ }
impl MockAgentLoopDriverHostBuilder {
    pub fn new() -> Self;
    pub fn run_context(self, ctx: LoopRunContext) -> Self;
    pub fn script(self, s: ScenarioScript) -> Self;
    pub fn cancel_after_calls(self, n: usize) -> Self;
    /// Force the cancellation-checkpoint write to fail (drives the residual
    /// `AgentLoopExecutorError::Cancelled` path; see WS-7 error mapping).
    pub fn fail_cancellation_checkpoint(self) -> Self;
    /// Override single-call retry outcomes (WS-6 §3.6 retry mechanic).
    /// Indexed in retry order; one entry per planned single-call `invoke_capability` retry.
    pub fn single_call_retry_outcomes(self, outcomes: Vec<ScriptedCapabilityOutcome>) -> Self;
    pub fn build(self) -> (MockAgentLoopDriverHost, Arc<CheckpointRecorder>);
}

#[derive(Debug, Clone)]
pub enum MockHostCall {
    BuildPromptBundle,
    StreamModel,
    InvokeCapabilityBatch { call_count: usize },
    InvokeCapabilitySingle { capability_name: String },  // for retry-path tests (WS-6 §3.6)
    FinalizeAssistantMessage,
    SaveCheckpoint(crate::state::CheckpointKind),
    TakePendingInputs(/* intent */),
    VisibleCapabilities,
    LoadCheckpointPayload,
    ObserveCancellation { fired: bool },                  // tracks cancellation accessor reads
}
```

### 3.2 `ScenarioScript`

Small DSL for scripting host responses across N model calls and M capability invocations:

```rust
pub struct ScenarioScript {
    pub model_responses: Vec<ScriptedModelResponse>,
    pub capability_outcomes: Vec<Vec<ScriptedCapabilityOutcome>>,  // outer = batch index, inner = per-call
    pub pending_inputs: Vec<Vec<LoopMessageRef>>,                  // per drain call
}

pub enum ScriptedModelResponse {
    Reply { text: &'static str },
    Calls(Vec<ScriptedCapabilityCall>),
    Error { class: ModelErrorClass },
}

pub enum ScriptedCapabilityOutcome {
    Completed { result_ref: LoopResultRef, terminate_hint: bool },
    ApprovalRequired { gate_ref: LoopGateRef },
    AuthRequired { gate_ref: LoopGateRef },
    ResourceBlocked { gate_ref: LoopGateRef },
    Failed { class: CapabilityErrorClass, failure_kind: LoopFailureKind },
}

impl ScenarioScript {
    /// Reply on first call. Used by the simplest happy-path test.
    pub fn reply_only(text: &'static str) -> Self;

    /// CapabilityCalls on first call (one Completed outcome), then Reply on second.
    pub fn calls_then_reply(call_name: &str) -> Self;

    /// Same Calls payload N times in a row (drives no-progress detection).
    pub fn same_calls_repeated(call_name: &str, n: usize) -> Self;

    /// CapabilityCalls returning the same Failed kind N times in a row (drives failure-run-length detection).
    pub fn same_failure_repeated(kind: LoopFailureKind, n: usize) -> Self;

    /// CapabilityCalls returning ApprovalRequired on first batch.
    pub fn approval_required(call_name: &str) -> Self;

    /// N consecutive `Failed { class: Transient }` outcomes for the same call (drives recovery exhaustion).
    pub fn transient_failure_repeated(call_name: &str, n: usize) -> Self;
}
```

### 3.3 `CheckpointRecorder`

```rust
#[derive(Debug, Default)]
pub struct CheckpointRecorder {
    sequence: Mutex<Vec<(crate::state::CheckpointKind, u32 /* iteration */)>>,
}

impl CheckpointRecorder {
    pub fn record(&self, kind: CheckpointKind, iteration: u32);
    pub fn sequence(&self) -> Vec<(CheckpointKind, u32)>;
    pub fn assert_sequence(&self, expected: &[(CheckpointKind, u32)]);
    pub fn assert_kinds(&self, expected: &[CheckpointKind]);
}
```

### 3.4 `LoopExecutionStateBuilder`

```rust
pub struct LoopExecutionStateBuilder { state: LoopExecutionState }
impl LoopExecutionStateBuilder {
    pub fn new() -> Self { Self { state: LoopExecutionState::initial() } }
    pub fn iteration(mut self, i: u32) -> Self;
    pub fn push_call_signature(mut self, sig: CapabilityCallSignature) -> Self;  // for repetition tests
    pub fn push_failure_kind(mut self, kind: LoopFailureKind) -> Self;            // for run-length tests
    pub fn recovery_attempts(mut self, n: u32) -> Self;
    pub fn build(self) -> LoopExecutionState;
}
```

## 4. Test scenarios

Each row below is one `#[tokio::test]` function. Cross-references (e.g. "executor §8") point at the master doc.

### 4.1 `executor_happy_paths.rs` (~5 tests)

| Test | Setup | Asserts |
|---|---|---|
| `reply_only_completes` | `script::reply_only("hi")` | `LoopExit::Completed { GracefulStop }`; `assistant_refs.len() == 1`; checkpoint sequence `[BeforeModel, Final]` |
| `calls_then_reply_completes` | `script::calls_then_reply("file_read")` | `LoopExit::Completed`; checkpoint sequence `[BeforeModel, BeforeSideEffect, BeforeModel, Final]`; `result_refs.len() == 1`; iteration count = 1 |
| `parallel_batch_runs_in_one_iteration` | Two SafeForParallel calls returning Completed | `BatchPolicy::Parallel` recorded; `result_refs.len() == 2` after one batch |
| `sequential_batch_when_exclusive_present` | One SafeForParallel + one Exclusive call | `BatchPolicy::Sequential` recorded |
| `multiple_turns_complete_after_final_reply` | `Calls → Calls → Reply` (3 model calls) | `LoopExit::Completed`; iteration count = 2; checkpoint count = 6 (`BeforeModel`, `BeforeSideEffect` × 2, `BeforeModel`, `Final`) |

### 4.2 `safety_nets.rs` (~7 tests)

| Test | Setup | Asserts |
|---|---|---|
| `iteration_cap_fails_at_boundary` | Planner with `BudgetStrategy::iteration_limit() = 3`; `script::same_calls_repeated("x", 100)` | `LoopExit::Failed { reason_kind: IterationLimit }`; mock-host call log shows **exactly 3** `stream_model` invocations (not 4) — confirms `>=` semantics |
| `iteration_cap_passes_just_below_boundary` | `iteration_limit = 32`; script that returns Reply on 32nd model call | `LoopExit::Completed`; mock-host call log shows **exactly 32** `stream_model` invocations |
| `repetition_escape_after_three_iterations` | `script::same_calls_repeated("x", 6)`; default planner | `LoopExit::Failed { reason_kind: NoProgressDetected }` after exactly 3 *iterations* each issuing the same signature once |
| `repetition_within_single_batch_does_not_trip` | One iteration whose batch returns the same signature 3× (then Reply on next iter) | `LoopExit::Completed` — single-iteration repetition is dedup'd to 1 push (per WS-0 §3.4); detector does NOT fire |
| `repetition_with_unrelated_calls_in_window_still_trips` | Iterations 1, 3, 5 issue `"x"`; iterations 2, 4 issue distinct other calls | `LoopExit::Failed { NoProgressDetected }` after iteration 5 — `"x"` appears in 3 of last 5 iterations even with intervening distinct calls |
| `failure_run_length_escape` | `script::same_failure_repeated(LoopFailureKind::CapabilityProtocolError, 3)` | `LoopExit::Failed { reason_kind: NoProgressDetected }` after 3 in a row |
| `recovery_budget_exhaustion_uses_single_call_retry` | `script::transient_failure_repeated("x", 3)`; `DefaultRecoveryStrategy { max_attempts_per_class: 2 }` | `LoopExit::Failed { reason_kind: <transient class> }` after 2 retries; mock-host call log shows 1 `invoke_capability_batch` followed by 2 single-call `invoke_capability` invocations (confirms WS-6 §3.6 retry mechanic actually re-issues) |

### 4.3 `strategy_interactions.rs` (~5 tests)

| Test | Setup | Asserts |
|---|---|---|
| `retries_do_not_push_signatures` | Single transient failure on `"x"`, then success on retry; verify ring contains exactly one entry for `"x"` (not two) | Retries do not re-push the signature; `LoopExit::Completed`; `recent_call_signatures.iter().filter(|s| s.name == "x").count() == 1`. *Locks the contract: WS-0 §3.4 dedupe rule applies to retries within an iteration too.* |
| `gate_blocks_before_recovery_budget_exhausts` | Capability returns `ApprovalRequired` on first call | `LoopExit::Blocked { gate_ref }`; `recovery_state.attempts == 0` (gate handler runs first) |
| `gate_skip_continues_batch` | Two calls in batch: first `ApprovalRequired` (planner with `Skip` gate strategy), second `Completed` | Batch finishes; `result_refs.len() == 1`; stop strategy consulted with `AfterCapabilityBatch`; loop continues to next iteration if `Continue` |
| `drain_followup_continues_after_natural_stop` | `script::reply_only`; planner with drain returning `(_, true)`; mock host has one pending followup message | `LoopExit::Completed` only after followup drained and second iteration completes |
| `terminate_hint_after_batch_stops_without_extra_model_call` | Single iteration: batch of 1 call returning `Completed { terminate_hint: true }` | `LoopExit::Completed { GracefulStop }`; mock-host call log shows **exactly 1** `stream_model` invocation. *Locks Issue 1 fix: stop strategy is consulted after capability batches, not just after Reply.* |
| `last_batch_total_resets_between_batches` | Batch 1: 2 of 3 calls have terminate_hint (stop strategy returns Continue); Batch 2: 1 of 1 call has terminate_hint | First batch's `terminate_hints_in_last_batch == 2`, `last_batch_total == 3`, strategy continues. Before second batch: counter resets. Second batch's `terminate_hints_in_last_batch == 1`, `last_batch_total == 1` → strategy returns `GracefulStop`. *Guards per-batch counter reset; without reset, second batch would compute 3/4 and fail to stop.* |

### 4.4 `state_lifecycle.rs` (~4 tests)

| Test | Setup | Asserts |
|---|---|---|
| `state_serializes_round_trips` | `LoopExecutionStateBuilder` populated with diverse fields | `serde_json` round-trip equals input |
| `from_checkpoint_payload_validates_schema` | Hand-crafted payload with wrong schema id | `Err(CheckpointPayloadError::SchemaMismatch)` |
| `resume_continues_from_before_block` | Run 1: trigger `ApprovalRequired` → `LoopExit::Blocked` with checkpoint id; Run 2: `from_checkpoint_payload` then re-execute against scripted host that completes the gate | Assistant ref present; final exit `Completed`; checkpoint sequence on resume starts mid-loop |
| `recent_call_signatures_survive_serialization` | State with 5 entries in `recent_call_signatures` | Round-trip preserves entries and order |

### 4.5 `planned_driver_e2e.rs` in `ironclaw_reborn` (~6 tests)

Pulls fixtures from `ironclaw_agent_loop::test_support` via dev-dependencies with `features = ["test-support"]`.

| Test | Setup | Asserts |
|---|---|---|
| `default_planned_driver_smoke` | `default_planned_driver()` + scripted MockHost (`reply_only`) | `driver.run(req, &host)` returns `Ok(LoopExit::Completed)`; descriptor id is `"reborn:default-loop"` |
| `planned_driver_resume_round_trip` | Run to `Blocked`; reload checkpoint; resume to `Completed` | Two-phase exit; assistant ref present at end |
| `planned_driver_executor_error_maps_to_unavailable` | Mock host returns failure on `stream_model` (HostUnavailable) | `Err(AgentLoopDriverError::Unavailable { reason: "Model: unavailable" })`; debug output contains no raw provider strings, no host paths, no secret-shaped strings |
| `planned_driver_cancellation_returns_cancelled_exit` | Mock host's cancellation accessor flips to `true` between turns | `driver.run(...)` returns **`Ok(LoopExit::Cancelled(...))`** (not `Err`); `reason_kind` is `HostInterrupt` or `HostCancellation`; `interrupted_message_refs` populated; checkpoint id present. *Locks Issue 2 fix: cancellation is a successful exit, not an error.* |
| `planned_driver_cancellation_checkpoint_failure_maps_to_failed_interrupted` | Mock host cancels AND fails the cancellation checkpoint write | `Err(AgentLoopDriverError::Failed { reason_kind: "interrupted_unexpectedly" })`. *Locks Issue 2 fix: residual `Cancelled` executor error maps to `Failed { interrupted_unexpectedly }`, NOT `Unavailable`.* |
| `planned_driver_no_raw_payloads_in_error` | Mock host returns a sanitized error containing strings like `"sk-fake"`, `"/host/path"` | Returned `AgentLoopDriverError`'s `Debug` output does NOT contain those strings (mirror existing `text_loop_driver` test pattern) |

## 5. Acceptance criteria

- [ ] `cargo build -p ironclaw_agent_loop --features test-support` succeeds; `cargo build -p ironclaw_agent_loop` (no features) also succeeds and contains no `test_support` symbols
- [ ] `cargo clippy --all --benches --tests --examples --all-features` zero warnings (per workspace standard)
- [ ] `cargo test -p ironclaw_agent_loop --features test-support` — every test in the four files above passes
- [ ] `cargo test -p ironclaw_reborn` — every test in `planned_driver_e2e.rs` passes
- [ ] `cargo test --workspace --features ironclaw_agent_loop/test-support` — full workspace green
- [ ] Coverage check (manual): every `match` arm in `CanonicalAgentLoopExecutor::execute` is hit by at least one scenario in `executor_happy_paths.rs` + `safety_nets.rs` + `strategy_interactions.rs`
- [ ] No `unwrap()` / `expect()` outside test code (per `error-handling.md`)
- [ ] No raw provider/secret/host-path strings appear in any returned `AgentLoopDriverError` from any test
- [ ] `test_support` module's public surface is documented (rustdoc on every pub item) so future loop-family briefs can adopt it without reverse-engineering

## 6. Out of scope

- Real `LoopCapabilityPort` impl — still `EmptyLoopCapabilityPort` per skeleton scope
- Real model gateway integration — `MockAgentLoopDriverHost` is the only host this brief uses
- Performance / load testing — not a skeleton concern
- Cross-channel scenarios (Telegram, Web, etc.) — those are channel-adapter concerns, not framework
- Loop-family-specific test scenarios — those land per follow-up loop-family PR, reusing this brief's fixtures
- E2E tests against a live LLM provider — out of skeleton; future framework users add provider-specific E2E suites in their own crates

## 7. Verification command sequence

```bash
# fixtures + framework integration
cargo build -p ironclaw_agent_loop                                    # production build excludes test_support
cargo build -p ironclaw_agent_loop --features test-support            # with-feature build includes it
cargo test  -p ironclaw_agent_loop --features test-support            # framework integration suite

# driver-side e2e
cargo test  -p ironclaw_reborn                                        # planned_driver_e2e.rs picks up fixtures via dev-dep

# whole workspace green-light
cargo test  --workspace --features ironclaw_agent_loop/test-support
cargo clippy --all --benches --tests --examples --all-features -- -D warnings
```

The final `--workspace` test command is the canonical proof-of-life for the entire skeleton: it goes green only when WS-0 through WS-7 are correctly composed.
