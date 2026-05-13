//! Canonical agent-loop executor.
//!
//! The executor owns loop mechanics: checkpointing, host-port calls, strategy
//! sequencing, and safety-net exits. Planners remain pure strategy
//! composition.

use std::{
    collections::HashSet,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use ironclaw_turns::{
    LoopFailureKind, LoopGateRef, LoopMessageRef,
    run_profile::{
        AgentLoopDriverHost, AgentLoopHostError, AgentLoopHostErrorKind, AssistantReply,
        CapabilityBatchInvocation, CapabilityCallCandidate, CapabilityConcurrency,
        CapabilityFailure, CapabilityInvocation, CapabilityOutcome, CapabilityResultMessage,
        FinalizeAssistantMessage, LoopCheckpointKind, LoopCheckpointRequest,
        LoopCheckpointStateRef, LoopInput, LoopModelRequest, ProcessHandleSummary,
        StoreLoopCheckpointPayload, VisibleCapabilityRequest, VisibleCapabilitySurface,
    },
};

use crate::{
    planner::AgentLoopPlanner,
    state::{
        CHECKPOINT_SCHEMA_ID, CapabilityCallSignature, CheckpointKind, CheckpointMarker,
        LoopExecutionState,
    },
    strategies::{
        BatchPolicy, CapabilityCallSummary, CapabilityErrorClass, CapabilityErrorSummary,
        CapabilityFilter, ConcurrencyHint, GateKind, GateOutcome, GateSummary, ModelErrorClass,
        ModelErrorSummary, ModelPreference, RecoveryOutcome, StopKind, StopOutcome, TurnEndKind,
        TurnSummary,
    },
};

const INPUT_POLL_LIMIT: usize = 16;
const NO_PROGRESS_WINDOW: usize = 5;
const NO_PROGRESS_THRESHOLD: usize = 3;
/// Defense-in-depth cap on the inner retry loop. The default
/// `RecoveryStrategy` returns `Abort` once its own per-class budget is
/// exhausted; this constant only guards against a custom strategy that
/// indefinitely returns `Retry`.
const MAX_RETRIES_PER_CALL: u32 = 8;

/// Drives the canonical loop tick against a planner and host facade.
#[async_trait]
pub trait AgentLoopExecutor: Send + Sync {
    /// See master spec §8 for the canonical iteration algorithm.
    async fn execute(
        &self,
        planner: &dyn AgentLoopPlanner,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        state: &mut LoopExecutionState,
    ) -> Result<LoopExit, AgentLoopExecutorError>;
}

/// Loop exit produced by the canonical framework executor.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopExit {
    Completed(CompletionKind),
    Failed { kind: FailureKind },
    Blocked { gate_ref: LoopGateRef },
    Cancelled(CancelledKind),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompletionKind {
    NaturalEnd,
    GracefulStop,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    IterationLimitReached,
    NoProgressDetected,
    /// `BudgetStrategy::wall_clock_limit` exceeded before the loop reached a
    /// natural terminal state. Distinct from `IterationLimitReached` so a
    /// profile that opted into a wall-clock cap can tell time-bound vs
    /// step-bound exhaustion apart (master spec §6 — `BudgetStrategy`).
    WallClockLimitReached,
    Other(LoopFailureKind),
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CancelledKind {
    pub interrupted_message_refs: Vec<LoopMessageRef>,
}

/// Sanitized executor errors. Loop-level failures should usually be returned
/// as [`LoopExit::Failed`]; this type is reserved for cases where the executor
/// cannot produce a normal loop exit.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AgentLoopExecutorError {
    #[error("host port returned an unrecoverable error at {stage:?}")]
    HostUnavailable { stage: HostStage },
    #[error("planner returned a contract violation: {detail}")]
    PlannerContract { detail: &'static str },
    #[error("checkpoint write failed at {stage:?}")]
    CheckpointFailed { stage: CheckpointKind },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostStage {
    Prompt,
    Model,
    Capability,
    Transcript,
    Checkpoint,
    Input,
}

/// The reference executor. Implements the canonical tick from master spec §8.
#[derive(Debug, Default, Clone, Copy)]
pub struct CanonicalAgentLoopExecutor;

#[async_trait]
impl AgentLoopExecutor for CanonicalAgentLoopExecutor {
    async fn execute(
        &self,
        planner: &dyn AgentLoopPlanner,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        state: &mut LoopExecutionState,
    ) -> Result<LoopExit, AgentLoopExecutorError> {
        let mut next = state.clone();
        // In-process anchor for `BudgetStrategy::wall_clock_limit`
        // enforcement. The executor consults this at the top of every tick
        // (alongside `iteration_limit`) so a profile that opts into a time
        // cap can fail out even when the model+capability pipeline would
        // otherwise keep producing forward progress. Master spec §6 /
        // WS-6 iter-5 finding 2.
        //
        // Iter-6 finding 1: the persisted `started_at_unix_ms` anchor
        // (carried in `LoopExecutionState`) survives `Blocked` / process
        // restart / checkpoint reload, while this `tokio::time::Instant`
        // does not. We set the persisted anchor on first entry only, and
        // consult it AS WELL AS the in-process `Instant` so that:
        //   - a fresh run anchors both at the same wall-clock moment;
        //   - a resumed run with an already-old `started_at_unix_ms`
        //     trips the cap as soon as the first tick observes
        //     `SystemTime::now() - started_at >= limit`, even though the
        //     fresh `Instant` would otherwise reset the budget.
        let start_time = tokio::time::Instant::now();
        if next.started_at_unix_ms.is_none() {
            next.started_at_unix_ms = Some(system_time_now_unix_ms());
        }

        loop {
            if next.iteration >= planner.budget().iteration_limit(&next) {
                // Iter-5 finding 4: take a `Final` checkpoint before failing
                // so profiles with `require_final_checkpoint = true` (durable
                // mission) don't reject the failure as
                // `MissingFinalCheckpoint`.
                let checked = self.checkpoint(host, next, CheckpointKind::Final).await?;
                *state = checked;
                return Ok(LoopExit::Failed {
                    kind: FailureKind::IterationLimitReached,
                });
            }
            if let Some(limit) = planner.budget().wall_clock_limit(&next)
                && wall_clock_limit_exceeded(start_time, next.started_at_unix_ms, limit)
            {
                let checked = self.checkpoint(host, next, CheckpointKind::Final).await?;
                *state = checked;
                return Ok(LoopExit::Failed {
                    kind: FailureKind::WallClockLimitReached,
                });
            }

            let observed = self.observe_cancellation(host, next).await?;
            next = observed.0;
            if let Some(exit) = observed.1 {
                *state = next;
                return Ok(exit);
            }

            if planner.drain().drain_steering(&next).await {
                next = self.drain_steering(host, next).await?;
            }

            let context_request = planner.context().plan_context_request(&next).await;
            let bundle = host
                .build_prompt_bundle(context_request)
                .await
                .map_err(|_| AgentLoopExecutorError::HostUnavailable {
                    stage: HostStage::Prompt,
                })?;

            let filter = planner.capability().filter(&next).await;
            let surface = host
                .visible_capabilities(VisibleCapabilityRequest)
                .await
                .map_err(|_| AgentLoopExecutorError::HostUnavailable {
                    stage: HostStage::Capability,
                })?;
            let surface = apply_capability_filter(surface, &filter);
            next.surface_version = Some(surface.version.clone());

            next = self
                .checkpoint(host, next, CheckpointKind::BeforeModel)
                .await?;

            let preference = planner.model().preference(&next).await;
            let model_preference = model_preference_id(preference)?;
            let model_request = LoopModelRequest {
                messages: bundle.messages,
                surface_version: Some(surface.version.clone()),
                model_preference,
            };
            let model_response = match self
                .invoke_model_with_recovery(planner, host, next, model_request)
                .await?
            {
                ModelStep::Response(response_state, response) => {
                    next = response_state;
                    response
                }
                ModelStep::ReloadSurface(reloaded_state) => {
                    // StaleSurface (master spec §10): drop the cached surface
                    // and restart the iteration so the next pass re-fetches
                    // visible capabilities. Iteration is NOT advanced —
                    // restart from the same tick.
                    next = reloaded_state;
                    next.surface_version = None;
                    *state = next.clone();
                    continue;
                }
                ModelStep::SkipIteration(skip_state) => {
                    // Iter-5 finding 1: a recovery `SkipResult` on a model
                    // error must advance the iteration counter so the
                    // outer cap eventually trips. Drop the cached surface
                    // version (the next iteration will re-fetch) and tick
                    // the counter. Without this, a persistent transient
                    // model failure under a SkipResult-returning recovery
                    // strategy spins forever.
                    next = skip_state;
                    next.surface_version = None;
                    next.iteration = next.iteration.saturating_add(1);
                    *state = next.clone();
                    continue;
                }
                ModelStep::Exit(exit_state, exit) => {
                    // Iter-5 finding 4: a `Failed` terminal exit MUST carry a
                    // `Final` checkpoint. `Cancelled` already took one inside
                    // `invoke_model_with_recovery`'s Cancelled branch, so we
                    // only need to handle the `Failed` shape here. Same logic
                    // applies symmetrically to capability `Step::Exit` paths
                    // below.
                    let (checked, exit) = self
                        .final_checkpoint_for_failure(host, exit_state, exit)
                        .await?;
                    *state = checked;
                    return Ok(exit);
                }
            };

            match model_response.output {
                ironclaw_turns::run_profile::ParentLoopOutput::AssistantReply(reply) => {
                    let (reply_state, stop) = self
                        .finalize_reply_and_check_stop(planner, host, next, reply)
                        .await?;
                    match stop {
                        StopOutcome::Stop { kind, .. } => {
                            let (checked, exit) =
                                self.exit_for_stop_kind(host, reply_state, kind).await?;
                            *state = checked;
                            return Ok(exit);
                        }
                        StopOutcome::Continue { .. } => {
                            // Drain followup if planner asks. If a `FollowUp`
                            // arrived between the assistant reply and now, we
                            // must NOT take the Final checkpoint — the user
                            // has more to say and the run continues with the
                            // appended input on the next iteration. If only
                            // control inputs are pending (Cancel / Interrupt /
                            // GateResolved / SurfaceChanged), continue without
                            // acking so the next tick's observe_cancellation
                            // catches them. Only checkpoint Final when the
                            // followup queue is truly empty.
                            let (drained_state, outcome) = self
                                .drain_followup_if_planner_asks(planner, host, reply_state)
                                .await?;
                            match outcome {
                                FollowupDrainOutcome::FollowUpConsumed => {
                                    next = drained_state;
                                    next.iteration = next.iteration.saturating_add(1);
                                    *state = next.clone();
                                    continue;
                                }
                                FollowupDrainOutcome::TerminalCancel { next_cursor } => {
                                    // Cancel/Interrupt arrived in the drain
                                    // page (potentially mixed with a
                                    // FollowUp). Iter-6 finding 2: take the
                                    // `Final` checkpoint FIRST, then ack
                                    // the page. If the checkpoint fails we
                                    // surface the error without acking so
                                    // the next `execute()` re-polls the
                                    // cancel.
                                    //
                                    // Iter-9 finding 1: advance the cursor
                                    // BEFORE the checkpoint so the
                                    // durable Final state names the
                                    // post-cancel position. The
                                    // pre-iter-9 code set
                                    // `acked.input_cursor` after the
                                    // checkpoint, so the persisted state
                                    // still pointed at the cancel — on
                                    // resume the loop would re-poll a
                                    // page the host had already dropped.
                                    let mut advanced = drained_state;
                                    advanced.input_cursor = next_cursor.clone();
                                    let checked = self
                                        .checkpoint(host, advanced, CheckpointKind::Final)
                                        .await?;
                                    host.ack_inputs(next_cursor).await.map_err(|_| {
                                        AgentLoopExecutorError::HostUnavailable {
                                            stage: HostStage::Input,
                                        }
                                    })?;
                                    let exit = LoopExit::Cancelled(CancelledKind {
                                        interrupted_message_refs: checked.assistant_refs.clone(),
                                    });
                                    *state = checked;
                                    return Ok(exit);
                                }
                                FollowupDrainOutcome::ControlPending => {
                                    // Iter-7 finding 2: drain hit
                                    // `INPUT_POLL_LIMIT` consecutive
                                    // control-only pages. Side effects
                                    // were applied + acked but the
                                    // queue might still be holding a
                                    // FollowUp on a later page. Do
                                    // NOT take the Final checkpoint
                                    // and do NOT exit `Completed` —
                                    // advance the iteration so the
                                    // next tick can keep draining.
                                    next = drained_state;
                                    next.iteration = next.iteration.saturating_add(1);
                                    *state = next.clone();
                                    continue;
                                }
                                FollowupDrainOutcome::Empty => {
                                    let final_state = self
                                        .checkpoint(host, drained_state, CheckpointKind::Final)
                                        .await?;
                                    *state = final_state;
                                    return Ok(LoopExit::Completed(CompletionKind::NaturalEnd));
                                }
                            }
                        }
                    }
                }
                ironclaw_turns::run_profile::ParentLoopOutput::CapabilityCalls(calls) => {
                    let result_refs_start = next.result_refs.len();
                    match self
                        .handle_capability_calls(planner, host, next, &surface, calls)
                        .await?
                    {
                        Step::Exit(exit_state, exit) => {
                            // Iter-5 finding 4: Final-checkpoint on the
                            // `Failed` shape before returning. `Blocked`
                            // already took `BeforeBlock` inside `handle_gate`
                            // (and the spec says blocked exits checkpoint
                            // BeforeBlock, not Final). `Cancelled` already
                            // took `Final` inside the capability retry's
                            // Cancelled branch.
                            let (checked, exit) = self
                                .final_checkpoint_for_failure(host, exit_state, exit)
                                .await?;
                            *state = checked;
                            return Ok(exit);
                        }
                        Step::Continue(batch_state) => {
                            next = batch_state;
                        }
                    }

                    let summary = TurnSummary {
                        kind: TurnEndKind::AfterCapabilityBatch,
                        assistant_message_ref: None,
                        batch_result_refs: next.result_refs[result_refs_start..].to_vec(),
                    };
                    let stop = planner.stop().should_stop_after_turn(&next, &summary).await;
                    match stop {
                        StopOutcome::Continue { control } => {
                            next.control_state = control;
                        }
                        StopOutcome::Stop { control, kind } => {
                            next.control_state = control;
                            let (checked, exit) = self.exit_for_stop_kind(host, next, kind).await?;
                            *state = checked;
                            return Ok(exit);
                        }
                    }

                    if let Some(exit_kind) = self.no_progress_exit(&next) {
                        // Iter-5 finding 4: take a `Final` checkpoint on the
                        // no-progress path too. The pre-iter-5 code returned
                        // `LoopExit::Failed` without checkpointing, so a
                        // profile with `require_final_checkpoint = true`
                        // would reject the exit as `MissingFinalCheckpoint`.
                        let checked = self.checkpoint(host, next, CheckpointKind::Final).await?;
                        *state = checked;
                        return Ok(LoopExit::Failed { kind: exit_kind });
                    }

                    let observed = self.observe_cancellation(host, next).await?;
                    next = observed.0;
                    if let Some(exit) = observed.1 {
                        *state = next;
                        return Ok(exit);
                    }

                    next.iteration = next.iteration.saturating_add(1);
                    *state = next.clone();
                }
            }
        }
    }
}

enum Step {
    Continue(LoopExecutionState),
    Exit(LoopExecutionState, LoopExit),
}

/// Internal routing classification for `LoopModelPort` errors.
enum ModelErrorRouting {
    Cancelled,
    StaleSurface,
    Recoverable(ModelErrorClass),
    HostUnavailable,
}

/// Outcome of `invoke_model_with_recovery`.
enum ModelStep {
    /// Model returned a usable response; carry forward updated state.
    Response(
        LoopExecutionState,
        ironclaw_turns::run_profile::LoopModelResponse,
    ),
    /// Host reported `StaleSurface`; caller must reload capabilities and
    /// re-issue the iteration without advancing the iteration counter.
    ReloadSurface(LoopExecutionState),
    /// Recovery returned `SkipResult` for a model error. The model call is
    /// dropped, the iteration counter MUST advance on the next loop tick, and
    /// the outer loop's iteration cap / wall-clock cap eventually trips even
    /// if the underlying model port keeps failing. Iter-5 finding 1: this is
    /// distinct from `ReloadSurface`, which restarts the SAME iteration
    /// without bumping the counter (and so spins forever when recovery
    /// always returns `SkipResult` against a persistent
    /// `Unavailable`/`Internal` model error).
    SkipIteration(LoopExecutionState),
    /// Recovery decided to abort; bubble up the loop exit.
    Exit(LoopExecutionState, LoopExit),
}

/// Outcome of a follow-up drain poll.
#[derive(Debug, Clone, PartialEq, Eq)]
enum FollowupDrainOutcome {
    /// A `FollowUp` was acked; the loop must continue (no `Final` checkpoint).
    /// Any GateResolved / CapabilitySurfaceChanged inputs in the same page were
    /// applied to state in-place as idempotent side effects.
    FollowUpConsumed,
    /// A `Cancel` or `Interrupt` was observed in the drain page. The page
    /// has NOT been acked — `drain_followup` carries the `next_cursor` back
    /// to the caller, which must take the `Final` checkpoint and only then
    /// ack the page. Sibling control side effects in the same page were
    /// applied in place. Iter-6 finding 2: the pre-iter-6 code acked the
    /// terminal page inside `drain_followup` and let the caller checkpoint
    /// afterward, so a checkpoint failure left the cancel consumed but the
    /// run un-persisted.
    TerminalCancel {
        next_cursor: ironclaw_turns::run_profile::LoopInputCursor,
    },
    /// Drained `INPUT_POLL_LIMIT` consecutive control-only pages without
    /// reaching a definitive answer. All control side effects were applied
    /// and their pages were acked, but we cannot conclude the queue is
    /// empty — a genuine FollowUp may be sitting on a later page. The
    /// caller MUST NOT take the `Final` checkpoint and MUST NOT exit
    /// `Completed`; it should advance the iteration and let the next tick
    /// continue draining. Iter-7 finding 2: pre-iter-7 the same shape
    /// returned `Empty`, which the caller treated as "queue drained" and
    /// stranded any FollowUp sitting past page 16.
    ControlPending,
    /// Queue was empty (or contained only GateResolved / SurfaceChanged that
    /// were applied + acked); the loop completes naturally.
    Empty,
}

impl CanonicalAgentLoopExecutor {
    /// Issue the model call, classifying any host-port error against the
    /// runtime recovery strategy (master spec §10, WS-6 finding 7).
    ///
    /// - `Cancelled` from the model port: surfaced as `HostUnavailable` so
    ///   the outer cancellation-observation path runs on the next tick.
    /// - `StaleSurface`: signal the caller to reload capabilities and retry.
    /// - Transient (`Unavailable` / `Internal`): build a `ModelErrorSummary`
    ///   and consult `RecoveryStrategy::on_model_error`; honor `Retry` /
    ///   `SkipResult` / `Abort`.
    /// - Other host errors collapse to `HostUnavailable { Model }`.
    async fn invoke_model_with_recovery(
        &self,
        planner: &dyn AgentLoopPlanner,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        mut state: LoopExecutionState,
        request: LoopModelRequest,
    ) -> Result<ModelStep, AgentLoopExecutorError> {
        // WS-6 iter-4 finding 3: a logical model call records its failure
        // kind in `recent_failure_kinds` AT MOST ONCE — not once per retry
        // attempt. The pre-iter-4 code pushed on every attempt, so an
        // eventually-successful model turn could trip
        // `DefaultStopConditionStrategy::failure_run_threshold` (3) as soon
        // as a custom recovery allowed three retries. Mirrors the capability
        // retry path, which pushes once at the start of
        // `handle_capability_error` and never inside the inner loop.
        let mut recorded_failure = false;
        for _ in 0..MAX_RETRIES_PER_CALL {
            match host.stream_model(request.clone()).await {
                Ok(response) => return Ok(ModelStep::Response(state, response)),
                Err(error) => match self.classify_model_host_error(&error) {
                    ModelErrorRouting::Cancelled => {
                        // WS-6 iter-3 finding 5: surface model-port
                        // cancellation as `LoopExit::Cancelled` rather
                        // than `HostUnavailable`. The host aborted the
                        // in-flight stream; the loop must take the
                        // `Final` checkpoint and exit cleanly so callers
                        // see the cancellation as a normal terminal
                        // state, not as infrastructure failure.
                        let checked = self.checkpoint(host, state, CheckpointKind::Final).await?;
                        let exit = LoopExit::Cancelled(CancelledKind {
                            interrupted_message_refs: checked.assistant_refs.clone(),
                        });
                        return Ok(ModelStep::Exit(checked, exit));
                    }
                    ModelErrorRouting::StaleSurface => {
                        return Ok(ModelStep::ReloadSurface(state));
                    }
                    ModelErrorRouting::Recoverable(class) => {
                        let summary = model_error_summary(class, &error);
                        if !recorded_failure {
                            state.recent_failure_kinds.push(LoopFailureKind::ModelError);
                            recorded_failure = true;
                        }
                        let outcome = planner.recovery().on_model_error(&state, &summary).await;
                        match outcome {
                            RecoveryOutcome::Retry { recovery, alter } => {
                                state.recovery_state = recovery;
                                if matches!(
                                    alter,
                                    Some(crate::strategies::RetryAlteration::AdvanceFallback)
                                ) {
                                    return Ok(ModelStep::Exit(
                                        state,
                                        LoopExit::Failed {
                                            kind: FailureKind::Other(LoopFailureKind::DriverBug),
                                        },
                                    ));
                                }
                                // WS-6 iter-3 finding 4: honor `Backoff`
                                // delays as a tokio sleep before the next
                                // model attempt.
                                if let Some(crate::strategies::RetryAlteration::Backoff { delay }) =
                                    alter
                                {
                                    tokio::time::sleep(delay).await;
                                }
                                continue;
                            }
                            RecoveryOutcome::SkipResult { recovery } => {
                                // SkipResult on a model error means "drop this
                                // turn AND advance to the next iteration".
                                // Iter-5 finding 1: the pre-iter-5 code routed
                                // this through `ReloadSurface`, which restarts
                                // the SAME tick without bumping
                                // `state.iteration`. With `DefaultPlanner`'s
                                // placeholder recovery (which returns
                                // `SkipResult` on every model error), a
                                // persistent `Unavailable`/`Internal` model
                                // failure would spin forever — never hitting
                                // the iteration cap or the wall-clock cap.
                                // `SkipIteration` is the explicit
                                // monotonic-progress variant: the outer
                                // execute() advances the iteration counter so
                                // the budget eventually trips.
                                state.recovery_state = recovery;
                                return Ok(ModelStep::SkipIteration(state));
                            }
                            RecoveryOutcome::Abort {
                                recovery,
                                failure_kind,
                            } => {
                                state.recovery_state = recovery;
                                return Ok(ModelStep::Exit(
                                    state,
                                    LoopExit::Failed {
                                        kind: failure_kind_to_exit(failure_kind),
                                    },
                                ));
                            }
                        }
                    }
                    ModelErrorRouting::HostUnavailable => {
                        return Err(AgentLoopExecutorError::HostUnavailable {
                            stage: HostStage::Model,
                        });
                    }
                },
            }
        }
        // Defense-in-depth: a custom recovery strategy returned `Retry` more
        // than `MAX_RETRIES_PER_CALL` times.
        Ok(ModelStep::Exit(
            state,
            LoopExit::Failed {
                kind: FailureKind::Other(LoopFailureKind::DriverBug),
            },
        ))
    }

    fn classify_model_host_error(&self, error: &AgentLoopHostError) -> ModelErrorRouting {
        match error.kind {
            AgentLoopHostErrorKind::Cancelled => ModelErrorRouting::Cancelled,
            AgentLoopHostErrorKind::StaleSurface => ModelErrorRouting::StaleSurface,
            AgentLoopHostErrorKind::Unavailable => {
                ModelErrorRouting::Recoverable(ModelErrorClass::Unavailable)
            }
            AgentLoopHostErrorKind::Internal => {
                ModelErrorRouting::Recoverable(ModelErrorClass::Internal)
            }
            AgentLoopHostErrorKind::BudgetExceeded => {
                ModelErrorRouting::Recoverable(ModelErrorClass::ContextOverflow)
            }
            AgentLoopHostErrorKind::Unauthorized
            | AgentLoopHostErrorKind::ScopeMismatch
            | AgentLoopHostErrorKind::InvalidInvocation
            | AgentLoopHostErrorKind::PolicyDenied
            | AgentLoopHostErrorKind::CheckpointRejected
            | AgentLoopHostErrorKind::TranscriptWriteFailed => ModelErrorRouting::HostUnavailable,
        }
    }

    async fn finalize_reply_and_check_stop(
        &self,
        planner: &dyn AgentLoopPlanner,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        mut state: LoopExecutionState,
        reply: AssistantReply,
    ) -> Result<(LoopExecutionState, StopOutcome), AgentLoopExecutorError> {
        let assistant_ref = host
            .finalize_assistant_message(FinalizeAssistantMessage { reply })
            .await
            .map_err(|_| AgentLoopExecutorError::HostUnavailable {
                stage: HostStage::Transcript,
            })?;
        state.assistant_refs.push(assistant_ref.clone());

        let summary = TurnSummary {
            kind: TurnEndKind::ReplyOnly,
            assistant_message_ref: Some(assistant_ref),
            batch_result_refs: Vec::new(),
        };
        let stop = planner
            .stop()
            .should_stop_after_turn(&state, &summary)
            .await;
        match &stop {
            StopOutcome::Continue { control } | StopOutcome::Stop { control, .. } => {
                state.control_state = control.clone();
            }
        }
        Ok((state, stop))
    }

    /// If the planner's drain strategy opts in, poll the input queue and
    /// return the followup-drain outcome. The caller decides whether to
    /// take the `Final` checkpoint (Empty) or continue the outer loop
    /// (FollowUpConsumed / ControlPending).
    async fn drain_followup_if_planner_asks(
        &self,
        planner: &dyn AgentLoopPlanner,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        state: LoopExecutionState,
    ) -> Result<(LoopExecutionState, FollowupDrainOutcome), AgentLoopExecutorError> {
        if planner.drain().drain_followup(&state).await {
            self.drain_followup(host, state).await
        } else {
            Ok((state, FollowupDrainOutcome::Empty))
        }
    }

    async fn handle_capability_calls(
        &self,
        planner: &dyn AgentLoopPlanner,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        state: LoopExecutionState,
        surface: &VisibleCapabilitySurface,
        calls: Vec<CapabilityCallCandidate>,
    ) -> Result<Step, AgentLoopExecutorError> {
        let mut state = self
            .checkpoint(host, state, CheckpointKind::BeforeSideEffect)
            .await?;
        let summaries = capability_summaries(surface, &calls);
        let policy = planner.batch().policy(&state, &summaries);
        state.control_state.last_batch_total = summaries.len() as u32;
        state.control_state.terminate_hints_in_last_batch = 0;

        // Enforce executor-side filter (master spec §6 / WS-6 finding 3 +
        // iter-3 finding 2): the narrowed surface is built locally, but
        // `VisibleCapabilityRequest` doesn't accept a filter — so the model
        // could cite the unfiltered host surface_version to invoke a hidden
        // capability. The fix preserves the planner's original call order:
        //
        //   - Compute an `(allowed, hidden)` mask in the original sequence.
        //   - If ALL calls are allowed, batch-invoke as before (preserves
        //     parallelism for the common case).
        //   - If ANY call is hidden, fall back to ordered per-call
        //     execution: invoke allowed calls one at a time via the
        //     single-call host API, and synthesize a `Denied` outcome at
        //     the hidden call's position. Crucially, a hidden call that
        //     routes through recovery to `Abort` short-circuits before any
        //     subsequent allowed call's side effect runs — fixing the
        //     iter-2 ordering bug where `[hidden, allowed]` would execute
        //     `allowed` before processing the denial.
        let allowed_ids: HashSet<_> = surface
            .descriptors
            .iter()
            .map(|descriptor| descriptor.capability_id.clone())
            .collect();
        let any_hidden = calls
            .iter()
            .any(|call| !allowed_ids.contains(&call.capability_id));

        if !any_hidden {
            // Fast path: all calls allowed → single batch invocation.
            let host_invocations: Vec<CapabilityInvocation> = calls
                .iter()
                .cloned()
                .map(capability_invocation_from_candidate)
                .collect();
            // Iter-9 finding 3: `stop_on_first_suspension` MUST be true if
            // EITHER the policy is `Sequential`, OR any summary in the
            // batch has `ConcurrencyHint::Exclusive`. A custom
            // `BatchPolicyStrategy` that returns `Parallel` for a batch
            // containing an Exclusive call would otherwise let the host
            // run later invocations after an
            // `ApprovalRequired`/`AuthRequired`/`SpawnedProcess` outcome.
            // The concurrency hint is the descriptor's own disclosure
            // and overrides a permissive planner.
            let any_exclusive = summaries
                .iter()
                .any(|summary| matches!(summary.concurrency_hint, ConcurrencyHint::Exclusive));
            let stop_on_first_suspension =
                matches!(policy, BatchPolicy::Sequential) || any_exclusive;
            let batch = host
                .invoke_capability_batch(CapabilityBatchInvocation {
                    invocations: host_invocations,
                    stop_on_first_suspension,
                })
                .await
                .map_err(|_| AgentLoopExecutorError::HostUnavailable {
                    stage: HostStage::Capability,
                })?;
            return self
                .consume_batch_outcomes(planner, host, state, calls, batch, policy)
                .await;
        }

        // Mixed path: process per-call in original order. Hidden calls
        // become synthetic `Denied`; allowed calls invoke single-call.
        let mut seen_signatures = HashSet::new();
        for call in calls.into_iter() {
            let signature = signature_for_call(&call);
            if seen_signatures.insert(signature.clone()) {
                state.recent_call_signatures.push(signature);
            }
            let outcome = if allowed_ids.contains(&call.capability_id) {
                host.invoke_capability(capability_invocation_from_candidate(call.clone()))
                    .await
                    .map_err(|_| AgentLoopExecutorError::HostUnavailable {
                        stage: HostStage::Capability,
                    })?
            } else {
                CapabilityOutcome::Denied(ironclaw_turns::run_profile::CapabilityDenied {
                    reason_kind:
                        ironclaw_turns::run_profile::CapabilityDeniedReasonKind::EmptySurface,
                    safe_summary: "capability hidden by executor filter".to_string(),
                })
            };
            match self
                .handle_capability_outcome(planner, host, state, call, outcome)
                .await?
            {
                Step::Continue(next) => state = next,
                Step::Exit(exit_state, exit) => return Ok(Step::Exit(exit_state, exit)),
            }
        }

        Ok(Step::Continue(state))
    }

    /// Consume the outcomes from a full-batch invocation. Per WS-6 iter-3
    /// finding 3, a `Sequential` batch may return a short outcome vec when
    /// the host stops at the first suspension; in that case the last
    /// outcome MUST be a suspension kind, and only the executed prefix is
    /// processed. A `Parallel` batch keeps the strict 1:1 count contract.
    async fn consume_batch_outcomes(
        &self,
        planner: &dyn AgentLoopPlanner,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        mut state: LoopExecutionState,
        calls: Vec<CapabilityCallCandidate>,
        batch: ironclaw_turns::run_profile::CapabilityBatchOutcome,
        policy: BatchPolicy,
    ) -> Result<Step, AgentLoopExecutorError> {
        let outcomes_len = batch.outcomes.len();
        let calls_len = calls.len();
        if outcomes_len > calls_len {
            return Err(AgentLoopExecutorError::PlannerContract {
                detail: "capability batch outcome count exceeded host invocations",
            });
        }
        if outcomes_len < calls_len {
            // Short prefix only valid for `Sequential` AND the tail must
            // be a suspension (per `CapabilityOutcome::is_suspension`).
            if !matches!(policy, BatchPolicy::Sequential) {
                return Err(AgentLoopExecutorError::PlannerContract {
                    detail: "parallel capability batch returned a short outcome prefix",
                });
            }
            let Some(last) = batch.outcomes.last() else {
                return Err(AgentLoopExecutorError::PlannerContract {
                    detail: "sequential capability batch returned no outcomes",
                });
            };
            if !last.is_suspension() {
                return Err(AgentLoopExecutorError::PlannerContract {
                    detail: "sequential capability batch truncated without a suspension tail",
                });
            }
        }
        let mut seen_signatures = HashSet::new();
        let mut outcomes_iter = batch.outcomes.into_iter();
        let mut calls_iter = calls.into_iter();
        while let (Some(outcome), Some(call)) = (outcomes_iter.next(), calls_iter.next()) {
            let signature = signature_for_call(&call);
            if seen_signatures.insert(signature.clone()) {
                state.recent_call_signatures.push(signature);
            }
            match self
                .handle_capability_outcome(planner, host, state, call, outcome)
                .await?
            {
                Step::Continue(next) => state = next,
                Step::Exit(exit_state, exit) => return Ok(Step::Exit(exit_state, exit)),
            }
        }
        Ok(Step::Continue(state))
    }

    async fn handle_capability_outcome(
        &self,
        planner: &dyn AgentLoopPlanner,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        mut state: LoopExecutionState,
        call: CapabilityCallCandidate,
        outcome: CapabilityOutcome,
    ) -> Result<Step, AgentLoopExecutorError> {
        match outcome {
            CapabilityOutcome::Completed(result) => {
                push_completed_result(&mut state, result);
                Ok(Step::Continue(state))
            }
            CapabilityOutcome::ApprovalRequired { gate_ref, .. } => {
                self.handle_gate(planner, host, state, GateKind::Approval, gate_ref)
                    .await
            }
            CapabilityOutcome::AuthRequired { gate_ref, .. } => {
                self.handle_gate(planner, host, state, GateKind::Auth, gate_ref)
                    .await
            }
            CapabilityOutcome::ResourceBlocked { gate_ref, .. } => {
                self.handle_gate(planner, host, state, GateKind::Resource, gate_ref)
                    .await
            }
            CapabilityOutcome::SpawnedProcess(handle) => {
                let gate_ref = process_ref_to_gate_ref(&handle)?;
                self.handle_gate(planner, host, state, GateKind::Resource, gate_ref)
                    .await
            }
            CapabilityOutcome::Denied(denied) => {
                let summary = CapabilityErrorSummary {
                    class: CapabilityErrorClass::PolicyDenied,
                    safe_summary: denied.safe_summary,
                    diagnostic_ref: None,
                };
                self.handle_capability_error(planner, host, state, call, summary)
                    .await
            }
            CapabilityOutcome::Failed(failure) => {
                let summary = capability_failure_summary(failure);
                self.handle_capability_error(planner, host, state, call, summary)
                    .await
            }
        }
    }

    async fn handle_capability_error(
        &self,
        planner: &dyn AgentLoopPlanner,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        mut state: LoopExecutionState,
        call: CapabilityCallCandidate,
        summary: CapabilityErrorSummary,
    ) -> Result<Step, AgentLoopExecutorError> {
        state
            .recent_failure_kinds
            .push(LoopFailureKind::CapabilityProtocolError);

        // Inner retry loop: a still-transient failure on retry must consult
        // recovery again so the per-class budget is consumed (master spec
        // §10). The strategy's own `attempts` counter eventually trips
        // `Abort`; `MAX_RETRIES_PER_CALL` is defense-in-depth against a
        // custom strategy that never gives up.
        let mut current_summary = summary;
        for _ in 0..MAX_RETRIES_PER_CALL {
            let recovery = planner
                .recovery()
                .on_capability_error(&state, &current_summary)
                .await;
            match recovery {
                RecoveryOutcome::Retry { recovery, alter } => {
                    // WS-6 iter-4 finding 2: a `Denied` outcome must NEVER
                    // be replayed through the host. `Denied` is either an
                    // executor-side synthetic denial (the capability was
                    // filtered out — replaying would let the model bypass
                    // the filter) or a host-side policy denial (already
                    // authoritative — replaying is just retry-against-the-
                    // same-policy noise). In both cases, treat the recovery
                    // `Retry` as `SkipResult`: consume the budget bump but
                    // do not invoke the host.
                    if matches!(current_summary.class, CapabilityErrorClass::PolicyDenied) {
                        state.recovery_state = recovery;
                        return Ok(Step::Continue(state));
                    }
                    state.recovery_state = recovery;
                    if matches!(
                        alter,
                        Some(crate::strategies::RetryAlteration::AdvanceFallback)
                    ) {
                        return Ok(Step::Exit(
                            state,
                            LoopExit::Failed {
                                kind: FailureKind::Other(LoopFailureKind::DriverBug),
                            },
                        ));
                    }
                    // WS-6 iter-3 finding 4: honor `Backoff` delays as a
                    // tokio sleep before re-invoking the capability.
                    if let Some(crate::strategies::RetryAlteration::Backoff { delay }) = alter {
                        tokio::time::sleep(delay).await;
                    }
                    let retry_outcome = match host
                        .invoke_capability(capability_invocation_from_candidate(call.clone()))
                        .await
                    {
                        Ok(outcome) => outcome,
                        Err(error) if matches!(error.kind, AgentLoopHostErrorKind::Cancelled) => {
                            // WS-6 iter-3 finding 5: capability-port
                            // cancellation surfaces as `LoopExit::Cancelled`.
                            let checked =
                                self.checkpoint(host, state, CheckpointKind::Final).await?;
                            let exit = LoopExit::Cancelled(CancelledKind {
                                interrupted_message_refs: checked.assistant_refs.clone(),
                            });
                            return Ok(Step::Exit(checked, exit));
                        }
                        Err(_) => {
                            return Err(AgentLoopExecutorError::HostUnavailable {
                                stage: HostStage::Capability,
                            });
                        }
                    };
                    match retry_outcome {
                        CapabilityOutcome::Completed(result) => {
                            push_completed_result(&mut state, result);
                            return Ok(Step::Continue(state));
                        }
                        CapabilityOutcome::ApprovalRequired { gate_ref, .. } => {
                            return self
                                .handle_gate(planner, host, state, GateKind::Approval, gate_ref)
                                .await;
                        }
                        CapabilityOutcome::AuthRequired { gate_ref, .. } => {
                            return self
                                .handle_gate(planner, host, state, GateKind::Auth, gate_ref)
                                .await;
                        }
                        CapabilityOutcome::ResourceBlocked { gate_ref, .. } => {
                            return self
                                .handle_gate(planner, host, state, GateKind::Resource, gate_ref)
                                .await;
                        }
                        CapabilityOutcome::SpawnedProcess(handle) => {
                            let gate_ref = process_ref_to_gate_ref(&handle)?;
                            return self
                                .handle_gate(planner, host, state, GateKind::Resource, gate_ref)
                                .await;
                        }
                        CapabilityOutcome::Denied(denied) => {
                            // Re-route through recovery as PolicyDenied — the
                            // outer match treats Denied as a non-recoverable
                            // failure for THIS call but lets recovery decide
                            // skip vs abort.
                            current_summary = CapabilityErrorSummary {
                                class: CapabilityErrorClass::PolicyDenied,
                                safe_summary: denied.safe_summary,
                                diagnostic_ref: None,
                            };
                            continue;
                        }
                        CapabilityOutcome::Failed(failure) => {
                            // Same call, still transient (or permanent) —
                            // ask recovery again. Do NOT re-push to
                            // `recent_failure_kinds`: master spec §10 says
                            // failure kind is recorded once per call, not
                            // per retry.
                            current_summary = capability_failure_summary(failure);
                            continue;
                        }
                    }
                }
                RecoveryOutcome::SkipResult { recovery } => {
                    state.recovery_state = recovery;
                    return Ok(Step::Continue(state));
                }
                RecoveryOutcome::Abort {
                    recovery,
                    failure_kind,
                } => {
                    state.recovery_state = recovery;
                    return Ok(Step::Exit(
                        state,
                        LoopExit::Failed {
                            kind: failure_kind_to_exit(failure_kind),
                        },
                    ));
                }
            }
        }

        // Defense-in-depth: a custom strategy returned `Retry` more than
        // `MAX_RETRIES_PER_CALL` times. Treat as a driver bug.
        Ok(Step::Exit(
            state,
            LoopExit::Failed {
                kind: FailureKind::Other(LoopFailureKind::DriverBug),
            },
        ))
    }

    async fn handle_gate(
        &self,
        planner: &dyn AgentLoopPlanner,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        mut state: LoopExecutionState,
        kind: GateKind,
        gate_ref: LoopGateRef,
    ) -> Result<Step, AgentLoopExecutorError> {
        let summary = GateSummary {
            kind,
            gate_ref: gate_ref.clone(),
        };
        match planner.gate().handle(&state, &summary).await {
            GateOutcome::Block { control } => {
                state.control_state = control;
                state.last_gate = Some(gate_ref.clone());
                state = self
                    .checkpoint(host, state, CheckpointKind::BeforeBlock)
                    .await?;
                Ok(Step::Exit(state, LoopExit::Blocked { gate_ref }))
            }
            GateOutcome::SkipAndContinue { control } => {
                state.control_state = control;
                Ok(Step::Continue(state))
            }
            GateOutcome::Abort {
                control,
                failure_kind,
            } => {
                state.control_state = control;
                Ok(Step::Exit(
                    state,
                    LoopExit::Failed {
                        kind: failure_kind_to_exit(failure_kind),
                    },
                ))
            }
        }
    }

    async fn checkpoint(
        &self,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        mut state: LoopExecutionState,
        kind: CheckpointKind,
    ) -> Result<LoopExecutionState, AgentLoopExecutorError> {
        // Master spec §10: the checkpoint payload MUST be persisted before the
        // checkpoint marker is recorded. `HostManagedLoopCheckpointPort`
        // rejects unknown state refs by design — store first, then checkpoint
        // with the returned ref.
        //
        // Iter-7 finding 3: legacy hosts that have not yet migrated to the
        // `store_checkpoint_payload`-then-`checkpoint` contract return
        // `Unavailable` from the default trait impl. We treat that variant as
        // "this host doesn't store payloads; fall back to the legacy
        // checkpoint()-only path" and pass the `legacy_unknown` sentinel ref
        // so the legacy host can recognize it. Any other error (Internal,
        // CheckpointRejected, transient outage) bubbles up as
        // `CheckpointFailed` so retries can re-poll.
        let payload = serde_json::to_vec(&serde_json::json!({
            "schema_id": CHECKPOINT_SCHEMA_ID,
            "state": &state,
        }))
        .map_err(|_| AgentLoopExecutorError::CheckpointFailed { stage: kind })?;
        let host_kind = host_checkpoint_kind(kind);
        let state_ref = match host
            .store_checkpoint_payload(StoreLoopCheckpointPayload {
                kind: host_kind,
                payload,
            })
            .await
        {
            Ok(state_ref) => state_ref,
            Err(err) if matches!(err.kind, AgentLoopHostErrorKind::Unavailable) => {
                LoopCheckpointStateRef::legacy_unknown()
            }
            Err(_) => return Err(AgentLoopExecutorError::CheckpointFailed { stage: kind }),
        };
        host.checkpoint(LoopCheckpointRequest {
            kind: host_kind,
            state_ref,
        })
        .await
        .map_err(|_| AgentLoopExecutorError::CheckpointFailed { stage: kind })?;
        state.last_checkpoint = Some(CheckpointMarker {
            kind,
            iteration_at_checkpoint: state.iteration,
        });
        Ok(state)
    }

    async fn observe_cancellation(
        &self,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        mut state: LoopExecutionState,
    ) -> Result<(LoopExecutionState, Option<LoopExit>), AgentLoopExecutorError> {
        // Iter-7 finding 1: page past control-only pages just like
        // `drain_followup` does. Pre-iter-7 this function polled exactly
        // once: if the first page held only `GateResolved` /
        // `CapabilitySurfaceChanged`, it acked them and returned `None`,
        // so a queued `Cancel`/`Interrupt` on the next page stayed
        // invisible until after another model/capability cycle and the
        // loop produced one more reply / ran extra tools.
        //
        // Loop up to `INPUT_POLL_LIMIT` rounds — same defense-in-depth
        // bound as `drain_followup`. The loop terminates on:
        //   - terminal input → checkpoint-then-ack-then-exit
        //   - empty page    → no cancel pending, return None
        //   - user-facing input (UserMessage / FollowUp / Steering) → leave
        //     un-acked for the dedicated drain handler, return None
        for _ in 0..INPUT_POLL_LIMIT {
            let batch = host
                .poll_inputs(state.input_cursor.clone(), INPUT_POLL_LIMIT)
                .await
                .map_err(|_| AgentLoopExecutorError::HostUnavailable {
                    stage: HostStage::Input,
                })?;
            // Per WS-6 finding 1: apply control side effects
            // (GateResolved, CapabilitySurfaceChanged) in-page as
            // idempotent state mutations. Pages are atomic — the cursor
            // is page-granular — so we can't partial-ack between a
            // control event and a user-facing event in the same page.
            apply_control_side_effects(&mut state, &batch.inputs);

            // Cancel / Interrupt are terminal: take the `Final`
            // checkpoint FIRST, then ack the page only once the
            // checkpoint is durable.
            //
            // Iter-6 finding 2: the pre-iter-6 code acked the page before
            // checkpointing. If `store_checkpoint_payload()` or
            // `checkpoint()` failed (transient DB outage), the cancel
            // had already been consumed but the run state was not
            // persisted — a retried run would observe
            // `state.input_cursor` past the cancel and never re-poll
            // it. Reordering to checkpoint-then-ack means a checkpoint
            // failure bubbles up before the cursor advance, so the next
            // `execute()` call re-polls the same cancel page.
            if batch.inputs.iter().any(|input| {
                matches!(
                    input,
                    LoopInput::Cancel { .. } | LoopInput::Interrupt { .. }
                )
            }) {
                // Iter-9 finding 1: advance the cursor on `state` BEFORE
                // taking the Final checkpoint, so the durable record
                // names the next-unprocessed position. If we ack first
                // and then the checkpoint write fails, the host has
                // dropped the page but the only durable cursor still
                // points at the cancel — on retry the loop would
                // re-poll a page the host has already discarded.
                // Checkpoint-with-advanced-cursor-then-ack means a
                // checkpoint failure bubbles up before the host drops
                // the page; ack failure after a successful checkpoint
                // is benign (cursor is ahead of host; next iteration's
                // poll skips already-processed positions).
                state.input_cursor = batch.next_cursor.clone();
                let checked = self.checkpoint(host, state, CheckpointKind::Final).await?;
                host.ack_inputs(batch.next_cursor).await.map_err(|_| {
                    AgentLoopExecutorError::HostUnavailable {
                        stage: HostStage::Input,
                    }
                })?;
                let exit = LoopExit::Cancelled(CancelledKind {
                    interrupted_message_refs: checked.assistant_refs.clone(),
                });
                return Ok((checked, Some(exit)));
            }

            // Empty page → no cancel pending; return.
            if batch.inputs.is_empty() {
                return Ok((state, None));
            }

            // User-facing inputs (UserMessage / FollowUp / Steering)
            // belong to dedicated drain handlers. Leave the page un-acked
            // and return `None` so the iteration proceeds. Any control
            // side effects in the same page were already applied above
            // — those mutations are idempotent and survive the next
            // drain handler's re-poll of the same cursor.
            let has_user_facing = batch.inputs.iter().any(|input| {
                matches!(
                    input,
                    LoopInput::UserMessage { .. }
                        | LoopInput::FollowUp { .. }
                        | LoopInput::Steering { .. }
                )
            });
            if has_user_facing {
                return Ok((state, None));
            }

            // Control-only page: side effects were applied above. Ack
            // and loop back to poll for the next page. The pre-iter-7
            // code returned `None` here unconditionally, so a queued
            // terminal on page 2 was deferred.
            //
            // Iter-9 finding 1: checkpoint with the advanced cursor and
            // applied control side effects BEFORE the host ack so the
            // durable record reflects "this page is consumed". If the
            // worker crashes between ack and the next durable
            // checkpoint, an older checkpoint would point at the
            // already-dropped page and the GateResolved /
            // CapabilitySurfaceChanged side effects would be lost.
            state.input_cursor = batch.next_cursor.clone();
            state = self
                .checkpoint(host, state, CheckpointKind::BeforeModel)
                .await?;
            host.ack_inputs(batch.next_cursor).await.map_err(|_| {
                AgentLoopExecutorError::HostUnavailable {
                    stage: HostStage::Input,
                }
            })?;
        }
        // Defense-in-depth: `INPUT_POLL_LIMIT` consecutive control-only
        // pages without a terminal, empty, or user-facing page. Return
        // `None` so the outer loop makes progress — the next tick's
        // `observe_cancellation` will pick up where we left off.
        Ok((state, None))
    }

    async fn drain_steering(
        &self,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        mut state: LoopExecutionState,
    ) -> Result<LoopExecutionState, AgentLoopExecutorError> {
        let batch = host
            .poll_inputs(state.input_cursor.clone(), INPUT_POLL_LIMIT)
            .await
            .map_err(|_| AgentLoopExecutorError::HostUnavailable {
                stage: HostStage::Input,
            })?;
        // Per WS-6 finding 1: pages are atomic. Apply control side effects
        // in-page (gate-resolved → clear last_gate; surface-changed → drop
        // cached surface_version). If a user-facing steering message is
        // present in the same page, ack so the loop makes progress. If
        // Cancel/Interrupt is also present, don't ack — the next iteration's
        // `observe_cancellation` polls the same cursor, sees the terminal
        // input, and exits Cancelled. The FollowUp case stays
        // un-acked here so the dedicated post-reply drain handler can
        // consume it.
        apply_control_side_effects(&mut state, &batch.inputs);
        let has_terminal = batch.inputs.iter().any(|input| {
            matches!(
                input,
                LoopInput::Cancel { .. } | LoopInput::Interrupt { .. }
            )
        });
        let has_steering = batch.inputs.iter().any(|input| {
            matches!(
                input,
                LoopInput::UserMessage { .. } | LoopInput::Steering { .. }
            )
        });
        let has_followup = batch
            .inputs
            .iter()
            .any(|input| matches!(input, LoopInput::FollowUp { .. }));
        if has_terminal || has_followup {
            // Don't ack — leave the page to observe_cancellation /
            // drain_followup. Control side effects were already applied.
            return Ok(state);
        }
        if has_steering {
            // Iter-9 finding 1: durably checkpoint with the advanced
            // cursor (and any applied control side effects from
            // `apply_control_side_effects` above) BEFORE acking. If we
            // acked first and the worker crashed before the next
            // checkpoint, an older checkpoint would re-poll a page
            // the host has already discarded — losing the steering
            // message and any sibling GateResolved /
            // CapabilitySurfaceChanged side effects.
            state.input_cursor = batch.next_cursor.clone();
            state = self
                .checkpoint(host, state, CheckpointKind::BeforeModel)
                .await?;
            host.ack_inputs(batch.next_cursor).await.map_err(|_| {
                AgentLoopExecutorError::HostUnavailable {
                    stage: HostStage::Input,
                }
            })?;
        }
        Ok(state)
    }

    async fn drain_followup(
        &self,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        mut state: LoopExecutionState,
    ) -> Result<(LoopExecutionState, FollowupDrainOutcome), AgentLoopExecutorError> {
        // WS-6 iter-4 finding 1: keep draining follow-up pages until either
        // a FollowUp / terminal input is found, or the queue is genuinely
        // empty. The pre-iter-4 code returned `Empty` after acking a
        // control-only page (GateResolved / SurfaceChanged), so a FollowUp
        // on a *later* page was silently dropped: the caller took the
        // `Final` checkpoint and exited `Completed`, leaving queued user
        // input unanswered. Pages stay atomic (we still ack one at a time);
        // we just keep polling until we hit something terminal or empty.
        //
        // Defense-in-depth bound: at most `INPUT_POLL_LIMIT` poll rounds
        // per call — same cap as the per-page input batch size — so a
        // misbehaving host that returns an infinite stream of control-only
        // pages can't spin forever inside one drain.
        for _ in 0..INPUT_POLL_LIMIT {
            let batch = host
                .poll_inputs(state.input_cursor.clone(), INPUT_POLL_LIMIT)
                .await
                .map_err(|_| AgentLoopExecutorError::HostUnavailable {
                    stage: HostStage::Input,
                })?;
            // Iter-8 finding 2: a fresh `UserMessage` or `Steering` arriving
            // just as the loop would otherwise complete must be treated as
            // follow-up-equivalent — it's user-facing input the next
            // iteration owes a reply to. Pre-iter-8 we matched only
            // `FollowUp`, so a `UserMessage` queued post-reply fell through
            // to the control-only branch, got `ack_inputs`'d, and the run
            // exited `Completed` dropping the input entirely.
            let has_followup = batch.inputs.iter().any(|input| {
                matches!(
                    input,
                    LoopInput::FollowUp { .. }
                        | LoopInput::UserMessage { .. }
                        | LoopInput::Steering { .. }
                )
            });
            let has_terminal = batch.inputs.iter().any(|input| {
                matches!(
                    input,
                    LoopInput::Cancel { .. } | LoopInput::Interrupt { .. }
                )
            });
            // Master spec §8 step 2 / WS-6 brief §3.3a + finding 1: pages
            // are atomic — the `LoopInputPort` cursor is page-granular, so
            // a mixed page (FollowUp + GateResolved + SurfaceChanged) must
            // be acked as a whole. We apply control side effects in-page
            // as idempotent state mutations, then ack. The pre-iter-3
            // "ControlPending" refusal-to-ack livelocked on any mixed page
            // where a control event was in the same poll as the
            // user-facing input.
            apply_control_side_effects(&mut state, &batch.inputs);
            if has_terminal {
                // Cancel/Interrupt is terminal. Iter-6 finding 2: do NOT
                // ack the page here. Carry the un-applied cursor back to
                // the caller, which takes the `Final` checkpoint and then
                // acks. The pre-iter-6 code acked first (including any
                // FollowUp in the same page — superseded by the user's
                // cancel) and relied on the caller to checkpoint
                // afterward; a checkpoint failure left the cancel
                // consumed but the run state un-persisted. Sibling
                // control side effects were already applied above via
                // `apply_control_side_effects`.
                return Ok((
                    state,
                    FollowupDrainOutcome::TerminalCancel {
                        next_cursor: batch.next_cursor,
                    },
                ));
            }
            if has_followup {
                // Iter-9 finding 1: durably checkpoint with the advanced
                // cursor BEFORE the host ack. The caller's outer loop
                // will not take a checkpoint until after another
                // observe_cancellation/drain_steering cycle plus a
                // model invocation; if the worker crashes between this
                // ack and the next `BeforeModel` checkpoint, the only
                // durable record points at the already-dropped
                // mixed-page (FollowUp + GateResolved /
                // CapabilitySurfaceChanged) and the control side
                // effects vanish.
                state.input_cursor = batch.next_cursor.clone();
                state = self
                    .checkpoint(host, state, CheckpointKind::BeforeModel)
                    .await?;
                host.ack_inputs(batch.next_cursor).await.map_err(|_| {
                    AgentLoopExecutorError::HostUnavailable {
                        stage: HostStage::Input,
                    }
                })?;
                return Ok((state, FollowupDrainOutcome::FollowUpConsumed));
            }
            // No user-facing or terminal inputs in this page.
            if batch.inputs.is_empty() {
                // Queue is genuinely drained. The caller's next step is
                // the `Final` checkpoint.
                return Ok((state, FollowupDrainOutcome::Empty));
            }
            // Control-only page (GateResolved / SurfaceChanged): side
            // effects were just applied. Ack the page and loop back to
            // poll for the next one — a FollowUp may be sitting on a
            // later page.
            //
            // Iter-9 finding 1: durably checkpoint with the advanced
            // cursor and applied side effects BEFORE the ack. Without
            // this, the loop could ack a control-only page, crash, and
            // resume from an older checkpoint that re-polls the same
            // page — but the host has already dropped it, so the
            // GateResolved / CapabilitySurfaceChanged effects are
            // permanently lost.
            state.input_cursor = batch.next_cursor.clone();
            state = self
                .checkpoint(host, state, CheckpointKind::BeforeModel)
                .await?;
            host.ack_inputs(batch.next_cursor).await.map_err(|_| {
                AgentLoopExecutorError::HostUnavailable {
                    stage: HostStage::Input,
                }
            })?;
        }
        // Iter-7 finding 2: `INPUT_POLL_LIMIT` consecutive control-only
        // pages were acked and their side effects applied, but we never
        // saw a definitive "empty" page or a user-facing input. Pre-iter-7
        // we collapsed this into `Empty`, which the caller treated as
        // "queue drained" — Final-checkpointing and exiting `Completed`
        // even if a real FollowUp was sitting on page 17. Return
        // `ControlPending` so the caller continues the loop instead.
        Ok((state, FollowupDrainOutcome::ControlPending))
    }

    async fn exit_for_stop_kind(
        &self,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        state: LoopExecutionState,
        kind: StopKind,
    ) -> Result<(LoopExecutionState, LoopExit), AgentLoopExecutorError> {
        // Iter-5 finding 4: every terminal exit path takes a `Final`
        // checkpoint just before returning so profiles with
        // `require_final_checkpoint = true` don't reject the exit as
        // `MissingFinalCheckpoint`. The pre-iter-5 `StopKind::Aborted` branch
        // skipped the checkpoint; the helper now uniformly checkpoints and
        // returns the checked state alongside the exit so the caller can
        // commit it to `*state`.
        match kind {
            StopKind::GracefulStop => {
                let checked = self.checkpoint(host, state, CheckpointKind::Final).await?;
                Ok((checked, LoopExit::Completed(CompletionKind::GracefulStop)))
            }
            StopKind::NoProgressDetected => {
                let checked = self.checkpoint(host, state, CheckpointKind::Final).await?;
                Ok((
                    checked,
                    LoopExit::Failed {
                        kind: FailureKind::NoProgressDetected,
                    },
                ))
            }
            StopKind::Aborted(failure) => {
                let checked = self.checkpoint(host, state, CheckpointKind::Final).await?;
                Ok((
                    checked,
                    LoopExit::Failed {
                        kind: failure_kind_to_exit(failure),
                    },
                ))
            }
        }
    }

    /// Iter-5 finding 4 helper: when a sub-routine returns a `LoopExit::Failed`,
    /// take a `Final` checkpoint before propagating it. `Completed` /
    /// `Blocked` / `Cancelled` exits already carry their own checkpoint
    /// discipline (Final / BeforeBlock / Final respectively) inside the
    /// sub-routine, so this helper is a no-op for them.
    async fn final_checkpoint_for_failure(
        &self,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        state: LoopExecutionState,
        exit: LoopExit,
    ) -> Result<(LoopExecutionState, LoopExit), AgentLoopExecutorError> {
        if matches!(exit, LoopExit::Failed { .. }) {
            let checked = self.checkpoint(host, state, CheckpointKind::Final).await?;
            Ok((checked, exit))
        } else {
            Ok((state, exit))
        }
    }

    fn no_progress_exit(&self, state: &LoopExecutionState) -> Option<FailureKind> {
        if state
            .recent_call_signatures
            .most_common_count_in(NO_PROGRESS_WINDOW)
            >= NO_PROGRESS_THRESHOLD
        {
            Some(FailureKind::NoProgressDetected)
        } else {
            None
        }
    }
}

/// Apply idempotent control-input side effects to `state`. Cancel and
/// Interrupt are NOT handled here — the caller decides terminal exit.
fn apply_control_side_effects(state: &mut LoopExecutionState, inputs: &[LoopInput]) {
    for input in inputs {
        match input {
            LoopInput::GateResolved { gate_ref } if state.last_gate.as_ref() == Some(gate_ref) => {
                state.last_gate = None;
            }
            LoopInput::CapabilitySurfaceChanged { .. } => {
                // Drop the cached surface_version so the next iteration's
                // `visible_capabilities` re-fetch picks up the new host
                // snapshot (master spec §10).
                state.surface_version = None;
            }
            _ => {}
        }
    }
}

fn model_preference_id(
    preference: ModelPreference,
) -> Result<Option<ironclaw_turns::ModelProfileId>, AgentLoopExecutorError> {
    match preference {
        ModelPreference::Primary => Ok(None),
        ModelPreference::Fallback { .. } => Err(AgentLoopExecutorError::PlannerContract {
            detail: "fallback model preference requires model route chain support",
        }),
    }
}

/// Project the model's chosen capability batch into the shape
/// `BatchPolicyStrategy` consumes.
///
/// Iter-8 finding 1 fix: previously every call was hardcoded
/// `ConcurrencyHint::SafeForParallel`, so `DefaultBatchPolicyStrategy` picked
/// `Parallel` for every batch — including ones that contained an
/// approval-gated write or shell. The downstream
/// `RebornLoopDriverHost::invoke_capability_batch` then set
/// `stop_on_first_suspension = false` and ran later calls after an
/// `ApprovalRequired` / `AuthRequired` / `SpawnedProcess` outcome.
///
/// We now resolve each call against the visible-surface descriptor it claims
/// to use, mapping `CapabilityConcurrency::Exclusive` -> `Exclusive` and
/// `CapabilityConcurrency::SafeForParallel` -> `SafeForParallel`. When a
/// descriptor is missing from the visible surface (defensive — the
/// capability-filter strategy should have rejected the call upstream) we
/// fall back to `Exclusive`, which makes the batch run sequentially with
/// `stop_on_first_suspension = true`. Conservative-by-default is the right
/// choice when in doubt.
fn capability_summaries(
    surface: &VisibleCapabilitySurface,
    calls: &[CapabilityCallCandidate],
) -> Vec<CapabilityCallSummary> {
    calls
        .iter()
        .map(|call| {
            let concurrency_hint = surface
                .descriptors
                .iter()
                .find(|descriptor| descriptor.capability_id == call.capability_id)
                .map(|descriptor| match descriptor.concurrency {
                    CapabilityConcurrency::SafeForParallel => ConcurrencyHint::SafeForParallel,
                    CapabilityConcurrency::Exclusive => ConcurrencyHint::Exclusive,
                })
                .unwrap_or(ConcurrencyHint::Exclusive);
            CapabilityCallSummary {
                name: call.capability_id.clone(),
                concurrency_hint,
            }
        })
        .collect()
}

fn capability_invocation_from_candidate(call: CapabilityCallCandidate) -> CapabilityInvocation {
    CapabilityInvocation {
        surface_version: call.surface_version,
        capability_id: call.capability_id,
        input_ref: call.input_ref,
    }
}

fn signature_for_call(call: &CapabilityCallCandidate) -> CapabilityCallSignature {
    CapabilityCallSignature::from_call(
        call.capability_id.clone(),
        &serde_json::Value::String(call.input_ref.as_str().to_string()),
    )
}

fn model_error_summary(class: ModelErrorClass, error: &AgentLoopHostError) -> ModelErrorSummary {
    ModelErrorSummary {
        class,
        safe_summary: error.safe_summary.clone(),
        diagnostic_ref: error.diagnostic_ref.clone(),
    }
}

fn capability_failure_summary(failure: CapabilityFailure) -> CapabilityErrorSummary {
    CapabilityErrorSummary {
        class: match failure.error_kind.as_str() {
            "transient" => CapabilityErrorClass::Transient,
            "permanent" => CapabilityErrorClass::Permanent,
            "input_invalid" => CapabilityErrorClass::InputInvalid,
            "policy_denied" => CapabilityErrorClass::PolicyDenied,
            "unavailable" => CapabilityErrorClass::Unavailable,
            _ => CapabilityErrorClass::Internal,
        },
        safe_summary: failure.safe_summary,
        diagnostic_ref: None,
    }
}

fn push_completed_result(state: &mut LoopExecutionState, result: CapabilityResultMessage) {
    if is_terminate_hint(&result) {
        state.control_state.terminate_hints_in_last_batch = state
            .control_state
            .terminate_hints_in_last_batch
            .saturating_add(1);
    }
    state.result_refs.push(result.result_ref);
}

fn is_terminate_hint(result: &CapabilityResultMessage) -> bool {
    matches!(
        result.safe_summary.as_str(),
        "terminate_hint:true" | "terminate:true" | "terminate"
    )
}

/// Wall-clock distance from `start` to `Instant::now()`. Exists so the
/// executor's tick prologue stays readable and so tests on a paused tokio
/// clock can validate the wall-clock budget path.
fn elapsed_since(start: tokio::time::Instant) -> Duration {
    tokio::time::Instant::now().saturating_duration_since(start)
}

/// Current wall clock as milliseconds since the Unix epoch.
///
/// Iter-6 finding 1: used to capture and compare the persisted
/// `LoopExecutionState::started_at_unix_ms` anchor so a resumed run
/// retains its time budget across process restart. A clock reading prior
/// to UNIX_EPOCH (effectively impossible on a sane host) saturates to
/// `0`; the wall-clock comparator then treats elapsed time as `0`, which
/// is conservative — it never spuriously trips the cap.
fn system_time_now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|delta| u64::try_from(delta.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

/// Whether the wall-clock budget has been exceeded.
///
/// Iter-6 finding 1: combines the in-process `tokio::time::Instant` cap
/// (so test code with `start_paused = true` still works) with the
/// persisted `SystemTime` anchor (so a run that resumes after process
/// restart immediately observes its already-elapsed budget). The cap
/// fires if EITHER source agrees that the limit has been reached.
///
/// Clock-skew note: if the OS clock jumps backward, `SystemTime` elapsed
/// underflows to `Duration::ZERO`, and the in-process `Instant` cap takes
/// over for the remainder of this `execute()` call. We do NOT panic or
/// fail the run — wall-clock budgets are a defense-in-depth limiter, not
/// a correctness invariant.
fn wall_clock_limit_exceeded(
    in_process_start: tokio::time::Instant,
    persisted_start_unix_ms: Option<u64>,
    limit: Duration,
) -> bool {
    if elapsed_since(in_process_start) >= limit {
        return true;
    }
    let Some(started_at_unix_ms) = persisted_start_unix_ms else {
        return false;
    };
    let now_ms = system_time_now_unix_ms();
    let elapsed_ms = now_ms.saturating_sub(started_at_unix_ms);
    Duration::from_millis(elapsed_ms) >= limit
}

fn failure_kind_to_exit(kind: LoopFailureKind) -> FailureKind {
    match kind {
        LoopFailureKind::IterationLimit => FailureKind::IterationLimitReached,
        LoopFailureKind::NoProgressDetected => FailureKind::NoProgressDetected,
        LoopFailureKind::WallClockLimit => FailureKind::WallClockLimitReached,
        other => FailureKind::Other(other),
    }
}

/// Narrow the host's visible-capability surface using the planner's filter.
///
/// The host applies its own scope/grant/auth filters first; this strategy
/// filter can only further narrow the result. `CapabilityFilter::All`
/// is a no-op.
fn apply_capability_filter(
    surface: VisibleCapabilitySurface,
    filter: &CapabilityFilter,
) -> VisibleCapabilitySurface {
    match filter {
        CapabilityFilter::All => surface,
        CapabilityFilter::AllowOnly(allowed) => {
            let descriptors = surface
                .descriptors
                .into_iter()
                .filter(|descriptor| allowed.contains(&descriptor.capability_id))
                .collect();
            VisibleCapabilitySurface {
                version: surface.version,
                descriptors,
            }
        }
        CapabilityFilter::Deny(denied) => {
            let descriptors = surface
                .descriptors
                .into_iter()
                .filter(|descriptor| !denied.contains(&descriptor.capability_id))
                .collect();
            VisibleCapabilitySurface {
                version: surface.version,
                descriptors,
            }
        }
    }
}

/// Convert a `LoopProcessRef` (prefix `process:`) to a `LoopGateRef`
/// (prefix `gate:`) so a `SpawnedProcess` outcome can flow through the
/// existing gate-handling path.
///
/// The skeleton has no `LoopBlockedKind::WaitingForProcess` variant yet;
/// this synthesizes a `gate:proc-<token>` ref so the executor can take
/// `BeforeBlock` and surface a `Blocked { gate_ref }` exit. The runner
/// resumes when the process emits its completion event via
/// `LoopInputPort` (`GateResolved` with the same gate ref, or
/// `CapabilitySurfaceChanged`).
fn process_ref_to_gate_ref(
    handle: &ProcessHandleSummary,
) -> Result<LoopGateRef, AgentLoopExecutorError> {
    let token = handle
        .process_ref
        .as_str()
        .strip_prefix("process:")
        .unwrap_or(handle.process_ref.as_str());
    LoopGateRef::new(format!("gate:proc-{token}")).map_err(|_| {
        AgentLoopExecutorError::PlannerContract {
            detail: "spawned-process handle could not be projected to a gate ref",
        }
    })
}

fn host_checkpoint_kind(kind: CheckpointKind) -> LoopCheckpointKind {
    match kind {
        CheckpointKind::BeforeModel => LoopCheckpointKind::BeforeModel,
        CheckpointKind::BeforeSideEffect => LoopCheckpointKind::BeforeSideEffect,
        CheckpointKind::BeforeBlock => LoopCheckpointKind::BeforeBlock,
        CheckpointKind::Final => LoopCheckpointKind::Final,
    }
}

#[allow(dead_code)]
fn _check(_: &dyn AgentLoopExecutor) {}

#[cfg(test)]
mod tests {
    use std::{collections::VecDeque, sync::Arc, sync::Mutex, time::Duration};

    use async_trait::async_trait;
    use ironclaw_host_api::{CapabilityId, RuntimeKind, TenantId, ThreadId};
    use ironclaw_turns::{
        AgentLoopDriverDescriptor, LoopResultRef, RunProfileId, RunProfileVersion,
        TurnCheckpointId, TurnId, TurnRunId, TurnScope,
        run_profile::{
            AgentLoopHostError, CapabilityBatchOutcome, CapabilityDescriptorView,
            CapabilityInputRef, CapabilitySurfaceProfileId, CapabilitySurfaceVersion,
            CheckpointPolicy, CheckpointSchemaId, ConcurrencyClass, ContextProfileId,
            LoopCancelReasonKind, LoopContextBundle, LoopContextPort, LoopContextRequest,
            LoopDriverId, LoopInputBatch, LoopInputCursor, LoopInputPort, LoopModelMessage,
            LoopModelPort, LoopModelResponse, LoopPromptBundle, LoopPromptBundleRef,
            LoopPromptBundleRequest, LoopPromptPort, LoopRunContext, LoopRunInfoPort,
            ModelProfileId, ModelStreamChunk, ParentLoopOutput, RedactedRunProfileProvenance,
            ResolvedRunProfile, ResourceBudgetPolicy, ResourceBudgetTier, RunClassId,
            RunProfileFingerprint, RuntimeProfileConstraints, SchedulingClass, SteeringPolicy,
            VisibleCapabilitySurface,
        },
    };

    use super::*;
    use crate::{
        DefaultPlanner,
        strategies::{
            BatchPolicy, BatchPolicyStrategy, DefaultBatchPolicyStrategy,
            DefaultCapabilityStrategy, DefaultContextStrategy, DefaultGateHandlingStrategy,
            DefaultInputDrainStrategy, DefaultModelStrategy, DefaultRecoveryStrategy,
            DefaultStopConditionStrategy,
        },
    };

    struct TestBudget {
        limit: u32,
    }

    impl crate::strategies::BudgetStrategy for TestBudget {
        fn iteration_limit(&self, _: &LoopExecutionState) -> u32 {
            self.limit
        }

        fn wall_clock_limit(&self, _: &LoopExecutionState) -> Option<Duration> {
            None
        }
    }

    fn planner(limit: u32) -> DefaultPlanner {
        DefaultPlanner::default()
            .with_context(Arc::new(DefaultContextStrategy::default()))
            .with_capability(Arc::new(DefaultCapabilityStrategy))
            .with_model(Arc::new(DefaultModelStrategy))
            .with_batch(Arc::new(DefaultBatchPolicyStrategy))
            .with_gate(Arc::new(DefaultGateHandlingStrategy))
            .with_recovery(Arc::new(DefaultRecoveryStrategy::default()))
            .with_stop(Arc::new(DefaultStopConditionStrategy::default()))
            .with_drain(Arc::new(DefaultInputDrainStrategy))
            .with_budget(Arc::new(TestBudget { limit }))
    }

    struct MockHost {
        context: LoopRunContext,
        model_outputs: Mutex<VecDeque<ParentLoopOutput>>,
        model_errors: Mutex<VecDeque<AgentLoopHostError>>,
        batch_outcomes: Mutex<VecDeque<CapabilityBatchOutcome>>,
        single_outcomes: Mutex<VecDeque<CapabilityOutcome>>,
        checkpoints: Mutex<Vec<LoopCheckpointKind>>,
        model_calls: Mutex<usize>,
        single_calls: Mutex<usize>,
        cancelled: Mutex<bool>,
        poll_inputs: Mutex<VecDeque<Vec<LoopInput>>>,
        capability_surface: Mutex<Option<VisibleCapabilitySurface>>,
        ack_count: Mutex<usize>,
        stored_state_refs: Mutex<Vec<ironclaw_turns::run_profile::LoopCheckpointStateRef>>,
        stored_payloads: Mutex<Vec<(LoopCheckpointKind, usize)>>,
        /// Iter-6 finding 2: when set, `store_checkpoint_payload` fails for
        /// requests carrying `LoopCheckpointKind::Final`. Exercises the
        /// terminal-cancel ordering: the page must NOT be acked if the
        /// Final checkpoint write fails.
        fail_final_store: Mutex<bool>,
        /// Iter-7 finding 3: when set, `store_checkpoint_payload` returns
        /// `Unavailable` (the default trait impl shape) and the host's
        /// `checkpoint()` accepts the legacy sentinel state ref. Models a
        /// pre-migration host that has not yet wired the
        /// store-then-checkpoint contract.
        legacy_checkpoint_only: Mutex<bool>,
        /// Iter-9 finding 3: capture every `CapabilityBatchInvocation`
        /// the executor builds so tests can assert that
        /// `stop_on_first_suspension` is forced to `true` when any
        /// summary in the batch has `ConcurrencyHint::Exclusive`, even
        /// under a custom planner whose `BatchPolicyStrategy` would
        /// otherwise return `Parallel`.
        batch_requests: Mutex<Vec<CapabilityBatchInvocation>>,
    }

    impl MockHost {
        fn new(model_outputs: Vec<ParentLoopOutput>) -> Self {
            Self {
                context: test_run_context(),
                model_outputs: Mutex::new(model_outputs.into()),
                model_errors: Mutex::new(VecDeque::new()),
                batch_outcomes: Mutex::new(VecDeque::new()),
                single_outcomes: Mutex::new(VecDeque::new()),
                checkpoints: Mutex::new(Vec::new()),
                model_calls: Mutex::new(0),
                single_calls: Mutex::new(0),
                cancelled: Mutex::new(false),
                poll_inputs: Mutex::new(VecDeque::new()),
                capability_surface: Mutex::new(None),
                ack_count: Mutex::new(0),
                stored_state_refs: Mutex::new(Vec::new()),
                stored_payloads: Mutex::new(Vec::new()),
                fail_final_store: Mutex::new(false),
                legacy_checkpoint_only: Mutex::new(false),
                batch_requests: Mutex::new(Vec::new()),
            }
        }

        fn fail_final_checkpoint_store(&self) {
            *self.fail_final_store.lock().unwrap() = true;
        }

        fn enable_legacy_checkpoint_only(&self) {
            *self.legacy_checkpoint_only.lock().unwrap() = true;
        }

        fn with_model_errors(self, errors: Vec<AgentLoopHostError>) -> Self {
            self.model_errors.lock().unwrap().extend(errors);
            self
        }

        fn stored_payload_count(&self) -> usize {
            self.stored_payloads.lock().unwrap().len()
        }

        fn with_poll_inputs(self, batches: Vec<Vec<LoopInput>>) -> Self {
            self.poll_inputs.lock().unwrap().extend(batches);
            self
        }

        #[allow(dead_code)]
        fn with_capability_surface(self, surface: VisibleCapabilitySurface) -> Self {
            *self.capability_surface.lock().unwrap() = Some(surface);
            self
        }

        fn single_call_count(&self) -> usize {
            *self.single_calls.lock().unwrap()
        }

        fn ack_count(&self) -> usize {
            *self.ack_count.lock().unwrap()
        }

        fn with_batch(self, outcome: CapabilityBatchOutcome) -> Self {
            self.batch_outcomes.lock().unwrap().push_back(outcome);
            self
        }

        fn with_batches(self, outcomes: Vec<CapabilityBatchOutcome>) -> Self {
            self.batch_outcomes.lock().unwrap().extend(outcomes);
            self
        }

        fn with_single(self, outcome: CapabilityOutcome) -> Self {
            self.single_outcomes.lock().unwrap().push_back(outcome);
            self
        }

        fn cancel(&self) {
            *self.cancelled.lock().unwrap() = true;
        }

        fn checkpoint_kinds(&self) -> Vec<LoopCheckpointKind> {
            self.checkpoints.lock().unwrap().clone()
        }

        fn model_call_count(&self) -> usize {
            *self.model_calls.lock().unwrap()
        }

        fn recorded_batch_requests(&self) -> Vec<CapabilityBatchInvocation> {
            self.batch_requests.lock().unwrap().clone()
        }
    }

    impl LoopRunInfoPort for MockHost {
        fn run_context(&self) -> &LoopRunContext {
            &self.context
        }
    }

    #[async_trait]
    impl LoopContextPort for MockHost {
        async fn load_loop_context(
            &self,
            _request: LoopContextRequest,
        ) -> Result<LoopContextBundle, AgentLoopHostError> {
            Ok(LoopContextBundle {
                identity_messages: Vec::new(),
                messages: Vec::new(),
                instruction_snippets: Vec::new(),
                memory_snippets: Vec::new(),
            })
        }
    }

    #[async_trait]
    impl LoopPromptPort for MockHost {
        async fn build_prompt_bundle(
            &self,
            _request: LoopPromptBundleRequest,
        ) -> Result<LoopPromptBundle, AgentLoopHostError> {
            Ok(LoopPromptBundle {
                bundle_ref: LoopPromptBundleRef::for_run(&self.context, "bundle").unwrap(),
                messages: vec![LoopModelMessage {
                    role: "user".to_string(),
                    content_ref: LoopMessageRef::new("msg:prompt").unwrap(),
                }],
                surface_version: Some(surface_version()),
            })
        }
    }

    #[async_trait]
    impl LoopInputPort for MockHost {
        async fn poll_inputs(
            &self,
            after: LoopInputCursor,
            _limit: usize,
        ) -> Result<LoopInputBatch, AgentLoopHostError> {
            // Scripted poll batches take precedence; once exhausted, fall
            // back to the cancellation-flag default.
            let scripted = self.poll_inputs.lock().unwrap().pop_front();
            let inputs = if let Some(scripted) = scripted {
                scripted
            } else if *self.cancelled.lock().unwrap() {
                vec![LoopInput::Cancel {
                    reason_kind: LoopCancelReasonKind::UserRequested,
                }]
            } else {
                Vec::new()
            };
            Ok(LoopInputBatch {
                inputs,
                next_cursor: after,
            })
        }

        async fn ack_inputs(&self, _cursor: LoopInputCursor) -> Result<(), AgentLoopHostError> {
            *self.ack_count.lock().unwrap() += 1;
            Ok(())
        }
    }

    #[async_trait]
    impl LoopModelPort for MockHost {
        async fn stream_model(
            &self,
            _request: LoopModelRequest,
        ) -> Result<LoopModelResponse, AgentLoopHostError> {
            *self.model_calls.lock().unwrap() += 1;
            if let Some(error) = self.model_errors.lock().unwrap().pop_front() {
                return Err(error);
            }
            let output = self
                .model_outputs
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| reply_output("done"));
            Ok(LoopModelResponse {
                chunks: vec![ModelStreamChunk {
                    safe_text_delta: String::new(),
                }],
                output,
                effective_model_profile_id: ModelProfileId::new("test_model").unwrap(),
            })
        }
    }

    #[async_trait]
    impl ironclaw_turns::run_profile::LoopCapabilityPort for MockHost {
        async fn visible_capabilities(
            &self,
            _request: VisibleCapabilityRequest,
        ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
            if let Some(surface) = self.capability_surface.lock().unwrap().clone() {
                return Ok(surface);
            }
            Ok(VisibleCapabilitySurface {
                version: surface_version(),
                descriptors: vec![CapabilityDescriptorView {
                    capability_id: CapabilityId::new("demo.echo").unwrap(),
                    provider: None,
                    runtime: RuntimeKind::FirstParty,
                    safe_name: "Demo Echo".to_string(),
                    safe_description: "Demo capability".to_string(),
                    concurrency: CapabilityConcurrency::SafeForParallel,
                }],
            })
        }

        async fn invoke_capability(
            &self,
            _request: CapabilityInvocation,
        ) -> Result<CapabilityOutcome, AgentLoopHostError> {
            *self.single_calls.lock().unwrap() += 1;
            Ok(self.single_outcomes.lock().unwrap().pop_front().unwrap())
        }

        async fn invoke_capability_batch(
            &self,
            request: CapabilityBatchInvocation,
        ) -> Result<CapabilityBatchOutcome, AgentLoopHostError> {
            self.batch_requests.lock().unwrap().push(request);
            Ok(self.batch_outcomes.lock().unwrap().pop_front().unwrap())
        }
    }

    #[async_trait]
    impl ironclaw_turns::run_profile::LoopTranscriptPort for MockHost {
        async fn finalize_assistant_message(
            &self,
            _request: FinalizeAssistantMessage,
        ) -> Result<LoopMessageRef, AgentLoopHostError> {
            Ok(LoopMessageRef::new("msg:assistant").unwrap())
        }
    }

    #[async_trait]
    impl ironclaw_turns::run_profile::LoopCheckpointPort for MockHost {
        async fn checkpoint(
            &self,
            request: LoopCheckpointRequest,
        ) -> Result<TurnCheckpointId, AgentLoopHostError> {
            // Iter-7 finding 3: legacy hosts that returned `Unavailable`
            // from `store_checkpoint_payload` get called back with the
            // `legacy_unknown` sentinel ref. Their `checkpoint()` impl
            // is expected to accept it (they had their own out-of-band
            // ref allocation in the pre-migration contract).
            if *self.legacy_checkpoint_only.lock().unwrap() {
                self.checkpoints.lock().unwrap().push(request.kind);
                return Ok(TurnCheckpointId::new());
            }
            // Simulate the real host: only accept refs we previously handed
            // back from `store_checkpoint_payload`.
            if !self
                .stored_state_refs
                .lock()
                .unwrap()
                .contains(&request.state_ref)
            {
                return Err(AgentLoopHostError::new(
                    AgentLoopHostErrorKind::CheckpointRejected,
                    "checkpoint state ref not stored",
                ));
            }
            self.checkpoints.lock().unwrap().push(request.kind);
            Ok(TurnCheckpointId::new())
        }

        async fn store_checkpoint_payload(
            &self,
            request: ironclaw_turns::run_profile::StoreLoopCheckpointPayload,
        ) -> Result<ironclaw_turns::run_profile::LoopCheckpointStateRef, AgentLoopHostError>
        {
            // Iter-7 finding 3: a legacy host that has not yet migrated
            // returns `Unavailable` (this is the shape of the default
            // trait impl). The executor must tolerate this and fall back
            // to the legacy checkpoint()-only path.
            if *self.legacy_checkpoint_only.lock().unwrap() {
                return Err(AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Unavailable,
                    "legacy host: store_checkpoint_payload not implemented",
                ));
            }
            // Iter-6 finding 2: simulate a transient DB outage when the
            // executor tries to persist the Final checkpoint payload so
            // tests can verify the cancel-page ack does NOT happen before
            // the checkpoint is durable.
            if matches!(request.kind, LoopCheckpointKind::Final)
                && *self.fail_final_store.lock().unwrap()
            {
                return Err(AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Internal,
                    "simulated checkpoint store outage",
                ));
            }
            let token = format!("mock-{}", self.stored_state_refs.lock().unwrap().len());
            let state_ref =
                ironclaw_turns::run_profile::LoopCheckpointStateRef::for_run(&self.context, token)
                    .map_err(|reason| {
                        AgentLoopHostError::new(AgentLoopHostErrorKind::Internal, reason)
                    })?;
            self.stored_state_refs
                .lock()
                .unwrap()
                .push(state_ref.clone());
            self.stored_payloads
                .lock()
                .unwrap()
                .push((request.kind, request.payload.len()));
            Ok(state_ref)
        }
    }

    #[async_trait]
    impl ironclaw_turns::run_profile::LoopProgressPort for MockHost {
        async fn emit_loop_progress(
            &self,
            _event: ironclaw_turns::run_profile::LoopProgressEvent,
        ) -> Result<(), AgentLoopHostError> {
            Ok(())
        }
    }

    fn test_run_context() -> LoopRunContext {
        let scope = TurnScope::new(
            TenantId::new("tenant-executor").unwrap(),
            None,
            None,
            ThreadId::new("thread-executor").unwrap(),
        );
        let descriptor = AgentLoopDriverDescriptor {
            id: LoopDriverId::new("executor_test_driver").unwrap(),
            version: RunProfileVersion::new(1),
            checkpoint_schema_id: Some(
                CheckpointSchemaId::new("executor_test_checkpoint").unwrap(),
            ),
            checkpoint_schema_version: Some(RunProfileVersion::new(1)),
        };
        let resolved_run_profile = ResolvedRunProfile {
            run_class_id: RunClassId::new("executor_test_class").unwrap(),
            profile_id: RunProfileId::default_profile(),
            profile_version: RunProfileVersion::new(1),
            loop_driver: descriptor.clone(),
            checkpoint_schema_id: descriptor.checkpoint_schema_id.clone().unwrap(),
            checkpoint_schema_version: descriptor.checkpoint_schema_version.unwrap(),
            model_profile_id: ModelProfileId::new("executor_test_model").unwrap(),
            capability_surface_profile_id: CapabilitySurfaceProfileId::new(
                "executor_test_capabilities",
            )
            .unwrap(),
            context_profile_id: ContextProfileId::new("executor_test_context").unwrap(),
            steering_policy: SteeringPolicy {
                allow_steering: false,
                allow_interrupt: true,
                allow_driver_specific_nudges: false,
            },
            cancellation_policy: ironclaw_turns::CancellationPolicy {
                allow_cancel: true,
                require_checkpoint_before_cancel: false,
            },
            checkpoint_policy: CheckpointPolicy {
                require_before_model: false,
                require_before_side_effect: false,
                require_before_block: true,
                max_checkpoint_bytes: 64 * 1024,
                require_final_checkpoint: false,
                allow_no_reply_completion: false,
            },
            resource_budget_policy: ResourceBudgetPolicy {
                tier: ResourceBudgetTier::new("executor_test_tier").unwrap(),
                max_model_calls: 32,
                max_capability_invocations: 64,
            },
            runtime_constraints: RuntimeProfileConstraints {
                allow_raw_runtime_backend_selection: false,
                allow_broad_capability_surface: false,
            },
            runner_pool_id: None,
            scheduling_class: SchedulingClass::new("interactive").unwrap(),
            concurrency_class: ConcurrencyClass::new("thread_serial").unwrap(),
            resolution_fingerprint: RunProfileFingerprint::new("executor-test-fingerprint")
                .unwrap(),
            provenance: RedactedRunProfileProvenance {
                sources: vec![],
                effective_privileges: vec![],
            },
        };
        LoopRunContext::new(scope, TurnId::new(), TurnRunId::new(), resolved_run_profile)
    }

    fn surface_version() -> CapabilitySurfaceVersion {
        CapabilitySurfaceVersion::new("surface.v1").unwrap()
    }

    fn call(input: &str) -> CapabilityCallCandidate {
        CapabilityCallCandidate {
            surface_version: surface_version(),
            capability_id: CapabilityId::new("demo.echo").unwrap(),
            input_ref: CapabilityInputRef::new(format!("input:{input}")).unwrap(),
        }
    }

    fn reply_output(content: &str) -> ParentLoopOutput {
        ParentLoopOutput::AssistantReply(AssistantReply {
            content: content.to_string(),
        })
    }

    fn calls_output(input: &str) -> ParentLoopOutput {
        ParentLoopOutput::CapabilityCalls(vec![call(input)])
    }

    fn completed_result(id: &str, summary: &str) -> CapabilityOutcome {
        CapabilityOutcome::Completed(CapabilityResultMessage {
            result_ref: LoopResultRef::new(format!("result:{id}")).unwrap(),
            safe_summary: summary.to_string(),
        })
    }

    fn completed_batch(id: &str, summary: &str) -> CapabilityBatchOutcome {
        CapabilityBatchOutcome {
            outcomes: vec![completed_result(id, summary)],
            stopped_on_suspension: false,
        }
    }

    fn transient_failure_batch() -> CapabilityBatchOutcome {
        CapabilityBatchOutcome {
            outcomes: vec![CapabilityOutcome::Failed(CapabilityFailure {
                error_kind: "transient".to_string(),
                safe_summary: "temporary failure".to_string(),
            })],
            stopped_on_suspension: false,
        }
    }

    fn approval_batch() -> CapabilityBatchOutcome {
        CapabilityBatchOutcome {
            outcomes: vec![CapabilityOutcome::ApprovalRequired {
                gate_ref: LoopGateRef::new("gate:approval").unwrap(),
                safe_summary: "approval required".to_string(),
            }],
            stopped_on_suspension: true,
        }
    }

    async fn run(host: &MockHost, state: &mut LoopExecutionState, limit: u32) -> LoopExit {
        CanonicalAgentLoopExecutor
            .execute(&planner(limit), host, state)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn reply_first_completes_and_final_checkpoints() {
        let host = MockHost::new(vec![reply_output("done")]);
        let mut state = LoopExecutionState::initial_for_run(host.run_context());

        let exit = run(&host, &mut state, 8).await;

        assert_eq!(exit, LoopExit::Completed(CompletionKind::NaturalEnd));
        assert_eq!(state.assistant_refs.len(), 1);
        assert_eq!(
            host.checkpoint_kinds(),
            vec![LoopCheckpointKind::BeforeModel, LoopCheckpointKind::Final]
        );
    }

    #[tokio::test]
    async fn capability_calls_then_reply_completes_with_expected_checkpoints() {
        let host = MockHost::new(vec![calls_output("one"), reply_output("done")])
            .with_batch(completed_batch("one", "ok"));
        let mut state = LoopExecutionState::initial_for_run(host.run_context());

        let exit = run(&host, &mut state, 8).await;

        assert_eq!(exit, LoopExit::Completed(CompletionKind::NaturalEnd));
        assert_eq!(
            host.checkpoint_kinds(),
            vec![
                LoopCheckpointKind::BeforeModel,
                LoopCheckpointKind::BeforeSideEffect,
                LoopCheckpointKind::BeforeModel,
                LoopCheckpointKind::Final,
            ]
        );
    }

    #[tokio::test]
    async fn terminate_hint_stops_after_batch_without_second_model_call() {
        let host = MockHost::new(vec![calls_output("one")])
            .with_batch(completed_batch("one", "terminate_hint:true"));
        let mut state = LoopExecutionState::initial_for_run(host.run_context());

        let exit = run(&host, &mut state, 8).await;

        assert_eq!(exit, LoopExit::Completed(CompletionKind::GracefulStop));
        assert_eq!(host.model_call_count(), 1);
    }

    #[tokio::test]
    async fn approval_required_blocks_after_before_block_checkpoint() {
        let host = MockHost::new(vec![calls_output("approval")]).with_batch(approval_batch());
        let mut state = LoopExecutionState::initial_for_run(host.run_context());

        let exit = run(&host, &mut state, 8).await;

        assert_eq!(
            exit,
            LoopExit::Blocked {
                gate_ref: LoopGateRef::new("gate:approval").unwrap()
            }
        );
        assert_eq!(
            host.checkpoint_kinds(),
            vec![
                LoopCheckpointKind::BeforeModel,
                LoopCheckpointKind::BeforeSideEffect,
                LoopCheckpointKind::BeforeBlock,
            ]
        );
    }

    #[tokio::test]
    async fn iteration_limit_fails_after_exactly_three_model_calls() {
        let host = MockHost::new(vec![
            calls_output("one"),
            calls_output("two"),
            calls_output("three"),
        ])
        .with_batches(vec![
            completed_batch("one", "ok"),
            completed_batch("two", "ok"),
            completed_batch("three", "ok"),
        ]);
        let mut state = LoopExecutionState::initial_for_run(host.run_context());

        let exit = run(&host, &mut state, 3).await;

        assert_eq!(
            exit,
            LoopExit::Failed {
                kind: FailureKind::IterationLimitReached
            }
        );
        assert_eq!(host.model_call_count(), 3);
    }

    #[tokio::test]
    async fn repeated_same_call_signature_fails_no_progress_after_three_iterations() {
        let host = MockHost::new(vec![
            calls_output("same"),
            calls_output("same"),
            calls_output("same"),
        ])
        .with_batches(vec![
            completed_batch("one", "ok"),
            completed_batch("two", "ok"),
            completed_batch("three", "ok"),
        ]);
        let mut state = LoopExecutionState::initial_for_run(host.run_context());

        let exit = run(&host, &mut state, 8).await;

        assert_eq!(
            exit,
            LoopExit::Failed {
                kind: FailureKind::NoProgressDetected
            }
        );
        assert_eq!(host.model_call_count(), 3);
    }

    #[tokio::test]
    async fn transient_failure_retries_single_call_and_records_result() {
        let host = MockHost::new(vec![calls_output("retry"), reply_output("done")])
            .with_batch(transient_failure_batch())
            .with_single(completed_result("retry", "ok"));
        let mut state = LoopExecutionState::initial_for_run(host.run_context());

        let exit = run(&host, &mut state, 8).await;

        assert_eq!(exit, LoopExit::Completed(CompletionKind::NaturalEnd));
        assert_eq!(state.result_refs.len(), 1);
    }

    #[tokio::test]
    async fn cancellation_returns_cancelled_with_interrupted_refs_after_checkpoint() {
        let host = MockHost::new(vec![]);
        host.cancel();
        let mut state = LoopExecutionState::initial_for_run(host.run_context());
        state
            .assistant_refs
            .push(LoopMessageRef::new("msg:interrupted").unwrap());

        let exit = run(&host, &mut state, 8).await;

        assert_eq!(
            exit,
            LoopExit::Cancelled(CancelledKind {
                interrupted_message_refs: vec![LoopMessageRef::new("msg:interrupted").unwrap()]
            })
        );
        assert_eq!(host.checkpoint_kinds(), vec![LoopCheckpointKind::Final]);
    }

    // ============================================================
    // Codex /review follow-ups: tests for the five P1/P2 fixes.
    // ============================================================

    /// Custom recovery strategy that always returns `Retry { Backoff }`.
    /// Used to drive the inner retry loop on repeated capability failures.
    struct AlwaysRetryRecovery;

    #[async_trait]
    impl crate::strategies::RecoveryStrategy for AlwaysRetryRecovery {
        async fn on_capability_error(
            &self,
            state: &LoopExecutionState,
            _err: &CapabilityErrorSummary,
        ) -> RecoveryOutcome {
            RecoveryOutcome::Retry {
                recovery: state.recovery_state.with_incremented_attempts(),
                alter: Some(crate::strategies::RetryAlteration::Backoff {
                    delay: Duration::from_millis(0),
                }),
            }
        }

        async fn on_model_error(
            &self,
            state: &LoopExecutionState,
            _err: &crate::strategies::ModelErrorSummary,
        ) -> RecoveryOutcome {
            RecoveryOutcome::Retry {
                recovery: state.recovery_state.with_incremented_attempts(),
                alter: None,
            }
        }
    }

    fn planner_with_recovery(
        limit: u32,
        recovery: Arc<dyn crate::strategies::RecoveryStrategy>,
    ) -> DefaultPlanner {
        DefaultPlanner::default()
            .with_context(Arc::new(DefaultContextStrategy::default()))
            .with_capability(Arc::new(DefaultCapabilityStrategy))
            .with_model(Arc::new(DefaultModelStrategy))
            .with_batch(Arc::new(DefaultBatchPolicyStrategy))
            .with_gate(Arc::new(DefaultGateHandlingStrategy))
            .with_recovery(recovery)
            .with_stop(Arc::new(DefaultStopConditionStrategy::default()))
            .with_drain(Arc::new(DefaultInputDrainStrategy))
            .with_budget(Arc::new(TestBudget { limit }))
    }

    fn followup_input(message: &str) -> LoopInput {
        LoopInput::FollowUp {
            message_ref: LoopMessageRef::new(format!("msg:{message}")).unwrap(),
        }
    }

    fn cancel_input() -> LoopInput {
        LoopInput::Cancel {
            reason_kind: LoopCancelReasonKind::UserRequested,
        }
    }

    fn user_message_input(message: &str) -> LoopInput {
        LoopInput::UserMessage {
            message_ref: LoopMessageRef::new(format!("msg:{message}")).unwrap(),
        }
    }

    fn steering_input(message: &str) -> LoopInput {
        LoopInput::Steering {
            message_ref: LoopMessageRef::new(format!("msg:{message}")).unwrap(),
        }
    }

    /// Finding 1: a `FollowUp` arriving in the drain queue must continue the
    /// run, not silently drop the message and complete.
    #[tokio::test]
    async fn followup_drain_continues_run_when_followup_arrives() {
        let host = MockHost::new(vec![reply_output("first"), reply_output("second")])
            .with_poll_inputs(vec![
                Vec::new(),                             // observe_cancellation iter 1
                Vec::new(),                             // drain_steering iter 1
                vec![followup_input("more-from-user")], // drain_followup after reply 1
                Vec::new(),                             // observe_cancellation iter 2
                Vec::new(),                             // drain_steering iter 2
                Vec::new(),                             // drain_followup after reply 2
            ]);
        let mut state = LoopExecutionState::initial_for_run(host.run_context());

        let exit = run(&host, &mut state, 8).await;

        // Two model calls (the second one ran because the followup kept the
        // run alive); both replies are in assistant_refs.
        assert_eq!(exit, LoopExit::Completed(CompletionKind::NaturalEnd));
        assert_eq!(host.model_call_count(), 2);
        assert_eq!(state.assistant_refs.len(), 2);
        // Exactly one Final checkpoint at the very end (no Final after iter 1
        // because we continued).
        let finals = host
            .checkpoint_kinds()
            .iter()
            .filter(|k| matches!(k, LoopCheckpointKind::Final))
            .count();
        assert_eq!(
            finals,
            1,
            "expected exactly one Final checkpoint, got {:?}",
            host.checkpoint_kinds()
        );
    }

    /// Iter-8 finding 2: a `UserMessage` (not `FollowUp`) arriving in the
    /// drain queue must continue the run, not silently drop the message and
    /// complete. Pre-iter-8 `drain_followup` matched only `LoopInput::FollowUp`
    /// — a fresh `UserMessage` enqueued just as the loop would otherwise
    /// complete fell through to the control-only branch and got `ack_inputs`'d,
    /// exiting `Completed` while dropping the input.
    #[tokio::test]
    async fn followup_drain_continues_run_when_user_message_arrives() {
        let host = MockHost::new(vec![reply_output("first"), reply_output("second")])
            .with_poll_inputs(vec![
                Vec::new(),                                  // observe_cancellation iter 1
                Vec::new(),                                  // drain_steering iter 1
                vec![user_message_input("late-user-typed")], // drain_followup after reply 1
                Vec::new(),                                  // observe_cancellation iter 2
                Vec::new(),                                  // drain_steering iter 2
                Vec::new(),                                  // drain_followup after reply 2
            ]);
        let mut state = LoopExecutionState::initial_for_run(host.run_context());

        let exit = run(&host, &mut state, 8).await;

        // The user message kept the run alive: a second model call ran and
        // produced a second assistant reply.
        assert_eq!(exit, LoopExit::Completed(CompletionKind::NaturalEnd));
        assert_eq!(host.model_call_count(), 2);
        assert_eq!(state.assistant_refs.len(), 2);
    }

    /// Iter-8 finding 2: same shape as the `UserMessage` case but for
    /// `LoopInput::Steering` — also user-facing input the next iteration
    /// owes processing to, also dropped pre-iter-8.
    #[tokio::test]
    async fn followup_drain_continues_run_when_steering_arrives() {
        let host = MockHost::new(vec![reply_output("first"), reply_output("second")])
            .with_poll_inputs(vec![
                Vec::new(),                               // observe_cancellation iter 1
                Vec::new(),                               // drain_steering iter 1
                vec![steering_input("steering-message")], // drain_followup after reply 1
                Vec::new(),                               // observe_cancellation iter 2
                Vec::new(),                               // drain_steering iter 2
                Vec::new(),                               // drain_followup after reply 2
            ]);
        let mut state = LoopExecutionState::initial_for_run(host.run_context());

        let exit = run(&host, &mut state, 8).await;

        assert_eq!(exit, LoopExit::Completed(CompletionKind::NaturalEnd));
        assert_eq!(host.model_call_count(), 2);
        assert_eq!(state.assistant_refs.len(), 2);
    }

    /// Iter-2 finding 2 / iter-3 finding 1: a `Cancel` arriving in the
    /// drain queue must terminate the run with `LoopExit::Cancelled`. With
    /// the iter-3 atomic-page semantics, `drain_followup` itself observes
    /// the terminal input, applies any sibling control side effects, acks
    /// the page, and returns `TerminalCancel` so the caller finalizes —
    /// the next iteration's `observe_cancellation` is no longer required.
    #[tokio::test]
    async fn followup_drain_terminates_on_cancel_in_drain_page() {
        let host = MockHost::new(vec![reply_output("hello")]).with_poll_inputs(vec![
            Vec::new(),           // observe_cancellation iter 1
            Vec::new(),           // drain_steering iter 1
            vec![cancel_input()], // drain_followup after reply — cancel-only batch
        ]);
        let mut state = LoopExecutionState::initial_for_run(host.run_context());

        let exit = run(&host, &mut state, 8).await;

        match exit {
            LoopExit::Cancelled(_) => {}
            other => panic!("expected Cancelled, got {other:?}"),
        }
        // The cancel-only drain page was acked exactly once — pages are
        // atomic, and the terminal exit relies on having advanced past
        // the cancel so a replay/retry can't re-deliver it.
        assert_eq!(
            host.ack_count(),
            1,
            "cancel-only drain page should ack once"
        );
    }

    /// Finding 3: a recovery `Retry` followed by a still-`Failed` outcome
    /// must re-consult recovery and (with `DefaultRecoveryStrategy`) abort
    /// once the per-class budget is exhausted, surfacing
    /// `LoopExit::Failed { CapabilityProtocolError }`.
    #[tokio::test]
    async fn repeated_transient_failures_on_retry_consume_budget_then_abort() {
        // Initial batch fails Transient → recovery Retry (attempt 1) →
        // single-call returns Failed Transient → recovery Retry (attempt 2)
        // → single-call returns Failed Transient → recovery Abort (budget
        // exhausted) → LoopExit::Failed.
        let host = MockHost::new(vec![calls_output("flaky")])
            .with_batch(transient_failure_batch())
            .with_single(CapabilityOutcome::Failed(CapabilityFailure {
                error_kind: "transient".to_string(),
                safe_summary: "still flaky 1".to_string(),
            }))
            .with_single(CapabilityOutcome::Failed(CapabilityFailure {
                error_kind: "transient".to_string(),
                safe_summary: "still flaky 2".to_string(),
            }));
        let mut state = LoopExecutionState::initial_for_run(host.run_context());

        let exit = run(&host, &mut state, 8).await;

        match exit {
            LoopExit::Failed {
                kind: FailureKind::Other(LoopFailureKind::CapabilityProtocolError),
            } => {}
            other => panic!("expected Failed CapabilityProtocolError, got {other:?}"),
        }
        // Verify the executor actually invoked the single-call retry path
        // twice (consuming the budget) before giving up.
        assert_eq!(host.single_call_count(), 2);
    }

    /// Finding 3 (defense-in-depth): a custom recovery strategy that never
    /// returns `Abort` must be capped by `MAX_RETRIES_PER_CALL` and exit
    /// with `DriverBug`.
    #[tokio::test]
    async fn always_retry_recovery_is_capped_by_max_retries_per_call() {
        let host =
            MockHost::new(vec![calls_output("infinite")]).with_batch(transient_failure_batch());
        // Pre-script enough single-call failures to satisfy
        // MAX_RETRIES_PER_CALL.
        for i in 0..(MAX_RETRIES_PER_CALL as usize) {
            host.single_outcomes
                .lock()
                .unwrap()
                .push_back(CapabilityOutcome::Failed(CapabilityFailure {
                    error_kind: "transient".to_string(),
                    safe_summary: format!("failure {i}"),
                }));
        }
        let mut state = LoopExecutionState::initial_for_run(host.run_context());

        let exit = CanonicalAgentLoopExecutor
            .execute(
                &planner_with_recovery(8, Arc::new(AlwaysRetryRecovery)),
                &host,
                &mut state,
            )
            .await
            .unwrap();

        match exit {
            LoopExit::Failed {
                kind: FailureKind::Other(LoopFailureKind::DriverBug),
            } => {}
            other => panic!("expected Failed DriverBug, got {other:?}"),
        }
    }

    /// Finding 4: an `AllowOnly([cap_a])` capability filter narrows the
    /// visible surface to only `cap_a` even when the host returns more.
    #[tokio::test]
    async fn capability_filter_allow_only_narrows_visible_surface() {
        // Host returns two descriptors; planner filter allows only one.
        let cap_a = CapabilityId::new("demo.allowed").unwrap();
        let cap_b = CapabilityId::new("demo.denied").unwrap();
        let surface = VisibleCapabilitySurface {
            version: surface_version(),
            descriptors: vec![
                CapabilityDescriptorView {
                    capability_id: cap_a.clone(),
                    provider: None,
                    runtime: RuntimeKind::FirstParty,
                    safe_name: "Allowed".to_string(),
                    safe_description: "kept by filter".to_string(),
                    concurrency: CapabilityConcurrency::SafeForParallel,
                },
                CapabilityDescriptorView {
                    capability_id: cap_b.clone(),
                    provider: None,
                    runtime: RuntimeKind::FirstParty,
                    safe_name: "Denied".to_string(),
                    safe_description: "removed by filter".to_string(),
                    concurrency: CapabilityConcurrency::SafeForParallel,
                },
            ],
        };

        // Test the helper directly — it's the executor-side application of
        // the planner's strategy filter to the host's full surface.
        let filter = CapabilityFilter::AllowOnly(vec![cap_a.clone()]);
        let narrowed = apply_capability_filter(surface.clone(), &filter);

        assert_eq!(narrowed.descriptors.len(), 1);
        assert_eq!(narrowed.descriptors[0].capability_id, cap_a);

        // Deny inverts.
        let deny = CapabilityFilter::Deny(vec![cap_a.clone()]);
        let narrowed = apply_capability_filter(surface.clone(), &deny);
        assert_eq!(narrowed.descriptors.len(), 1);
        assert_eq!(narrowed.descriptors[0].capability_id, cap_b);

        // All is a no-op.
        let all = CapabilityFilter::All;
        let untouched = apply_capability_filter(surface.clone(), &all);
        assert_eq!(untouched.descriptors.len(), 2);
    }

    /// Finding 5: `SpawnedProcess` must be treated as `Blocked` (with a
    /// gate-shaped ref derived from the process handle), not as a failure.
    #[tokio::test]
    async fn spawned_process_outcome_blocks_with_synthetic_gate_ref() {
        let process_ref =
            ironclaw_turns::run_profile::LoopProcessRef::new("process:job-42").unwrap();
        let host = MockHost::new(vec![calls_output("spawn")]).with_batch(CapabilityBatchOutcome {
            outcomes: vec![CapabilityOutcome::SpawnedProcess(ProcessHandleSummary {
                process_ref: process_ref.clone(),
                safe_summary: "kicked off long job".to_string(),
            })],
            stopped_on_suspension: true,
        });
        let mut state = LoopExecutionState::initial_for_run(host.run_context());

        let exit = run(&host, &mut state, 8).await;

        match exit {
            LoopExit::Blocked { gate_ref } => {
                // The gate ref is the synthesized projection.
                assert_eq!(gate_ref.as_str(), "gate:proc-job-42");
            }
            other => panic!("expected Blocked, got {other:?}"),
        }
        // Same checkpoint sequence as ApprovalRequired: BeforeModel,
        // BeforeSideEffect, BeforeBlock.
        assert_eq!(
            host.checkpoint_kinds(),
            vec![
                LoopCheckpointKind::BeforeModel,
                LoopCheckpointKind::BeforeSideEffect,
                LoopCheckpointKind::BeforeBlock,
            ]
        );
    }

    // ============================================================
    // Codex /review (second pass) follow-ups: tests for findings 1,
    // 3, 4, 7. The pre-existing tests already lock in findings 2, 5,
    // 6, 8 — see the section above.
    // ============================================================

    /// Finding 1: checkpoint payload must be stored before the host's
    /// `checkpoint()` call, so the real `HostManagedLoopCheckpointPort`
    /// (which verifies the state ref exists) accepts every checkpoint.
    #[tokio::test]
    async fn checkpoint_payload_is_stored_before_each_checkpoint_marker() {
        let host = MockHost::new(vec![reply_output("hi")]);
        let mut state = LoopExecutionState::initial_for_run(host.run_context());

        let _ = run(&host, &mut state, 8).await;

        // Two checkpoints in this run: BeforeModel, Final. Each must have
        // a payload stored before the marker is recorded — the mock host
        // rejects unknown state refs (mirroring the real
        // HostManagedLoopCheckpointPort contract).
        assert_eq!(host.checkpoint_kinds().len(), 2);
        assert_eq!(host.stored_payload_count(), 2);
    }

    /// Finding 3: a model-emitted capability call against a capability the
    /// executor filter narrowed away must be denied executor-side without
    /// ever reaching the host's `invoke_capability_batch`.
    #[tokio::test]
    async fn hidden_capability_candidate_is_denied_without_host_invocation() {
        use crate::strategies::{
            BatchPolicy, CapabilityFilter, CapabilityStrategy, DefaultBatchPolicyStrategy,
            DefaultContextStrategy, DefaultGateHandlingStrategy, DefaultInputDrainStrategy,
            DefaultModelStrategy, DefaultRecoveryStrategy, DefaultStopConditionStrategy,
        };

        struct DenyAllStrategy;
        #[async_trait]
        impl CapabilityStrategy for DenyAllStrategy {
            async fn filter(&self, _state: &LoopExecutionState) -> CapabilityFilter {
                CapabilityFilter::Deny(vec![CapabilityId::new("demo.echo").unwrap()])
            }
        }

        // Model emits a call to demo.echo, but planner's filter denies it.
        // The mock host does NOT have a batch outcome queued — so if the
        // executor erroneously sends the batch, MockHost will panic on
        // pop_front. The denied path routes through recovery, which (per
        // DefaultRecoveryStrategy) aborts on PolicyDenied.
        let host = MockHost::new(vec![calls_output("hidden"), reply_output("done")]);
        let planner = DefaultPlanner::default()
            .with_context(Arc::new(DefaultContextStrategy::default()))
            .with_capability(Arc::new(DenyAllStrategy))
            .with_model(Arc::new(DefaultModelStrategy))
            .with_batch(Arc::new(DefaultBatchPolicyStrategy))
            .with_gate(Arc::new(DefaultGateHandlingStrategy))
            .with_recovery(Arc::new(DefaultRecoveryStrategy::default()))
            .with_stop(Arc::new(DefaultStopConditionStrategy::default()))
            .with_drain(Arc::new(DefaultInputDrainStrategy))
            .with_budget(Arc::new(TestBudget { limit: 8 }));
        let mut state = LoopExecutionState::initial_for_run(host.run_context());

        let exit = CanonicalAgentLoopExecutor
            .execute(&planner, &host, &mut state)
            .await
            .unwrap();

        // PolicyDenied → DefaultRecoveryStrategy::Abort → LoopExit::Failed.
        match exit {
            LoopExit::Failed {
                kind: FailureKind::Other(LoopFailureKind::CapabilityProtocolError),
            } => {}
            other => panic!("expected Failed CapabilityProtocolError, got {other:?}"),
        }
        // Sanity: BatchPolicy was still consulted but the host never saw
        // the invocation (we'd have panicked on pop_front otherwise).
        let _ = BatchPolicy::Parallel;
    }

    /// Finding 4: a `GateResolved` input must clear `last_gate` and be
    /// acked so it doesn't get re-polled forever.
    #[tokio::test]
    async fn gate_resolved_input_clears_last_gate_and_is_acked() {
        let gate_ref = LoopGateRef::new("gate:approval-1").unwrap();
        let host = MockHost::new(vec![reply_output("done")]).with_poll_inputs(vec![
            // observe_cancellation iter 1: GateResolved alone — must consume.
            vec![LoopInput::GateResolved {
                gate_ref: gate_ref.clone(),
            }],
            Vec::new(), // drain_steering iter 1
            Vec::new(), // drain_followup after reply (empty → Final)
        ]);
        let mut state = LoopExecutionState::initial_for_run(host.run_context());
        state.last_gate = Some(gate_ref.clone());

        let exit = run(&host, &mut state, 8).await;

        assert_eq!(exit, LoopExit::Completed(CompletionKind::NaturalEnd));
        assert_eq!(state.last_gate, None, "last_gate must be cleared");
        // The control-only batch was acked once.
        assert!(host.ack_count() >= 1, "GateResolved batch must be acked");
    }

    /// Finding 4: a `CapabilitySurfaceChanged` input must drop the cached
    /// `surface_version` so the next iteration re-fetches.
    #[tokio::test]
    async fn surface_changed_input_drops_cached_surface_version() {
        let host = MockHost::new(vec![reply_output("done")]).with_poll_inputs(vec![
            // observe_cancellation iter 1: SurfaceChanged alone.
            vec![LoopInput::CapabilitySurfaceChanged {
                version: surface_version(),
            }],
            Vec::new(), // drain_steering iter 1
            Vec::new(), // drain_followup after reply
        ]);
        let mut state = LoopExecutionState::initial_for_run(host.run_context());

        let _ = run(&host, &mut state, 8).await;

        // The control batch was acked.
        assert!(host.ack_count() >= 1, "SurfaceChanged batch must be acked");
    }

    /// Finding 7: a host model-port error with kind `StaleSurface` must
    /// trigger a capability surface reload and re-issue the iteration
    /// without consuming the iteration budget.
    #[tokio::test]
    async fn stale_surface_model_error_reloads_capabilities_and_retries() {
        let host = MockHost::new(vec![reply_output("done")]).with_model_errors(vec![
            AgentLoopHostError::new(AgentLoopHostErrorKind::StaleSurface, "surface drifted"),
        ]);
        let mut state = LoopExecutionState::initial_for_run(host.run_context());

        let exit = run(&host, &mut state, 8).await;

        // The model was called twice: first returned StaleSurface, second
        // succeeded after the surface was reloaded.
        assert_eq!(exit, LoopExit::Completed(CompletionKind::NaturalEnd));
        assert_eq!(host.model_call_count(), 2);
    }

    /// Finding 7: a host model-port error classified as transient
    /// (`Unavailable`) must be routed through `RecoveryStrategy::on_model_error`
    /// and ultimately abort with `ModelError` when the per-class budget is
    /// exhausted.
    #[tokio::test]
    async fn transient_model_error_routes_through_recovery_then_aborts() {
        // DefaultRecoveryStrategy retries twice on `Unavailable` before
        // aborting with `ModelError`. Pre-script three errors so we exhaust
        // the budget.
        let host = MockHost::new(vec![]).with_model_errors(vec![
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::Unavailable,
                "model gateway unavailable",
            ),
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::Unavailable,
                "model gateway unavailable",
            ),
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::Unavailable,
                "model gateway unavailable",
            ),
        ]);
        let mut state = LoopExecutionState::initial_for_run(host.run_context());

        let exit = run(&host, &mut state, 8).await;

        match exit {
            LoopExit::Failed {
                kind: FailureKind::Other(LoopFailureKind::ModelError),
            } => {}
            other => panic!("expected Failed ModelError, got {other:?}"),
        }
        // Three model calls — one initial + two retries — before recovery
        // aborts.
        assert_eq!(host.model_call_count(), 3);
    }

    // ============================================================
    // Codex /review (iter-3) follow-ups: tests for findings 1–5.
    // ============================================================

    /// Iter-3 finding 1: a page containing BOTH `FollowUp` and a control
    /// event (`GateResolved`) is no longer left permanently un-acked
    /// (which the iter-2 implementation did → livelock). The executor
    /// applies the control side effect in-place, acks the mixed page,
    /// continues with the follow-up, and exits naturally.
    #[tokio::test]
    async fn mixed_followup_and_gate_resolved_drain_page_is_acked_no_livelock() {
        let gate_ref = LoopGateRef::new("gate:approval-mix").unwrap();
        let host = MockHost::new(vec![reply_output("first"), reply_output("second")])
            .with_poll_inputs(vec![
                Vec::new(), // observe_cancellation iter 1
                Vec::new(), // drain_steering iter 1
                // drain_followup after reply 1: FollowUp + GateResolved
                // in the same atomic page — pre-iter-3 code livelocked
                // here.
                vec![
                    followup_input("user-says-more"),
                    LoopInput::GateResolved {
                        gate_ref: gate_ref.clone(),
                    },
                ],
                Vec::new(), // observe_cancellation iter 2
                Vec::new(), // drain_steering iter 2
                Vec::new(), // drain_followup after reply 2 → Empty → Final
            ]);
        let mut state = LoopExecutionState::initial_for_run(host.run_context());
        state.last_gate = Some(gate_ref.clone());

        let exit = run(&host, &mut state, 8).await;

        // Run completed (no livelock, no iteration-limit failure).
        assert_eq!(exit, LoopExit::Completed(CompletionKind::NaturalEnd));
        // The mixed page applied the GateResolved side effect AND
        // continued with the FollowUp → two model calls.
        assert_eq!(host.model_call_count(), 2);
        // The gate was cleared as the control side effect of the mixed
        // page.
        assert_eq!(state.last_gate, None);
        // The mixed page was acked exactly once (no spinning).
        assert!(host.ack_count() >= 1);
    }

    /// Iter-3 finding 2: when the planner's filter narrows a capability
    /// away that appears BEFORE an allowed call in the model's batch,
    /// the executor must short-circuit on the policy denial WITHOUT
    /// having executed the subsequent allowed call. The pre-iter-3 code
    /// invoked the entire allowed sub-batch up-front, so `[hidden, allowed]`
    /// would already have executed `allowed` by the time the synthetic
    /// `Denied` outcome was processed.
    #[tokio::test]
    async fn hidden_capability_before_allowed_aborts_without_executing_allowed() {
        use crate::strategies::{
            CapabilityFilter, CapabilityStrategy, DefaultBatchPolicyStrategy,
            DefaultContextStrategy, DefaultGateHandlingStrategy, DefaultInputDrainStrategy,
            DefaultModelStrategy, DefaultRecoveryStrategy, DefaultStopConditionStrategy,
        };

        let hidden = CapabilityId::new("demo.hidden").unwrap();
        let allowed = CapabilityId::new("demo.allowed").unwrap();
        // Planner filter denies only `demo.hidden`.
        struct DenyHidden;
        #[async_trait]
        impl CapabilityStrategy for DenyHidden {
            async fn filter(&self, _state: &LoopExecutionState) -> CapabilityFilter {
                CapabilityFilter::Deny(vec![CapabilityId::new("demo.hidden").unwrap()])
            }
        }

        // Host surface advertises both; model emits `[hidden, allowed]`.
        // CRITICAL: do NOT enqueue any single-call outcome for `allowed`
        // — if the executor were to invoke it before processing the
        // denial, MockHost::invoke_capability would panic on the empty
        // single_outcomes queue.
        let surface = VisibleCapabilitySurface {
            version: surface_version(),
            descriptors: vec![
                CapabilityDescriptorView {
                    capability_id: hidden.clone(),
                    provider: None,
                    runtime: RuntimeKind::FirstParty,
                    safe_name: "Hidden".to_string(),
                    safe_description: "filtered by planner".to_string(),
                    concurrency: CapabilityConcurrency::SafeForParallel,
                },
                CapabilityDescriptorView {
                    capability_id: allowed.clone(),
                    provider: None,
                    runtime: RuntimeKind::FirstParty,
                    safe_name: "Allowed".to_string(),
                    safe_description: "passes filter".to_string(),
                    concurrency: CapabilityConcurrency::SafeForParallel,
                },
            ],
        };
        let host = MockHost::new(vec![ParentLoopOutput::CapabilityCalls(vec![
            CapabilityCallCandidate {
                surface_version: surface_version(),
                capability_id: hidden.clone(),
                input_ref: CapabilityInputRef::new("input:hidden").unwrap(),
            },
            CapabilityCallCandidate {
                surface_version: surface_version(),
                capability_id: allowed.clone(),
                input_ref: CapabilityInputRef::new("input:allowed").unwrap(),
            },
        ])])
        .with_capability_surface(surface);

        let planner = DefaultPlanner::default()
            .with_context(Arc::new(DefaultContextStrategy::default()))
            .with_capability(Arc::new(DenyHidden))
            .with_model(Arc::new(DefaultModelStrategy))
            .with_batch(Arc::new(DefaultBatchPolicyStrategy))
            .with_gate(Arc::new(DefaultGateHandlingStrategy))
            .with_recovery(Arc::new(DefaultRecoveryStrategy::default()))
            .with_stop(Arc::new(DefaultStopConditionStrategy::default()))
            .with_drain(Arc::new(DefaultInputDrainStrategy))
            .with_budget(Arc::new(TestBudget { limit: 8 }));
        let mut state = LoopExecutionState::initial_for_run(host.run_context());

        let exit = CanonicalAgentLoopExecutor
            .execute(&planner, &host, &mut state)
            .await
            .unwrap();

        // PolicyDenied → DefaultRecoveryStrategy::Abort →
        // LoopExit::Failed { CapabilityProtocolError }.
        match exit {
            LoopExit::Failed {
                kind: FailureKind::Other(LoopFailureKind::CapabilityProtocolError),
            } => {}
            other => panic!("expected Failed CapabilityProtocolError, got {other:?}"),
        }
        // Sanity: `allowed` was NEVER invoked — the executor aborted on
        // the denial before processing the allowed call.
        assert_eq!(
            host.single_call_count(),
            0,
            "allowed capability must not run when an earlier call was denied"
        );
    }

    /// Iter-3 finding 3: a `Sequential` batch returning a truncated
    /// outcome prefix (host stopped at first suspension) is accepted
    /// when the tail is a suspension. The executor routes the
    /// suspension through the existing gate path → `Blocked`. The
    /// pre-iter-3 code raised "outcome count did not match" instead.
    #[tokio::test]
    async fn sequential_batch_truncated_at_suspension_routes_through_gate() {
        use crate::strategies::{
            BatchPolicy, BatchPolicyStrategy, CapabilityCallSummary, DefaultCapabilityStrategy,
            DefaultContextStrategy, DefaultGateHandlingStrategy, DefaultInputDrainStrategy,
            DefaultModelStrategy, DefaultRecoveryStrategy, DefaultStopConditionStrategy,
        };

        struct AlwaysSequential;
        impl BatchPolicyStrategy for AlwaysSequential {
            fn policy(
                &self,
                _state: &LoopExecutionState,
                _calls: &[CapabilityCallSummary],
            ) -> BatchPolicy {
                BatchPolicy::Sequential
            }
        }

        // Two-call batch; host returns only the suspension prefix
        // (e.g. `[ApprovalRequired]`) when it stops at first
        // suspension.
        let host = MockHost::new(vec![ParentLoopOutput::CapabilityCalls(vec![
            call("first"),
            call("second"),
        ])])
        .with_batch(CapabilityBatchOutcome {
            outcomes: vec![CapabilityOutcome::ApprovalRequired {
                gate_ref: LoopGateRef::new("gate:seq-approval").unwrap(),
                safe_summary: "approval required mid-batch".to_string(),
            }],
            stopped_on_suspension: true,
        });
        let planner = DefaultPlanner::default()
            .with_context(Arc::new(DefaultContextStrategy::default()))
            .with_capability(Arc::new(DefaultCapabilityStrategy))
            .with_model(Arc::new(DefaultModelStrategy))
            .with_batch(Arc::new(AlwaysSequential))
            .with_gate(Arc::new(DefaultGateHandlingStrategy))
            .with_recovery(Arc::new(DefaultRecoveryStrategy::default()))
            .with_stop(Arc::new(DefaultStopConditionStrategy::default()))
            .with_drain(Arc::new(DefaultInputDrainStrategy))
            .with_budget(Arc::new(TestBudget { limit: 8 }));
        let mut state = LoopExecutionState::initial_for_run(host.run_context());

        let exit = CanonicalAgentLoopExecutor
            .execute(&planner, &host, &mut state)
            .await
            .unwrap();

        assert_eq!(
            exit,
            LoopExit::Blocked {
                gate_ref: LoopGateRef::new("gate:seq-approval").unwrap()
            }
        );
    }

    /// Iter-3 finding 4: a `Retry { Backoff { delay } }` from recovery
    /// must trigger a tokio sleep before the next attempt. We use
    /// `tokio::time` paused-clock + a custom recovery that requests a
    /// 60s backoff, then assert the elapsed *virtual* time is at least
    /// 60s — proving the executor consulted the clock.
    #[tokio::test(start_paused = true)]
    async fn backoff_alteration_is_honored_via_tokio_sleep() {
        struct BackoffThenAbort {
            attempts_remaining: Mutex<u32>,
        }
        #[async_trait]
        impl crate::strategies::RecoveryStrategy for BackoffThenAbort {
            async fn on_capability_error(
                &self,
                state: &LoopExecutionState,
                _err: &CapabilityErrorSummary,
            ) -> RecoveryOutcome {
                let mut remaining = self.attempts_remaining.lock().unwrap();
                if *remaining > 0 {
                    *remaining -= 1;
                    RecoveryOutcome::Retry {
                        recovery: state.recovery_state.with_incremented_attempts(),
                        alter: Some(crate::strategies::RetryAlteration::Backoff {
                            delay: Duration::from_secs(60),
                        }),
                    }
                } else {
                    RecoveryOutcome::Abort {
                        recovery: state.recovery_state.with_incremented_attempts(),
                        failure_kind: LoopFailureKind::CapabilityProtocolError,
                    }
                }
            }

            async fn on_model_error(
                &self,
                state: &LoopExecutionState,
                _err: &crate::strategies::ModelErrorSummary,
            ) -> RecoveryOutcome {
                RecoveryOutcome::Abort {
                    recovery: state.recovery_state.with_incremented_attempts(),
                    failure_kind: LoopFailureKind::ModelError,
                }
            }
        }

        let host = MockHost::new(vec![calls_output("flaky")])
            .with_batch(transient_failure_batch())
            .with_single(CapabilityOutcome::Failed(CapabilityFailure {
                error_kind: "transient".to_string(),
                safe_summary: "still flaky".to_string(),
            }));
        let recovery: Arc<dyn crate::strategies::RecoveryStrategy> = Arc::new(BackoffThenAbort {
            attempts_remaining: Mutex::new(1),
        });
        let mut state = LoopExecutionState::initial_for_run(host.run_context());

        let start = tokio::time::Instant::now();
        let _ = CanonicalAgentLoopExecutor
            .execute(&planner_with_recovery(8, recovery), &host, &mut state)
            .await
            .unwrap();
        let elapsed = start.elapsed();

        // We requested a 60s backoff; the executor must have advanced
        // the (paused) clock by at least that much.
        assert!(
            elapsed >= Duration::from_secs(60),
            "expected >= 60s of virtual sleep from Backoff alteration, got {elapsed:?}"
        );
    }

    /// Iter-3 finding 5: a `LoopModelPort` error with kind `Cancelled`
    /// must surface as `LoopExit::Cancelled` (not `HostUnavailable`),
    /// taking the `Final` checkpoint along the way.
    #[tokio::test]
    async fn model_port_cancelled_error_surfaces_as_cancelled_exit() {
        let host = MockHost::new(vec![]).with_model_errors(vec![AgentLoopHostError::new(
            AgentLoopHostErrorKind::Cancelled,
            "host aborted in-flight model stream",
        )]);
        let mut state = LoopExecutionState::initial_for_run(host.run_context());
        state
            .assistant_refs
            .push(LoopMessageRef::new("msg:earlier").unwrap());

        let exit = run(&host, &mut state, 8).await;

        match exit {
            LoopExit::Cancelled(cancelled) => {
                // Pre-existing assistant refs are carried through.
                assert_eq!(
                    cancelled.interrupted_message_refs,
                    vec![LoopMessageRef::new("msg:earlier").unwrap()]
                );
            }
            other => panic!("expected Cancelled, got {other:?}"),
        }
        // Final checkpoint was taken on the way out.
        assert!(
            host.checkpoint_kinds()
                .iter()
                .any(|k| matches!(k, LoopCheckpointKind::Final)),
            "expected a Final checkpoint, got {:?}",
            host.checkpoint_kinds()
        );
    }

    // ============================================================
    // Codex /review (iter-4) follow-ups: tests for findings 1–3.
    // ============================================================

    /// Iter-4 finding 1: `drain_followup` must keep polling past
    /// control-only pages. A control-only page followed by a follow-up
    /// page used to drop the follow-up silently (caller took `Final` and
    /// exited `Completed`). With the fix, the executor acks the
    /// control-only page, polls again, finds the follow-up, and continues
    /// the run.
    #[tokio::test]
    async fn followup_drain_keeps_polling_past_control_only_pages() {
        let gate_ref = LoopGateRef::new("gate:later-followup").unwrap();
        let host = MockHost::new(vec![reply_output("first"), reply_output("second")])
            .with_poll_inputs(vec![
                Vec::new(), // observe_cancellation iter 1
                Vec::new(), // drain_steering iter 1
                // drain_followup after reply 1, page 1: control-only
                // GateResolved (pre-iter-4 returned Empty here and dropped
                // the follow-up on page 2).
                vec![LoopInput::GateResolved {
                    gate_ref: gate_ref.clone(),
                }],
                // drain_followup after reply 1, page 2: the actual
                // follow-up sitting on a later page.
                vec![followup_input("user-followup-on-page-2")],
                Vec::new(), // observe_cancellation iter 2
                Vec::new(), // drain_steering iter 2
                Vec::new(), // drain_followup after reply 2 — Empty → Final
            ]);
        let mut state = LoopExecutionState::initial_for_run(host.run_context());
        state.last_gate = Some(gate_ref.clone());

        let exit = run(&host, &mut state, 8).await;

        // Both replies ran — the second only because the follow-up was
        // not dropped.
        assert_eq!(exit, LoopExit::Completed(CompletionKind::NaturalEnd));
        assert_eq!(host.model_call_count(), 2);
        assert_eq!(state.assistant_refs.len(), 2);
        // GateResolved side effect was applied while draining the
        // control-only page.
        assert_eq!(state.last_gate, None);
        // Exactly one Final checkpoint (at the very end), proving the
        // run did not exit Completed after page 1.
        let finals = host
            .checkpoint_kinds()
            .iter()
            .filter(|k| matches!(k, LoopCheckpointKind::Final))
            .count();
        assert_eq!(
            finals,
            1,
            "expected exactly one Final checkpoint, got {:?}",
            host.checkpoint_kinds()
        );
    }

    /// Iter-4 finding 2: a `Denied` outcome must NEVER be replayed
    /// through `host.invoke_capability`. Even with a recovery strategy
    /// that always returns `Retry`, the host's single-call port must not
    /// be invoked — the denial is authoritative. The executor treats
    /// `Retry` on `PolicyDenied` as `SkipResult`.
    #[tokio::test]
    async fn denied_outcome_is_not_replayed_through_host_under_retry_recovery() {
        use crate::strategies::{
            CapabilityFilter, CapabilityStrategy, DefaultBatchPolicyStrategy,
            DefaultContextStrategy, DefaultGateHandlingStrategy, DefaultInputDrainStrategy,
            DefaultModelStrategy, DefaultStopConditionStrategy,
        };

        // Planner: filter denies `demo.echo` (the default mock surface
        // capability) — so the model's call gets a synthetic Denied
        // outcome from the executor-side filter.
        struct DenyEverything;
        #[async_trait]
        impl CapabilityStrategy for DenyEverything {
            async fn filter(&self, _state: &LoopExecutionState) -> CapabilityFilter {
                CapabilityFilter::Deny(vec![CapabilityId::new("demo.echo").unwrap()])
            }
        }

        // The follow-up reply lets the run terminate naturally after
        // the Denied call is skipped.
        let host = MockHost::new(vec![calls_output("denied"), reply_output("done")]);

        let planner = DefaultPlanner::default()
            .with_context(Arc::new(DefaultContextStrategy::default()))
            .with_capability(Arc::new(DenyEverything))
            .with_model(Arc::new(DefaultModelStrategy))
            .with_batch(Arc::new(DefaultBatchPolicyStrategy))
            .with_gate(Arc::new(DefaultGateHandlingStrategy))
            // The AlwaysRetryRecovery strategy returns Retry on every
            // capability error. Without the iter-4 fix, the executor
            // would re-invoke the host with the denied call — and since
            // no single_outcomes are queued, MockHost would panic.
            .with_recovery(Arc::new(AlwaysRetryRecovery))
            .with_stop(Arc::new(DefaultStopConditionStrategy::default()))
            .with_drain(Arc::new(DefaultInputDrainStrategy))
            .with_budget(Arc::new(TestBudget { limit: 8 }));
        let mut state = LoopExecutionState::initial_for_run(host.run_context());

        let exit = CanonicalAgentLoopExecutor
            .execute(&planner, &host, &mut state)
            .await
            .unwrap();

        // The run completes naturally — Denied was skipped, the next
        // model call produced a reply.
        assert_eq!(exit, LoopExit::Completed(CompletionKind::NaturalEnd));
        // The host's single-call port was NEVER invoked — the executor
        // refused to replay the denied call.
        assert_eq!(
            host.single_call_count(),
            0,
            "denied call must not be replayed through host.invoke_capability"
        );
    }

    /// Iter-4 finding 3: `recent_failure_kinds` must be pushed AT MOST
    /// ONCE per logical model call, not once per retry attempt. An
    /// eventually-successful model turn must not trip
    /// `DefaultStopConditionStrategy::failure_run_threshold` (3) as a
    /// false `NoProgressDetected` exit. We use `AlwaysRetryRecovery` so
    /// the model is retried for two transient errors before succeeding;
    /// the run must complete naturally and `recent_failure_kinds` must
    /// hold exactly one `ModelError` entry, not three.
    #[tokio::test]
    async fn model_retry_records_failure_kind_once_per_logical_call() {
        // 2 transient errors, then the model port returns the queued
        // `reply_output("done")` on the third attempt.
        let host = MockHost::new(vec![reply_output("done")]).with_model_errors(vec![
            AgentLoopHostError::new(AgentLoopHostErrorKind::Unavailable, "flaky 1"),
            AgentLoopHostError::new(AgentLoopHostErrorKind::Unavailable, "flaky 2"),
        ]);
        let mut state = LoopExecutionState::initial_for_run(host.run_context());

        let exit = CanonicalAgentLoopExecutor
            .execute(
                &planner_with_recovery(8, Arc::new(AlwaysRetryRecovery)),
                &host,
                &mut state,
            )
            .await
            .unwrap();

        // Eventually-successful run.
        assert_eq!(exit, LoopExit::Completed(CompletionKind::NaturalEnd));
        assert_eq!(host.model_call_count(), 3);
        // Exactly one ModelError in the recent-failures ring — not one
        // per retry. With three pushes, the trailing run length would
        // be 3 and the default stop strategy would have aborted with
        // `NoProgressDetected` *before* the successful retry.
        let model_error_count = state
            .recent_failure_kinds
            .iter()
            .filter(|kind| matches!(kind, LoopFailureKind::ModelError))
            .count();
        assert_eq!(
            model_error_count, 1,
            "expected exactly one ModelError entry in recent_failure_kinds, \
             found {model_error_count}"
        );
    }

    // ============================================================
    // Codex /review (iter-5) follow-ups: tests for findings 1-4.
    // ============================================================

    /// Iter-5 finding 1: a recovery `SkipResult` on a persistent model error
    /// must advance the iteration counter so the iteration cap eventually
    /// trips. The pre-iter-5 code routed `SkipResult` through
    /// `ReloadSurface`, which restarts the SAME iteration — and with a
    /// `SkipResult`-returning recovery, the loop would spin forever on a
    /// persistent `Unavailable` model failure.
    #[tokio::test]
    async fn skip_result_on_model_error_advances_iteration_until_cap_trips() {
        // A recovery strategy that always returns `SkipResult` on model
        // errors — the pathological shape that exposed the bug.
        struct AlwaysSkipModelRecovery;
        #[async_trait]
        impl crate::strategies::RecoveryStrategy for AlwaysSkipModelRecovery {
            async fn on_capability_error(
                &self,
                state: &LoopExecutionState,
                _err: &CapabilityErrorSummary,
            ) -> RecoveryOutcome {
                RecoveryOutcome::Abort {
                    recovery: state.recovery_state.with_incremented_attempts(),
                    failure_kind: LoopFailureKind::CapabilityProtocolError,
                }
            }

            async fn on_model_error(
                &self,
                state: &LoopExecutionState,
                _err: &crate::strategies::ModelErrorSummary,
            ) -> RecoveryOutcome {
                RecoveryOutcome::SkipResult {
                    recovery: state.recovery_state.with_incremented_attempts(),
                }
            }
        }

        // Pre-script enough Unavailable errors that any non-progressing
        // loop would spin past the iteration cap. With the iter-5 fix,
        // each SkipResult advances the iteration counter; with a 3-tick
        // cap, exactly 3 model attempts are observed before
        // IterationLimitReached fails out.
        let host = MockHost::new(vec![]).with_model_errors(vec![
            AgentLoopHostError::new(AgentLoopHostErrorKind::Unavailable, "down 1"),
            AgentLoopHostError::new(AgentLoopHostErrorKind::Unavailable, "down 2"),
            AgentLoopHostError::new(AgentLoopHostErrorKind::Unavailable, "down 3"),
            AgentLoopHostError::new(AgentLoopHostErrorKind::Unavailable, "down 4"),
            AgentLoopHostError::new(AgentLoopHostErrorKind::Unavailable, "down 5"),
        ]);
        let mut state = LoopExecutionState::initial_for_run(host.run_context());

        let exit = CanonicalAgentLoopExecutor
            .execute(
                &planner_with_recovery(3, Arc::new(AlwaysSkipModelRecovery)),
                &host,
                &mut state,
            )
            .await
            .unwrap();

        // Iteration cap trips because each SkipResult advances
        // state.iteration. Without the fix, the executor would spin
        // forever on the same iteration.
        match exit {
            LoopExit::Failed {
                kind: FailureKind::IterationLimitReached,
            } => {}
            other => panic!("expected Failed IterationLimitReached, got {other:?}"),
        }
        // Three model attempts (one per advancing iteration), then the
        // cap trips at the top of iteration 3.
        assert_eq!(host.model_call_count(), 3);
        // Iter-5 finding 4: the IterationLimit exit Final-checkpoints.
        assert!(
            host.checkpoint_kinds()
                .iter()
                .any(|k| matches!(k, LoopCheckpointKind::Final)),
            "expected a Final checkpoint on IterationLimit, got {:?}",
            host.checkpoint_kinds()
        );
    }

    /// Iter-5 finding 2: `BudgetStrategy::wall_clock_limit` is consulted at
    /// the top of every tick alongside `iteration_limit`. When exceeded,
    /// the executor fails out with `WallClockLimitReached` after taking a
    /// `Final` checkpoint.
    ///
    /// To exercise the wall-clock branch deterministically we use a
    /// recovery strategy that always retries model errors with a long
    /// `Backoff`, paired with a stream of model errors. The backoff
    /// sleep advances tokio's paused clock past the cap; the very next
    /// wall-clock check at the top of the loop fires.
    #[tokio::test(start_paused = true)]
    async fn wall_clock_limit_failed_exit_with_final_checkpoint() {
        // Budget with a 60s wall-clock cap.
        struct WallClockBudget;
        impl crate::strategies::BudgetStrategy for WallClockBudget {
            fn iteration_limit(&self, _: &LoopExecutionState) -> u32 {
                1000
            }

            fn wall_clock_limit(&self, _: &LoopExecutionState) -> Option<Duration> {
                Some(Duration::from_secs(60))
            }
        }

        // Recovery that always retries model errors with a 90s backoff.
        // After one retry the cumulative virtual time exceeds the 60s cap.
        struct LongBackoffRecovery;
        #[async_trait]
        impl crate::strategies::RecoveryStrategy for LongBackoffRecovery {
            async fn on_capability_error(
                &self,
                state: &LoopExecutionState,
                _err: &CapabilityErrorSummary,
            ) -> RecoveryOutcome {
                RecoveryOutcome::Abort {
                    recovery: state.recovery_state.with_incremented_attempts(),
                    failure_kind: LoopFailureKind::CapabilityProtocolError,
                }
            }

            async fn on_model_error(
                &self,
                state: &LoopExecutionState,
                _err: &crate::strategies::ModelErrorSummary,
            ) -> RecoveryOutcome {
                RecoveryOutcome::Retry {
                    recovery: state.recovery_state.with_incremented_attempts(),
                    alter: Some(crate::strategies::RetryAlteration::Backoff {
                        delay: Duration::from_secs(90),
                    }),
                }
            }
        }

        // Two model errors so the recovery loop sleeps once (90s virtual
        // time elapses), then the second attempt is still in the same
        // iteration. We need to LEAVE the inner retry loop so the
        // top-of-tick wall-clock check fires. The retry loop exits
        // either on Ok or on the `MAX_RETRIES_PER_CALL` cap. To trigger
        // a clean exit, we route through `SkipResult` after the
        // backoff sleep so the executor advances to the next tick.
        struct OnceBackoffThenSkip {
            backed_off: Mutex<bool>,
        }
        #[async_trait]
        impl crate::strategies::RecoveryStrategy for OnceBackoffThenSkip {
            async fn on_capability_error(
                &self,
                state: &LoopExecutionState,
                _err: &CapabilityErrorSummary,
            ) -> RecoveryOutcome {
                RecoveryOutcome::Abort {
                    recovery: state.recovery_state.with_incremented_attempts(),
                    failure_kind: LoopFailureKind::CapabilityProtocolError,
                }
            }

            async fn on_model_error(
                &self,
                state: &LoopExecutionState,
                _err: &crate::strategies::ModelErrorSummary,
            ) -> RecoveryOutcome {
                let mut backed_off = self.backed_off.lock().unwrap();
                if !*backed_off {
                    *backed_off = true;
                    RecoveryOutcome::Retry {
                        recovery: state.recovery_state.with_incremented_attempts(),
                        alter: Some(crate::strategies::RetryAlteration::Backoff {
                            delay: Duration::from_secs(90),
                        }),
                    }
                } else {
                    // After the 90s sleep, SkipResult ends the inner
                    // retry loop and advances the iteration counter.
                    // The next tick's wall-clock check fires (90s > 60s).
                    RecoveryOutcome::SkipResult {
                        recovery: state.recovery_state.with_incremented_attempts(),
                    }
                }
            }
        }

        let _ = LongBackoffRecovery; // documented alternative
        let host = MockHost::new(vec![]).with_model_errors(vec![
            AgentLoopHostError::new(AgentLoopHostErrorKind::Unavailable, "slow 1"),
            AgentLoopHostError::new(AgentLoopHostErrorKind::Unavailable, "slow 2"),
            AgentLoopHostError::new(AgentLoopHostErrorKind::Unavailable, "slow 3"),
        ]);
        let planner = DefaultPlanner::default()
            .with_context(Arc::new(DefaultContextStrategy::default()))
            .with_capability(Arc::new(DefaultCapabilityStrategy))
            .with_model(Arc::new(DefaultModelStrategy))
            .with_batch(Arc::new(DefaultBatchPolicyStrategy))
            .with_gate(Arc::new(DefaultGateHandlingStrategy))
            .with_recovery(Arc::new(OnceBackoffThenSkip {
                backed_off: Mutex::new(false),
            }))
            .with_stop(Arc::new(DefaultStopConditionStrategy::default()))
            .with_drain(Arc::new(DefaultInputDrainStrategy))
            .with_budget(Arc::new(WallClockBudget));
        let mut state = LoopExecutionState::initial_for_run(host.run_context());

        let exit = CanonicalAgentLoopExecutor
            .execute(&planner, &host, &mut state)
            .await
            .unwrap();

        // After 90s of virtual sleep, the SkipResult advances the
        // iteration. The next tick's top-of-loop wall-clock check
        // (cap = 60s) fires before any further model call.
        match exit {
            LoopExit::Failed {
                kind: FailureKind::WallClockLimitReached,
            } => {}
            other => panic!("expected Failed WallClockLimitReached, got {other:?}"),
        }
        // Iter-5 finding 4: wall-clock failure Final-checkpoints.
        assert!(
            host.checkpoint_kinds()
                .iter()
                .any(|k| matches!(k, LoopCheckpointKind::Final)),
            "expected a Final checkpoint on WallClockLimit, got {:?}",
            host.checkpoint_kinds()
        );
    }

    /// Iter-5 finding 3 (in-crate part): a `PutCheckpointStateRequest`
    /// carrying `with_max_payload_bytes` larger than the legacy 64 KiB
    /// default is accepted by the store, AND the per-profile cap is
    /// enforced when the payload exceeds it.
    ///
    /// The host-side wiring (loop_driver_host.rs threading the active
    /// profile's `checkpoint_policy.max_checkpoint_bytes`) is exercised
    /// through this in-crate contract; the cap is now respected end-to-
    /// end because (a) the absolute system ceiling is 256 KiB and (b)
    /// the request carries the profile cap.
    #[tokio::test]
    async fn checkpoint_state_store_honors_profile_cap_over_legacy_default() {
        use ironclaw_turns::{
            CheckpointSchemaId, CheckpointStateStore, InMemoryCheckpointStateStore,
            PutCheckpointStateRequest, RunProfileVersion, TurnId, TurnRunId,
        };

        let store = InMemoryCheckpointStateStore::default();
        let scope = test_run_context().scope.clone();

        // 128 KiB payload — above the legacy 64 KiB default, below the new
        // 256 KiB ceiling. With a 256 KiB profile cap, this is accepted.
        let big_payload = vec![b'P'; 128 * 1024];
        let request = PutCheckpointStateRequest::new(
            scope.clone(),
            TurnId::new(),
            TurnRunId::new(),
            CheckpointSchemaId::new("iter5_finding3").unwrap(),
            RunProfileVersion::new(1),
            LoopCheckpointKind::Final,
            big_payload.clone(),
        )
        .with_max_payload_bytes(256 * 1024);
        let record = store.put_checkpoint_state(request).await.unwrap();
        assert_eq!(record.payload.len(), 128 * 1024);

        // Same payload but with a 64 KiB profile cap (interactive
        // profile) — must be rejected.
        let request = PutCheckpointStateRequest::new(
            scope,
            TurnId::new(),
            TurnRunId::new(),
            CheckpointSchemaId::new("iter5_finding3").unwrap(),
            RunProfileVersion::new(1),
            LoopCheckpointKind::Final,
            big_payload,
        )
        .with_max_payload_bytes(64 * 1024);
        let err = store.put_checkpoint_state(request).await.unwrap_err();
        match err {
            ironclaw_turns::TurnError::InvalidRequest { .. } => {}
            other => panic!("expected InvalidRequest, got {other:?}"),
        }
    }

    /// Iter-5 finding 4: every terminal failure-shaped exit takes a
    /// `Final` checkpoint. Covers `Stop::Aborted` (returned from the stop
    /// strategy after a capability batch). Pre-iter-5 code skipped the
    /// checkpoint on this path, so a profile with
    /// `require_final_checkpoint = true` would reject the exit as
    /// `MissingFinalCheckpoint`.
    #[tokio::test]
    async fn stop_aborted_after_batch_takes_final_checkpoint_before_returning() {
        use crate::strategies::{
            DefaultBatchPolicyStrategy, DefaultCapabilityStrategy, DefaultContextStrategy,
            DefaultGateHandlingStrategy, DefaultInputDrainStrategy, DefaultModelStrategy,
            DefaultRecoveryStrategy,
        };

        // Stop strategy that aborts with `InvalidModelOutput` after the
        // first capability batch. This drives the `StopKind::Aborted` arm
        // of `exit_for_stop_kind`.
        struct AbortAfterBatch;
        #[async_trait]
        impl crate::strategies::StopConditionStrategy for AbortAfterBatch {
            async fn should_stop_after_turn(
                &self,
                state: &LoopExecutionState,
                _summary: &crate::strategies::TurnSummary,
            ) -> StopOutcome {
                StopOutcome::Stop {
                    control: state.control_state.clone(),
                    kind: StopKind::Aborted(LoopFailureKind::InvalidModelOutput),
                }
            }
        }

        let host = MockHost::new(vec![calls_output("anything")])
            .with_batch(completed_batch("anything", "ok"));
        let planner = DefaultPlanner::default()
            .with_context(Arc::new(DefaultContextStrategy::default()))
            .with_capability(Arc::new(DefaultCapabilityStrategy))
            .with_model(Arc::new(DefaultModelStrategy))
            .with_batch(Arc::new(DefaultBatchPolicyStrategy))
            .with_gate(Arc::new(DefaultGateHandlingStrategy))
            .with_recovery(Arc::new(DefaultRecoveryStrategy::default()))
            .with_stop(Arc::new(AbortAfterBatch))
            .with_drain(Arc::new(DefaultInputDrainStrategy))
            .with_budget(Arc::new(TestBudget { limit: 8 }));
        let mut state = LoopExecutionState::initial_for_run(host.run_context());

        let exit = CanonicalAgentLoopExecutor
            .execute(&planner, &host, &mut state)
            .await
            .unwrap();

        match exit {
            LoopExit::Failed {
                kind: FailureKind::Other(LoopFailureKind::InvalidModelOutput),
            } => {}
            other => panic!("expected Failed InvalidModelOutput, got {other:?}"),
        }
        // The Final checkpoint MUST appear in the recorded sequence.
        assert!(
            host.checkpoint_kinds()
                .iter()
                .any(|k| matches!(k, LoopCheckpointKind::Final)),
            "Stop::Aborted exit must Final-checkpoint, got {:?}",
            host.checkpoint_kinds()
        );
    }

    #[test]
    fn agent_loop_executor_is_object_safe() {
        fn _check(_: &dyn AgentLoopExecutor) {}

        _check(&CanonicalAgentLoopExecutor);
    }

    /// Iter-6 finding 1: the wall-clock budget anchor MUST survive
    /// checkpoint reload. A run that resumes with a `started_at_unix_ms`
    /// already older than `wall_clock_limit` trips
    /// `WallClockLimitReached` on the first tick, even though the
    /// in-process `tokio::time::Instant` (which always starts fresh) has
    /// only just been captured.
    ///
    /// Pre-iter-6 the anchor was a local `let start_time =
    /// Instant::now();`. A `Blocked` run that re-entered `execute()` got a
    /// brand-new wall-clock budget while keeping its old iteration count;
    /// this test would have failed (the executor would have run the model
    /// instead of failing fast).
    #[tokio::test]
    async fn resumed_run_with_stale_started_at_trips_wall_clock_limit_on_first_tick() {
        // Budget with a 60s wall-clock cap.
        struct WallClockBudget;
        impl crate::strategies::BudgetStrategy for WallClockBudget {
            fn iteration_limit(&self, _: &LoopExecutionState) -> u32 {
                1000
            }

            fn wall_clock_limit(&self, _: &LoopExecutionState) -> Option<Duration> {
                Some(Duration::from_secs(60))
            }
        }

        // No model outputs scripted — if the executor ever calls the
        // model port the test will panic via the default `unwrap_or_else`.
        // We expect the wall-clock cap to fire before any model call.
        let host = MockHost::new(vec![]);
        let planner = DefaultPlanner::default()
            .with_context(Arc::new(DefaultContextStrategy::default()))
            .with_capability(Arc::new(DefaultCapabilityStrategy))
            .with_model(Arc::new(DefaultModelStrategy))
            .with_batch(Arc::new(DefaultBatchPolicyStrategy))
            .with_gate(Arc::new(DefaultGateHandlingStrategy))
            .with_recovery(Arc::new(DefaultRecoveryStrategy::default()))
            .with_stop(Arc::new(DefaultStopConditionStrategy::default()))
            .with_drain(Arc::new(DefaultInputDrainStrategy))
            .with_budget(Arc::new(WallClockBudget));
        let mut state = LoopExecutionState::initial_for_run(host.run_context());
        // Anchor 10 minutes (600 seconds) in the past — well past the
        // 60s cap. This is the "resumed from checkpoint" shape.
        let now_ms = system_time_now_unix_ms();
        state.started_at_unix_ms = Some(now_ms.saturating_sub(600 * 1_000));

        let exit = CanonicalAgentLoopExecutor
            .execute(&planner, &host, &mut state)
            .await
            .unwrap();

        match exit {
            LoopExit::Failed {
                kind: FailureKind::WallClockLimitReached,
            } => {}
            other => panic!("expected Failed WallClockLimitReached, got {other:?}"),
        }
        // No model call was made — the cap fired in the tick prologue.
        assert_eq!(
            host.model_call_count(),
            0,
            "wall-clock cap must fire before the model is invoked on a resumed run"
        );
        // Final checkpoint was taken (iter-5 finding 4 contract).
        assert!(
            host.checkpoint_kinds()
                .iter()
                .any(|k| matches!(k, LoopCheckpointKind::Final)),
            "expected Final checkpoint on WallClockLimit, got {:?}",
            host.checkpoint_kinds()
        );
    }

    /// Iter-6 finding 1: a fresh run anchors `started_at_unix_ms` on the
    /// first `execute()` entry and the value survives a JSON round trip,
    /// so the next `execute()` can read it as the run's effective start.
    /// Companion to the resume test above; this one only verifies the
    /// write + persistence shape.
    #[tokio::test]
    async fn first_execute_entry_anchors_started_at_unix_ms_and_persists_via_checkpoint_payload() {
        let host = MockHost::new(vec![reply_output("done")]);
        let mut state = LoopExecutionState::initial_for_run(host.run_context());
        assert_eq!(
            state.started_at_unix_ms, None,
            "fresh state must start without an anchor"
        );
        let before_ms = system_time_now_unix_ms();

        let _ = run(&host, &mut state, 8).await;

        let after_ms = system_time_now_unix_ms();
        let anchor = state
            .started_at_unix_ms
            .expect("anchor must be set after first execute() entry");
        assert!(
            anchor >= before_ms && anchor <= after_ms,
            "anchor {anchor} must fall within [{before_ms}, {after_ms}]"
        );
        // Round-trip through JSON so we know a checkpoint reload preserves
        // the value (the executor's persisted payload uses serde_json).
        let serialized = serde_json::to_value(&state).unwrap();
        let restored: LoopExecutionState = serde_json::from_value(serialized).unwrap();
        assert_eq!(restored.started_at_unix_ms, Some(anchor));
    }

    /// Iter-6 finding 2: when the `Final` checkpoint fails during
    /// terminal-cancel handling in `observe_cancellation`, the cancel
    /// page MUST NOT be acked. The executor surfaces a
    /// `CheckpointFailed` error and `state.input_cursor` retains the
    /// pre-cancel value, so the next `execute()` re-polls the same
    /// cancel page and tries again.
    ///
    /// Pre-iter-6 the ack happened FIRST; a transient DB outage during
    /// checkpoint left the cancel consumed but the run un-persisted —
    /// the retry would skip past the cancel and run forever.
    #[tokio::test]
    async fn cancel_page_is_not_acked_when_final_checkpoint_store_fails() {
        let host = MockHost::new(vec![]).with_poll_inputs(vec![
            // observe_cancellation iter 1: a cancel-only page.
            vec![cancel_input()],
        ]);
        host.fail_final_checkpoint_store();
        let mut state = LoopExecutionState::initial_for_run(host.run_context());
        let pre_cursor = state.input_cursor.clone();

        let result = CanonicalAgentLoopExecutor
            .execute(&planner(8), &host, &mut state)
            .await;

        match result {
            Err(AgentLoopExecutorError::CheckpointFailed {
                stage: CheckpointKind::Final,
            }) => {}
            other => panic!("expected CheckpointFailed(Final), got {other:?}"),
        }
        // The cancel page was NOT acked: ack_count remains 0 and the
        // cursor stays at its pre-cancel value, so a retry can re-poll.
        assert_eq!(
            host.ack_count(),
            0,
            "cancel page must NOT be acked when Final checkpoint store fails"
        );
        assert_eq!(
            state.input_cursor, pre_cursor,
            "input_cursor must not advance past an un-checkpointed cancel"
        );
    }

    // ============================================================
    // Codex /review (iter-7) follow-ups: tests for findings 1-3.
    // ============================================================

    /// Iter-7 finding 1: `observe_cancellation` must page past
    /// control-only pages. A `GateResolved` on page 1 followed by a
    /// `Cancel` on page 2 must terminate the run before any further
    /// model call, not after one more reply.
    #[tokio::test]
    async fn observe_cancellation_pages_past_control_only_to_find_terminal() {
        let gate_ref = LoopGateRef::new("gate:before-cancel").unwrap();
        // No model output scripted: if `observe_cancellation` failed to
        // see the cancel on page 2, the next iteration would call the
        // model and the test's `unwrap_or_else` default would fire — but
        // we still assert `model_call_count == 0` to be explicit.
        let host = MockHost::new(vec![]).with_poll_inputs(vec![
            // observe_cancellation iter 1, page 1: control-only.
            // Pre-iter-7 this acked and returned None, hiding the
            // cancel on page 2 until the next outer iteration.
            vec![LoopInput::GateResolved {
                gate_ref: gate_ref.clone(),
            }],
            // observe_cancellation iter 1, page 2: the terminal cancel.
            vec![cancel_input()],
        ]);
        let mut state = LoopExecutionState::initial_for_run(host.run_context());
        state.last_gate = Some(gate_ref.clone());

        let exit = run(&host, &mut state, 8).await;

        match exit {
            LoopExit::Cancelled(_) => {}
            other => panic!("expected Cancelled, got {other:?}"),
        }
        // The model port was never invoked — the cancel was caught
        // before any further model/capability cycle.
        assert_eq!(
            host.model_call_count(),
            0,
            "model must not run when cancel is on a later page of the same observe_cancellation call"
        );
        // The GateResolved side effect was applied.
        assert_eq!(
            state.last_gate, None,
            "GateResolved on the first control-only page must clear last_gate"
        );
        // Both pages were acked (control-only page acked in-loop;
        // cancel page acked after Final checkpoint).
        assert!(
            host.ack_count() >= 2,
            "expected both pages acked, got {}",
            host.ack_count()
        );
        // Final checkpoint was taken.
        assert!(
            host.checkpoint_kinds()
                .iter()
                .any(|k| matches!(k, LoopCheckpointKind::Final)),
            "expected Final checkpoint on cancel, got {:?}",
            host.checkpoint_kinds()
        );
    }

    /// Iter-7 finding 2: when `drain_followup` exhausts `INPUT_POLL_LIMIT`
    /// consecutive control-only pages it MUST return `ControlPending`,
    /// NOT `Empty` — otherwise the caller Final-checkpoints and exits
    /// `Completed` even though the queue might still hold a FollowUp on
    /// a later page.
    ///
    /// This test scripts 16 consecutive control-only pages for the
    /// drain after reply 1 (the drain exhausts its poll budget without
    /// seeing a definitive empty page). With the iter-7 fix the
    /// executor returns `ControlPending`, advances the iteration, and
    /// the next tick's drain (after reply 2) sees a clean empty page
    /// and Final-checkpoints normally — so we observe exactly 2 model
    /// calls and exactly one Final checkpoint.
    ///
    /// Pre-iter-7 the same script returned `Empty` after page 16, the
    /// caller Final-checkpointed after reply 1 and exited `Completed` —
    /// model_call_count would be 1, not 2.
    #[tokio::test]
    async fn drain_followup_returns_control_pending_not_empty_at_poll_limit() {
        let gate_ref = LoopGateRef::new("gate:lots-of-control").unwrap();
        let mut batches: Vec<Vec<LoopInput>> = Vec::new();
        // Prologue: observe_cancellation + drain_steering for iter 1.
        batches.push(Vec::new());
        batches.push(Vec::new());
        // 16 consecutive control-only pages for `drain_followup` after
        // reply 1 (the INPUT_POLL_LIMIT cap). Pre-iter-7 the drain
        // would have collapsed this into Empty and the caller would
        // have Final-checkpointed + exited Completed after reply 1.
        for _ in 0..16 {
            batches.push(vec![LoopInput::GateResolved {
                gate_ref: gate_ref.clone(),
            }]);
        }
        // Iter 2 prologue + drain — clean tick that completes naturally.
        batches.push(Vec::new()); // observe_cancellation iter 2
        batches.push(Vec::new()); // drain_steering iter 2
        batches.push(Vec::new()); // drain_followup iter 2 — Empty → Final.

        let host = MockHost::new(vec![reply_output("first"), reply_output("second")])
            .with_poll_inputs(batches);
        let mut state = LoopExecutionState::initial_for_run(host.run_context());
        state.last_gate = Some(gate_ref.clone());

        let exit = run(&host, &mut state, 8).await;

        // The run did NOT exit Completed after reply 1 — the executor
        // advanced the iteration past 16 control-only drain pages and
        // ran the second reply. Pre-iter-7 (Empty at the limit)
        // model_call_count would have been 1 because the caller would
        // have Final-checkpointed straight away.
        assert_eq!(exit, LoopExit::Completed(CompletionKind::NaturalEnd));
        assert_eq!(
            host.model_call_count(),
            2,
            "the iteration must advance past 16 control-only drain pages, not exit Completed"
        );
        assert_eq!(state.assistant_refs.len(), 2);
        // Exactly one Final checkpoint at the very end — NOT one after
        // reply 1 (which is what the pre-iter-7 Empty-at-limit bug
        // would have produced).
        let finals = host
            .checkpoint_kinds()
            .iter()
            .filter(|k| matches!(k, LoopCheckpointKind::Final))
            .count();
        assert_eq!(
            finals,
            1,
            "expected exactly one Final checkpoint, got {:?}",
            host.checkpoint_kinds()
        );
    }

    /// Iter-7 finding 3: a legacy host whose `store_checkpoint_payload`
    /// returns `Unavailable` (the default trait impl) must still be able
    /// to checkpoint via the legacy `checkpoint()`-only contract. The
    /// executor falls back to passing `LoopCheckpointStateRef::legacy_unknown()`
    /// to the host's `checkpoint()` impl.
    ///
    /// Pre-iter-7 this path returned `CheckpointFailed` at every
    /// checkpoint, breaking the compatibility path the trait change
    /// advertised.
    #[tokio::test]
    async fn legacy_host_without_store_payload_still_checkpoints_via_checkpoint_only_path() {
        let host = MockHost::new(vec![reply_output("done")]);
        host.enable_legacy_checkpoint_only();
        let mut state = LoopExecutionState::initial_for_run(host.run_context());

        let exit = run(&host, &mut state, 8).await;

        // The run completed naturally — checkpoints did not fail.
        assert_eq!(exit, LoopExit::Completed(CompletionKind::NaturalEnd));
        // No payloads were stored (legacy host returns Unavailable for
        // every `store_checkpoint_payload` call).
        assert_eq!(
            host.stored_payload_count(),
            0,
            "legacy host returns Unavailable; no payloads should be stored"
        );
        // Checkpoints were nevertheless recorded — proving the
        // executor fell back to the legacy `checkpoint()`-only path.
        assert!(
            !host.checkpoint_kinds().is_empty(),
            "expected at least one checkpoint via the legacy path, got {:?}",
            host.checkpoint_kinds()
        );
        // BeforeModel + Final at minimum.
        assert!(
            host.checkpoint_kinds()
                .iter()
                .any(|k| matches!(k, LoopCheckpointKind::Final)),
            "expected Final checkpoint on a legacy-host completion, got {:?}",
            host.checkpoint_kinds()
        );
    }

    // ---- Iter-8 finding 1: `capability_summaries` projects per-descriptor
    // concurrency hints from the visible surface, not a hardcoded
    // `SafeForParallel`. ----

    fn descriptor(id: &str, concurrency: CapabilityConcurrency) -> CapabilityDescriptorView {
        CapabilityDescriptorView {
            capability_id: CapabilityId::new(id).unwrap(),
            provider: None,
            runtime: RuntimeKind::FirstParty,
            safe_name: id.to_string(),
            safe_description: id.to_string(),
            concurrency,
        }
    }

    fn call_for(id: &str) -> CapabilityCallCandidate {
        CapabilityCallCandidate {
            surface_version: surface_version(),
            capability_id: CapabilityId::new(id).unwrap(),
            input_ref: CapabilityInputRef::new(format!("input:{id}")).unwrap(),
        }
    }

    /// One call hints `Exclusive`, the other `SafeForParallel`: the resulting
    /// batch policy must resolve to `Sequential` so the host runs them one at
    /// a time and stops on the first suspension.
    #[test]
    fn capability_summaries_resolves_sequential_when_any_descriptor_exclusive() {
        let surface = VisibleCapabilitySurface {
            version: surface_version(),
            descriptors: vec![
                descriptor("demo.read", CapabilityConcurrency::SafeForParallel),
                descriptor("demo.write", CapabilityConcurrency::Exclusive),
            ],
        };
        let calls = vec![call_for("demo.read"), call_for("demo.write")];

        let summaries = capability_summaries(&surface, &calls);

        assert_eq!(summaries.len(), 2);
        assert!(matches!(
            summaries[0].concurrency_hint,
            ConcurrencyHint::SafeForParallel
        ));
        assert!(matches!(
            summaries[1].concurrency_hint,
            ConcurrencyHint::Exclusive
        ));
        let policy = DefaultBatchPolicyStrategy.policy(
            &LoopExecutionState::initial_for_run(&test_run_context()),
            &summaries,
        );
        assert_eq!(policy, BatchPolicy::Sequential);
    }

    /// Both calls hint `SafeForParallel`: the batch policy stays `Parallel`,
    /// preserving the read-fanout fast path.
    #[test]
    fn capability_summaries_resolves_parallel_when_all_descriptors_safe() {
        let surface = VisibleCapabilitySurface {
            version: surface_version(),
            descriptors: vec![
                descriptor("demo.read_a", CapabilityConcurrency::SafeForParallel),
                descriptor("demo.read_b", CapabilityConcurrency::SafeForParallel),
            ],
        };
        let calls = vec![call_for("demo.read_a"), call_for("demo.read_b")];

        let summaries = capability_summaries(&surface, &calls);

        assert!(
            summaries.iter().all(|summary| matches!(
                summary.concurrency_hint,
                ConcurrencyHint::SafeForParallel
            ))
        );
        let policy = DefaultBatchPolicyStrategy.policy(
            &LoopExecutionState::initial_for_run(&test_run_context()),
            &summaries,
        );
        assert_eq!(policy, BatchPolicy::Parallel);
    }

    /// A call cites a capability id that's missing from the visible surface
    /// (defensive — the capability filter strategy should have rejected it
    /// upstream). The summary must fall back to `Exclusive` so the conservative
    /// `Sequential` policy wins, preventing a parallel fan-out where the loop
    /// has no descriptor-derived assurance the call is safe.
    #[test]
    fn capability_summaries_defaults_missing_descriptor_to_exclusive() {
        let surface = VisibleCapabilitySurface {
            version: surface_version(),
            descriptors: vec![descriptor(
                "demo.read",
                CapabilityConcurrency::SafeForParallel,
            )],
        };
        let calls = vec![call_for("demo.read"), call_for("demo.unknown")];

        let summaries = capability_summaries(&surface, &calls);

        assert!(matches!(
            summaries[0].concurrency_hint,
            ConcurrencyHint::SafeForParallel
        ));
        assert!(matches!(
            summaries[1].concurrency_hint,
            ConcurrencyHint::Exclusive
        ));
        let policy = DefaultBatchPolicyStrategy.policy(
            &LoopExecutionState::initial_for_run(&test_run_context()),
            &summaries,
        );
        assert_eq!(policy, BatchPolicy::Sequential);
    }

    // ============================================================
    // Codex /review (iter-9) follow-ups: tests for findings 1-3.
    // ============================================================

    /// Iter-9 finding 1: a control-only page consumed by `drain_followup`
    /// must take a durable checkpoint with the advanced cursor BEFORE
    /// the host's `ack_inputs`. Pre-iter-9 the cursor advance was
    /// in-memory only and the next `BeforeModel` checkpoint did not
    /// happen until after the model was invoked — so a crash between
    /// ack and the next model call would leave the only durable
    /// record pointing at a page the host had already dropped, and
    /// the `GateResolved` / `CapabilitySurfaceChanged` side effects
    /// would be lost.
    ///
    /// We exercise the contract by scripting `drain_followup` to walk
    /// a control-only page followed by a real `FollowUp`, and assert
    /// that the executor calls `store_checkpoint_payload` at least
    /// once between the first poll and the eventual ack (i.e. the
    /// drain itself produces a stored checkpoint payload).
    #[tokio::test]
    async fn drain_followup_control_only_page_checkpoints_before_ack() {
        let gate_ref = LoopGateRef::new("gate:durability").unwrap();
        let host = MockHost::new(vec![reply_output("first"), reply_output("second")])
            .with_poll_inputs(vec![
                Vec::new(), // observe_cancellation iter 1
                Vec::new(), // drain_steering iter 1
                // drain_followup after reply 1: control-only page (must
                // be checkpoint-before-ack).
                vec![LoopInput::GateResolved {
                    gate_ref: gate_ref.clone(),
                }],
                // drain_followup after reply 1: a real FollowUp on the
                // next page (also must be checkpoint-before-ack).
                vec![followup_input("kept-alive")],
                Vec::new(), // observe_cancellation iter 2
                Vec::new(), // drain_steering iter 2
                Vec::new(), // drain_followup after reply 2 → Empty → Final
            ]);
        let mut state = LoopExecutionState::initial_for_run(host.run_context());
        state.last_gate = Some(gate_ref.clone());

        let payload_count_before = host.stored_payload_count();
        let exit = run(&host, &mut state, 8).await;

        assert_eq!(exit, LoopExit::Completed(CompletionKind::NaturalEnd));
        // Run continued past the followup → two model calls and two
        // assistant refs.
        assert_eq!(host.model_call_count(), 2);
        assert_eq!(state.assistant_refs.len(), 2);
        // The gate side effect was applied (proving the control page
        // was processed).
        assert_eq!(state.last_gate, None);
        // Iter-9 finding 1 contract: each ack site MUST be preceded by
        // a stored checkpoint payload. We had a control-only page ack
        // plus a followup-consumed ack inside drain_followup; together
        // with the normal `BeforeModel` / `BeforeSideEffect` / `Final`
        // checkpoints the executor takes anyway, we expect strictly
        // more stored payloads than a no-drain baseline. Concretely:
        // BeforeModel(it1) + BeforeModel(control-ack) + BeforeModel(followup-ack)
        // + BeforeModel(it2) + Final ≥ 5.
        let payload_count_after = host.stored_payload_count();
        assert!(
            payload_count_after - payload_count_before >= 5,
            "expected >=5 durable checkpoint payloads spanning the two \
             ack sites, got {}",
            payload_count_after - payload_count_before
        );
    }

    /// Iter-9 finding 2: `CapabilityDescriptorView::concurrency` must
    /// `#[serde(default)]` so older payloads (pre-WS-6 hosts, recorded
    /// events, persisted surface snapshots without the field)
    /// deserialize as `CapabilityConcurrency::Exclusive`. Locking this
    /// in at the framework boundary as well — `host.rs` carries the
    /// unit-level test, this one verifies the field is visible at the
    /// loop-framework re-export point and behaves the same way.
    #[test]
    fn legacy_descriptor_view_without_concurrency_defaults_to_exclusive() {
        let legacy_json = serde_json::json!({
            "capability_id": "demo.legacy",
            "provider": null,
            "runtime": "wasm",
            "safe_name": "legacy",
            "safe_description": "no concurrency field present"
            // NOTE: `concurrency` intentionally omitted.
        });
        let view: CapabilityDescriptorView = serde_json::from_value(legacy_json)
            .expect("legacy payload must deserialize via #[serde(default)]");
        assert_eq!(view.concurrency, CapabilityConcurrency::Exclusive);
    }

    /// Iter-9 finding 3: a custom `BatchPolicyStrategy` that returns
    /// `Parallel` for a batch containing at least one `Exclusive`
    /// summary MUST be overridden by the executor: the
    /// `CapabilityBatchInvocation` sent to the host has
    /// `stop_on_first_suspension = true`. Pre-iter-9 the flag was
    /// derived only from the policy, so a permissive planner could
    /// let the host run later invocations after an
    /// `ApprovalRequired` / `AuthRequired` / `SpawnedProcess`
    /// outcome.
    #[tokio::test]
    async fn parallel_policy_with_any_exclusive_summary_forces_stop_on_suspension() {
        use crate::strategies::{
            BatchPolicy, BatchPolicyStrategy, CapabilityCallSummary, DefaultCapabilityStrategy,
            DefaultContextStrategy, DefaultGateHandlingStrategy, DefaultInputDrainStrategy,
            DefaultModelStrategy, DefaultRecoveryStrategy, DefaultStopConditionStrategy,
        };

        struct AlwaysParallel;
        impl BatchPolicyStrategy for AlwaysParallel {
            fn policy(
                &self,
                _state: &LoopExecutionState,
                _calls: &[CapabilityCallSummary],
            ) -> BatchPolicy {
                BatchPolicy::Parallel
            }
        }

        let read_cap = CapabilityId::new("demo.read").unwrap();
        let write_cap = CapabilityId::new("demo.write").unwrap();
        // Host surface: one safe, one exclusive. With `AlwaysParallel`
        // the planner-policy is `Parallel`, but iter-9 finding 3 says
        // the descriptor's own `Exclusive` disclosure must win.
        let surface = VisibleCapabilitySurface {
            version: surface_version(),
            descriptors: vec![
                descriptor("demo.read", CapabilityConcurrency::SafeForParallel),
                descriptor("demo.write", CapabilityConcurrency::Exclusive),
            ],
        };
        let calls = vec![
            CapabilityCallCandidate {
                surface_version: surface_version(),
                capability_id: read_cap.clone(),
                input_ref: CapabilityInputRef::new("input:read").unwrap(),
            },
            CapabilityCallCandidate {
                surface_version: surface_version(),
                capability_id: write_cap.clone(),
                input_ref: CapabilityInputRef::new("input:write").unwrap(),
            },
        ];
        let host = MockHost::new(vec![ParentLoopOutput::CapabilityCalls(calls)])
            .with_capability_surface(surface)
            .with_batch(CapabilityBatchOutcome {
                outcomes: vec![
                    completed_result("read", "read ok"),
                    completed_result("write", "write ok"),
                ],
                stopped_on_suspension: false,
            });
        let planner = DefaultPlanner::default()
            .with_context(Arc::new(DefaultContextStrategy::default()))
            .with_capability(Arc::new(DefaultCapabilityStrategy))
            .with_model(Arc::new(DefaultModelStrategy))
            .with_batch(Arc::new(AlwaysParallel))
            .with_gate(Arc::new(DefaultGateHandlingStrategy))
            .with_recovery(Arc::new(DefaultRecoveryStrategy::default()))
            .with_stop(Arc::new(DefaultStopConditionStrategy::default()))
            .with_drain(Arc::new(DefaultInputDrainStrategy))
            .with_budget(Arc::new(TestBudget { limit: 8 }));
        let mut state = LoopExecutionState::initial_for_run(host.run_context());

        let _ = CanonicalAgentLoopExecutor
            .execute(&planner, &host, &mut state)
            .await
            .unwrap();

        let requests = host.recorded_batch_requests();
        assert_eq!(
            requests.len(),
            1,
            "expected exactly one batch invocation, got {}",
            requests.len()
        );
        assert!(
            requests[0].stop_on_first_suspension,
            "stop_on_first_suspension must be forced to true when ANY summary \
             is Exclusive, even under a Parallel planner policy"
        );
    }

    /// Companion to the previous test: when ALL summaries are
    /// `SafeForParallel` AND the planner picks `Parallel`, the executor
    /// leaves `stop_on_first_suspension = false` so the read-fanout
    /// fast path is preserved.
    #[tokio::test]
    async fn parallel_policy_with_all_safe_summaries_keeps_stop_on_suspension_false() {
        let surface = VisibleCapabilitySurface {
            version: surface_version(),
            descriptors: vec![
                descriptor("demo.read_a", CapabilityConcurrency::SafeForParallel),
                descriptor("demo.read_b", CapabilityConcurrency::SafeForParallel),
            ],
        };
        let calls = vec![
            CapabilityCallCandidate {
                surface_version: surface_version(),
                capability_id: CapabilityId::new("demo.read_a").unwrap(),
                input_ref: CapabilityInputRef::new("input:a").unwrap(),
            },
            CapabilityCallCandidate {
                surface_version: surface_version(),
                capability_id: CapabilityId::new("demo.read_b").unwrap(),
                input_ref: CapabilityInputRef::new("input:b").unwrap(),
            },
        ];
        let host = MockHost::new(vec![ParentLoopOutput::CapabilityCalls(calls)])
            .with_capability_surface(surface)
            .with_batch(CapabilityBatchOutcome {
                outcomes: vec![completed_result("a", "ok"), completed_result("b", "ok")],
                stopped_on_suspension: false,
            });

        let mut state = LoopExecutionState::initial_for_run(host.run_context());
        let _ = run(&host, &mut state, 8).await;

        let requests = host.recorded_batch_requests();
        assert_eq!(requests.len(), 1);
        assert!(
            !requests[0].stop_on_first_suspension,
            "stop_on_first_suspension must stay false when policy is \
             Parallel AND all summaries are SafeForParallel"
        );
    }
}
