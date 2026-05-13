//! Canonical executor tick (master spec §8): the inherent
//! `execute_canonical` method that the `AgentLoopExecutor` trait impl
//! delegates to.

use ironclaw_turns::run_profile::{
    AgentLoopDriverHost, LoopModelRequest, VisibleCapabilityRequest,
};

use crate::{
    planner::AgentLoopPlanner,
    state::{CheckpointKind, LoopExecutionState},
    strategies::{StopOutcome, TurnEndKind, TurnSummary},
};

use super::capability::apply_capability_filter;
use super::drain::FollowupDrainOutcome;
use super::model::{ModelStep, model_preference_id};
use super::util::{system_time_now_unix_ms, wall_clock_limit_exceeded};
use super::{
    AgentLoopExecutorError, CancelledKind, CanonicalAgentLoopExecutor, CompletionKind, FailureKind,
    HostStage, LoopExit,
};

pub(super) enum Step {
    Continue(LoopExecutionState),
    Exit(LoopExecutionState, LoopExit),
}

impl CanonicalAgentLoopExecutor {
    pub(super) async fn execute_canonical(
        &self,
        planner: &dyn AgentLoopPlanner,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        state: &mut LoopExecutionState,
    ) -> Result<LoopExit, AgentLoopExecutorError> {
        let mut next = state.clone();
        // The persisted `started_at_unix_ms` anchor survives `Blocked` /
        // process restart / checkpoint reload, while this in-process
        // `tokio::time::Instant` does not. Both are consulted at the top
        // of every tick so a fresh run anchors at the same moment and a
        // resumed run with an already-old `started_at_unix_ms` trips
        // the cap immediately rather than getting a brand-new budget.
        let start_time = tokio::time::Instant::now();
        if next.started_at_unix_ms.is_none() {
            next.started_at_unix_ms = Some(system_time_now_unix_ms());
        }

        loop {
            if next.iteration >= planner.budget().iteration_limit(&next) {
                // Take `Final` before failing so profiles with
                // `require_final_checkpoint = true` don't reject the
                // failure as `MissingFinalCheckpoint`.
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
                    // A recovery `SkipResult` on a persistent model error
                    // must advance the iteration counter so the outer cap
                    // eventually trips. Drop the cached surface version
                    // and tick the counter; otherwise a SkipResult-returning
                    // recovery against a persistent failure spins forever.
                    next = skip_state;
                    next.surface_version = None;
                    next.iteration = next.iteration.saturating_add(1);
                    *state = next.clone();
                    continue;
                }
                ModelStep::Exit(exit_state, exit) => {
                    // A `Failed` terminal exit must carry a `Final`
                    // checkpoint. `Cancelled` already took one inside
                    // `invoke_model_with_recovery`'s Cancelled branch.
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
                                    // Take `Final` BEFORE acking the page
                                    // so a checkpoint failure leaves the
                                    // cancel re-pollable on next
                                    // `execute()`. Advance the cursor
                                    // BEFORE the checkpoint so the durable
                                    // Final state names the post-cancel
                                    // position; otherwise resume re-polls
                                    // a page the host already dropped.
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
                                    // Drain hit `INPUT_POLL_LIMIT`
                                    // consecutive control-only pages.
                                    // Side effects were applied + acked
                                    // but a FollowUp may sit on a later
                                    // page, so do NOT Final-checkpoint
                                    // or exit `Completed` — advance the
                                    // iteration so the next tick keeps
                                    // draining.
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
                            // `Failed` shape Final-checkpoints here.
                            // `Blocked` already took `BeforeBlock` in
                            // `handle_gate` (per spec, blocked exits
                            // checkpoint BeforeBlock, not Final).
                            // `Cancelled` already took `Final` in the
                            // capability retry's Cancelled branch.
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
                        // Take `Final` on the no-progress path so profiles
                        // with `require_final_checkpoint = true` accept the
                        // exit instead of rejecting it as
                        // `MissingFinalCheckpoint`.
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
