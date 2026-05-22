use super::*;

impl CanonicalAgentLoopExecutor {
    pub(super) async fn execute_canonical(
        &self,
        family: &LoopFamily,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        mut state: LoopExecutionState,
    ) -> Result<LoopExit, AgentLoopExecutorError> {
        let planner = family.planner();
        let mut pending_input_ack = PendingInputAck::default();

        loop {
            state = match self.checkpoint_and_exit_if_cancelled(host, state).await? {
                CancelCheck::Continue(state) => *state,
                CancelCheck::Exit(exit) => return Ok(exit),
            };

            if state.iteration >= planner.budget().iteration_limit(&state) {
                let checked = self.checkpoint(host, state, CheckpointKind::Final).await?;
                pending_input_ack.ack(host).await?;
                return failed_exit(
                    host,
                    checked.state,
                    LoopFailureKind::IterationLimit,
                    Some(checked.checkpoint_id),
                );
            }

            self.emit_progress(
                host,
                LoopProgressEvent::IterationStarted {
                    iteration: state.iteration,
                },
            )
            .await;

            if pending_input_ack.is_empty() && planner.drain().drain_steering(&state).await {
                state = match self.checkpoint_and_exit_if_cancelled(host, state).await? {
                    CancelCheck::Continue(state) => *state,
                    CancelCheck::Exit(exit) => return Ok(exit),
                };
                let drained = self.drain_user_inputs(host, state).await?;
                state = drained.state;
                pending_input_ack.replace(drained.ack_tokens)?;
                if let Some(reason_kind) = drained.cancelled_reason_kind {
                    let checked = self.checkpoint(host, state, CheckpointKind::Final).await?;
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
                .checkpoint_and_exit_if_cancelled_after_pending_input_ack(
                    host,
                    state,
                    &mut pending_input_ack,
                )
                .await?
            {
                CancelCheck::Continue(state) => *state,
                CancelCheck::Exit(exit) => return Ok(exit),
            };

            let surface_filter = planner.capability().filter(&state).await;
            state = match self
                .checkpoint_and_exit_if_cancelled_after_pending_input_ack(
                    host,
                    state,
                    &mut pending_input_ack,
                )
                .await?
            {
                CancelCheck::Continue(state) => *state,
                CancelCheck::Exit(exit) => return Ok(exit),
            };
            let mut surface = host
                .visible_capabilities(VisibleCapabilityRequest)
                .await
                .map_err(|_| AgentLoopExecutorError::HostUnavailable {
                    stage: HostStage::Capability,
                })?;
            apply_capability_filter(&mut surface, &surface_filter);
            let capability_view = LoopModelCapabilityView {
                visible_capability_ids: surface
                    .descriptors
                    .iter()
                    .map(|descriptor| descriptor.capability_id.clone())
                    .collect(),
            };
            state.surface_version = Some(surface.version.clone());
            state = match self
                .checkpoint_and_exit_if_cancelled_after_pending_input_ack(
                    host,
                    state,
                    &mut pending_input_ack,
                )
                .await?
            {
                CancelCheck::Continue(state) => *state,
                CancelCheck::Exit(exit) => return Ok(exit),
            };

            let mut context_request = planner.context().plan_context_request(&state).await;
            context_request.surface_version = Some(surface.version.clone());
            context_request.capability_view = Some(capability_view.clone());
            let prompt_mode = context_request.mode;
            state = match self
                .checkpoint_and_exit_if_cancelled_after_pending_input_ack(
                    host,
                    state,
                    &mut pending_input_ack,
                )
                .await?
            {
                CancelCheck::Continue(state) => *state,
                CancelCheck::Exit(exit) => return Ok(exit),
            };
            let prompt_bundle = host
                .build_prompt_bundle(context_request)
                .await
                .map_err(|_| AgentLoopExecutorError::HostUnavailable {
                    stage: HostStage::Prompt,
                })?;
            self.emit_progress(
                host,
                LoopProgressEvent::PromptBundleBuilt {
                    iteration: state.iteration,
                    bundle_ref: prompt_bundle.bundle_ref.clone(),
                    mode: prompt_mode,
                    surface_version: prompt_bundle.surface_version.clone(),
                    message_count: prompt_bundle.messages.len() as u32,
                    identity_message_count: prompt_bundle.identity_message_count,
                    instruction_snippet_count: prompt_bundle.instruction_snippet_count,
                },
            )
            .await;
            state = match self
                .checkpoint_and_exit_if_cancelled_after_pending_input_ack(
                    host,
                    state,
                    &mut pending_input_ack,
                )
                .await?
            {
                CancelCheck::Continue(state) => *state,
                CancelCheck::Exit(exit) => return Ok(exit),
            };

            state = self
                .checkpoint(host, state, CheckpointKind::BeforeModel)
                .await?
                .state;
            pending_input_ack.ack(host).await?;
            state = match self.checkpoint_and_exit_if_cancelled(host, state).await? {
                CancelCheck::Continue(state) => *state,
                CancelCheck::Exit(exit) => return Ok(exit),
            };

            let model_preference =
                model_preference_to_host(planner.model().preference(&state).await)?;
            state = match self.checkpoint_and_exit_if_cancelled(host, state).await? {
                CancelCheck::Continue(state) => *state,
                CancelCheck::Exit(exit) => return Ok(exit),
            };
            let model_response = match self
                .stream_model_with_recovery(
                    planner,
                    host,
                    state,
                    LoopModelRequest {
                        messages: prompt_bundle.messages,
                        surface_version: Some(surface.version.clone()),
                        model_preference,
                        capability_view: Some(capability_view),
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
                    state = match self.checkpoint_and_exit_if_cancelled(host, state).await? {
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
                            state = match self.checkpoint_and_exit_if_cancelled(host, state).await?
                            {
                                CancelCheck::Continue(state) => *state,
                                CancelCheck::Exit(exit) => return Ok(exit),
                            };
                            let exit = self.exit_for_stop(host, state, kind).await?;
                            pending_input_ack.ack(host).await?;
                            return Ok(exit);
                        }
                        StopOutcome::Continue { stop } => {
                            state.stop_state = stop;
                            state = match self.checkpoint_and_exit_if_cancelled(host, state).await?
                            {
                                CancelCheck::Continue(state) => *state,
                                CancelCheck::Exit(exit) => return Ok(exit),
                            };
                            if planner.drain().drain_followup(&state).await {
                                state =
                                    match self.checkpoint_and_exit_if_cancelled(host, state).await?
                                    {
                                        CancelCheck::Continue(state) => *state,
                                        CancelCheck::Exit(exit) => return Ok(exit),
                                    };
                                let drained_inputs = self.drain_followup(host, state).await?;
                                state = drained_inputs.state;
                                pending_input_ack.replace(drained_inputs.ack_tokens)?;
                                if let Some(reason_kind) = drained_inputs.cancelled_reason_kind {
                                    let checked =
                                        self.checkpoint(host, state, CheckpointKind::Final).await?;
                                    pending_input_ack.ack(host).await?;
                                    return cancelled_exit_with_reason(
                                        host,
                                        checked.state,
                                        reason_kind,
                                        Some(checked.checkpoint_id),
                                    );
                                }
                                state = match self
                                    .checkpoint_and_exit_if_cancelled_after_pending_input_ack(
                                        host,
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
                            let checked =
                                self.checkpoint(host, state, CheckpointKind::Final).await?;
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
                        .execute_capability_batch(planner, host, state, &surface, calls)
                        .await?
                    {
                        BatchStep::Continue(next) => state = *next,
                        BatchStep::Exit(exit) => return Ok(exit),
                    }
                    state = match self.checkpoint_and_exit_if_cancelled(host, state).await? {
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
                            state = match self.checkpoint_and_exit_if_cancelled(host, state).await?
                            {
                                CancelCheck::Continue(state) => *state,
                                CancelCheck::Exit(exit) => return Ok(exit),
                            };
                            let exit = self.exit_for_stop(host, state, kind).await?;
                            pending_input_ack.ack(host).await?;
                            return Ok(exit);
                        }
                        StopOutcome::Continue { stop } => {
                            state.stop_state = stop;
                            state = match self.checkpoint_and_exit_if_cancelled(host, state).await?
                            {
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
