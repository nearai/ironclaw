use ironclaw_turns::{
    LoopExit,
    run_profile::{AgentLoopDriverHost, LoopDriverNoteKind, LoopProgressEvent, ParentLoopOutput},
};
use tracing::debug;

use crate::{family::LoopFamily, state::LoopExecutionState, strategies::TurnEndKind};

use super::{
    AgentLoopExecutorError, ApprovalResumePromptOutput, AssistantReplyInput, BudgetInput,
    BudgetStep, CancelCheck, CapabilityInput, CheckpointInput, CheckpointKind, CheckpointStage,
    DefaultExecutorPipeline, DrainInput, ExitInput, InputStep, ModelInput, ModelStep,
    PendingInputAck, PromptInput, PromptOutput, PromptStep, ReplyAdmissionInput,
    ReplyAdmissionStep, StageContext, StopInput, StopObservationInput, StopObservationStep,
    StopStep, TurnCompletedStep, UserFacingInputDrainMode, timed,
};

/// Outcome of a single dispatched turn (Prepared / Resume* / SkipModel):
/// either the loop keeps running with an updated state, or the run is over.
/// The three per-branch methods below return this uniformly so `execute()`'s
/// dispatcher can treat all three branches the same way instead of each
/// inlining its own continue/return control flow.
enum TurnStep {
    Continue {
        state: Box<LoopExecutionState>,
        pending_input_ack: PendingInputAck,
    },
    Exit(LoopExit),
}

impl DefaultExecutorPipeline {
    pub(super) async fn execute(
        &self,
        family: &LoopFamily,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        mut state: LoopExecutionState,
    ) -> Result<LoopExit, AgentLoopExecutorError> {
        let planner = family.planner();
        let ctx = StageContext { planner, host };
        let mut pending_input_ack = PendingInputAck::default();

        loop {
            state = match CheckpointStage
                .cancel_if_requested_timed("cancel_check", ctx, state.iteration, state)
                .await?
            {
                CancelCheck::Continue(state) => *state,
                CancelCheck::Exit(exit) => return Ok(exit),
            };

            match timed(
                &self.budget,
                "budget",
                ctx,
                state.iteration,
                BudgetInput {
                    state,
                    pending_input_ack: std::mem::take(&mut pending_input_ack),
                },
            )
            .await?
            {
                BudgetStep::Continue {
                    state: next,
                    pending_input_ack: ack,
                } => {
                    state = *next;
                    pending_input_ack = ack;
                }
                BudgetStep::Exit(exit) => return Ok(exit),
            }

            CheckpointStage
                .emit_progress_timed(
                    "emit_iteration_started",
                    ctx,
                    state.iteration,
                    LoopProgressEvent::IterationStarted {
                        iteration: state.iteration,
                    },
                )
                .await;

            match timed(
                &self.input,
                "input_drain_steering",
                ctx,
                state.iteration,
                DrainInput {
                    state,
                    pending_input_ack: std::mem::take(&mut pending_input_ack),
                    mode: UserFacingInputDrainMode::Steering,
                },
            )
            .await?
            {
                InputStep::Continue {
                    state: next,
                    pending_input_ack: ack,
                    ..
                } => {
                    state = *next;
                    pending_input_ack = ack;
                }
                InputStep::Exit(exit) => return Ok(exit),
            }

            let turn_step = match timed(
                &self.prompt,
                "prompt",
                ctx,
                state.iteration,
                PromptInput {
                    state,
                    pending_input_ack: std::mem::take(&mut pending_input_ack),
                },
            )
            .await?
            {
                PromptStep::Exit(exit) => return Ok(exit),
                PromptStep::Prepared(prompt) => self.execute_prepared_turn(ctx, *prompt).await?,
                PromptStep::ResumeApproval(resume)
                | PromptStep::ResumeAuth(resume)
                | PromptStep::ResumeExternalTool(resume) => {
                    self.execute_resume_turn(ctx, *resume).await?
                }
                PromptStep::SkipModel(skipped_state, ack) => {
                    self.execute_skip_model_turn(ctx, *skipped_state, ack)
                        .await?
                }
            };

            match turn_step {
                TurnStep::Continue {
                    state: next,
                    pending_input_ack: ack,
                } => {
                    state = *next;
                    pending_input_ack = ack;
                }
                TurnStep::Exit(exit) => return Ok(exit),
            }
        }
    }

    /// The main per-iteration turn: a prompt was built and the model was
    /// called. Handles the assistant-reply/capability-batch split, stop
    /// decision (with the completion-nudge check), and exit.
    async fn execute_prepared_turn(
        &self,
        ctx: StageContext<'_>,
        prompt: PromptOutput,
    ) -> Result<TurnStep, AgentLoopExecutorError> {
        let host = ctx.host;
        let mut state = prompt.state;
        let mut pending_input_ack = prompt.pending_input_ack;

        state = timed(
            &CheckpointStage,
            "checkpoint_before_model",
            ctx,
            state.iteration,
            CheckpointInput {
                state,
                kind: CheckpointKind::BeforeModel,
            },
        )
        .await?
        .state;
        if prompt.rendered_repeated_call_warning {
            state.stop_state.mark_repeated_call_warning_rendered();
            CheckpointStage
                .emit_progress_timed(
                    "emit_repeated_call_warning",
                    ctx,
                    state.iteration,
                    LoopProgressEvent::driver_note(
                        LoopDriverNoteKind::Planning,
                        "repeated capability call warning rendered",
                    )
                    .map_err(|_| AgentLoopExecutorError::PlannerContract {
                        detail: "repeated-call warning progress summary was invalid",
                    })?,
                )
                .await;
        }
        pending_input_ack
            .ack_timed("ack_pending_input_before_model", host, state.iteration)
            .await?;

        let model_response = match timed(
            &self.model,
            "model",
            ctx,
            state.iteration,
            ModelInput {
                state,
                messages: prompt.messages,
                inline_messages: prompt.inline_messages,
                surface_version: prompt.surface.version.clone(),
                capability_view: prompt.capability_view,
            },
        )
        .await?
        {
            ModelStep::Response(next, response) => {
                state = *next;
                response
            }
            ModelStep::RetryIteration(next) => {
                return Ok(TurnStep::Continue {
                    state: next,
                    pending_input_ack,
                });
            }
            ModelStep::Exit(exit) => return Ok(TurnStep::Exit(exit)),
        };

        // Capture provider-reported usage before the `match` consumes
        // `model_response`. Only assistant-reply turns feed the
        // diminishing-returns window (#3841 follow-up F1): a
        // capability-batch turn produces tool calls, not output
        // tokens, and would otherwise look like four "no progress"
        // turns in a row. `None` is "unknown" and must NOT count as
        // a zero-output turn against the detector.
        let response_usage = model_response.usage;
        // Accumulate the run's cumulative usage for EVERY model
        // response, before branching on its output. A capability
        // batch also carries `response_usage`; recording it only on
        // the assistant-reply branch would drop the token/cost for
        // every tool-using turn.
        state.accumulate_model_usage(response_usage);
        let turn_iteration = state.iteration;
        let completed = match model_response.output {
            ParentLoopOutput::AssistantReply(reply) => {
                match timed(
                    &self.reply_admission,
                    "reply_admission",
                    ctx,
                    turn_iteration,
                    ReplyAdmissionInput { state, reply },
                )
                .await?
                {
                    ReplyAdmissionStep::Accept { state, reply } => {
                        timed(
                            &self.assistant_reply,
                            "assistant_reply",
                            ctx,
                            turn_iteration,
                            AssistantReplyInput {
                                state: *state,
                                reply,
                                usage: response_usage,
                            },
                        )
                        .await?
                    }
                    ReplyAdmissionStep::Reject { state } => TurnCompletedStep::Continue {
                        state,
                        summary: crate::strategies::TurnSummary::reply_rejected(),
                    },
                }
            }
            ParentLoopOutput::CapabilityCalls(calls) => {
                timed(
                    &self.capabilities,
                    "capabilities",
                    ctx,
                    turn_iteration,
                    CapabilityInput {
                        state,
                        surface: prompt.surface,
                        calls,
                    },
                )
                .await?
            }
        };

        let completed_iteration = completed.iteration_or(turn_iteration);
        let completed = timed(
            &self.post_capability,
            "post_capability",
            ctx,
            completed_iteration,
            completed,
        )
        .await?;

        let (next_state, summary) = match completed {
            TurnCompletedStep::Continue { state, summary } => (*state, summary),
            TurnCompletedStep::Exit(exit) => return Ok(TurnStep::Exit(exit)),
        };
        let completed_kind = summary.kind;

        let (mut next_state, summary) = match self
            .stop
            .observe_timed(
                "stop_observe",
                ctx,
                next_state.iteration,
                StopObservationInput {
                    state: next_state,
                    summary,
                },
            )
            .await?
        {
            StopObservationStep::Continue { state, summary } => (*state, summary),
            StopObservationStep::Exit(exit) => return Ok(TurnStep::Exit(exit)),
        };

        if completed_kind == TurnEndKind::ReplyOnly {
            debug!(
                iteration = next_state.iteration,
                "agent loop checking follow-up input after reply-only turn end"
            );
            match timed(
                &self.input,
                "input_drain_follow_up",
                ctx,
                next_state.iteration,
                DrainInput {
                    state: next_state,
                    pending_input_ack: std::mem::take(&mut pending_input_ack),
                    mode: UserFacingInputDrainMode::FollowUp,
                },
            )
            .await?
            {
                InputStep::Continue {
                    state: next,
                    pending_input_ack: ack,
                    drained,
                } => {
                    next_state = *next;
                    pending_input_ack = ack;
                    if drained {
                        debug!(
                            iteration = next_state.iteration,
                            "agent loop continuing after queued follow-up input"
                        );
                        next_state.iteration = next_state.iteration.saturating_add(1);
                        return Ok(TurnStep::Continue {
                            state: Box::new(next_state),
                            pending_input_ack,
                        });
                    }
                }
                InputStep::Exit(exit) => return Ok(TurnStep::Exit(exit)),
            }
        }

        match self
            .stop
            .decide_with_completion_nudge_timed(
                "stop_decide",
                ctx,
                next_state.iteration,
                StopInput {
                    state: next_state,
                    summary,
                    pending_input_ack: std::mem::take(&mut pending_input_ack),
                },
            )
            .await?
        {
            StopStep::Stop {
                state: stop_state,
                kind,
                pending_input_ack: mut ack,
            } => {
                let exit_iteration = stop_state.iteration;
                let exit = timed(
                    &self.exit,
                    "exit",
                    ctx,
                    exit_iteration,
                    ExitInput {
                        state: stop_state,
                        kind,
                    },
                )
                .await?;
                ack.ack_timed("ack_pending_input_before_exit", host, exit_iteration)
                    .await?;
                Ok(TurnStep::Exit(exit))
            }
            StopStep::Continue {
                state: mut next,
                pending_input_ack: ack,
            } => {
                next.iteration = next.iteration.saturating_add(1);
                Ok(TurnStep::Continue {
                    state: Box::new(next),
                    pending_input_ack: ack,
                })
            }
            StopStep::Exit(exit) => Ok(TurnStep::Exit(exit)),
        }
    }

    /// Re-dispatch of a parked approval/auth/external-tool capability call,
    /// without a model turn.
    async fn execute_resume_turn(
        &self,
        ctx: StageContext<'_>,
        resume: ApprovalResumePromptOutput,
    ) -> Result<TurnStep, AgentLoopExecutorError> {
        let host = ctx.host;
        let resume_iteration = resume.state.iteration;
        let mut pending_input_ack = resume.pending_input_ack;
        pending_input_ack
            .ack_timed(
                "ack_pending_input_before_resume_capability",
                host,
                resume_iteration,
            )
            .await?;
        let completed = timed(
            &self.capabilities,
            "capabilities_resume",
            ctx,
            resume_iteration,
            CapabilityInput {
                state: resume.state,
                surface: resume.surface,
                calls: vec![resume.call],
            },
        )
        .await?;

        let completed_iteration = completed.iteration_or(resume_iteration);
        let completed = timed(
            &self.post_capability,
            "post_capability_resume",
            ctx,
            completed_iteration,
            completed,
        )
        .await?;

        let (next_state, summary) = match completed {
            TurnCompletedStep::Continue { state, summary } => (*state, summary),
            TurnCompletedStep::Exit(exit) => return Ok(TurnStep::Exit(exit)),
        };

        let (next_state, summary) = match self
            .stop
            .observe_timed(
                "stop_observe_resume",
                ctx,
                next_state.iteration,
                StopObservationInput {
                    state: next_state,
                    summary,
                },
            )
            .await?
        {
            StopObservationStep::Continue { state, summary } => (*state, summary),
            StopObservationStep::Exit(exit) => return Ok(TurnStep::Exit(exit)),
        };

        match self
            .stop
            .decide_timed(
                "stop_decide_resume",
                ctx,
                next_state.iteration,
                StopInput {
                    state: next_state,
                    summary,
                    pending_input_ack: std::mem::take(&mut pending_input_ack),
                },
            )
            .await?
        {
            StopStep::Stop {
                state,
                kind,
                pending_input_ack: mut ack,
            } => {
                let exit_iteration = state.iteration;
                let exit = timed(
                    &self.exit,
                    "exit_resume",
                    ctx,
                    exit_iteration,
                    ExitInput { state, kind },
                )
                .await?;
                ack.ack_timed("ack_pending_input_before_exit_resume", host, exit_iteration)
                    .await?;
                Ok(TurnStep::Exit(exit))
            }
            StopStep::Continue {
                state: mut next,
                pending_input_ack: ack,
            } => {
                next.iteration = next.iteration.saturating_add(1);
                Ok(TurnStep::Continue {
                    state: Box::new(next),
                    pending_input_ack: ack,
                })
            }
            StopStep::Exit(exit) => Ok(TurnStep::Exit(exit)),
        }
    }

    /// Compaction-only turn: the prompt stage ran compaction and set
    /// `skip_model_this_iteration`. Bypasses `CheckpointStage` (`BeforeModel`),
    /// `ModelStage`, reply/capability, and `PostCapabilityStage` entirely.
    /// Routes the synthetic summary through `stop.observe` + `stop.decide` so
    /// no-progress detection sees the turn (without counting it as
    /// `AfterCapabilityBatch` evidence). Follow-up drain is skipped:
    /// `CompactionOnly != ReplyOnly`, so that condition is naturally false.
    async fn execute_skip_model_turn(
        &self,
        ctx: StageContext<'_>,
        skipped_state: LoopExecutionState,
        ack: PendingInputAck,
    ) -> Result<TurnStep, AgentLoopExecutorError> {
        let host = ctx.host;
        // Restore the inbound ack that `PromptStep::SkipModel` carries
        // (`PromptCompactionStep` only acks on Compacted; the Skipped branch
        // returns without acking, so without this the ack would be lost).
        let mut pending_input_ack = ack;
        // Deliver the ack before stop.observe, mirroring the timing of the
        // Prepared path (ack before ModelStage).
        pending_input_ack
            .ack_timed(
                "ack_pending_input_skip_model",
                host,
                skipped_state.iteration,
            )
            .await?;
        let summary = crate::strategies::TurnSummary::compaction_only();

        let (next_state, summary) = match self
            .stop
            .observe_timed(
                "stop_observe_skip_model",
                ctx,
                skipped_state.iteration,
                StopObservationInput {
                    state: skipped_state,
                    summary,
                },
            )
            .await?
        {
            StopObservationStep::Continue { state, summary } => (*state, summary),
            StopObservationStep::Exit(exit) => return Ok(TurnStep::Exit(exit)),
        };

        match self
            .stop
            .decide_timed(
                "stop_decide_skip_model",
                ctx,
                next_state.iteration,
                StopInput {
                    state: next_state,
                    summary,
                    pending_input_ack: std::mem::take(&mut pending_input_ack),
                },
            )
            .await?
        {
            StopStep::Stop {
                state,
                kind,
                pending_input_ack: mut ack,
            } => {
                let exit_iteration = state.iteration;
                let exit = timed(
                    &self.exit,
                    "exit_skip_model",
                    ctx,
                    exit_iteration,
                    ExitInput { state, kind },
                )
                .await?;
                ack.ack_timed(
                    "ack_pending_input_before_exit_skip_model",
                    host,
                    exit_iteration,
                )
                .await?;
                Ok(TurnStep::Exit(exit))
            }
            StopStep::Continue {
                state: mut next,
                pending_input_ack: ack,
            } => {
                // N3 analysis: do NOT ack here. The ack returned by
                // stop.decide is the token for the *next* iteration's input,
                // not for the current one. The current-turn ack was already
                // consumed above (before stop.observe), mirroring the
                // Prepared path's ack before ModelStage. After stop.decide,
                // both paths defer the returned ack to the next iteration via
                // pending_input_ack; the next iteration's checkpoint drains
                // it via std::mem::take. Acking here would double-consume the
                // token before the next iteration has a chance to use it.
                next.iteration = next.iteration.saturating_add(1);
                Ok(TurnStep::Continue {
                    state: Box::new(next),
                    pending_input_ack: ack,
                })
            }
            StopStep::Exit(exit) => Ok(TurnStep::Exit(exit)),
        }
    }
}
