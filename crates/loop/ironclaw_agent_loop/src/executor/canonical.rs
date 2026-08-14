use ironclaw_loop_contracts::{
    AgentLoopDriverHost, LoopDriverNoteKind, LoopExit, LoopProgressEvent, ParentLoopOutput,
};
use tracing::debug;

use crate::{family::LoopFamily, state::LoopExecutionState, strategies::TurnEndKind};

use super::{
    AgentLoopExecutorError, AssistantReplyInput, BudgetInput, BudgetStep, COMPLETION_NUDGE_LIMIT,
    CancelCheck, CapabilityInput, CheckpointInput, CheckpointKind, CheckpointStage,
    DefaultExecutorPipeline, DrainInput, ExecutorStage, ExitInput, InputStep, ModelInput,
    ModelStep, PromptInput, PromptStep, ReplyAdmissionInput, ReplyAdmissionStep, StageContext,
    StopInput, StopKind, StopObservationInput, StopObservationStep, StopStep, TurnCompletedStep,
    UserFacingInputDrainMode, latency, scheduled_trigger_run,
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

        loop {
            state = match latency::stage!(
                "cancel_check",
                host.run_context(),
                state.iteration,
                CheckpointStage.cancel_if_requested(ctx, state),
            )? {
                CancelCheck::Continue(state) => *state,
                CancelCheck::Exit(exit) => return Ok(exit),
            };

            match latency::stage!(
                "budget",
                host.run_context(),
                state.iteration,
                self.budget.process(ctx, BudgetInput { state },),
            )? {
                BudgetStep::Continue { state: next } => {
                    state = *next;
                }
                BudgetStep::Exit(exit) => return Ok(exit),
            }

            let progress_started_at = latency::started_at();
            CheckpointStage
                .emit_progress(
                    ctx,
                    LoopProgressEvent::IterationStarted {
                        iteration: state.iteration,
                    },
                )
                .await;
            latency::operation_ok(
                "emit_iteration_started",
                host.run_context(),
                state.iteration,
                progress_started_at,
            );

            match latency::stage!(
                "input_drain_steering",
                host.run_context(),
                state.iteration,
                self.input.process(
                    ctx,
                    DrainInput {
                        state,
                        mode: UserFacingInputDrainMode::Steering,
                    },
                ),
            )? {
                InputStep::Continue { state: next, .. } => {
                    state = *next;
                }
                InputStep::Exit(exit) => return Ok(exit),
            }

            match latency::stage!(
                "prompt",
                host.run_context(),
                state.iteration,
                self.prompt.process(ctx, PromptInput { state },),
            )? {
                PromptStep::Exit(exit) => return Ok(exit),

                PromptStep::Prepared(prompt) => {
                    let prompt = *prompt;
                    state = prompt.state;

                    state = latency::stage!(
                        "checkpoint_before_model",
                        host.run_context(),
                        state.iteration,
                        CheckpointStage.process(
                            ctx,
                            CheckpointInput {
                                state,
                                kind: CheckpointKind::BeforeModel,
                            },
                        ),
                    )?
                    .state;
                    if prompt.rendered_repeated_call_warning {
                        state.stop_state.mark_repeated_call_warning_rendered();
                        let note_started_at = latency::started_at();
                        CheckpointStage
                            .emit_progress(
                                ctx,
                                LoopProgressEvent::driver_note(
                                    LoopDriverNoteKind::Planning,
                                    "repeated capability call warning rendered",
                                )
                                .map_err(|_| AgentLoopExecutorError::PlannerContract {
                                    detail: "repeated-call warning progress summary was invalid",
                                })?,
                            )
                            .await;
                        latency::operation_ok(
                            "emit_repeated_call_warning",
                            host.run_context(),
                            state.iteration,
                            note_started_at,
                        );
                    }
                    let model_response = match latency::stage!(
                        "model",
                        host.run_context(),
                        state.iteration,
                        self.model.process(
                            ctx,
                            ModelInput {
                                state,
                                messages: prompt.messages,
                                inline_messages: prompt.inline_messages,
                                surface_version: prompt.surface.version.clone(),
                                capability_view: prompt.capability_view,
                            },
                        ),
                    )? {
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

                    // Capture provider-reported usage before the `match`
                    // consumes `model_response`.
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
                            match latency::stage!(
                                "reply_admission",
                                host.run_context(),
                                turn_iteration,
                                self.reply_admission
                                    .process(ctx, ReplyAdmissionInput { state, reply }),
                            )? {
                                ReplyAdmissionStep::Accept { state, reply } => latency::stage!(
                                    "assistant_reply",
                                    host.run_context(),
                                    turn_iteration,
                                    self.assistant_reply.process(
                                        ctx,
                                        AssistantReplyInput {
                                            state: *state,
                                            reply,
                                        },
                                    ),
                                )?,
                                ReplyAdmissionStep::Reject { state } => {
                                    TurnCompletedStep::Continue {
                                        state,
                                        summary: crate::strategies::TurnSummary::reply_rejected(),
                                    }
                                }
                            }
                        }
                        ParentLoopOutput::CapabilityCalls(calls) => latency::stage!(
                            "capabilities",
                            host.run_context(),
                            turn_iteration,
                            self.capabilities.process(
                                ctx,
                                CapabilityInput {
                                    state,
                                    surface: prompt.surface,
                                    calls,
                                },
                            ),
                        )?,
                    };

                    let completed = latency::stage!(
                        "post_capability",
                        host.run_context(),
                        completed.iteration_or(turn_iteration),
                        self.post_capability.process(ctx, completed),
                    )?;

                    let (next_state, summary) = match completed {
                        TurnCompletedStep::Continue { state, summary } => (*state, summary),
                        TurnCompletedStep::Exit(exit) => return Ok(exit),
                    };
                    let completed_kind = summary.kind;

                    let (mut next_state, summary) = match latency::stage!(
                        "stop_observe",
                        host.run_context(),
                        next_state.iteration,
                        self.stop.observe(
                            ctx,
                            StopObservationInput {
                                state: next_state,
                                summary,
                            },
                        ),
                    )? {
                        StopObservationStep::Continue { state, summary } => (*state, summary),
                        StopObservationStep::Exit(exit) => return Ok(exit),
                    };

                    if completed_kind == TurnEndKind::ReplyOnly {
                        debug!(
                            iteration = next_state.iteration,
                            "agent loop checking follow-up input after reply-only turn end"
                        );
                        match latency::stage!(
                            "input_drain_follow_up",
                            host.run_context(),
                            next_state.iteration,
                            self.input.process(
                                ctx,
                                DrainInput {
                                    state: next_state,
                                    mode: UserFacingInputDrainMode::FollowUp,
                                },
                            ),
                        )? {
                            InputStep::Continue {
                                state: next,
                                drained,
                            } => {
                                next_state = *next;
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

                    match latency::stage!(
                        "stop_decide",
                        host.run_context(),
                        next_state.iteration,
                        self.stop.decide(
                            ctx,
                            StopInput {
                                state: next_state,
                                summary,
                            },
                        ),
                    )? {
                        StopStep::Stop {
                            state: mut stop_state,
                            kind,
                        } => {
                            if completion_nudge_should_fire(host, &stop_state, &kind) {
                                // Instead of terminating, re-enter the loop for one
                                // more iteration with the full tool surface and a
                                // completion-nudge directive, so the model can finish
                                // the task (e.g. write a required output file) before
                                // answering. Mirrors the drained-follow-up continue
                                // rather than terminating.
                                stop_state.completion_nudges_used += 1;
                                stop_state.completion_nudge_pending = true;
                                stop_state.last_reply_trailed_off = false;
                                stop_state.last_reply_empty = false;
                                stop_state.last_reply_ended_with_question = false;
                                debug!(
                                    iteration = stop_state.iteration,
                                    ?kind,
                                    completion_nudges_used = stop_state.completion_nudges_used,
                                    "agent loop issuing tools-capable completion nudge instead of stopping"
                                );
                                state = stop_state;
                            } else {
                                let exit_iteration = stop_state.iteration;
                                let exit = latency::stage!(
                                    "exit",
                                    host.run_context(),
                                    exit_iteration,
                                    self.exit.process(
                                        ctx,
                                        ExitInput {
                                            state: stop_state,
                                            kind,
                                        },
                                    ),
                                )?;
                                let _ = exit_iteration;
                                return Ok(exit);
                            }
                        }
                        StopStep::Continue { state: next } => {
                            state = next;
                        }
                        StopStep::Exit(exit) => return Ok(exit),
                    }

                    state.iteration = state.iteration.saturating_add(1);
                }

                PromptStep::ResumeApproval(resume)
                | PromptStep::ResumeAuth(resume)
                | PromptStep::ResumeExternalTool(resume) => {
                    let resume = *resume;
                    let resume_iteration = resume.state.iteration;
                    let completed = latency::stage!(
                        "capabilities_resume",
                        host.run_context(),
                        resume_iteration,
                        self.capabilities.process(
                            ctx,
                            CapabilityInput {
                                state: resume.state,
                                surface: resume.surface,
                                calls: vec![resume.call],
                            },
                        ),
                    )?;

                    let completed = latency::stage!(
                        "post_capability_resume",
                        host.run_context(),
                        completed.iteration_or(resume_iteration),
                        self.post_capability.process(ctx, completed),
                    )?;

                    let (next_state, summary) = match completed {
                        TurnCompletedStep::Continue { state, summary } => (*state, summary),
                        TurnCompletedStep::Exit(exit) => return Ok(exit),
                    };

                    let (next_state, summary) = match latency::stage!(
                        "stop_observe_resume",
                        host.run_context(),
                        next_state.iteration,
                        self.stop.observe(
                            ctx,
                            StopObservationInput {
                                state: next_state,
                                summary,
                            },
                        ),
                    )? {
                        StopObservationStep::Continue { state, summary } => (*state, summary),
                        StopObservationStep::Exit(exit) => return Ok(exit),
                    };

                    match latency::stage!(
                        "stop_decide_resume",
                        host.run_context(),
                        next_state.iteration,
                        self.stop.decide(
                            ctx,
                            StopInput {
                                state: next_state,
                                summary,
                            },
                        ),
                    )? {
                        StopStep::Stop {
                            state: stopped_state,
                            kind,
                        } => {
                            let exit_iteration = stopped_state.iteration;
                            let exit = latency::stage!(
                                "exit_resume",
                                host.run_context(),
                                exit_iteration,
                                self.exit.process(
                                    ctx,
                                    ExitInput {
                                        state: stopped_state,
                                        kind,
                                    },
                                ),
                            )?;
                            return Ok(exit);
                        }
                        StopStep::Continue { state: next } => {
                            state = next;
                        }
                        StopStep::Exit(exit) => return Ok(exit),
                    }

                    state.iteration = state.iteration.saturating_add(1);
                }

                PromptStep::SkipModel(skipped_state) => {
                    // Compaction-only turn: the prompt stage ran compaction and
                    // set skip_model_this_iteration. Bypass CheckpointStage
                    // (BeforeModel), ModelStage, reply/capability, and
                    // PostCapabilityStage entirely. Route the synthetic summary
                    // through stop.observe + stop.decide so noprogress detection
                    // sees the turn (without counting it as AfterCapabilityBatch
                    // evidence).
                    let skipped_state = *skipped_state;
                    let summary = crate::strategies::TurnSummary::compaction_only();

                    let (mut next_state, summary) = match latency::stage!(
                        "stop_observe_skip_model",
                        host.run_context(),
                        skipped_state.iteration,
                        self.stop.observe(
                            ctx,
                            StopObservationInput {
                                state: skipped_state,
                                summary,
                            },
                        ),
                    )? {
                        StopObservationStep::Continue { state, summary } => (*state, summary),
                        StopObservationStep::Exit(exit) => return Ok(exit),
                    };

                    // Follow-up drain is skipped: CompactionOnly != ReplyOnly,
                    // so the condition is naturally false here.

                    match latency::stage!(
                        "stop_decide_skip_model",
                        host.run_context(),
                        next_state.iteration,
                        self.stop.decide(
                            ctx,
                            StopInput {
                                state: next_state,
                                summary,
                            },
                        ),
                    )? {
                        StopStep::Stop {
                            state: stopped_state,
                            kind,
                        } => {
                            let exit_iteration = stopped_state.iteration;
                            let exit = latency::stage!(
                                "exit_skip_model",
                                host.run_context(),
                                exit_iteration,
                                self.exit.process(
                                    ctx,
                                    ExitInput {
                                        state: stopped_state,
                                        kind,
                                    },
                                ),
                            )?;
                            return Ok(exit);
                        }
                        StopStep::Continue { state: next } => {
                            next_state = next;
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

/// Decide whether a stop should be converted into one more tools-capable
/// completion-nudge iteration instead of terminating the run.
///
/// Gated by `SteeringPolicy.allow_driver_specific_nudges` (off in production
/// unless the run profile opts in) and capped at [`COMPLETION_NUDGE_LIMIT`]
/// nudges per run. A `NoProgressDetected` stop is already a failure terminal, so
/// a tools-capable retry can only help; a `GracefulStop` is nudged only when the
/// closing reply trailed off (the model announced a next step but never carried
/// it out) — leaving genuinely complete replies untouched. Aborts are never
/// nudged.
fn completion_nudge_should_fire(
    host: &(dyn AgentLoopDriverHost + Send + Sync),
    state: &LoopExecutionState,
    kind: &StopKind,
) -> bool {
    if !host
        .run_context()
        .resolved_run_profile
        .steering_policy
        .allow_driver_specific_nudges
    {
        return false;
    }
    if state.completion_nudges_used >= COMPLETION_NUDGE_LIMIT {
        return false;
    }
    match kind {
        StopKind::NoProgressDetected => false,
        StopKind::GracefulStop => {
            state.last_reply_trailed_off
                || (scheduled_trigger_run(host) && state.last_reply_ended_with_question)
        }
        StopKind::Aborted(_) => false,
    }
}
