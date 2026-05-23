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

            if state.iteration >= planner.budget().iteration_limit(&state) {
                let checked = self
                    .checkpoint
                    .write(ctx, state, CheckpointKind::Final)
                    .await?;
                pending_input_ack.ack(host).await?;
                return failed_exit(
                    host,
                    checked.state,
                    LoopFailureKind::IterationLimit,
                    Some(checked.checkpoint_id),
                );
            }

            self.checkpoint
                .emit_progress(
                    ctx,
                    LoopProgressEvent::IterationStarted {
                        iteration: state.iteration,
                    },
                )
                .await;

            if pending_input_ack.is_empty() && planner.drain().drain_steering(&state).await {
                state = match self.checkpoint.cancel_if_requested(ctx, state).await? {
                    CancelCheck::Continue(state) => *state,
                    CancelCheck::Exit(exit) => return Ok(exit),
                };
                let drained = self.checkpoint.drain_user_inputs(ctx, state).await?;
                state = drained.state;
                pending_input_ack.replace(drained.ack_tokens)?;
                if let Some(reason_kind) = drained.cancelled_reason_kind {
                    let checked = self
                        .checkpoint
                        .write(ctx, state, CheckpointKind::Final)
                        .await?;
                    pending_input_ack.ack(host).await?;
                    return cancelled_exit_with_reason(
                        host,
                        checked.state,
                        reason_kind,
                        Some(checked.checkpoint_id),
                    );
                }
            }
            state = match self
                .checkpoint
                .cancel_if_requested_after_pending_input_ack(ctx, state, &mut pending_input_ack)
                .await?
            {
                CancelCheck::Continue(state) => *state,
                CancelCheck::Exit(exit) => return Ok(exit),
            };

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
                .write(ctx, state, CheckpointKind::BeforeModel)
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

            match model_response.output {
                ParentLoopOutput::AssistantReply(reply) => {
                    let reply_ref = host
                        .finalize_assistant_message(FinalizeAssistantMessage { reply })
                        .await
                        .map_err(|_| AgentLoopExecutorError::HostUnavailable {
                            stage: HostStage::Transcript,
                        })?;
                    state.assistant_refs.push(reply_ref.clone());
                    state = match self.checkpoint.cancel_if_requested(ctx, state).await? {
                        CancelCheck::Continue(state) => *state,
                        CancelCheck::Exit(exit) => return Ok(exit),
                    };

                    let summary = TurnSummary {
                        kind: TurnEndKind::ReplyOnly,
                        assistant_message_ref: Some(reply_ref),
                        batch_result_refs: Vec::new(),
                    };
                    match planner
                        .stop()
                        .should_stop_after_turn(&state, &summary)
                        .await
                    {
                        StopOutcome::Stop { stop, kind } => {
                            state.stop_state = stop;
                            state = match self.checkpoint.cancel_if_requested(ctx, state).await? {
                                CancelCheck::Continue(state) => *state,
                                CancelCheck::Exit(exit) => return Ok(exit),
                            };
                            let exit = self.exit.for_stop(ctx, state, kind).await?;
                            pending_input_ack.ack(host).await?;
                            return Ok(exit);
                        }
                        StopOutcome::Continue { stop } => {
                            state.stop_state = stop;
                            state = match self.checkpoint.cancel_if_requested(ctx, state).await? {
                                CancelCheck::Continue(state) => *state,
                                CancelCheck::Exit(exit) => return Ok(exit),
                            };
                            if planner.drain().drain_followup(&state).await {
                                state =
                                    match self.checkpoint.cancel_if_requested(ctx, state).await? {
                                        CancelCheck::Continue(state) => *state,
                                        CancelCheck::Exit(exit) => return Ok(exit),
                                    };
                                let drained_inputs =
                                    self.checkpoint.drain_followup(ctx, state).await?;
                                state = drained_inputs.state;
                                pending_input_ack.replace(drained_inputs.ack_tokens)?;
                                if let Some(reason_kind) = drained_inputs.cancelled_reason_kind {
                                    let checked = self
                                        .checkpoint
                                        .write(ctx, state, CheckpointKind::Final)
                                        .await?;
                                    pending_input_ack.ack(host).await?;
                                    return cancelled_exit_with_reason(
                                        host,
                                        checked.state,
                                        reason_kind,
                                        Some(checked.checkpoint_id),
                                    );
                                }
                                state = match self
                                    .checkpoint
                                    .cancel_if_requested_after_pending_input_ack(
                                        ctx,
                                        state,
                                        &mut pending_input_ack,
                                    )
                                    .await?
                                {
                                    CancelCheck::Continue(state) => *state,
                                    CancelCheck::Exit(exit) => return Ok(exit),
                                };
                                if drained_inputs.drained {
                                    state.iteration = state.iteration.saturating_add(1);
                                    continue;
                                }
                            }
                            let checked = self
                                .checkpoint
                                .write(ctx, state, CheckpointKind::Final)
                                .await?;
                            pending_input_ack.ack(host).await?;
                            return completed_exit(
                                host,
                                checked.state,
                                Some(checked.checkpoint_id),
                            );
                        }
                    }
                }
                ParentLoopOutput::CapabilityCalls(calls) => {
                    let result_refs_start = state.result_refs.len();
                    match self
                        .capabilities
                        .process(
                            ctx,
                            CapabilityBatchInput {
                                state,
                                surface: prompt.surface,
                                calls,
                                gates: self.gates,
                            },
                        )
                        .await?
                    {
                        BatchStep::Continue(next) => state = *next,
                        BatchStep::Exit(exit) => return Ok(exit),
                    }
                    state = match self.checkpoint.cancel_if_requested(ctx, state).await? {
                        CancelCheck::Continue(state) => *state,
                        CancelCheck::Exit(exit) => return Ok(exit),
                    };

                    let summary = TurnSummary {
                        kind: TurnEndKind::AfterCapabilityBatch,
                        assistant_message_ref: None,
                        batch_result_refs: state.result_refs[result_refs_start..].to_vec(),
                    };
                    match planner
                        .stop()
                        .should_stop_after_turn(&state, &summary)
                        .await
                    {
                        StopOutcome::Stop { stop, kind } => {
                            state.stop_state = stop;
                            state = match self.checkpoint.cancel_if_requested(ctx, state).await? {
                                CancelCheck::Continue(state) => *state,
                                CancelCheck::Exit(exit) => return Ok(exit),
                            };
                            let exit = self.exit.for_stop(ctx, state, kind).await?;
                            pending_input_ack.ack(host).await?;
                            return Ok(exit);
                        }
                        StopOutcome::Continue { stop } => {
                            state.stop_state = stop;
                            state = match self.checkpoint.cancel_if_requested(ctx, state).await? {
                                CancelCheck::Continue(state) => *state,
                                CancelCheck::Exit(exit) => return Ok(exit),
                            };
                            state.iteration = state.iteration.saturating_add(1);
                        }
                    }
                }
            }
        }
    }
}
