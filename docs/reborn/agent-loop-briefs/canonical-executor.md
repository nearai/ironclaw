# WS-6 — Canonical Executor

**Workstream:** WS-6
**Crate touched:** `ironclaw_agent_loop`
**Depends on:** WS-4 (planner facade), WS-5 (default strategies)
**Master doc:** [`../agent-loop-skeleton.md`](../agent-loop-skeleton.md) §3, §5, §8

---

## 1. Scope

Land the loop body — the canonical tick that drives every planner.

- `AgentLoopExecutor` trait — boundary for the executor abstraction.
- `CanonicalAgentLoopExecutor` struct — the one canonical implementation, body matching master doc §8.
- `AgentLoopExecutorError` — sanitized error type returned alongside `LoopExit` in error paths.

The executor takes a `&dyn AgentLoopPlanner`, an `&dyn AgentLoopDriverHost` (host facade from `ironclaw_turns`), and an initial `LoopExecutionState`. It runs the canonical tick, applies strategy outcomes, populates the executor-observed state fields, takes checkpoints at the four boundary kinds, and returns a `LoopExit` (defined in `ironclaw_turns`).

The executor never calls into the runner-facing `AgentLoopDriver` trait. That bridge belongs to WS-7.

## 2. Files

### NEW
- `crates/ironclaw_agent_loop/src/executor.rs` — `AgentLoopExecutor` trait + supporting types
- `crates/ironclaw_agent_loop/src/canonical_executor.rs` — `CanonicalAgentLoopExecutor` body

### EXTEND
- `crates/ironclaw_agent_loop/src/lib.rs` — export `executor`, `canonical_executor`

## 3. Specification

### 3.1 `AgentLoopExecutor` trait

```rust
//! crates/ironclaw_agent_loop/src/executor.rs

use async_trait::async_trait;
use ironclaw_turns::{
    LoopExit,
    run_profile::AgentLoopDriverHost,
};

use crate::{planner::AgentLoopPlanner, state::LoopExecutionState};

/// Drives the canonical loop tick by consulting a planner's strategies and
/// invoking host ports. The trait exists so future variants (instrumented,
/// replay, fault-injecting test) can slot in without touching planners or
/// the driver adapter.
///
/// Implementations MUST honor the contract in master doc §8:
/// - checkpoint at the four boundary kinds (BeforeModel, BeforeSideEffect,
///   BeforeBlock, optionally Final) and nowhere else;
/// - observe cancellation between every strategy call;
/// - rebind state in exactly one place per branch (no interior mutability,
///   no `&mut LoopExecutionState` across strategy calls).
#[async_trait]
pub trait AgentLoopExecutor: Send + Sync {
    async fn execute(
        &self,
        planner: &dyn AgentLoopPlanner,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        initial_state: LoopExecutionState,
    ) -> Result<LoopExit, AgentLoopExecutorError>;
}

/// Sanitized executor errors. Distinct from `LoopExit::Failed` — these are
/// errors returning the LoopExit itself failed (host crash before any exit
/// could be produced, planner contract violation, etc.). The runner-facing
/// `PlannedDriver` (WS-7) maps these to `AgentLoopDriverError`.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AgentLoopExecutorError {
    #[error("host port returned an unrecoverable error: {stage}")]
    HostUnavailable { stage: HostStage },
    #[error("planner returned a contract violation: {detail}")]
    PlannerContract { detail: &'static str },
    #[error("checkpoint write failed at {stage:?}")]
    CheckpointFailed { stage: crate::state::CheckpointKind },
    #[error("cancelled by host before any LoopExit could be produced")]
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostStage { Prompt, Model, Capability, Transcript, Checkpoint, Progress, Input }
```

### 3.2 `CanonicalAgentLoopExecutor`

```rust
//! crates/ironclaw_agent_loop/src/canonical_executor.rs

use async_trait::async_trait;

use crate::{
    executor::{AgentLoopExecutor, AgentLoopExecutorError, HostStage},
    planner::AgentLoopPlanner,
    state::{CheckpointKind, CheckpointMarker, LoopExecutionState},
    strategies::{CapabilityCallSummary, GateOutcome, RecoveryOutcome, StopOutcome, StopKind},
};

/// The reference executor. Implements the canonical tick from master doc §8.
#[derive(Debug, Default, Clone, Copy)]
pub struct CanonicalAgentLoopExecutor;

#[async_trait]
impl AgentLoopExecutor for CanonicalAgentLoopExecutor {
    async fn execute(
        &self,
        planner: &dyn AgentLoopPlanner,
        host: &(dyn ironclaw_turns::run_profile::AgentLoopDriverHost + Send + Sync),
        mut state: LoopExecutionState,
    ) -> Result<ironclaw_turns::LoopExit, AgentLoopExecutorError> {
        loop {
            // 1. Cancellation observation
            state = self.checkpoint_and_exit_if_cancelled(host, state).await?;

            // 2. Steering drain (per planner.drain())
            if planner.drain().drain_steering(&state).await {
                state = self.drain_steering_into(host, state).await?;
            }

            // 3. Context + visible surface
            let ctx_req = planner.context().plan_context_request(&state).await;
            let bundle = host
                .build_prompt_bundle(ctx_req)
                .await
                .map_err(|_| AgentLoopExecutorError::HostUnavailable { stage: HostStage::Prompt })?;

            let surface_filter = planner.capability().filter(&state).await;
            let surface = host
                .visible_capabilities(/* applies surface_filter */)
                .await
                .map_err(|_| AgentLoopExecutorError::HostUnavailable { stage: HostStage::Capability })?;
            state.surface_version = Some(surface.version);

            // 4. Checkpoint BeforeModel
            state = self.checkpoint(host, state, CheckpointKind::BeforeModel).await?;

            // 5. Stream model
            let model_pref = planner.model().preference(&state).await;
            let model_resp = host
                .stream_model(/* construct LoopModelRequest from bundle + surface + model_pref */)
                .await
                .map_err(|_| AgentLoopExecutorError::HostUnavailable { stage: HostStage::Model })?;

            // 6. Branch on model output
            match model_resp.output {
                ParentLoopOutput::AssistantReply(reply) => {
                    // Check stop condition
                    let summary = TurnSummary {
                        kind: TurnEndKind::ReplyOnly,
                        assistant_message_ref: None,  // not yet finalized
                        batch_result_refs: Vec::new(),
                    };
                    let stop = planner.stop().should_stop_after_turn(&state, &summary).await;

                    match stop {
                        StopOutcome::Stop { control, kind: StopKind::GracefulStop } => {
                            state.control_state = control;
                            let reply_ref = host
                                .finalize_assistant_message(/* reply */)
                                .await
                                .map_err(|_| AgentLoopExecutorError::HostUnavailable {
                                    stage: HostStage::Transcript,
                                })?;
                            state.assistant_refs.push(reply_ref);
                            state = self.checkpoint(host, state, CheckpointKind::Final).await?;
                            return Ok(/* LoopExit::Completed { GracefulStop, … } */);
                        }
                        StopOutcome::Stop { control, kind: StopKind::NoProgressDetected } => {
                            state.control_state = control;
                            state = self.checkpoint(host, state, CheckpointKind::Final).await?;
                            return Ok(/* LoopExit::Failed { NoProgressDetected, … } */);
                        }
                        StopOutcome::Stop { control, kind: StopKind::Aborted(failure_kind) } => {
                            state.control_state = control;
                            return Ok(/* LoopExit::Failed { failure_kind, … } */);
                        }
                        StopOutcome::Continue { control } => {
                            state.control_state = control;
                            // Continue path: drain followup if planner wants
                            if planner.drain().drain_followup(&state).await {
                                let any_drained = self.drain_followup_into(host, &mut state).await?;
                                if !any_drained {
                                    return Ok(/* LoopExit::Completed { … } */);
                                }
                                // else: fall through to next iteration
                            } else {
                                return Ok(/* LoopExit::Completed { … } */);
                            }
                        }
                    }
                }
                ParentLoopOutput::CapabilityCalls(calls) => {
                    state = self.execute_capability_batch(planner, host, state, calls).await?;

                    // Capability batches must consult the stop strategy too, otherwise
                    // terminate-hint detection and no-progress escapes would only fire
                    // on Reply-ending turns. (Issue: tool-only loops would run to
                    // the iteration cap before stopping.)
                    let summary = TurnSummary {
                        kind: TurnEndKind::AfterCapabilityBatch,
                        assistant_message_ref: None,
                        batch_result_refs: state.result_refs.iter().rev()
                            .take(state.control_state.last_batch_total as usize)
                            .cloned().rev().collect(),
                    };
                    let stop = planner.stop().should_stop_after_turn(&state, &summary).await;
                    match stop {
                        StopOutcome::Stop { control, kind: StopKind::GracefulStop } => {
                            state.control_state = control;
                            state = self.checkpoint(host, state, CheckpointKind::Final).await?;
                            return Ok(/* LoopExit::Completed { GracefulStop, … } */);
                        }
                        StopOutcome::Stop { control, kind: StopKind::NoProgressDetected } => {
                            state.control_state = control;
                            state = self.checkpoint(host, state, CheckpointKind::Final).await?;
                            return Ok(/* LoopExit::Failed { NoProgressDetected, … } */);
                        }
                        StopOutcome::Stop { control, kind: StopKind::Aborted(failure_kind) } => {
                            state.control_state = control;
                            return Ok(/* LoopExit::Failed { failure_kind, … } */);
                        }
                        StopOutcome::Continue { control } => {
                            state.control_state = control;
                            // Continue: fall through to iteration counter
                        }
                    }
                }
            }

            // 7. Iteration / wall-clock budget. Use `>=` so a documented limit of N
            // permits exactly N iterations, not N+1. (state.iteration starts at 0
            // and is incremented at the end of each iteration, so after the Nth
            // iteration state.iteration == N — and `>=` catches it before an
            // (N+1)th body runs.)
            state.iteration = state.iteration.saturating_add(1);
            if state.iteration >= planner.budget().iteration_limit(&state) {
                return Ok(/* LoopExit::Failed { IterationLimit } */);
            }
        }
    }
}
```

### 3.3 Helpers (private to `canonical_executor.rs`)

```rust
impl CanonicalAgentLoopExecutor {
    async fn execute_capability_batch(
        &self,
        planner: &dyn AgentLoopPlanner,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        mut state: LoopExecutionState,
        calls: Vec<CapabilityCall>,
    ) -> Result<LoopExecutionState, AgentLoopExecutorError> {
        // Reset per-batch counters in control_state
        state.control_state.last_batch_total = calls.len() as u32;
        state.control_state.terminate_hints_in_last_batch = 0;

        // Project to summaries for batch policy
        let summaries: Vec<CapabilityCallSummary> = calls.iter().map(summary_of).collect();
        let policy = planner.batch().policy(&state, &summaries);

        state = self.checkpoint(host, state, CheckpointKind::BeforeSideEffect).await?;

        // Invoke batch through host. Loop crate does not directly call individual
        // capabilities — host owns the dispatch and applies the policy hint.
        let outcomes = host
            .invoke_capability_batch(/* calls, policy */)
            .await
            .map_err(|_| AgentLoopExecutorError::HostUnavailable { stage: HostStage::Capability })?;

        for (call, outcome) in calls.iter().zip(outcomes.into_iter()) {
            state.recent_call_signatures.push(CapabilityCallSignature::from_call(
                call.name.clone(),
                &call.args,
            ));

            match outcome {
                CapabilityOutcome::Completed(result) => {
                    state.result_refs.push(result.ref_.clone());
                    if result.terminate_hint {
                        state.control_state.terminate_hints_in_last_batch += 1;
                    }
                }
                CapabilityOutcome::ApprovalRequired(g)
                | CapabilityOutcome::AuthRequired(g)
                | CapabilityOutcome::ResourceBlocked(g) => {
                    let gate_summary = project_gate(&outcome, &g);
                    let gate_outcome = planner.gate().handle(&state, &gate_summary).await;
                    match gate_outcome {
                        GateOutcome::Block { control } => {
                            state.control_state = control;
                            state.last_gate = Some(g.gate_ref);
                            state = self.checkpoint(host, state, CheckpointKind::BeforeBlock).await?;
                            return Ok(/* propagate via early-return wrapper to top-level Blocked */);
                        }
                        GateOutcome::SkipAndContinue { control } => {
                            state.control_state = control;
                        }
                        GateOutcome::Abort { control, failure_kind } => {
                            state.control_state = control;
                            return Ok(/* propagate via early-return wrapper to top-level Failed */);
                        }
                    }
                }
                CapabilityOutcome::Failed(err) => {
                    // Inner retry loop: planner.recovery() can return Retry repeatedly
                    // until its own budget says Abort. Each Retry actually re-issues
                    // the failed call via the host's single-call invocation API
                    // (see §3.6). State updates and backoff are honored between
                    // attempts; the resolved outcome (success, skip, abort) leaves
                    // the inner loop and is handled by the outer batch loop.
                    let mut current_failure = err;
                    loop {
                        state.recent_failure_kinds.push(current_failure.failure_kind);
                        let summary = sanitize(&current_failure);
                        let recovery = planner.recovery()
                            .on_capability_error(&state, &summary).await;
                        match recovery {
                            RecoveryOutcome::Retry { recovery, alter } => {
                                state.recovery_state = recovery;
                                self.honor_alteration(&alter)?;  // backoff sleep, reject AdvanceFallback in skeleton
                                // Re-issue THIS call. Uses the existing
                                // LoopCapabilityPort::invoke_capability (single-call) method
                                // — see §3.6.
                                let retry_outcome = host
                                    .invoke_capability(CapabilityInvocation::from_call(call.clone()))
                                    .await
                                    .map_err(|_| AgentLoopExecutorError::HostUnavailable {
                                        stage: HostStage::Capability,
                                    })?;
                                match retry_outcome {
                                    CapabilityOutcome::Completed(result) => {
                                        state.result_refs.push(result.ref_.clone());
                                        if result.terminate_hint {
                                            state.control_state.terminate_hints_in_last_batch += 1;
                                        }
                                        break;  // resolved — leave inner retry loop
                                    }
                                    CapabilityOutcome::Failed(next_err) => {
                                        current_failure = next_err;  // try recovery again
                                        continue;
                                    }
                                    CapabilityOutcome::ApprovalRequired(_)
                                    | CapabilityOutcome::AuthRequired(_)
                                    | CapabilityOutcome::ResourceBlocked(_) => {
                                        // Gate appeared on retry — re-run the gate
                                        // path. Promote the outcome and reuse the
                                        // same gate-handling logic above. (Implementation
                                        // factors gate handling into a helper.)
                                        return self.handle_gate_outcome(
                                            planner, host, state, call, retry_outcome
                                        ).await;
                                    }
                                }
                            }
                            RecoveryOutcome::SkipResult { recovery } => {
                                state.recovery_state = recovery;
                                break;  // drop result; continue outer batch loop
                            }
                            RecoveryOutcome::Abort { recovery, failure_kind } => {
                                state.recovery_state = recovery;
                                return Ok(/* propagate to top-level Failed */);
                            }
                        }
                    }
                }
            }
        }

        Ok(state)
    }
}
```

The early-return-via-wrapper pattern (where `execute_capability_batch` needs to short-circuit `execute`) deserves care: the cleanest shape is for the helper to return a small enum `BatchProgress { Continue(LoopExecutionState), ExitNow(LoopExit, LoopExecutionState) }` that the top-level `execute` matches on. The pseudocode above elides this for readability; the implementation should make the early-return path explicit and typed.

### 3.3a Drain + cancellation helpers (private to `canonical_executor.rs`)

```rust
impl CanonicalAgentLoopExecutor {
    /// Drain the steering queue once. Calls `LoopInputPort::poll_inputs(after,
    /// limit)` followed by `ack_inputs(cursor)` if any messages came back.
    /// Updates `state.input_cursor` on success.
    async fn drain_steering_into(
        &self,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        mut state: LoopExecutionState,
    ) -> Result<LoopExecutionState, AgentLoopExecutorError> {
        let batch = host
            .poll_inputs(state.input_cursor.clone(), MAX_PER_DRAIN)
            .await
            .map_err(|_| AgentLoopExecutorError::HostUnavailable { stage: HostStage::Input })?;
        if !batch.messages.is_empty() {
            host.ack_inputs(batch.cursor.clone())
                .await
                .map_err(|_| AgentLoopExecutorError::HostUnavailable { stage: HostStage::Input })?;
            state.input_cursor = batch.cursor;
            // Append batch.messages into transcript-bound state — concrete shape
            // depends on how messages flow into the next prompt bundle (host-owned
            // projection per master doc §6).
        }
        Ok(state)
    }

    /// Drain the followup queue. Returns `(state, drained_any)`. If `drained_any`
    /// is false the executor returns `LoopExit::Completed`.
    async fn drain_followup_into(
        &self,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        state: &mut LoopExecutionState,
    ) -> Result<bool, AgentLoopExecutorError> {
        let batch = host
            .poll_inputs(state.input_cursor.clone(), MAX_PER_DRAIN)
            .await
            .map_err(|_| AgentLoopExecutorError::HostUnavailable { stage: HostStage::Input })?;
        if batch.messages.is_empty() {
            return Ok(false);
        }
        host.ack_inputs(batch.cursor.clone())
            .await
            .map_err(|_| AgentLoopExecutorError::HostUnavailable { stage: HostStage::Input })?;
        state.input_cursor = batch.cursor;
        Ok(true)
    }

    /// Cancellation observation. Host exposes a cancellation accessor (added in
    /// WS-13; see §3.5). When fired: checkpoint with the current-step kind and
    /// return `Ok(LoopExit::Cancelled(...))`. The state-mutation pattern below
    /// keeps the rebinding signature consistent with other helpers.
    async fn checkpoint_and_exit_if_cancelled(
        &self,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        state: LoopExecutionState,
    ) -> Result<LoopExecutionState, ExecutorEarlyExit> {
        // ExecutorEarlyExit is a private control-flow enum:
        //   Continue(LoopExecutionState) | ReturnExit(Result<LoopExit, AgentLoopExecutorError>)
        // The top-level `execute` `?`-propagates and pattern-matches.
        // Real impl detail; pseudocode for clarity.
        ...
    }
}

const MAX_PER_DRAIN: usize = 32;
```

### 3.3b Projecting `CapabilityCallSummary` from model-response calls

The model returns a `Vec<CapabilityCall>` (or provider-specific equivalent normalized into Reborn's `CapabilityInvocation`). `BatchPolicyStrategy::policy(&state, &[CapabilityCallSummary])` requires a different shape — name + concurrency hint, no args. The executor's projection:

```rust
fn summary_of(call: &CapabilityCall, surface: &VisibleCapabilitySurface) -> CapabilityCallSummary {
    let hint = surface
        .descriptor_for(&call.name)
        .map(|d| d.concurrency_hint())
        .unwrap_or(ConcurrencyHint::Exclusive);  // unknown → conservative
    CapabilityCallSummary { name: call.name.clone(), concurrency_hint: hint }
}
```

The concurrency hint comes from the visible-capability descriptor returned by `LoopCapabilityPort::visible_capabilities` earlier in the iteration. Unknown capabilities (not present in the surface — the model invented or hallucinated a name) are treated as `Exclusive` for safety; the host will reject the call at `invoke_capability_batch` time anyway, but the conservative hint prevents the loop from speculatively parallelizing alongside unknown calls.

### 3.4 Checkpoint helper

```rust
impl CanonicalAgentLoopExecutor {
    async fn checkpoint(
        &self,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        mut state: LoopExecutionState,
        kind: CheckpointKind,
    ) -> Result<LoopExecutionState, AgentLoopExecutorError> {
        // Serialize state into checkpoint payload (schema id from WS-0)
        let payload = serialize_checkpoint(&state);
        host.save_checkpoint(/* request with kind + payload + schema id */)
            .await
            .map_err(|_| AgentLoopExecutorError::CheckpointFailed { stage: kind })?;
        state.last_checkpoint = Some(CheckpointMarker { kind, iteration_at_checkpoint: state.iteration });
        Ok(state)
    }
}
```

### 3.5 Cancellation observation

The host facade should expose a way to observe cancellation between strategy calls (a method on `AgentLoopDriverHost` returning a current-cancel-state, or an `AbortSignal`-shaped accessor). The executor checks it at the top of every iteration. If the existing host API does not yet expose this, this brief documents the requirement and either:

- (a) adds the missing accessor to `AgentLoopDriverHost` (small, additive change in `ironclaw_turns`); or
- (b) uses a tokio `CancellationToken` passed through `AgentLoopExecutor::execute` as an additional parameter.

Pick (a) if the host already has cancellation plumbing; (b) otherwise.

**Cancellation is a successful exit, not an executor error.** When the signal fires:

1. Checkpoint with whatever `CheckpointKind` is appropriate for the current step (`BeforeModel` / `BeforeSideEffect` / `BeforeBlock`).
2. Build a `LoopExit::Cancelled(LoopCancelled { reason_kind: HostInterrupt | HostCancellation, checkpoint_id: …, interrupted_message_refs: state.assistant_refs.clone(), exit_id: … })` (variant defined in `crates/ironclaw_turns/src/loop_exit.rs:400`).
3. Return `Ok(LoopExit::Cancelled(...))` directly from `execute()`.

`AgentLoopExecutorError::Cancelled` is **only** for the truly-unrecoverable case where the executor cannot even produce a `LoopExit::Cancelled` (e.g. the cancellation checkpoint write itself failed and we have no valid checkpoint id to embed). WS-7 maps that residual case to `AgentLoopDriverError::Failed { reason_kind: "interrupted_unexpectedly" }`, not to `Unavailable`. Normal cancellation never visits the error mapping path.

### 3.6 Host single-call invocation API

The retry mechanic in §3.3 reuses an **existing** `LoopCapabilityPort` method:

```rust
// Already defined in crates/ironclaw_turns/src/run_profile/host.rs:1019
async fn invoke_capability(
    &self,
    request: CapabilityInvocation,
) -> Result<CapabilityOutcome, AgentLoopHostError>;
```

The retry path in §3.3 calls `host.invoke_capability(CapabilityInvocation::from_call(...))` — the existing single-call method. The batch API (`invoke_capability_batch`) handles initial dispatch; the single-call method is the retry primitive. No new method needs to be added to `LoopCapabilityPort` for this skeleton — both methods already exist on the trait. WS-9 (the follow-up that wires `LoopCapabilityPort` against the host runtime) is responsible for ensuring both paths actually invoke through `CapabilityHost` with consistent authorization.

## 4. Acceptance criteria

- [ ] `cargo check -p ironclaw_agent_loop` passes
- [ ] `cargo clippy --all --benches --tests --examples --all-features` zero warnings
- [ ] Trait surface test: `fn _check(_: &dyn AgentLoopExecutor) {}`
- [ ] Smoke test: with a `MockHost` that returns a Reply on first call, `CanonicalAgentLoopExecutor::execute(DefaultPlanner::default(), &host, initial_state)` returns `LoopExit::Completed` with `assistant_refs.len() == 1`. Final checkpoint observed in mock recorder.
- [ ] Smoke test: with a `MockHost` whose first model call returns `CapabilityCalls` and whose second returns Reply, executor takes `BeforeModel`, `BeforeSideEffect`, `BeforeModel`, `Final` checkpoints in order; returns `Completed`.
- [ ] **Stop-after-batch smoke test:** with a `MockHost` whose batch returns one outcome with `terminate_hint: true`, executor calls `should_stop_after_turn` with `TurnEndKind::AfterCapabilityBatch` after the batch and returns `LoopExit::Completed { GracefulStop }` *without* a follow-up model call.
- [ ] Smoke test: with a `MockHost` whose model call returns `CapabilityCalls` whose only outcome is `ApprovalRequired`, executor takes `BeforeModel`, `BeforeSideEffect`, `BeforeBlock` checkpoints; returns `LoopExit::Blocked` with `gate_ref` set.
- [ ] Iteration limit smoke test: with a `MockHost` that always returns `CapabilityCalls`, planner with `iteration_limit() = 3`, executor returns `LoopExit::Failed { IterationLimit }` after **exactly 3** model-call iterations (using `>=` semantics — not 4).
- [ ] No-progress smoke test: with a `MockHost` whose batch returns the same single call signature on every iteration, executor returns `LoopExit::Failed { NoProgressDetected }` once 3 distinct iterations have produced that signature within the last 5 iterations (per the dedupe rule in WS-0 §3.4).
- [ ] **Retry smoke test:** with a `MockHost` whose batch returns one `Failed { Transient }` outcome and whose single-call API (`invoke_capability`) returns `Completed` on the second attempt, executor produces `LoopExit::Completed`; `state.result_refs.len() == 1`; mock-host call log shows one `invoke_capability_batch` followed by one `invoke_capability`.
- [ ] **Cancellation smoke test:** with a `MockHost` whose cancellation accessor flips to `true` between turns, executor returns `Ok(LoopExit::Cancelled(...))` (not `Err`); checkpoint recorded with appropriate `CheckpointKind` and `interrupted_message_refs` populated from `state.assistant_refs`.
- [ ] No `unwrap()` / `expect()` outside test code (per `error-handling.md`)
- [ ] No raw provider/secret/host-path/tool-input strings ever appear in `state` or returned errors
- [ ] Doc comments on `CanonicalAgentLoopExecutor::execute` cite master doc §8

## 5. Out of scope

- `PlannedDriver` adapter implementing `AgentLoopDriver` — WS-7
- A real `LoopCapabilityPort` impl — still `EmptyLoopCapabilityPort` per skeleton scope
- `RetryAlteration::AdvanceFallback` honoring — executor must reject (return `AgentLoopExecutorError::PlannerContract`) until the deferred `ModelRouteChain` lands
- Wall-clock cap enforcement: skeleton may stub this with a TODO if the host has no clock surface; otherwise enforce
- Loop-family-specific behavior — out of skeleton entirely

## 6. Verification command sequence

```bash
cargo check -p ironclaw_agent_loop
cargo clippy --all --benches --tests --examples --all-features -- -D warnings
cargo test -p ironclaw_agent_loop
```
