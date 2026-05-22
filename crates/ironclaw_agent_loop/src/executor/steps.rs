use std::collections::HashSet;

use super::input::*;
use super::*;

impl CanonicalAgentLoopExecutor {
    pub(super) async fn stream_model_with_recovery(
        &self,
        planner: &dyn AgentLoopPlannerInternal,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        mut state: LoopExecutionState,
        request: LoopModelRequest,
    ) -> Result<ModelStep, AgentLoopExecutorError> {
        let mut recorded_failure = false;
        for _ in 0..MAX_MODEL_RETRIES {
            match host.stream_model(request.clone()).await {
                Ok(response) => {
                    state.recovery_state = state.recovery_state.cleared_attempts();
                    return Ok(ModelStep::Response(Box::new(state), response));
                }
                Err(error) => {
                    if error.kind == AgentLoopHostErrorKind::Cancelled {
                        return Err(AgentLoopExecutorError::Cancelled);
                    }
                    let Some(class) = model_error_class(&error) else {
                        return Err(AgentLoopExecutorError::HostUnavailable {
                            stage: HostStage::Model,
                        });
                    };
                    if !recorded_failure {
                        state.recent_failure_kinds.push(LoopFailureKind::ModelError);
                        recorded_failure = true;
                    }
                    let summary = ModelErrorSummary {
                        class,
                        safe_summary: sanitized_strategy_summary(error.safe_summary)?,
                        diagnostic_ref: error.diagnostic_ref,
                    };
                    match planner.recovery().on_model_error(&state, &summary).await {
                        RecoveryOutcome::Retry {
                            recovery, alter, ..
                        } => {
                            state.recovery_state = recovery;
                            match self.checkpoint_and_exit_if_cancelled(host, state).await? {
                                CancelCheck::Continue(next) => state = *next,
                                CancelCheck::Exit(exit) => return Ok(ModelStep::Exit(exit)),
                            }
                            honor_retry_alteration(alter.as_ref())?;
                            self.emit_progress(
                                host,
                                LoopProgressEvent::driver_note(
                                    LoopDriverNoteKind::Retrying,
                                    "retrying model request",
                                )
                                .map_err(|_| {
                                    AgentLoopExecutorError::PlannerContract {
                                        detail: "retry progress summary was invalid",
                                    }
                                })?,
                            )
                            .await;
                        }
                        RecoveryOutcome::SkipResult { .. } => {
                            return Err(AgentLoopExecutorError::PlannerContract {
                                detail: "SkipResult on model error",
                            });
                        }
                        RecoveryOutcome::Abort {
                            recovery,
                            failure_kind,
                        } => {
                            state.recovery_state = recovery;
                            match self.checkpoint_and_exit_if_cancelled(host, state).await? {
                                CancelCheck::Continue(next) => state = *next,
                                CancelCheck::Exit(exit) => return Ok(ModelStep::Exit(exit)),
                            }
                            let checked =
                                self.checkpoint(host, state, CheckpointKind::Final).await?;
                            return Ok(ModelStep::Exit(failed_exit(
                                host,
                                checked.state,
                                failure_kind,
                                Some(checked.checkpoint_id),
                            )?));
                        }
                    }
                }
            }
        }

        let checked = self.checkpoint(host, state, CheckpointKind::Final).await?;
        Ok(ModelStep::Exit(failed_exit(
            host,
            checked.state,
            LoopFailureKind::DriverBug,
            Some(checked.checkpoint_id),
        )?))
    }

    pub(super) async fn execute_capability_batch(
        &self,
        planner: &dyn AgentLoopPlannerInternal,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        mut state: LoopExecutionState,
        surface: &VisibleCapabilitySurface,
        calls: Vec<CapabilityCallCandidate>,
    ) -> Result<BatchStep, AgentLoopExecutorError> {
        state.stop_state.last_batch_total = 0;
        state.stop_state.terminate_hints_in_last_batch = 0;

        let mut visible_calls = Vec::new();
        let mut denied_calls = Vec::new();
        for call in calls {
            if capability_is_visible(surface, &call) {
                visible_calls.push(call);
                continue;
            }

            denied_calls.push(call);
        }

        let summaries = visible_calls
            .iter()
            .map(|call| capability_summary(surface, call))
            .collect::<Vec<_>>();
        let policy = planner.batch().policy(&state, &summaries);
        let stop_on_first_suspension = matches!(policy, BatchPolicy::Sequential);
        match self.checkpoint_and_exit_if_cancelled(host, state).await? {
            CancelCheck::Continue(next) => state = *next,
            CancelCheck::Exit(exit) => return Ok(BatchStep::Exit(exit)),
        }

        state = self
            .checkpoint(host, state, CheckpointKind::BeforeSideEffect)
            .await?
            .state;
        match self.checkpoint_and_exit_if_cancelled(host, state).await? {
            CancelCheck::Continue(next) => state = *next,
            CancelCheck::Exit(exit) => return Ok(BatchStep::Exit(exit)),
        }

        let mut signatures = HashSet::new();
        for call in denied_calls {
            push_call_signature_once(&mut state, &mut signatures, &call)?;
            state
                .recent_failure_kinds
                .push(LoopFailureKind::PolicyDenied);
            let summary = CapabilityErrorSummary {
                class: CapabilityErrorClass::PolicyDenied,
                safe_summary: SanitizedStrategySummary::from_trusted_static(
                    "capability is not visible in the filtered surface",
                ),
                diagnostic_ref: None,
            };
            match self
                .handle_capability_error(planner, host, state, call, summary)
                .await?
            {
                BatchStep::Continue(next) => state = *next,
                BatchStep::Exit(exit) => return Ok(BatchStep::Exit(exit)),
            }
        }

        state.stop_state.last_batch_total = visible_calls.len() as u32;
        if visible_calls.is_empty() {
            return Ok(BatchStep::Continue(Box::new(state)));
        }

        self.emit_progress(
            host,
            LoopProgressEvent::CapabilityBatchStarted {
                iteration: state.iteration,
                call_count: visible_calls.len() as u32,
                policy: batch_policy_kind(policy),
            },
        )
        .await;

        let batch = host
            .invoke_capability_batch(CapabilityBatchInvocation {
                invocations: visible_calls
                    .iter()
                    .cloned()
                    .map(capability_invocation_from_candidate)
                    .collect(),
                stop_on_first_suspension,
            })
            .await
            .map_err(capability_host_error)?;

        if batch.outcomes.len() > visible_calls.len()
            || (!batch.stopped_on_suspension && batch.outcomes.len() != visible_calls.len())
        {
            return Err(AgentLoopExecutorError::PlannerContract {
                detail: "capability batch outcome count does not match invocations",
            });
        }

        let (result_count, denied_count, gated_count, failed_count) =
            capability_batch_counts(&batch.outcomes);
        self.emit_progress(
            host,
            LoopProgressEvent::CapabilityBatchCompleted {
                iteration: state.iteration,
                result_count,
                denied_count,
                gated_count,
                failed_count,
            },
        )
        .await;

        for (call, outcome) in visible_calls.into_iter().zip(batch.outcomes) {
            push_call_signature_once(&mut state, &mut signatures, &call)?;
            match self
                .handle_capability_outcome(planner, host, state, call, outcome)
                .await?
            {
                BatchStep::Continue(next) => {
                    state = *next;
                }
                BatchStep::Exit(exit) => return Ok(BatchStep::Exit(exit)),
            }
        }

        Ok(BatchStep::Continue(Box::new(state)))
    }

    pub(super) async fn handle_capability_outcome(
        &self,
        planner: &dyn AgentLoopPlannerInternal,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        mut state: LoopExecutionState,
        call: CapabilityCallCandidate,
        outcome: CapabilityOutcome,
    ) -> Result<BatchStep, AgentLoopExecutorError> {
        match outcome {
            CapabilityOutcome::Completed(result) => {
                append_capability_result_ref(host, &call, &result).await?;
                push_completed_result(&mut state, result);
                Ok(BatchStep::Continue(Box::new(state)))
            }
            CapabilityOutcome::ApprovalRequired { gate_ref, .. } => {
                self.handle_gate(planner, host, state, call, GateKind::Approval, gate_ref)
                    .await
            }
            CapabilityOutcome::AuthRequired { gate_ref, .. } => {
                self.handle_gate(planner, host, state, call, GateKind::Auth, gate_ref)
                    .await
            }
            CapabilityOutcome::ResourceBlocked { gate_ref, .. } => {
                self.handle_gate(planner, host, state, call, GateKind::Resource, gate_ref)
                    .await
            }
            CapabilityOutcome::SpawnedProcess(handle) => {
                self.fail_unsupported_process_wait(host, state, &call, &handle.process_ref)
                    .await
            }
            CapabilityOutcome::Denied(denied) => {
                state
                    .recent_failure_kinds
                    .push(LoopFailureKind::PolicyDenied);
                let summary = CapabilityErrorSummary {
                    class: CapabilityErrorClass::PolicyDenied,
                    safe_summary: sanitized_strategy_summary(denied.safe_summary)?,
                    diagnostic_ref: None,
                };
                self.handle_capability_error(planner, host, state, call, summary)
                    .await
            }
            CapabilityOutcome::Failed(failure) => {
                if failure.error_kind == CapabilityFailureKind::Cancelled {
                    return self.cancelled_after_checkpoint(host, state).await;
                }
                state
                    .recent_failure_kinds
                    .push(capability_failure_kind(&failure.error_kind));
                let summary = CapabilityErrorSummary {
                    class: capability_error_class(&failure.error_kind),
                    safe_summary: sanitized_strategy_summary(failure.safe_summary)?,
                    diagnostic_ref: None,
                };
                self.handle_capability_error(planner, host, state, call, summary)
                    .await
            }
        }
    }

    pub(super) async fn handle_capability_error(
        &self,
        planner: &dyn AgentLoopPlannerInternal,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        mut state: LoopExecutionState,
        call: CapabilityCallCandidate,
        mut summary: CapabilityErrorSummary,
    ) -> Result<BatchStep, AgentLoopExecutorError> {
        for _ in 0..MAX_CAPABILITY_RETRIES {
            match planner
                .recovery()
                .on_capability_error(&state, &summary)
                .await
            {
                RecoveryOutcome::SkipResult { recovery } => {
                    state.recovery_state = recovery;
                    append_capability_error_ref(host, &mut state, &call, &summary).await?;
                    match self.checkpoint_and_exit_if_cancelled(host, state).await? {
                        CancelCheck::Continue(next) => state = *next,
                        CancelCheck::Exit(exit) => return Ok(BatchStep::Exit(exit)),
                    }
                    return Ok(BatchStep::Continue(Box::new(state)));
                }
                RecoveryOutcome::Abort {
                    recovery,
                    failure_kind,
                } => {
                    state.recovery_state = recovery;
                    append_capability_error_ref(host, &mut state, &call, &summary).await?;
                    match self.checkpoint_and_exit_if_cancelled(host, state).await? {
                        CancelCheck::Continue(next) => state = *next,
                        CancelCheck::Exit(exit) => return Ok(BatchStep::Exit(exit)),
                    }
                    let checked = self.checkpoint(host, state, CheckpointKind::Final).await?;
                    return Ok(BatchStep::Exit(failed_exit(
                        host,
                        checked.state,
                        failure_kind,
                        Some(checked.checkpoint_id),
                    )?));
                }
                RecoveryOutcome::Retry {
                    recovery, alter, ..
                } => {
                    if matches!(summary.class, CapabilityErrorClass::PolicyDenied) {
                        state.recovery_state = recovery;
                        append_capability_error_ref(host, &mut state, &call, &summary).await?;
                        match self.checkpoint_and_exit_if_cancelled(host, state).await? {
                            CancelCheck::Continue(next) => state = *next,
                            CancelCheck::Exit(exit) => return Ok(BatchStep::Exit(exit)),
                        }
                        return Ok(BatchStep::Continue(Box::new(state)));
                    }
                    state.recovery_state = recovery;
                    match self.checkpoint_and_exit_if_cancelled(host, state).await? {
                        CancelCheck::Continue(next) => state = *next,
                        CancelCheck::Exit(exit) => return Ok(BatchStep::Exit(exit)),
                    }
                    honor_retry_alteration(alter.as_ref())?;
                    self.emit_progress(
                        host,
                        LoopProgressEvent::driver_note(
                            LoopDriverNoteKind::Retrying,
                            "retrying capability invocation",
                        )
                        .map_err(|_| {
                            AgentLoopExecutorError::PlannerContract {
                                detail: "retry progress summary was invalid",
                            }
                        })?,
                    )
                    .await;
                    let retry = host
                        .invoke_capability(capability_invocation_from_candidate(call.clone()))
                        .await
                        .map_err(capability_host_error)?;
                    match retry {
                        CapabilityOutcome::Failed(failure) => {
                            if failure.error_kind == CapabilityFailureKind::Cancelled {
                                return self.cancelled_after_checkpoint(host, state).await;
                            }
                            summary = CapabilityErrorSummary {
                                class: capability_error_class(&failure.error_kind),
                                safe_summary: sanitized_strategy_summary(failure.safe_summary)?,
                                diagnostic_ref: None,
                            };
                        }
                        promoted => match promoted {
                            CapabilityOutcome::Completed(result) => {
                                append_capability_result_ref(host, &call, &result).await?;
                                push_completed_result(&mut state, result);
                                return Ok(BatchStep::Continue(Box::new(state)));
                            }
                            CapabilityOutcome::ApprovalRequired { gate_ref, .. } => {
                                return self
                                    .handle_gate(
                                        planner,
                                        host,
                                        state,
                                        call,
                                        GateKind::Approval,
                                        gate_ref,
                                    )
                                    .await;
                            }
                            CapabilityOutcome::AuthRequired { gate_ref, .. } => {
                                return self
                                    .handle_gate(
                                        planner,
                                        host,
                                        state,
                                        call,
                                        GateKind::Auth,
                                        gate_ref,
                                    )
                                    .await;
                            }
                            CapabilityOutcome::ResourceBlocked { gate_ref, .. } => {
                                return self
                                    .handle_gate(
                                        planner,
                                        host,
                                        state,
                                        call,
                                        GateKind::Resource,
                                        gate_ref,
                                    )
                                    .await;
                            }
                            CapabilityOutcome::SpawnedProcess(handle) => {
                                return self
                                    .fail_unsupported_process_wait(
                                        host,
                                        state,
                                        &call,
                                        &handle.process_ref,
                                    )
                                    .await;
                            }
                            CapabilityOutcome::Denied(denied) => {
                                state
                                    .recent_failure_kinds
                                    .push(LoopFailureKind::PolicyDenied);
                                summary = CapabilityErrorSummary {
                                    class: CapabilityErrorClass::PolicyDenied,
                                    safe_summary: sanitized_strategy_summary(denied.safe_summary)?,
                                    diagnostic_ref: None,
                                };
                            }
                            CapabilityOutcome::Failed(failure) => {
                                summary = CapabilityErrorSummary {
                                    class: capability_error_class(&failure.error_kind),
                                    safe_summary: sanitized_strategy_summary(failure.safe_summary)?,
                                    diagnostic_ref: None,
                                };
                            }
                        },
                    }
                }
            }
        }

        append_capability_error_ref(host, &mut state, &call, &summary).await?;
        let checked = self.checkpoint(host, state, CheckpointKind::Final).await?;
        Ok(BatchStep::Exit(failed_exit(
            host,
            checked.state,
            LoopFailureKind::DriverBug,
            Some(checked.checkpoint_id),
        )?))
    }

    pub(super) async fn handle_gate(
        &self,
        planner: &dyn AgentLoopPlannerInternal,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        mut state: LoopExecutionState,
        call: CapabilityCallCandidate,
        kind: GateKind,
        gate_ref: ironclaw_turns::LoopGateRef,
    ) -> Result<BatchStep, AgentLoopExecutorError> {
        let summary = crate::strategies::GateSummary {
            kind,
            gate_ref: gate_ref.clone(),
        };
        match planner.gate().handle(&state, &summary).await {
            GateOutcome::Block { gate } => {
                state.gate_state = gate;
                state.last_gate = Some(gate_ref.clone());
                match self.checkpoint_and_exit_if_cancelled(host, state).await? {
                    CancelCheck::Continue(next) => state = *next,
                    CancelCheck::Exit(exit) => return Ok(BatchStep::Exit(exit)),
                }
                self.emit_progress(
                    host,
                    LoopProgressEvent::GateBlocked {
                        iteration: state.iteration,
                        gate_kind: loop_gate_kind(kind),
                    },
                )
                .await;
                let checked = self
                    .checkpoint(host, state, CheckpointKind::BeforeBlock)
                    .await?;
                Ok(BatchStep::Exit(LoopExit::Blocked(LoopBlocked {
                    kind: blocked_kind(kind),
                    gate_ref,
                    checkpoint_id: checked.checkpoint_id,
                    state_ref: checked.state_ref,
                    exit_id: exit_id(host, "blocked")?,
                })))
            }
            GateOutcome::SkipAndContinue { gate } => {
                state.gate_state = gate;
                append_capability_safe_summary_ref(
                    host,
                    &mut state,
                    &call,
                    gate_tool_result_summary(kind, "skipped"),
                )
                .await?;
                match self.checkpoint_and_exit_if_cancelled(host, state).await? {
                    CancelCheck::Continue(next) => state = *next,
                    CancelCheck::Exit(exit) => return Ok(BatchStep::Exit(exit)),
                }
                Ok(BatchStep::Continue(Box::new(state)))
            }
            GateOutcome::Abort { gate, failure_kind } => {
                state.gate_state = gate;
                append_capability_safe_summary_ref(
                    host,
                    &mut state,
                    &call,
                    gate_tool_result_summary(kind, "aborted"),
                )
                .await?;
                match self.checkpoint_and_exit_if_cancelled(host, state).await? {
                    CancelCheck::Continue(next) => state = *next,
                    CancelCheck::Exit(exit) => return Ok(BatchStep::Exit(exit)),
                }
                let checked = self.checkpoint(host, state, CheckpointKind::Final).await?;
                Ok(BatchStep::Exit(failed_exit(
                    host,
                    checked.state,
                    failure_kind,
                    Some(checked.checkpoint_id),
                )?))
            }
        }
    }

    pub(super) async fn fail_unsupported_process_wait(
        &self,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        mut state: LoopExecutionState,
        call: &CapabilityCallCandidate,
        _process_ref: &ironclaw_turns::run_profile::LoopProcessRef,
    ) -> Result<BatchStep, AgentLoopExecutorError> {
        append_capability_safe_summary_ref(
            host,
            &mut state,
            call,
            "capability process wait is not supported".to_string(),
        )
        .await?;
        let checked = self.checkpoint(host, state, CheckpointKind::Final).await?;
        Ok(BatchStep::Exit(failed_exit(
            host,
            checked.state,
            LoopFailureKind::CapabilityProtocolError,
            Some(checked.checkpoint_id),
        )?))
    }

    pub(super) async fn cancelled_after_checkpoint(
        &self,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        state: LoopExecutionState,
    ) -> Result<BatchStep, AgentLoopExecutorError> {
        // Called when a capability invocation surfaced `CapabilityFailureKind::Cancelled`
        // and no `LoopCancellationSignal` is in scope, so the cooperative-boundary
        // reason cannot be derived from a signal. `cancelled_exit` hardcodes
        // `LoopCancelledReasonKind::HostCancellation` which currently coarsens
        // every reason variant; if `LoopCancelledReasonKind` gains finer-grained
        // variants this site must switch to `cancelled_exit_with_reason` with the
        // capability-specific reason.
        let checked = self.checkpoint(host, state, CheckpointKind::Final).await?;
        Ok(BatchStep::Exit(cancelled_exit(
            host,
            checked.state,
            Some(checked.checkpoint_id),
        )?))
    }

    pub(super) async fn exit_for_stop(
        &self,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        state: LoopExecutionState,
        kind: StopKind,
    ) -> Result<LoopExit, AgentLoopExecutorError> {
        match kind {
            StopKind::GracefulStop => {
                let checked = self.checkpoint(host, state, CheckpointKind::Final).await?;
                completed_exit(host, checked.state, Some(checked.checkpoint_id))
            }
            StopKind::NoProgressDetected => {
                let checked = self.checkpoint(host, state, CheckpointKind::Final).await?;
                failed_exit(
                    host,
                    checked.state,
                    LoopFailureKind::NoProgressDetected,
                    Some(checked.checkpoint_id),
                )
            }
            StopKind::Aborted(failure_kind) => {
                let checked = self.checkpoint(host, state, CheckpointKind::Final).await?;
                failed_exit(
                    host,
                    checked.state,
                    failure_kind,
                    Some(checked.checkpoint_id),
                )
            }
        }
    }

    pub(super) async fn checkpoint(
        &self,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        mut state: LoopExecutionState,
        kind: CheckpointKind,
    ) -> Result<CheckpointWrite, AgentLoopExecutorError> {
        state.last_checkpoint = Some(crate::state::CheckpointMarker {
            kind,
            iteration_at_checkpoint: state.iteration,
        });
        let payload = serde_json::to_vec(&state)
            .map_err(|_| AgentLoopExecutorError::CheckpointFailed { stage: kind })?;
        let host_kind = checkpoint_kind_to_host(kind);
        let state_ref = host
            .stage_checkpoint_payload(StageCheckpointPayloadRequest {
                kind: host_kind,
                schema_id: crate::state::CHECKPOINT_SCHEMA_ID.to_string(),
                payload,
            })
            .await
            .map_err(|_| AgentLoopExecutorError::CheckpointFailed { stage: kind })?;
        let checkpoint_id = host
            .checkpoint(LoopCheckpointRequest {
                kind: host_kind,
                state_ref: state_ref.clone(),
            })
            .await
            .map_err(|_| AgentLoopExecutorError::CheckpointFailed { stage: kind })?;
        self.emit_progress(
            host,
            LoopProgressEvent::CheckpointWritten {
                iteration: state.iteration,
                kind: host_kind,
            },
        )
        .await;
        Ok(CheckpointWrite {
            state,
            checkpoint_id,
            state_ref,
        })
    }

    pub(super) async fn emit_progress(
        &self,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        event: LoopProgressEvent,
    ) {
        let _ = host.emit_loop_progress(event).await;
    }

    // Cancellation is checked cooperatively at N boundary points between external calls.
    // A macro refactor was considered but deferred; the explicit sites are self-documenting.
    pub(super) async fn checkpoint_and_exit_if_cancelled(
        &self,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        state: LoopExecutionState,
    ) -> Result<CancelCheck, AgentLoopExecutorError> {
        let Some(signal) = host.observe_cancellation() else {
            return Ok(CancelCheck::Continue(Box::new(state)));
        };

        let fallback_state = state.clone();
        match self.checkpoint(host, state, CheckpointKind::Final).await {
            Ok(checked) => Ok(CancelCheck::Exit(cancelled_exit_with_reason(
                host,
                checked.state,
                cancelled_reason_from_signal(&signal),
                Some(checked.checkpoint_id),
            )?)),
            // Permissive profile: only checkpoint-write failures are absorbed
            // into a checkpoint-free `Cancelled` exit. Other variants (e.g.
            // `HostUnavailable`) must propagate so the runner can apply its
            // recovery policy.
            Err(AgentLoopExecutorError::CheckpointFailed { .. })
                if !host
                    .run_context()
                    .resolved_run_profile
                    .checkpoint_policy
                    .require_final_checkpoint =>
            {
                Ok(CancelCheck::Exit(cancelled_exit_with_reason(
                    host,
                    fallback_state,
                    cancelled_reason_from_signal(&signal),
                    None,
                )?))
            }
            Err(error) => Err(error),
        }
    }

    pub(super) async fn checkpoint_and_exit_if_cancelled_after_pending_input_ack(
        &self,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        state: LoopExecutionState,
        pending_input_ack: &mut PendingInputAck,
    ) -> Result<CancelCheck, AgentLoopExecutorError> {
        let Some(signal) = host.observe_cancellation() else {
            return Ok(CancelCheck::Continue(Box::new(state)));
        };

        let fallback_state = state.clone();
        match self.checkpoint(host, state, CheckpointKind::Final).await {
            Ok(checked) => {
                pending_input_ack.ack(host).await?;
                Ok(CancelCheck::Exit(cancelled_exit_with_reason(
                    host,
                    checked.state,
                    cancelled_reason_from_signal(&signal),
                    Some(checked.checkpoint_id),
                )?))
            }
            // Permissive profile: absorb only checkpoint-write failures. The
            // pending ack is intentionally NOT flushed here — no durable
            // checkpoint was written, so advancing the input cursor would
            // commit progress that the runner has no record of.
            Err(AgentLoopExecutorError::CheckpointFailed { .. })
                if !host
                    .run_context()
                    .resolved_run_profile
                    .checkpoint_policy
                    .require_final_checkpoint =>
            {
                Ok(CancelCheck::Exit(cancelled_exit_with_reason(
                    host,
                    fallback_state,
                    cancelled_reason_from_signal(&signal),
                    None,
                )?))
            }
            // Strict profile (or non-checkpoint error variant): propagate the
            // error so the runner sees the same failure mode as
            // `checkpoint_and_exit_if_cancelled`. Returning `Ok(LoopExit::failed)`
            // would silently mask `HostUnavailable` and break the strict
            // require-final-checkpoint contract.
            Err(error) => Err(error),
        }
    }

    pub(super) async fn drain_user_inputs(
        &self,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        mut state: LoopExecutionState,
    ) -> Result<DrainedInputs, AgentLoopExecutorError> {
        let batch = host
            .poll_inputs(state.input_cursor.clone(), MAX_INPUT_DRAIN)
            .await
            .map_err(|_| AgentLoopExecutorError::HostUnavailable {
                stage: HostStage::Input,
            })?;
        let (drained, ack_tokens, cancelled_reason_kind) =
            consume_drainable_inputs(&batch, UserFacingInputDrainMode::Steering, &mut state)?;
        Ok(DrainedInputs {
            state,
            drained,
            ack_tokens,
            cancelled_reason_kind,
        })
    }

    pub(super) async fn drain_followup(
        &self,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        mut state: LoopExecutionState,
    ) -> Result<DrainedInputs, AgentLoopExecutorError> {
        let batch = host
            .poll_inputs(state.input_cursor.clone(), MAX_INPUT_DRAIN)
            .await
            .map_err(|_| AgentLoopExecutorError::HostUnavailable {
                stage: HostStage::Input,
            })?;
        let (drained, ack_tokens, cancelled_reason_kind) =
            consume_drainable_inputs(&batch, UserFacingInputDrainMode::FollowUp, &mut state)?;
        Ok(DrainedInputs {
            state,
            drained,
            ack_tokens,
            cancelled_reason_kind,
        })
    }
}
