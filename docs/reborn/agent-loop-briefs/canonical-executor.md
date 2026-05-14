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

The executor's public entry point is `execute_family(family: &LoopFamily, host: &dyn AgentLoopDriverHost, initial_state: LoopExecutionState) -> Result<LoopExit, _>`. It runs the canonical tick, applies strategy outcomes (consulted via the crate-private `AgentLoopPlannerInternal` extension trait — see WS-4), populates the executor-observed state fields, takes checkpoints at the four boundary kinds, and returns a `LoopExit` (defined in `ironclaw_turns`).

Internally the executor extracts `family.planner()` (crate-private accessor) and consults strategies through `AgentLoopPlannerInternal`. Neither is visible outside `ironclaw_agent_loop`; this is the sealed-strategy invariant from master doc §9.

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

use crate::{family::LoopFamily, state::LoopExecutionState};

/// Drives the canonical loop tick by consulting a family's planner strategies
/// (via the crate-private `AgentLoopPlannerInternal` extension trait) and
/// invoking host ports. The trait exists so future variants (instrumented,
/// replay, fault-injecting test) can slot in without touching families or
/// the driver adapter.
///
/// `execute_family` is the public entry point. The executor reaches strategies
/// via `family.planner()` (crate-private) — downstream crates hold opaque
/// `Arc<LoopFamily>` values and cannot see strategies through this trait.
///
/// Implementations MUST honor the contract in master doc §8:
/// - checkpoint at the four boundary kinds (BeforeModel, BeforeSideEffect,
///   BeforeBlock, optionally Final) and nowhere else;
/// - observe cancellation between every strategy call;
/// - rebind state in exactly one place per branch (no interior mutability,
///   no `&mut LoopExecutionState` across strategy calls).
#[async_trait]
pub trait AgentLoopExecutor: Send + Sync {
    async fn execute_family(
        &self,
        family: &LoopFamily,
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
    family::LoopFamily,
    planner::AgentLoopPlannerInternal,    // crate-private; gives strategy accessors
    state::{CheckpointKind, CheckpointMarker, LoopExecutionState},
    strategies::{CapabilityCallSummary, GateOutcome, RecoveryOutcome, StopOutcome, StopKind},
};

/// The reference executor. Implements the canonical tick from master doc §8.
#[derive(Debug, Default, Clone, Copy)]
pub struct CanonicalAgentLoopExecutor;

#[async_trait]
impl AgentLoopExecutor for CanonicalAgentLoopExecutor {
    async fn execute_family(
        &self,
        family: &LoopFamily,
        host: &(dyn ironclaw_turns::run_profile::AgentLoopDriverHost + Send + Sync),
        mut state: LoopExecutionState,
    ) -> Result<ironclaw_turns::LoopExit, AgentLoopExecutorError> {
        // Crate-private accessor: pull the planner out of the opaque family.
        // Downstream callers cannot do this.
        let planner = family.planner();
        loop {
            // 0. Iteration cap check at the TOP of the loop, BEFORE the body.
            // This way a resumed executor with state.iteration == limit exits
            // immediately instead of running one extra body. With state.iteration
            // starting at 0 and limit = N, the body runs for iterations 0..N-1
            // and the check at iteration N triggers the exit — exactly N bodies.
            if state.iteration >= planner.budget().iteration_limit(&state) {
                return Ok(/* LoopExit::Failed { IterationLimit, … } */);
            }

            // 1. Cancellation observation (top of iteration).
            //
            // Per master doc §9, cancellation is observed BETWEEN EVERY
            // strategy call — not just at top of iteration. This pseudocode
            // shows the top-of-iteration site explicitly; the explicit
            // call sites between subsequent strategy calls are marked
            // `// CANCEL_BOUNDARY` below. WS-13's `LoopCancellationPort`
            // is the sync accessor consulted at each boundary (per §3.5).
            // Production-safe rationale (master doc §10): strategies are
            // sealed Builtin code that returns promptly; cooperative
            // checks at strategy-call boundaries are sufficient without
            // preemptive `tokio::select!` wrapping.
            state = self.checkpoint_and_exit_if_cancelled(host, state).await?;

            // CANCEL_BOUNDARY before drain_steering — check elided in this
            // pseudocode for readability; implementer MUST call
            // `checkpoint_and_exit_if_cancelled` here.
            // 2. Steering drain (per planner.drain())
            if planner.drain().drain_steering(&state).await {
                state = self.drain_steering_into(host, state).await?;
            }

            // CANCEL_BOUNDARY before plan_context_request
            // 3. Context + visible surface
            let ctx_req = planner.context().plan_context_request(&state).await;
            let bundle = host
                .build_prompt_bundle(ctx_req)
                .await
                .map_err(|_| AgentLoopExecutorError::HostUnavailable { stage: HostStage::Prompt })?;

            // CANCEL_BOUNDARY after build_prompt_bundle, before capability().filter
            let surface_filter = planner.capability().filter(&state).await;
            let surface = host
                .visible_capabilities(VisibleCapabilityRequest {
                    filter: surface_filter,
                })
                .await
                .map_err(|_| AgentLoopExecutorError::HostUnavailable { stage: HostStage::Capability })?;
            state.surface_version = Some(surface.version);

            // 4. Checkpoint BeforeModel
            state = self.checkpoint(host, state, CheckpointKind::BeforeModel).await?;

            // 5. Stream model. On error, route through RecoveryStrategy —
            // `on_model_error` is not dead code;
            // it gets a real call site here. The host port returns a
            // sanitized `ModelErrorSummary` (per WS-2 §3.3) and the
            // recovery strategy decides Retry/SkipResult/Abort. Skeleton
            // executor rejects `RetryAlteration::AdvanceFallback` as
            // `PlannerContract` (deferred until `ModelRouteChain` lands;
            // master doc §9). Bounded retry; the iteration cap is the
            // structural backstop if recovery never aborts.
            // CANCEL_BOUNDARY before model().preference (checkpoint BeforeModel just landed)
            let model_pref = planner.model().preference(&state).await;
            let model_resp = loop {
                let attempt = host
                    .stream_model(/* construct LoopModelRequest from bundle + surface + model_pref */)
                    .await;
                match attempt {
                    Ok(resp) => break resp,
                    Err(host_err) => {
                        let summary = sanitize_model_error(&host_err);
                        let recovery = planner.recovery()
                            .on_model_error(&state, &summary).await;
                        match recovery {
                            RecoveryOutcome::Retry { recovery, alter } => {
                                state.recovery_state = recovery;
                                self.honor_alteration(&alter)?;  // rejects AdvanceFallback
                                continue;  // loop back to stream_model
                            }
                            RecoveryOutcome::SkipResult { recovery: _ } => {
                                // Skip on model error is meaningless — there's no
                                // result to skip. Treat as PlannerContract.
                                return Err(AgentLoopExecutorError::PlannerContract {
                                    detail: "SkipResult on model error",
                                });
                            }
                            RecoveryOutcome::Abort { recovery, failure_kind } => {
                                state.recovery_state = recovery;
                                return Ok(/* propagate LoopExit::Failed { failure_kind } */);
                            }
                        }
                    }
                }
            };

            // 6. Branch on model output
            match model_resp.output {
                ParentLoopOutput::AssistantReply(reply) => {
                    // Finalize FIRST, before any stop-condition branch, so every
                    // exit path (Completed or Failed) carries the assistant ref.
                    // LoopExit validation rejects a non-NoReply Completed without
                    // a reply_message_ref, so the prior "finalize only on
                    // GracefulStop" shape would silently lose the message on
                    // Continue→Completed and on NoProgressDetected paths.
                    let reply_ref = host
                        .finalize_assistant_message(/* reply */)
                        .await
                        .map_err(|_| AgentLoopExecutorError::HostUnavailable {
                            stage: HostStage::Transcript,
                        })?;
                    state.assistant_refs.push(reply_ref.clone());

                    let summary = TurnSummary {
                        kind: TurnEndKind::ReplyOnly,
                        assistant_message_ref: Some(reply_ref),
                        batch_result_refs: Vec::new(),
                    };
                    // CANCEL_BOUNDARY before stop().should_stop_after_turn (Reply path)
                    let stop = planner.stop().should_stop_after_turn(&state, &summary).await;

                    match stop {
                        StopOutcome::Stop { stop, kind: StopKind::GracefulStop } => {
                            state.stop_state = stop;
                            state = self.checkpoint(host, state, CheckpointKind::Final).await?;
                            return Ok(/* LoopExit::Completed { GracefulStop, reply_message_refs: … } */);
                        }
                        StopOutcome::Stop { stop, kind: StopKind::NoProgressDetected } => {
                            state.stop_state = stop;
                            state = self.checkpoint(host, state, CheckpointKind::Final).await?;
                            return Ok(/* LoopExit::Failed { NoProgressDetected, … } */);
                        }
                        StopOutcome::Stop { stop, kind: StopKind::Aborted(failure_kind) } => {
                            state.stop_state = stop;
                            return Ok(/* LoopExit::Failed { failure_kind, … } */);
                        }
                        StopOutcome::Continue { stop } => {
                            state.stop_state = stop;
                            // Continue path: drain followup if planner wants;
                            // either way, every exit here is Completed and the
                            // reply ref is already in state.assistant_refs.
                            let drained = if planner.drain().drain_followup(&state).await {
                                let (next, any) = self.drain_followup_into(host, state).await?;
                                state = next;
                                any
                            } else {
                                false
                            };
                            if !drained {
                                state = self.checkpoint(host, state, CheckpointKind::Final).await?;
                                return Ok(/* LoopExit::Completed { reply_message_refs: state.assistant_refs.clone(), … } */);
                            }
                            // else: fall through to next iteration with appended inputs
                        }
                    }
                }
                ParentLoopOutput::CapabilityCalls(calls) => {
                    // Snapshot the result-refs index before invoking the batch
                    // so the post-batch TurnSummary can slice exactly THIS
                    // batch's refs (not by call count, which would over-include
                    // refs from prior iterations whenever this batch had any
                    // non-completing outcome).
                    let result_refs_start = state.result_refs.len();
                    state = self.execute_capability_batch(planner, host, state, &surface, calls).await?;

                    // Capability batches must consult the stop strategy too, otherwise
                    // terminate-hint detection and no-progress escapes would only fire
                    // on Reply-ending turns. (Issue: tool-only loops would run to
                    // the iteration cap before stopping.)
                    let summary = TurnSummary {
                        kind: TurnEndKind::AfterCapabilityBatch,
                        assistant_message_ref: None,
                        // Slice from the snapshot index — only refs pushed by
                        // THIS batch. (Both `execute_capability_batch` and
                        // this caller compute the same snapshot for symmetry;
                        // the snapshot here is the one observed before invoking
                        // the helper.)
                        batch_result_refs: state.result_refs[result_refs_start..].to_vec(),
                    };
                    let stop = planner.stop().should_stop_after_turn(&state, &summary).await;
                    match stop {
                        StopOutcome::Stop { stop, kind: StopKind::GracefulStop } => {
                            state.stop_state = stop;
                            state = self.checkpoint(host, state, CheckpointKind::Final).await?;
                            return Ok(/* LoopExit::Completed { GracefulStop, … } */);
                        }
                        StopOutcome::Stop { stop, kind: StopKind::NoProgressDetected } => {
                            state.stop_state = stop;
                            state = self.checkpoint(host, state, CheckpointKind::Final).await?;
                            return Ok(/* LoopExit::Failed { NoProgressDetected, … } */);
                        }
                        StopOutcome::Stop { stop, kind: StopKind::Aborted(failure_kind) } => {
                            state.stop_state = stop;
                            return Ok(/* LoopExit::Failed { failure_kind, … } */);
                        }
                        StopOutcome::Continue { stop } => {
                            state.stop_state = stop;
                            // Continue: fall through to iteration counter
                        }
                    }
                }
            }

            // 7. Increment iteration counter for the budget check at top of
            // the next iteration. Wall-clock cap (if set) is also evaluated
            // at the top of the next iteration, alongside iteration_limit.
            state.iteration = state.iteration.saturating_add(1);
        }
    }
}
```

### 3.3 Helpers (private to `canonical_executor.rs`)

```rust
impl CanonicalAgentLoopExecutor {
    async fn execute_capability_batch(
        &self,
        planner: &dyn AgentLoopPlannerInternal,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        mut state: LoopExecutionState,
        surface: &VisibleCapabilitySurface,    // for `summary_of(...)` concurrency hints
        calls: Vec<CapabilityCall>,
    ) -> Result<LoopExecutionState, AgentLoopExecutorError> {
        // Reset per-batch counters in stop_state (the stop strategy reads
        // these to decide terminate-hint stops).
        state.stop_state.last_batch_total = calls.len() as u32;
        state.stop_state.terminate_hints_in_last_batch = 0;

        // Snapshot the result-refs index BEFORE the batch. Only refs pushed
        // by THIS batch are included in the post-batch TurnSummary.
        // (last_batch_total counts CALLS — slicing from the tail by call count
        // includes refs from prior iterations whenever this batch had any
        // non-completing outcome like Skip/Block/Failed-with-no-retry.)
        let result_refs_start = state.result_refs.len();

        // Per-iteration signature dedup set (master doc §10 + WS-0 §3.4): a
        // signature is pushed AT MOST ONCE per iteration regardless of how
        // many calls or retries reference it. Without this, three identical
        // calls in one batch would trip NoProgressDetected immediately.
        let mut iteration_signatures: std::collections::HashSet<CapabilityCallSignature> =
            std::collections::HashSet::new();

        // Project to summaries for batch policy. summary_of needs the visible
        // capability surface to look up per-capability concurrency hints.
        let summaries: Vec<CapabilityCallSummary> =
            calls.iter().map(|c| summary_of(c, surface)).collect();
        let policy = planner.batch().policy(&state, &summaries);

        state = self.checkpoint(host, state, CheckpointKind::BeforeSideEffect).await?;

        // Invoke batch through host. Loop crate does not directly call individual
        // capabilities — host owns the dispatch and applies the policy hint.
        let outcomes = host
            .invoke_capability_batch(/* calls, policy */)
            .await
            .map_err(|_| AgentLoopExecutorError::HostUnavailable { stage: HostStage::Capability })?;

        for (call, outcome) in calls.iter().zip(outcomes.into_iter()) {
            // Per-iteration dedup: push at most once per distinct signature.
            let sig = CapabilityCallSignature::from_call(call.name.clone(), &call.args);
            if iteration_signatures.insert(sig.clone()) {
                state.recent_call_signatures.push(sig);
            }

            match outcome {
                CapabilityOutcome::Completed(result) => {
                    state.result_refs.push(result.ref_.clone());
                    if result.terminate_hint {
                        state.stop_state.terminate_hints_in_last_batch += 1;
                    }
                }
                CapabilityOutcome::ApprovalRequired(g)
                | CapabilityOutcome::AuthRequired(g)
                | CapabilityOutcome::ResourceBlocked(g) => {
                    let gate_summary = project_gate(&outcome, &g);
                    let gate_outcome = planner.gate().handle(&state, &gate_summary).await;
                    match gate_outcome {
                        GateOutcome::Block { gate } => {
                            state.gate_state = gate;
                            state.last_gate = Some(g.gate_ref);
                            state = self.checkpoint(host, state, CheckpointKind::BeforeBlock).await?;
                            return Ok(/* propagate via early-return wrapper to top-level Blocked */);
                        }
                        GateOutcome::SkipAndContinue { gate } => {
                            state.gate_state = gate;
                        }
                        GateOutcome::Abort { gate, failure_kind } => {
                            state.gate_state = gate;
                            return Ok(/* propagate via early-return wrapper to top-level Failed */);
                        }
                    }
                }
                CapabilityOutcome::Denied(reason) => {
                    // EmptyLoopCapabilityPort returns Denied today (until WS-9
                    // wires the real impl), and capability policy can deny
                    // legitimately at any time. Treat as a non-recoverable
                    // failure for THIS call, but consult Recovery to decide
                    // whether to skip-and-continue or abort the batch.
                    state.recent_failure_kinds.push(LoopFailureKind::PolicyDenied);
                    let summary = sanitize_denial(&reason);
                    let recovery = planner.recovery()
                        .on_capability_error(&state, &summary).await;
                    match recovery {
                        RecoveryOutcome::SkipResult { recovery } => {
                            state.recovery_state = recovery;
                        }
                        RecoveryOutcome::Abort { recovery, failure_kind } => {
                            state.recovery_state = recovery;
                            return Ok(/* propagate to top-level Failed */);
                        }
                        RecoveryOutcome::Retry { .. } => {
                            // Retrying a Denied call without state change would
                            // hit the same denial; the executor treats Retry on
                            // Denied as Abort. Document loud so loop families
                            // can override Recovery to do something smarter.
                            return Ok(/* propagate Failed { PolicyDenied } */);
                        }
                    }
                }
                CapabilityOutcome::SpawnedProcess(handle) => {
                    // The skeleton does not define a process-wait protocol.
                    // Current LoopBlockedKind has approval/auth/resource gates,
                    // not ProcessWaiting, and ProcessHandleSummary has no
                    // gate_ref/resume input contract. Do not invent one here:
                    // spawned process waiting is rejected until the process
                    // contract lands an explicit wait evidence + resume input.
                    return Ok(/* propagate Failed { UnsupportedProcessWait } */);
                }
                CapabilityOutcome::Failed(err) => {
                    // Push the originating failure kind ONCE per call (not once
                    // per retry attempt). Retries of the same call within a
                    // single iteration must not re-fill the failure-kind ring,
                    // or three retries would falsely trip NoProgressDetected
                    // (failure-run-length escape).
                    state.recent_failure_kinds.push(err.failure_kind);

                    // Inner retry loop: planner.recovery() can return Retry
                    // until its own budget says Abort. Each Retry re-issues the
                    // failed call via the existing single-call API (§3.6).
                    //
                    // Durability note (master doc §10): retry attempts mutate
                    // `state.recovery_state.attempts` in place between
                    // checkpoints. The four checkpoint kinds fire at iteration
                    // boundaries, NOT between retry attempts. A crash mid-retry
                    // resumes from the BeforeSideEffect checkpoint with
                    // recovery_state.attempts == 0; the retry budget effectively
                    // resets. This is intentional — the iteration cap (the
                    // structural net) still bounds total retries across
                    // resumes because each resume costs one iteration.
                    let mut current_failure = err;
                    loop {
                        let summary = sanitize(&current_failure);
                        let recovery = planner.recovery()
                            .on_capability_error(&state, &summary).await;
                        match recovery {
                            RecoveryOutcome::Retry { recovery, alter } => {
                                state.recovery_state = recovery;
                                self.honor_alteration(&alter)?;  // backoff sleep, reject AdvanceFallback in skeleton
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
                                            state.stop_state.terminate_hints_in_last_batch += 1;
                                        }
                                        break;  // resolved — leave inner retry loop
                                    }
                                    CapabilityOutcome::Failed(next_err) => {
                                        // DO NOT push next_err.failure_kind to
                                        // recent_failure_kinds — already pushed
                                        // for the originating call above.
                                        current_failure = next_err;
                                        continue;
                                    }
                                    CapabilityOutcome::ApprovalRequired(_)
                                    | CapabilityOutcome::AuthRequired(_)
                                    | CapabilityOutcome::ResourceBlocked(_)
                                    | CapabilityOutcome::Denied(_)
                                    | CapabilityOutcome::SpawnedProcess(_) => {
                                        // Promotion: a non-Failed outcome
                                        // appeared on retry. Re-route through
                                        // the matching outer arm via a helper.
                                        return self.handle_promoted_outcome(
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
    /// limit)`, partitions the returned envelopes, and records exact ack tokens
    /// for the user-facing messages that were appended. The tokens are acked
    /// only after the next checkpoint persists the advanced cursor.
    ///
    /// IMPORTANT: `LoopInputPort` carries multiple kinds — `UserMessage`,
    /// `Cancel`, `Interrupt`, `GateResolved`, `CapabilitySurfaceChanged`, etc.
    /// This drain ONLY consumes the contiguous user-facing prefix for the
    /// steering channel. A control event before the next steering message wins:
    /// return to the control path without moving `state.input_cursor` or acking
    /// the later steering message. This avoids the old unsafe
    /// `ack_through(cursor)` shape where `[Cancel, UserMessage]` either lost
    /// the cancel or redelivered the user message forever.
    async fn drain_steering_into(
        &self,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        mut state: LoopExecutionState,
    ) -> Result<LoopExecutionState, AgentLoopExecutorError> {
        let batch = host
            .poll_inputs(state.input_cursor.clone(), MAX_PER_DRAIN)
            .await
            .map_err(|_| AgentLoopExecutorError::HostUnavailable { stage: HostStage::Input })?;
        let DrainPartition { messages: steering_msgs, last_cursor, ack_tokens } =
            partition_steering_kinds(&batch)?;  // stops at first control input
        if !steering_msgs.is_empty() {
            state.input_cursor = last_cursor;
            state.pending_input_acks.extend(ack_tokens);
            // Append steering_msgs into transcript-bound state — concrete shape
            // depends on how messages flow into the next prompt bundle (host-owned
            // projection per master doc §6).
        }
        Ok(state)
    }

    /// Drain the followup queue. Returns `(state, drained_any)`. If
    /// `drained_any` is false the executor returns `LoopExit::Completed`.
    /// Same control-event filtering as `drain_steering_into`: only
    /// user-facing message kinds count toward "any drained."
    ///
    /// Returns owned state to honor the value-immutable contract (master doc
    /// §8 property 3 — no `&mut LoopExecutionState` across helper boundaries).
    async fn drain_followup_into(
        &self,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        mut state: LoopExecutionState,
    ) -> Result<(LoopExecutionState, bool), AgentLoopExecutorError> {
        let batch = host
            .poll_inputs(state.input_cursor.clone(), MAX_PER_DRAIN)
            .await
            .map_err(|_| AgentLoopExecutorError::HostUnavailable { stage: HostStage::Input })?;
        let DrainPartition { messages: followup_msgs, last_cursor, ack_tokens } =
            partition_followup_kinds(&batch)?;
        if followup_msgs.is_empty() {
            return Ok((state, false));
        }
        state.input_cursor = last_cursor;
        state.pending_input_acks.extend(ack_tokens);
        Ok((state, true))
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

**Where `concurrency_hint` lives:** `CapabilityDescriptorView` (in `ironclaw_turns::run_profile::host`) gains a `concurrency_hint: ConcurrencyHint` field — additive contract change introduced by WS-0 (see [`state-and-checkpoints.md`](state-and-checkpoints.md) §2 EXTEND list). The hint is **derived at the adapter boundary** in WS-9 (`HostRuntimeLoopCapabilityPort::visible_capabilities`) from the underlying `CapabilityDescriptor.effects` Vec: presence of any write/spawn/exclusive effect → `Exclusive`; otherwise → `SafeForParallel`. Lower-layer `CapabilityDescriptor` is NOT modified — `effects` is already the source of truth, and computing the hint at the view-layer adapter keeps the inference in one place. Tool authors don't have to remember to declare a hint correctly; they declare effects (which they already do), and the system infers conservatively.

### 3.4 Checkpoint helper

The checkpoint flow is **two-step**: the executor serializes state to bytes,
stages those bytes via a storage-layer port to receive a validated
`LoopCheckpointStateRef`, then hands the metadata-write port only the
**ref**. The metadata port (`LoopCheckpointPort::checkpoint`) never sees
raw payload bytes; it receives an opaque token + the checkpoint kind +
the schema id.

This split lets the metadata write be small and side-effect-free at the
contract layer (the metadata is mostly the ref pointer), while the actual
byte storage layer can be backed by any blob substrate (PostgreSQL, S3,
local filesystem) without changing the contract.

```rust
impl CanonicalAgentLoopExecutor {
    async fn checkpoint(
        &self,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        mut state: LoopExecutionState,
        kind: CheckpointKind,
    ) -> Result<LoopExecutionState, AgentLoopExecutorError> {
        // Step 1: executor serializes state into bytes (schema id from WS-0).
        // Use a JCS-canonicalized payload so the bytes are reproducible
        // across runs and the content digest in checkpoint metadata is
        // stable (see master doc §9 canonicalization bullet).
        let payload = serialize_checkpoint(&state, CHECKPOINT_SCHEMA_ID);

        // Step 2: stage the bytes via the storage layer. Returns an
        // opaque, validated `LoopCheckpointStateRef` of form
        // `"checkpoint:{run_id}:{token}"` (defined in
        // `ironclaw_turns::run_profile::host` line 210).
        // `stage_checkpoint_payload` is owned by the host facade — it
        // wraps the underlying `CheckpointStateStore::put_checkpoint_state`.
        let state_ref = host
            .stage_checkpoint_payload(StageCheckpointPayloadRequest {
                schema_id: CHECKPOINT_SCHEMA_ID,
                payload,
            })
            .await
            .map_err(|_| AgentLoopExecutorError::CheckpointFailed { stage: kind })?;

        // Step 3: write the metadata via the existing
        // `LoopCheckpointPort::checkpoint(kind, state_ref)` contract.
        // The port never sees the raw bytes — only the validated ref.
        host.checkpoint(LoopCheckpointRequest { kind, state_ref })
            .await
            .map_err(|_| AgentLoopExecutorError::CheckpointFailed { stage: kind })?;

        state.last_checkpoint = Some(CheckpointMarker { kind, iteration_at_checkpoint: state.iteration });
        let pending_acks = std::mem::take(&mut state.pending_input_acks);
        if !pending_acks.is_empty() {
            host.ack_inputs(pending_acks)
                .await
                .map_err(|_| AgentLoopExecutorError::HostUnavailable { stage: HostStage::Input })?;
        }
        Ok(state)
    }
}
```

The pseudocode names `state.pending_input_acks` for locality; the real
implementation should carry it as executor-local scratch
(`PendingInputAcks` threaded beside `LoopExecutionState`), not as a
serialized checkpoint field. The durable fact is the advanced
`state.input_cursor` written by `checkpoint(...)`; acking after that
checkpoint is a storage reclamation step. A crash before checkpoint
redelivers the input from the old cursor. A crash after checkpoint but
before ack resumes from the advanced cursor and therefore does not
re-append the already-checkpointed message.

`stage_checkpoint_payload` is a small additive method on
`AgentLoopDriverHost` introduced by this brief (lives in `ironclaw_turns`
per master doc §12 crate-ownership rule; concrete impl wraps the existing
`CheckpointStateStore::put_checkpoint_state` in `ironclaw_loop_support`).
WS-10's `load_checkpoint_payload` is its read-side dual.


### 3.5 Cancellation observation

The host facade exposes a way to observe cancellation between strategy calls — a sync method on `AgentLoopDriverHost` (or the `LoopCancellationPort` defined in WS-13) returning a current-cancel-state. The executor MUST call it at **every awaited boundary in the canonical tick**, not just at top-of-iteration. The explicit awaited boundaries (cross-referenced as `CANCEL_BOUNDARY` markers in §3.2 above):

| # | Site | Before strategy call |
|---|---|---|
| 1 | Top of iteration | first thing in each tick |
| 2 | Pre-`drain().drain_steering` | between iteration prelude and drain |
| 3 | Pre-`context().plan_context_request` | between drain and context |
| 4 | Pre-`capability().filter` (after `build_prompt_bundle` returns) | between context-build and capability-surface |
| 5 | Pre-`model().preference` (after `BeforeModel` checkpoint) | between checkpoint and model-pref |
| 6 | Pre-`stop().should_stop_after_turn` (model-response branch arm) | fires on either Reply or CapabilityCalls path — exactly one path per iteration; the boundary sits at the branch point |
| 7 | Inside the capability-batch loop, before each `gate().handle` | between successive outcome arms |
| 8 | Inside the inner retry loop, before each `recovery().on_capability_error` | between retry attempts |

Eight sites. Per master doc §10, strategies are sealed Builtin code that returns promptly; **cooperative cancellation at these awaited boundaries is sufficient without preemptive `tokio::select!` wrapping**. The boundary list IS the contract — adding a new strategy call to the executor MUST add a matching `CANCEL_BOUNDARY`. WS-8's integration suite includes a cancellation-fires-at-each-boundary test covering all eight sites.

The boundary helper has one canonical name across all briefs: **`checkpoint_and_exit_if_cancelled`** (used by WS-6 as the consumer). Master doc §8 pseudocode and WS-13's `LoopCancellationPort` brief use the same name.

If the existing host API does not yet expose a cancellation accessor, this brief documents the requirement and either:

- (a) adds the missing accessor to `AgentLoopDriverHost` (small, additive change in `ironclaw_turns`); or
- (b) uses a tokio `CancellationToken` passed through `AgentLoopExecutor::execute_family` as an additional parameter.

Pick (a) if the host already has cancellation plumbing; (b) otherwise.

**Cancellation is a successful exit, not an executor error.** When the signal fires:

1. Checkpoint with whatever `CheckpointKind` is appropriate for the current step (`BeforeModel` / `BeforeSideEffect` / `BeforeBlock`).
2. Build a `LoopExit::Cancelled(LoopCancelled { reason_kind: HostInterrupt | HostCancellation, checkpoint_id: …, interrupted_message_refs: state.assistant_refs.clone(), exit_id: … })` (variant defined in `crates/ironclaw_turns/src/loop_exit.rs:400`).
3. Return `Ok(LoopExit::Cancelled(...))` directly from `execute_family()`.

`AgentLoopExecutorError::Cancelled` is **only** for the truly-unrecoverable case where the executor cannot even produce a `LoopExit::Cancelled` (e.g. the cancellation checkpoint write itself failed and we have no valid checkpoint id to embed). WS-7 maps that residual case to `AgentLoopDriverError::Failed { reason_kind: "interrupted_unexpectedly" }`, not to `Unavailable`. Normal cancellation never visits the error mapping path.

### 3.5a Strategy-decision observability

Strategies make consequential decisions (retry/skip/abort, stop/continue, drain-now/wait). Operators investigating a misbehaving run need to see which strategy decided what. `LoopProgressPort` (WS-12) covers **executor** milestones; strategy outcomes are not enumerated as `LoopProgressEvent` variants in the skeleton.

Skeleton-stage observability: the executor emits `tracing::debug!` at every strategy call site, naming the strategy, its inputs (refs and state slot values only — never raw content), and its outcome. Example:

```rust
let stop = planner.stop().should_stop_after_turn(&state, &summary).await;
tracing::debug!(
    target: "ironclaw_agent_loop::executor",
    family_id = %family.id(),
    iteration = state.iteration,
    summary_kind = ?summary.kind,
    outcome = ?stop,
    "StopConditionStrategy::should_stop_after_turn",
);
```

Durable strategy-decision telemetry — a typed event log separate from `LoopProgressEvent` milestones — is deferred. When a production debugging need materializes (someone files a "why did the loop abort?" ticket that `tracing::debug!` doesn't answer), a follow-up workstream introduces `StrategyDecisionEvent` variants on `LoopProgressPort`. Until then, `tracing` is sufficient and avoids over-designing the observability surface.

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
- [ ] Smoke test: with a `MockHost` that returns a Reply on first call, `CanonicalAgentLoopExecutor::execute_family(&families::default(), &host, initial_state)` returns `LoopExit::Completed` with `assistant_refs.len() == 1`. Final checkpoint observed in mock recorder.
- [ ] Smoke test: with a `MockHost` whose first model call returns `CapabilityCalls` and whose second returns Reply, executor takes `BeforeModel`, `BeforeSideEffect`, `BeforeModel`, `Final` checkpoints in order; returns `Completed`.
- [ ] **Stop-after-batch smoke test:** with a `MockHost` whose batch returns one outcome with `terminate_hint: true`, executor calls `should_stop_after_turn` with `TurnEndKind::AfterCapabilityBatch` after the batch and returns `LoopExit::Completed { GracefulStop }` *without* a follow-up model call.
- [ ] Smoke test: with a `MockHost` whose model call returns `CapabilityCalls` whose only outcome is `ApprovalRequired`, executor takes `BeforeModel`, `BeforeSideEffect`, `BeforeBlock` checkpoints; returns `LoopExit::Blocked` with `gate_ref` set.
- [ ] Iteration limit smoke test: with a `MockHost` that always returns `CapabilityCalls`, planner with `iteration_limit() = 3`, executor returns `LoopExit::Failed { IterationLimit }` after **exactly 3** model-call iterations (using `>=` semantics — not 4).
- [ ] No-progress smoke test: with a `MockHost` whose batch returns the same single call signature on every iteration, executor returns `LoopExit::Failed { NoProgressDetected }` once 3 distinct iterations have produced that signature within the last 5 iterations (per the dedupe rule in WS-0 §3.4).
- [ ] **Retry smoke test:** with a `MockHost` whose batch returns one `Failed { Transient }` outcome and whose single-call API (`invoke_capability`) returns `Completed` on the second attempt, executor produces `LoopExit::Completed`; `state.result_refs.len() == 1`; mock-host call log shows one `invoke_capability_batch` followed by one `invoke_capability`.
- [ ] **Cancellation smoke test:** with a `MockHost` whose cancellation accessor flips to `true` between turns, executor returns `Ok(LoopExit::Cancelled(...))` (not `Err`); checkpoint recorded with appropriate `CheckpointKind` and `interrupted_message_refs` populated from `state.assistant_refs`.
- [ ] No `unwrap()` / `expect()` outside test code (per `error-handling.md`)
- [ ] No raw provider/secret/host-path/tool-input strings ever appear in `state` or returned errors
- [ ] Doc comments on `CanonicalAgentLoopExecutor::execute_family` cite master doc §8

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
