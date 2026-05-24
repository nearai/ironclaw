use super::*;

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
            state = match self.checkpoint.cancel_if_requested(ctx, state).await? {
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

            self.checkpoint
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
                    InputInput {
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

            let prompt = match self
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
                PromptStep::Prepared(prompt) => *prompt,
                PromptStep::Exit(exit) => return Ok(exit),
            };
            state = prompt.state;
            pending_input_ack = prompt.pending_input_ack;

            state = self
                .checkpoint
                .process(
                    ctx,
                    CheckpointInput {
                        state,
                        kind: CheckpointKind::BeforeModel,
                    },
                )
                .await?
                .state;
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
                ModelStep::Exit(exit) => return Ok(exit),
            };

            let completed = match model_response.output {
                ParentLoopOutput::AssistantReply(reply) => {
                    self.assistant_reply
                        .process(ctx, AssistantReplyInput { state, reply })
                        .await?
                }
                ParentLoopOutput::CapabilityCalls(calls) => {
                    self.capabilities
                        .process(
                            ctx,
                            CapabilityBatchInput {
                                state,
                                surface: prompt.surface,
                                calls,
                            },
                        )
                        .await?
                }
            };

            let (next_state, summary) = match completed {
                TurnCompletedStep::Continue { state, summary } => (*state, summary),
                TurnCompletedStep::Exit(exit) => return Ok(exit),
            };
            let completed_kind = summary.kind;

            match self
                .stop
                .process(
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

            match completed_kind {
                TurnEndKind::ReplyOnly => {
                    match self
                        .input
                        .process(
                            ctx,
                            InputInput {
                                state,
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
                            state = *next;
                            pending_input_ack = ack;
                            if drained {
                                state.iteration = state.iteration.saturating_add(1);
                                continue;
                            }
                        }
                        InputStep::Exit(exit) => return Ok(exit),
                    }

                    let checked = self
                        .checkpoint
                        .process(
                            ctx,
                            CheckpointInput {
                                state,
                                kind: CheckpointKind::Final,
                            },
                        )
                        .await?;
                    pending_input_ack.ack(host).await?;
                    return completed_exit(host, checked.state, Some(checked.checkpoint_id));
                }
                TurnEndKind::AfterCapabilityBatch => {
                    state.iteration = state.iteration.saturating_add(1);
                }
            }
        }
    }
}
