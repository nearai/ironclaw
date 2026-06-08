use ironclaw_turns::{
    LoopExit,
    run_profile::{AgentLoopDriverHost, LoopDriverNoteKind, LoopProgressEvent, ParentLoopOutput},
};
use tracing::debug;

use crate::{family::LoopFamily, state::LoopExecutionState, strategies::TurnEndKind};

use super::{
    AgentLoopExecutorError, AssistantReplyInput, BudgetInput, BudgetStep, CancelCheck,
    CapabilityInput, CheckpointInput, CheckpointKind, CheckpointStage, DefaultExecutorPipeline,
    DrainInput, ExecutorStage, ExitInput, InputStep, ModelInput, ModelStep, PendingInputAck,
    PromptInput, PromptStep, ReplyAdmissionInput, ReplyAdmissionStep, StageContext, StopInput,
    StopObservationInput, StopObservationStep, StopStep, TurnCompletedStep,
    UserFacingInputDrainMode,
};

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
            state = match CheckpointStage.cancel_if_requested(ctx, state).await? {
                CancelCheck::Continue(state) => *state,
                CancelCheck::Exit(exit) => return Ok(exit),
            };

            match self
                .budget
                .process(
                    ctx,
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
                .emit_progress(
                    ctx,
                    LoopProgressEvent::IterationStarted {
                        iteration: state.iteration,
                    },
                )
                .await;

            match self
                .input
                .process(
                    ctx,
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

            match self
                .prompt
                .process(
                    ctx,
                    PromptInput {
                        state,
                        pending_input_ack: std::mem::take(&mut pending_input_ack),
                    },
                )
                .await?
            {
                PromptStep::Exit(exit) => return Ok(exit),

                PromptStep::Prepared(prompt) => {
                    let prompt = *prompt;
                    state = prompt.state;
                    pending_input_ack = prompt.pending_input_ack;

                    state = CheckpointStage
                        .process(
                            ctx,
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
                            .emit_progress(
                                ctx,
                                LoopProgressEvent::driver_note(
                                    LoopDriverNoteKind::Planning,
                                    "repeated capability call warning rendered",
                                )
                                .map_err(|_| {
                                    AgentLoopExecutorError::PlannerContract {
                                        detail: "repeated-call warning progress summary was invalid",
                                    }
                                })?,
                            )
                            .await;
                    }
                    pending_input_ack.ack(host).await?;

                    let model_response = match self
                        .model
                        .process(
                            ctx,
                            ModelInput {
                                state,
                                messages: prompt.messages,
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
                            state = *next;
                            continue;
                        }
                        ModelStep::Exit(exit) => return Ok(exit),
                    };

                    // Capture provider-reported usage before the `match` consumes
                    // `model_response`. Only assistant-reply turns feed the
                    // diminishing-returns window (#3841 follow-up F1): a
                    // capability-batch turn produces tool calls, not output
                    // tokens, and would otherwise look like four "no progress"
                    // turns in a row. `None` is "unknown" and must NOT count as
                    // a zero-output turn against the detector.
                    let response_usage = model_response.usage;
                    let completed = match model_response.output {
                        ParentLoopOutput::AssistantReply(reply) => {
                            match self
                                .reply_admission
                                .process(ctx, ReplyAdmissionInput { state, reply })
                                .await?
                            {
                                ReplyAdmissionStep::Accept { state, reply } => {
                                    self.assistant_reply
                                        .process(
                                            ctx,
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
                            self.capabilities
                                .process(
                                    ctx,
                                    CapabilityInput {
                                        state,
                                        surface: prompt.surface,
                                        calls,
                                    },
                                )
                                .await?
                        }
                    };

                    let completed = self.post_capability.process(ctx, completed).await?;

                    let (next_state, summary) = match completed {
                        TurnCompletedStep::Continue { state, summary } => (*state, summary),
                        TurnCompletedStep::Exit(exit) => return Ok(exit),
                    };
                    let completed_kind = summary.kind;

                    let (mut next_state, summary) = match self
                        .stop
                        .observe(
                            ctx,
                            StopObservationInput {
                                state: next_state,
                                summary,
                            },
                        )
                        .await?
                    {
                        StopObservationStep::Continue { state, summary } => (*state, summary),
                        StopObservationStep::Exit(exit) => return Ok(exit),
                    };

                    if completed_kind == TurnEndKind::ReplyOnly {
                        debug!(
                            iteration = next_state.iteration,
                            "agent loop checking follow-up input after reply-only turn end"
                        );
                        match self
                            .input
                            .process(
                                ctx,
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
                                    state = next_state;
                                    continue;
                                }
                            }
                            InputStep::Exit(exit) => return Ok(exit),
                        }
                    }

                    match self
                        .stop
                        .decide(
                            ctx,
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
                            let exit = self.exit.process(ctx, ExitInput { state, kind }).await?;
                            ack.ack(host).await?;
                            return Ok(exit);
                        }
                        StopStep::Continue {
                            state: next,
                            pending_input_ack: ack,
                        } => {
                            state = next;
                            pending_input_ack = ack;
                        }
                        StopStep::Exit(exit) => return Ok(exit),
                    }

                    state.iteration = state.iteration.saturating_add(1);
                }

                PromptStep::SkipModel(skipped_state) => {
                    // Compaction-only turn: the prompt stage ran compaction and
                    // set skip_model_this_iteration. Bypass CheckpointStage
                    // (BeforeModel), pending_input_ack.ack, ModelStage,
                    // reply/capability, and PostCapabilityStage entirely. Route
                    // the synthetic summary through stop.observe + stop.decide
                    // so noprogress detection sees the turn (without counting it
                    // as AfterCapabilityBatch evidence).
                    let skipped_state = *skipped_state;
                    let summary = crate::strategies::TurnSummary::compaction_only();

                    let (mut next_state, summary) = match self
                        .stop
                        .observe(
                            ctx,
                            StopObservationInput {
                                state: skipped_state,
                                summary,
                            },
                        )
                        .await?
                    {
                        StopObservationStep::Continue { state, summary } => (*state, summary),
                        StopObservationStep::Exit(exit) => return Ok(exit),
                    };

                    // Follow-up drain is skipped: CompactionOnly != ReplyOnly,
                    // so the condition is naturally false here.

                    match self
                        .stop
                        .decide(
                            ctx,
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
                            let exit = self.exit.process(ctx, ExitInput { state, kind }).await?;
                            ack.ack(host).await?;
                            return Ok(exit);
                        }
                        StopStep::Continue {
                            state: next,
                            pending_input_ack: ack,
                        } => {
                            next_state = next;
                            pending_input_ack = ack;
                        }
                        StopStep::Exit(exit) => return Ok(exit),
                    }

                    next_state.iteration = next_state.iteration.saturating_add(1);
                    state = next_state;
                    continue;
                }
            }
        }
    }
}
