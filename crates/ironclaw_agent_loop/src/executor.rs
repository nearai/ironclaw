//! Canonical agent-loop executor.
//!
//! The executor owns loop mechanics. Loop families own strategy composition.
//! See `docs/reborn/agent-loop-skeleton.md` section 8 for the canonical tick.

use std::collections::HashSet;

use async_trait::async_trait;
use ironclaw_turns::{
    LoopBlocked, LoopBlockedKind, LoopCompleted, LoopCompletionKind, LoopExit, LoopExitId,
    LoopFailed, LoopFailureKind,
    run_profile::{
        AgentLoopDriverHost, AgentLoopHostError, AgentLoopHostErrorKind, CapabilityBatchInvocation,
        CapabilityCallCandidate, CapabilityInvocation, CapabilityOutcome, CapabilityResultMessage,
        FinalizeAssistantMessage, LoopCheckpointKind, LoopCheckpointRequest, LoopInput,
        LoopModelRequest, ParentLoopOutput, StageCheckpointPayloadRequest,
        VisibleCapabilityRequest, VisibleCapabilitySurface,
    },
};

use crate::{
    family::LoopFamily,
    planner::AgentLoopPlannerInternal,
    state::{CapabilityCallSignature, CheckpointKind, LoopExecutionState},
    strategies::{
        BatchPolicy, CapabilityCallSummary, CapabilityErrorClass, CapabilityErrorSummary,
        CapabilityFilter, GateKind, GateOutcome, ModelErrorClass, ModelErrorSummary,
        ModelPreference, RecoveryOutcome, RetryAlteration, StopKind, StopOutcome, TurnEndKind,
        TurnSummary,
    },
};

const MAX_CAPABILITY_RETRIES: usize = 8;
const MAX_MODEL_RETRIES: usize = 8;
const MAX_INPUT_DRAIN: usize = 32;

/// Drives the canonical loop tick by consulting a resolved [`LoopFamily`].
///
/// `execute_family` is the public entry point required by the skeleton spec:
/// downstream crates pass opaque families through, while strategy access stays
/// crate-private through [`AgentLoopPlannerInternal`].
#[async_trait]
pub trait AgentLoopExecutor: Send + Sync {
    async fn execute_family(
        &self,
        family: &LoopFamily,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        initial_state: LoopExecutionState,
    ) -> Result<LoopExit, AgentLoopExecutorError>;
}

/// Sanitized executor errors. Loop-level terminal states should usually be
/// returned as [`LoopExit`]; this type is for failures that prevent producing a
/// trustworthy exit.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AgentLoopExecutorError {
    #[error("host port returned an unrecoverable error: {stage:?}")]
    HostUnavailable { stage: HostStage },
    #[error("planner returned a contract violation: {detail}")]
    PlannerContract { detail: &'static str },
    #[error("checkpoint write failed at {stage:?}")]
    CheckpointFailed { stage: CheckpointKind },
    #[error("cancelled by host before any LoopExit could be produced")]
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostStage {
    Prompt,
    Model,
    Capability,
    Transcript,
    Checkpoint,
    Progress,
    Input,
}

/// Reference executor for the Reborn skeleton loop.
#[derive(Debug, Default, Clone, Copy)]
pub struct CanonicalAgentLoopExecutor;

#[async_trait]
impl AgentLoopExecutor for CanonicalAgentLoopExecutor {
    async fn execute_family(
        &self,
        family: &LoopFamily,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        initial_state: LoopExecutionState,
    ) -> Result<LoopExit, AgentLoopExecutorError> {
        self.execute_canonical(family, host, initial_state).await
    }
}

#[derive(Debug)]
struct CheckpointWrite {
    state: LoopExecutionState,
    checkpoint_id: ironclaw_turns::TurnCheckpointId,
    state_ref: ironclaw_turns::run_profile::LoopCheckpointStateRef,
}

#[derive(Debug)]
enum BatchStep {
    Continue(LoopExecutionState),
    Exit(LoopExit),
}

impl CanonicalAgentLoopExecutor {
    async fn execute_canonical(
        &self,
        family: &LoopFamily,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        mut state: LoopExecutionState,
    ) -> Result<LoopExit, AgentLoopExecutorError> {
        let planner = family.planner();

        loop {
            if state.iteration >= planner.budget().iteration_limit(&state) {
                let checked = self.checkpoint(host, state, CheckpointKind::Final).await?;
                return Ok(failed_exit(
                    host,
                    checked.state,
                    LoopFailureKind::IterationLimit,
                    Some(checked.checkpoint_id),
                ));
            }

            if planner.drain().drain_steering(&state).await {
                state = self.drain_user_inputs(host, state).await?;
            }

            let context_request = planner.context().plan_context_request(&state).await;
            let prompt_bundle = host
                .build_prompt_bundle(context_request)
                .await
                .map_err(|_| AgentLoopExecutorError::HostUnavailable {
                    stage: HostStage::Prompt,
                })?;

            let surface_filter = planner.capability().filter(&state).await;
            let mut surface = host
                .visible_capabilities(VisibleCapabilityRequest)
                .await
                .map_err(|_| AgentLoopExecutorError::HostUnavailable {
                    stage: HostStage::Capability,
                })?;
            apply_capability_filter(&mut surface, &surface_filter);
            state.surface_version = Some(surface.version.clone());

            state = self
                .checkpoint(host, state, CheckpointKind::BeforeModel)
                .await?
                .state;

            let model_preference =
                model_preference_to_host(planner.model().preference(&state).await)?;
            let model_response = match self
                .stream_model_with_recovery(
                    planner,
                    host,
                    state,
                    LoopModelRequest {
                        messages: prompt_bundle.messages,
                        surface_version: Some(surface.version.clone()),
                        model_preference,
                        context_summaries: prompt_bundle.context_summaries,
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
                            return self.exit_for_stop(host, state, kind).await;
                        }
                        StopOutcome::Continue { stop } => {
                            state.stop_state = stop;
                            if planner.drain().drain_followup(&state).await {
                                let (next, drained) = self.drain_followup(host, state).await?;
                                state = next;
                                if drained {
                                    state.iteration = state.iteration.saturating_add(1);
                                    continue;
                                }
                            }
                            let checked =
                                self.checkpoint(host, state, CheckpointKind::Final).await?;
                            return Ok(completed_exit(
                                host,
                                checked.state,
                                Some(checked.checkpoint_id),
                            ));
                        }
                    }
                }
                ParentLoopOutput::CapabilityCalls(calls) => {
                    let result_refs_start = state.result_refs.len();
                    match self
                        .execute_capability_batch(planner, host, state, &surface, calls)
                        .await?
                    {
                        BatchStep::Continue(next) => state = next,
                        BatchStep::Exit(exit) => return Ok(exit),
                    }

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
                            return self.exit_for_stop(host, state, kind).await;
                        }
                        StopOutcome::Continue { stop } => {
                            state.stop_state = stop;
                            state.iteration = state.iteration.saturating_add(1);
                        }
                    }
                }
            }
        }
    }

    async fn stream_model_with_recovery(
        &self,
        planner: &dyn AgentLoopPlannerInternal,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        mut state: LoopExecutionState,
        request: LoopModelRequest,
    ) -> Result<ModelStep, AgentLoopExecutorError> {
        let mut recorded_failure = false;
        for _ in 0..MAX_MODEL_RETRIES {
            match host.stream_model(request.clone()).await {
                Ok(response) => return Ok(ModelStep::Response(Box::new(state), response)),
                Err(error) => {
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
                        safe_summary: error.safe_summary,
                        diagnostic_ref: error.diagnostic_ref,
                    };
                    match planner.recovery().on_model_error(&state, &summary).await {
                        RecoveryOutcome::Retry { recovery, alter } => {
                            state.recovery_state = recovery;
                            honor_retry_alteration(alter.as_ref())?;
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
                            let checked =
                                self.checkpoint(host, state, CheckpointKind::Final).await?;
                            return Ok(ModelStep::Exit(failed_exit(
                                host,
                                checked.state,
                                failure_kind,
                                Some(checked.checkpoint_id),
                            )));
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
        )))
    }

    async fn execute_capability_batch(
        &self,
        planner: &dyn AgentLoopPlannerInternal,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        mut state: LoopExecutionState,
        surface: &VisibleCapabilitySurface,
        calls: Vec<CapabilityCallCandidate>,
    ) -> Result<BatchStep, AgentLoopExecutorError> {
        state.stop_state.last_batch_total = 0;
        state.stop_state.terminate_hints_in_last_batch = 0;

        let summaries = calls
            .iter()
            .map(|call| capability_summary(surface, call))
            .collect::<Vec<_>>();
        let policy = planner.batch().policy(&state, &summaries);
        let stop_on_first_suspension = matches!(policy, BatchPolicy::Sequential);

        state = self
            .checkpoint(host, state, CheckpointKind::BeforeSideEffect)
            .await?
            .state;

        let mut signatures = HashSet::new();
        let mut visible_calls = Vec::new();
        for call in calls {
            if capability_is_visible(surface, &call) {
                visible_calls.push(call);
                continue;
            }

            push_call_signature_once(&mut state, &mut signatures, &call)?;
            state
                .recent_failure_kinds
                .push(LoopFailureKind::PolicyDenied);
            let summary = CapabilityErrorSummary {
                class: CapabilityErrorClass::PolicyDenied,
                safe_summary: "capability is not visible in the filtered surface".to_string(),
                diagnostic_ref: None,
            };
            match self
                .handle_capability_error(planner, host, state, call, summary)
                .await?
            {
                BatchStep::Continue(next) => state = next,
                BatchStep::Exit(exit) => return Ok(BatchStep::Exit(exit)),
            }
        }

        state.stop_state.last_batch_total = visible_calls.len() as u32;
        if visible_calls.is_empty() {
            return Ok(BatchStep::Continue(state));
        }

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
            .map_err(|_| AgentLoopExecutorError::HostUnavailable {
                stage: HostStage::Capability,
            })?;

        if batch.outcomes.len() > visible_calls.len()
            || (!batch.stopped_on_suspension && batch.outcomes.len() != visible_calls.len())
        {
            return Err(AgentLoopExecutorError::PlannerContract {
                detail: "capability batch outcome count does not match invocations",
            });
        }

        for (call, outcome) in visible_calls.into_iter().zip(batch.outcomes) {
            push_call_signature_once(&mut state, &mut signatures, &call)?;
            match self
                .handle_capability_outcome(planner, host, state, call, outcome)
                .await?
            {
                BatchStep::Continue(next) => state = next,
                BatchStep::Exit(exit) => return Ok(BatchStep::Exit(exit)),
            }
        }

        Ok(BatchStep::Continue(state))
    }

    async fn handle_capability_outcome(
        &self,
        planner: &dyn AgentLoopPlannerInternal,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        mut state: LoopExecutionState,
        call: CapabilityCallCandidate,
        outcome: CapabilityOutcome,
    ) -> Result<BatchStep, AgentLoopExecutorError> {
        match outcome {
            CapabilityOutcome::Completed(result) => {
                push_completed_result(&mut state, result);
                Ok(BatchStep::Continue(state))
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
                let gate_ref = ironclaw_turns::LoopGateRef::new(format!(
                    "gate:process-{}",
                    opaque_token(handle.process_ref.as_str())
                ))
                .map_err(|_| AgentLoopExecutorError::PlannerContract {
                    detail: "process ref could not be converted to gate ref",
                })?;
                self.handle_gate(planner, host, state, GateKind::Resource, gate_ref)
                    .await
            }
            CapabilityOutcome::Denied(denied) => {
                state
                    .recent_failure_kinds
                    .push(LoopFailureKind::PolicyDenied);
                let summary = CapabilityErrorSummary {
                    class: CapabilityErrorClass::PolicyDenied,
                    safe_summary: denied.safe_summary,
                    diagnostic_ref: None,
                };
                self.handle_capability_error(planner, host, state, call, summary)
                    .await
            }
            CapabilityOutcome::Failed(failure) => {
                state
                    .recent_failure_kinds
                    .push(capability_failure_kind(&failure.error_kind));
                let summary = CapabilityErrorSummary {
                    class: capability_error_class(&failure.error_kind),
                    safe_summary: failure.safe_summary,
                    diagnostic_ref: None,
                };
                self.handle_capability_error(planner, host, state, call, summary)
                    .await
            }
        }
    }

    async fn handle_capability_error(
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
                    return Ok(BatchStep::Continue(state));
                }
                RecoveryOutcome::Abort {
                    recovery,
                    failure_kind,
                } => {
                    state.recovery_state = recovery;
                    let checked = self.checkpoint(host, state, CheckpointKind::Final).await?;
                    return Ok(BatchStep::Exit(failed_exit(
                        host,
                        checked.state,
                        failure_kind,
                        Some(checked.checkpoint_id),
                    )));
                }
                RecoveryOutcome::Retry { recovery, alter } => {
                    if matches!(summary.class, CapabilityErrorClass::PolicyDenied) {
                        state.recovery_state = recovery;
                        return Ok(BatchStep::Continue(state));
                    }
                    state.recovery_state = recovery;
                    honor_retry_alteration(alter.as_ref())?;
                    let retry = host
                        .invoke_capability(capability_invocation_from_candidate(call.clone()))
                        .await
                        .map_err(|_| AgentLoopExecutorError::HostUnavailable {
                            stage: HostStage::Capability,
                        })?;
                    match retry {
                        CapabilityOutcome::Failed(failure) => {
                            summary = CapabilityErrorSummary {
                                class: capability_error_class(&failure.error_kind),
                                safe_summary: failure.safe_summary,
                                diagnostic_ref: None,
                            };
                        }
                        promoted => match promoted {
                            CapabilityOutcome::Completed(result) => {
                                push_completed_result(&mut state, result);
                                return Ok(BatchStep::Continue(state));
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
                                let gate_ref = ironclaw_turns::LoopGateRef::new(format!(
                                    "gate:process-{}",
                                    opaque_token(handle.process_ref.as_str())
                                ))
                                .map_err(|_| {
                                    AgentLoopExecutorError::PlannerContract {
                                        detail: "process ref could not be converted to gate ref",
                                    }
                                })?;
                                return self
                                    .handle_gate(planner, host, state, GateKind::Resource, gate_ref)
                                    .await;
                            }
                            CapabilityOutcome::Denied(denied) => {
                                state
                                    .recent_failure_kinds
                                    .push(LoopFailureKind::PolicyDenied);
                                summary = CapabilityErrorSummary {
                                    class: CapabilityErrorClass::PolicyDenied,
                                    safe_summary: denied.safe_summary,
                                    diagnostic_ref: None,
                                };
                            }
                            CapabilityOutcome::Failed(failure) => {
                                summary = CapabilityErrorSummary {
                                    class: capability_error_class(&failure.error_kind),
                                    safe_summary: failure.safe_summary,
                                    diagnostic_ref: None,
                                };
                            }
                        },
                    }
                }
            }
        }

        let checked = self.checkpoint(host, state, CheckpointKind::Final).await?;
        Ok(BatchStep::Exit(failed_exit(
            host,
            checked.state,
            LoopFailureKind::DriverBug,
            Some(checked.checkpoint_id),
        )))
    }

    async fn handle_gate(
        &self,
        planner: &dyn AgentLoopPlannerInternal,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        mut state: LoopExecutionState,
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
                Ok(BatchStep::Continue(state))
            }
            GateOutcome::Abort { gate, failure_kind } => {
                state.gate_state = gate;
                let checked = self.checkpoint(host, state, CheckpointKind::Final).await?;
                Ok(BatchStep::Exit(failed_exit(
                    host,
                    checked.state,
                    failure_kind,
                    Some(checked.checkpoint_id),
                )))
            }
        }
    }

    async fn exit_for_stop(
        &self,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        state: LoopExecutionState,
        kind: StopKind,
    ) -> Result<LoopExit, AgentLoopExecutorError> {
        match kind {
            StopKind::GracefulStop => {
                let checked = self.checkpoint(host, state, CheckpointKind::Final).await?;
                Ok(completed_exit(
                    host,
                    checked.state,
                    Some(checked.checkpoint_id),
                ))
            }
            StopKind::NoProgressDetected => {
                let checked = self.checkpoint(host, state, CheckpointKind::Final).await?;
                Ok(failed_exit(
                    host,
                    checked.state,
                    LoopFailureKind::NoProgressDetected,
                    Some(checked.checkpoint_id),
                ))
            }
            StopKind::Aborted(failure_kind) => {
                let checked = self.checkpoint(host, state, CheckpointKind::Final).await?;
                Ok(failed_exit(
                    host,
                    checked.state,
                    failure_kind,
                    Some(checked.checkpoint_id),
                ))
            }
        }
    }

    async fn checkpoint(
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
        Ok(CheckpointWrite {
            state,
            checkpoint_id,
            state_ref,
        })
    }

    async fn drain_user_inputs(
        &self,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        mut state: LoopExecutionState,
    ) -> Result<LoopExecutionState, AgentLoopExecutorError> {
        let batch = host
            .poll_inputs(state.input_cursor.clone(), MAX_INPUT_DRAIN)
            .await
            .map_err(|_| AgentLoopExecutorError::HostUnavailable {
                stage: HostStage::Input,
            })?;
        let consumed = batch.inputs.iter().any(|input| {
            matches!(
                input,
                LoopInput::UserMessage { .. } | LoopInput::Steering { .. }
            )
        });
        if consumed {
            host.ack_inputs(batch.next_cursor.clone())
                .await
                .map_err(|_| AgentLoopExecutorError::HostUnavailable {
                    stage: HostStage::Input,
                })?;
            state.input_cursor = batch.next_cursor;
        }
        Ok(state)
    }

    async fn drain_followup(
        &self,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        mut state: LoopExecutionState,
    ) -> Result<(LoopExecutionState, bool), AgentLoopExecutorError> {
        let batch = host
            .poll_inputs(state.input_cursor.clone(), MAX_INPUT_DRAIN)
            .await
            .map_err(|_| AgentLoopExecutorError::HostUnavailable {
                stage: HostStage::Input,
            })?;
        let consumed = batch.inputs.iter().any(|input| {
            matches!(
                input,
                LoopInput::FollowUp { .. } | LoopInput::UserMessage { .. }
            )
        });
        if consumed {
            host.ack_inputs(batch.next_cursor.clone())
                .await
                .map_err(|_| AgentLoopExecutorError::HostUnavailable {
                    stage: HostStage::Input,
                })?;
            state.input_cursor = batch.next_cursor;
        }
        Ok((state, consumed))
    }
}

enum ModelStep {
    Response(
        Box<LoopExecutionState>,
        ironclaw_turns::run_profile::LoopModelResponse,
    ),
    Exit(LoopExit),
}

fn completed_exit(
    host: &(dyn AgentLoopDriverHost + Send + Sync),
    state: LoopExecutionState,
    final_checkpoint_id: Option<ironclaw_turns::TurnCheckpointId>,
) -> LoopExit {
    LoopExit::Completed(LoopCompleted {
        completion_kind: if state.assistant_refs.is_empty() {
            LoopCompletionKind::NoReply
        } else {
            LoopCompletionKind::FinalReply
        },
        reply_message_refs: state.assistant_refs,
        result_refs: state.result_refs,
        final_checkpoint_id,
        usage_summary_ref: None,
        exit_id: exit_id(host, "completed").unwrap_or_else(|_| {
            LoopExitId::new("exit:completed").expect("static exit id is valid")
        }),
    })
}

fn failed_exit(
    host: &(dyn AgentLoopDriverHost + Send + Sync),
    _state: LoopExecutionState,
    reason_kind: LoopFailureKind,
    checkpoint_id: Option<ironclaw_turns::TurnCheckpointId>,
) -> LoopExit {
    LoopExit::Failed(LoopFailed {
        reason_kind,
        checkpoint_id,
        usage_summary_ref: None,
        diagnostic_ref: None,
        exit_id: exit_id(host, "failed")
            .unwrap_or_else(|_| LoopExitId::new("exit:failed").expect("static exit id is valid")),
    })
}

fn exit_id(
    host: &(dyn AgentLoopDriverHost + Send + Sync),
    suffix: &'static str,
) -> Result<LoopExitId, AgentLoopExecutorError> {
    LoopExitId::new(format!("exit:{}-{suffix}", host.run_context().run_id)).map_err(|_| {
        AgentLoopExecutorError::PlannerContract {
            detail: "run id could not be represented as loop exit id",
        }
    })
}

fn checkpoint_kind_to_host(kind: CheckpointKind) -> LoopCheckpointKind {
    match kind {
        CheckpointKind::BeforeModel => LoopCheckpointKind::BeforeModel,
        CheckpointKind::BeforeSideEffect => LoopCheckpointKind::BeforeSideEffect,
        CheckpointKind::BeforeBlock => LoopCheckpointKind::BeforeBlock,
        CheckpointKind::Final => LoopCheckpointKind::Final,
    }
}

fn blocked_kind(kind: GateKind) -> LoopBlockedKind {
    match kind {
        GateKind::Approval => LoopBlockedKind::Approval,
        GateKind::Auth => LoopBlockedKind::Auth,
        GateKind::Resource => LoopBlockedKind::Resource,
    }
}

fn model_preference_to_host(
    preference: ModelPreference,
) -> Result<Option<ironclaw_turns::ModelProfileId>, AgentLoopExecutorError> {
    match preference {
        ModelPreference::Primary => Ok(None),
        ModelPreference::Fallback { .. } => Err(AgentLoopExecutorError::PlannerContract {
            detail: "fallback model preference requires model route chain support",
        }),
    }
}

fn model_error_class(error: &AgentLoopHostError) -> Option<ModelErrorClass> {
    match error.kind {
        AgentLoopHostErrorKind::Unavailable => Some(ModelErrorClass::Unavailable),
        AgentLoopHostErrorKind::Internal => Some(ModelErrorClass::Internal),
        AgentLoopHostErrorKind::BudgetExceeded => Some(ModelErrorClass::ContextOverflow),
        AgentLoopHostErrorKind::Cancelled => Some(ModelErrorClass::Transient),
        AgentLoopHostErrorKind::Unauthorized
        | AgentLoopHostErrorKind::ScopeMismatch
        | AgentLoopHostErrorKind::StaleSurface
        | AgentLoopHostErrorKind::InvalidInvocation
        | AgentLoopHostErrorKind::PolicyDenied
        | AgentLoopHostErrorKind::CheckpointRejected
        | AgentLoopHostErrorKind::TranscriptWriteFailed => None,
    }
}

fn capability_error_class(kind: &str) -> CapabilityErrorClass {
    match kind {
        "transient" | "unavailable" => CapabilityErrorClass::Transient,
        "input_invalid" => CapabilityErrorClass::InputInvalid,
        "policy_denied" => CapabilityErrorClass::PolicyDenied,
        "internal" => CapabilityErrorClass::Internal,
        _ => CapabilityErrorClass::Permanent,
    }
}

fn capability_failure_kind(kind: &str) -> LoopFailureKind {
    if kind == "policy_denied" {
        LoopFailureKind::PolicyDenied
    } else {
        LoopFailureKind::CapabilityProtocolError
    }
}

fn honor_retry_alteration(
    alteration: Option<&RetryAlteration>,
) -> Result<(), AgentLoopExecutorError> {
    if matches!(alteration, Some(RetryAlteration::AdvanceFallback)) {
        return Err(AgentLoopExecutorError::PlannerContract {
            detail: "fallback model route alteration requires model route chain support",
        });
    }
    Ok(())
}

fn capability_invocation_from_candidate(call: CapabilityCallCandidate) -> CapabilityInvocation {
    CapabilityInvocation {
        surface_version: call.surface_version,
        capability_id: call.capability_id,
        input_ref: call.input_ref,
    }
}

fn capability_summary(
    surface: &VisibleCapabilitySurface,
    call: &CapabilityCallCandidate,
) -> CapabilityCallSummary {
    let concurrency_hint = surface
        .descriptors
        .iter()
        .find(|descriptor| descriptor.capability_id == call.capability_id)
        .map(|descriptor| descriptor.concurrency_hint)
        .unwrap_or(ironclaw_turns::run_profile::ConcurrencyHint::Exclusive);
    CapabilityCallSummary {
        name: call.capability_id.clone(),
        concurrency_hint,
    }
}

fn capability_is_visible(
    surface: &VisibleCapabilitySurface,
    call: &CapabilityCallCandidate,
) -> bool {
    if call.surface_version != surface.version {
        return false;
    }
    surface
        .descriptors
        .iter()
        .any(|descriptor| descriptor.capability_id == call.capability_id)
}

fn apply_capability_filter(surface: &mut VisibleCapabilitySurface, filter: &CapabilityFilter) {
    match filter {
        CapabilityFilter::All => {}
        CapabilityFilter::AllowOnly(allowed) => {
            surface
                .descriptors
                .retain(|descriptor| allowed.contains(&descriptor.capability_id));
        }
        CapabilityFilter::Deny(denied) => {
            surface
                .descriptors
                .retain(|descriptor| !denied.contains(&descriptor.capability_id));
        }
    }
}

fn push_call_signature_once(
    state: &mut LoopExecutionState,
    signatures: &mut HashSet<CapabilityCallSignature>,
    call: &CapabilityCallCandidate,
) -> Result<(), AgentLoopExecutorError> {
    let args = serde_json::json!({ "input_ref": call.input_ref.as_str() });
    let signature =
        CapabilityCallSignature::from_call(call.capability_id.clone(), &args).map_err(|_| {
            AgentLoopExecutorError::PlannerContract {
                detail: "capability call signature could not be built",
            }
        })?;
    if signatures.insert(signature.clone()) {
        state.recent_call_signatures.push(signature);
    }
    Ok(())
}

fn push_completed_result(state: &mut LoopExecutionState, result: CapabilityResultMessage) {
    state.result_refs.push(result.result_ref);
    if result.terminate_hint {
        state.stop_state.terminate_hints_in_last_batch = state
            .stop_state
            .terminate_hints_in_last_batch
            .saturating_add(1);
    }
}

fn opaque_token(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.') {
                character
            } else {
                '-'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use ironclaw_host_api::{CapabilityId, RuntimeKind, TenantId, ThreadId};
    use ironclaw_turns::{
        AgentLoopDriverDescriptor, LoopGateRef, LoopMessageRef, LoopResultRef, RunProfileId,
        RunProfileVersion, TurnCheckpointId, TurnId, TurnRunId, TurnScope,
        run_profile::{
            AgentLoopHostError, AgentLoopHostErrorKind, CancellationPolicy,
            CapabilityDescriptorView, CapabilityInputRef, CapabilitySurfaceProfileId,
            CapabilitySurfaceVersion, CheckpointPolicy, CheckpointSchemaId, ConcurrencyClass,
            ContextProfileId, LoopCheckpointRequest, LoopCheckpointStateRef, LoopContextBundle,
            LoopContextRequest, LoopContextSummary, LoopDriverId, LoopInputBatch, LoopInputCursor,
            LoopModelMessage, LoopModelResponse, LoopPromptBundle, LoopPromptBundleRef,
            LoopPromptBundleRequest, LoopRunContext, LoopRunInfoPort, ModelProfileId,
            ModelStreamChunk, RedactedRunProfileProvenance, ResolvedRunProfile,
            ResourceBudgetPolicy, ResourceBudgetTier, RunClassId, RunProfileFingerprint,
            RuntimeProfileConstraints, SchedulingClass, StageCheckpointPayloadRequest,
            SteeringPolicy,
        },
    };

    use crate::default_planner::DefaultPlanner;
    use crate::family::{ComponentDigest, ComponentIdentity, LoopFamily, LoopFamilyId};
    use crate::strategies::CapabilityStrategy;

    use super::*;

    #[allow(dead_code)]
    fn _check(_: &dyn AgentLoopExecutor) {}

    #[derive(Clone)]
    struct MockHost {
        context: LoopRunContext,
        model_responses: Arc<Mutex<VecDeque<LoopModelResponse>>>,
        model_requests: Arc<Mutex<Vec<LoopModelRequest>>>,
        batch_outcomes: Arc<Mutex<VecDeque<ironclaw_turns::run_profile::CapabilityBatchOutcome>>>,
        single_outcomes: Arc<Mutex<VecDeque<CapabilityOutcome>>>,
        checkpoints: Arc<Mutex<Vec<LoopCheckpointKind>>>,
        batch_invocations: Arc<Mutex<Vec<CapabilityBatchInvocation>>>,
        single_invocations: Arc<Mutex<Vec<CapabilityInvocation>>>,
        staged_payloads: Arc<Mutex<Vec<StageCheckpointPayloadRequest>>>,
        prompt_surface_version: Option<CapabilitySurfaceVersion>,
        visible_surface_version: CapabilitySurfaceVersion,
    }

    impl MockHost {
        fn new(model_responses: Vec<LoopModelResponse>) -> Self {
            Self {
                context: test_run_context(),
                model_responses: Arc::new(Mutex::new(model_responses.into())),
                model_requests: Arc::new(Mutex::new(Vec::new())),
                batch_outcomes: Arc::new(Mutex::new(VecDeque::new())),
                single_outcomes: Arc::new(Mutex::new(VecDeque::new())),
                checkpoints: Arc::new(Mutex::new(Vec::new())),
                batch_invocations: Arc::new(Mutex::new(Vec::new())),
                single_invocations: Arc::new(Mutex::new(Vec::new())),
                staged_payloads: Arc::new(Mutex::new(Vec::new())),
                prompt_surface_version: Some(surface_version()),
                visible_surface_version: surface_version(),
            }
        }

        fn with_prompt_surface_version(
            mut self,
            version: Option<CapabilitySurfaceVersion>,
        ) -> Self {
            self.prompt_surface_version = version;
            self
        }

        fn with_batch_outcomes(
            self,
            outcomes: Vec<ironclaw_turns::run_profile::CapabilityBatchOutcome>,
        ) -> Self {
            *self.batch_outcomes.lock().expect("lock") = outcomes.into();
            self
        }

        fn with_single_outcomes(self, outcomes: Vec<CapabilityOutcome>) -> Self {
            *self.single_outcomes.lock().expect("lock") = outcomes.into();
            self
        }

        fn checkpoint_kinds(&self) -> Vec<LoopCheckpointKind> {
            self.checkpoints.lock().expect("lock").clone()
        }

        fn batch_invocations(&self) -> Vec<CapabilityBatchInvocation> {
            self.batch_invocations.lock().expect("lock").clone()
        }

        fn single_invocations(&self) -> Vec<CapabilityInvocation> {
            self.single_invocations.lock().expect("lock").clone()
        }

        fn model_requests(&self) -> Vec<LoopModelRequest> {
            self.model_requests.lock().expect("lock").clone()
        }

        fn staged_payloads(&self) -> Vec<StageCheckpointPayloadRequest> {
            self.staged_payloads.lock().expect("lock").clone()
        }
    }

    struct FixedCapabilityStrategy {
        filter: CapabilityFilter,
    }

    #[async_trait]
    impl CapabilityStrategy for FixedCapabilityStrategy {
        async fn filter(&self, _state: &LoopExecutionState) -> CapabilityFilter {
            self.filter.clone()
        }
    }

    impl ironclaw_turns::run_profile::LoopRunInfoPort for MockHost {
        fn run_context(&self) -> &LoopRunContext {
            &self.context
        }
    }

    #[async_trait]
    impl ironclaw_turns::run_profile::LoopContextPort for MockHost {
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
    impl ironclaw_turns::run_profile::LoopPromptPort for MockHost {
        async fn build_prompt_bundle(
            &self,
            _request: LoopPromptBundleRequest,
        ) -> Result<LoopPromptBundle, AgentLoopHostError> {
            Ok(LoopPromptBundle {
                bundle_ref: LoopPromptBundleRef::for_run(&self.context, "bundle").expect("valid"),
                messages: vec![LoopModelMessage {
                    role: "user".to_string(),
                    content_ref: LoopMessageRef::new("msg:user").expect("valid"),
                }],
                surface_version: self.prompt_surface_version.clone(),
                context_summaries: std::collections::HashMap::<String, LoopContextSummary>::new(),
            })
        }
    }

    #[async_trait]
    impl ironclaw_turns::run_profile::LoopInputPort for MockHost {
        async fn poll_inputs(
            &self,
            after: LoopInputCursor,
            _limit: usize,
        ) -> Result<LoopInputBatch, AgentLoopHostError> {
            Ok(LoopInputBatch {
                inputs: Vec::new(),
                next_cursor: after,
            })
        }

        async fn ack_inputs(&self, _cursor: LoopInputCursor) -> Result<(), AgentLoopHostError> {
            Ok(())
        }
    }

    #[async_trait]
    impl ironclaw_turns::run_profile::LoopModelPort for MockHost {
        async fn stream_model(
            &self,
            request: LoopModelRequest,
        ) -> Result<LoopModelResponse, AgentLoopHostError> {
            self.model_requests.lock().expect("lock").push(request);
            self.model_responses
                .lock()
                .expect("lock")
                .pop_front()
                .ok_or_else(|| {
                    AgentLoopHostError::new(
                        AgentLoopHostErrorKind::Internal,
                        "model script exhausted",
                    )
                })
        }
    }

    #[async_trait]
    impl ironclaw_turns::run_profile::LoopCapabilityPort for MockHost {
        async fn visible_capabilities(
            &self,
            _request: VisibleCapabilityRequest,
        ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
            Ok(VisibleCapabilitySurface {
                version: self.visible_surface_version.clone(),
                descriptors: vec![CapabilityDescriptorView {
                    capability_id: capability_id(),
                    provider: None,
                    runtime: RuntimeKind::FirstParty,
                    safe_name: "demo".to_string(),
                    safe_description: "demo capability".to_string(),
                    concurrency_hint: ironclaw_turns::run_profile::ConcurrencyHint::SafeForParallel,
                }],
            })
        }

        async fn invoke_capability(
            &self,
            request: CapabilityInvocation,
        ) -> Result<CapabilityOutcome, AgentLoopHostError> {
            self.single_invocations.lock().expect("lock").push(request);
            self.single_outcomes
                .lock()
                .expect("lock")
                .pop_front()
                .ok_or_else(|| {
                    AgentLoopHostError::new(
                        AgentLoopHostErrorKind::Internal,
                        "single script exhausted",
                    )
                })
        }

        async fn invoke_capability_batch(
            &self,
            request: CapabilityBatchInvocation,
        ) -> Result<ironclaw_turns::run_profile::CapabilityBatchOutcome, AgentLoopHostError>
        {
            self.batch_invocations.lock().expect("lock").push(request);
            self.batch_outcomes
                .lock()
                .expect("lock")
                .pop_front()
                .ok_or_else(|| {
                    AgentLoopHostError::new(
                        AgentLoopHostErrorKind::Internal,
                        "batch script exhausted",
                    )
                })
        }
    }

    #[async_trait]
    impl ironclaw_turns::run_profile::LoopTranscriptPort for MockHost {
        async fn finalize_assistant_message(
            &self,
            _request: FinalizeAssistantMessage,
        ) -> Result<LoopMessageRef, AgentLoopHostError> {
            Ok(LoopMessageRef::new("msg:assistant").expect("valid"))
        }
    }

    #[async_trait]
    impl ironclaw_turns::run_profile::LoopCheckpointPort for MockHost {
        async fn checkpoint(
            &self,
            request: LoopCheckpointRequest,
        ) -> Result<TurnCheckpointId, AgentLoopHostError> {
            self.checkpoints.lock().expect("lock").push(request.kind);
            Ok(TurnCheckpointId::new())
        }

        async fn stage_checkpoint_payload(
            &self,
            request: StageCheckpointPayloadRequest,
        ) -> Result<LoopCheckpointStateRef, AgentLoopHostError> {
            self.staged_payloads.lock().expect("lock").push(request);
            LoopCheckpointStateRef::for_run(&self.context, "state")
                .map_err(|error| AgentLoopHostError::new(AgentLoopHostErrorKind::Internal, error))
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

    #[tokio::test]
    async fn reply_only_completes_with_final_checkpoint() {
        let host = MockHost::new(vec![reply_response()]);
        let executor = CanonicalAgentLoopExecutor;
        let state = LoopExecutionState::initial_for_run(host.run_context());

        let exit = executor
            .execute_family(&crate::families::default(), &host, state)
            .await
            .expect("execute");

        match exit {
            LoopExit::Completed(completed) => {
                assert_eq!(completed.reply_message_refs.len(), 1);
                assert!(completed.final_checkpoint_id.is_some());
            }
            other => panic!("expected completed, got {other:?}"),
        }
        assert_eq!(
            host.checkpoint_kinds(),
            vec![LoopCheckpointKind::BeforeModel, LoopCheckpointKind::Final]
        );
    }

    #[tokio::test]
    async fn terminate_hint_after_batch_completes_without_extra_model_call() {
        let host = MockHost::new(vec![calls_response()]).with_batch_outcomes(vec![
            ironclaw_turns::run_profile::CapabilityBatchOutcome {
                outcomes: vec![CapabilityOutcome::Completed(CapabilityResultMessage {
                    result_ref: LoopResultRef::new("result:done").expect("valid"),
                    safe_summary: "done".to_string(),
                    terminate_hint: true,
                })],
                stopped_on_suspension: false,
            },
        ]);
        let executor = CanonicalAgentLoopExecutor;
        let state = LoopExecutionState::initial_for_run(host.run_context());

        let exit = executor
            .execute_family(&crate::families::default(), &host, state)
            .await
            .expect("execute");

        assert!(matches!(exit, LoopExit::Completed(_)));
        assert_eq!(
            host.checkpoint_kinds(),
            vec![
                LoopCheckpointKind::BeforeModel,
                LoopCheckpointKind::BeforeSideEffect,
                LoopCheckpointKind::Final,
            ]
        );
    }

    #[tokio::test]
    async fn gate_blocks_with_before_block_checkpoint() {
        let host = MockHost::new(vec![calls_response()]).with_batch_outcomes(vec![
            ironclaw_turns::run_profile::CapabilityBatchOutcome {
                outcomes: vec![CapabilityOutcome::ApprovalRequired {
                    gate_ref: LoopGateRef::new("gate:approval").expect("valid"),
                    safe_summary: "approval required".to_string(),
                }],
                stopped_on_suspension: true,
            },
        ]);
        let executor = CanonicalAgentLoopExecutor;
        let state = LoopExecutionState::initial_for_run(host.run_context());

        let exit = executor
            .execute_family(&crate::families::default(), &host, state)
            .await
            .expect("execute");

        assert!(matches!(exit, LoopExit::Blocked(_)));
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
    async fn strategy_filtered_capability_denial_does_not_invoke_host_and_records_policy_denied() {
        let family = family_with_capability_filter(CapabilityFilter::Deny(vec![capability_id()]));
        let host = MockHost::new(vec![calls_response(), reply_response()]);
        let executor = CanonicalAgentLoopExecutor;
        let state = LoopExecutionState::initial_for_run(host.run_context());

        let exit = executor
            .execute_family(&family, &host, state)
            .await
            .expect("execute");

        assert!(matches!(exit, LoopExit::Completed(_)));
        assert!(host.batch_invocations().is_empty());
        assert!(host.single_invocations().is_empty());

        let staged_states = host
            .staged_payloads()
            .into_iter()
            .map(|request| {
                LoopExecutionState::from_checkpoint_payload(
                    &request.payload,
                    checkpoint_kind_from_host(request.kind),
                )
                .expect("checkpoint payload")
            })
            .collect::<Vec<_>>();
        assert!(staged_states.iter().any(|state| {
            state
                .recent_failure_kinds
                .iter()
                .any(|kind| *kind == LoopFailureKind::PolicyDenied)
        }));
    }

    #[tokio::test]
    async fn model_request_uses_current_visible_surface_not_prompt_bundle_version() {
        let host = MockHost::new(vec![reply_response()])
            .with_prompt_surface_version(Some(stale_surface_version()));
        let executor = CanonicalAgentLoopExecutor;
        let state = LoopExecutionState::initial_for_run(host.run_context());

        let exit = executor
            .execute_family(&crate::families::default(), &host, state)
            .await
            .expect("execute");

        assert!(matches!(exit, LoopExit::Completed(_)));
        let requests = host.model_requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].surface_version, Some(surface_version()));
    }

    #[tokio::test]
    async fn stale_surface_capability_call_is_policy_denied_before_host_invocation() {
        let host = MockHost::new(vec![stale_surface_calls_response(), reply_response()]);
        let executor = CanonicalAgentLoopExecutor;
        let state = LoopExecutionState::initial_for_run(host.run_context());

        let exit = executor
            .execute_family(&crate::families::default(), &host, state)
            .await
            .expect("execute");

        assert!(matches!(exit, LoopExit::Completed(_)));
        assert!(host.batch_invocations().is_empty());
        assert!(host.single_invocations().is_empty());

        let staged_states = host
            .staged_payloads()
            .into_iter()
            .map(|request| {
                LoopExecutionState::from_checkpoint_payload(
                    &request.payload,
                    checkpoint_kind_from_host(request.kind),
                )
                .expect("checkpoint payload")
            })
            .collect::<Vec<_>>();
        assert!(staged_states.iter().any(|state| {
            state
                .recent_failure_kinds
                .iter()
                .any(|kind| *kind == LoopFailureKind::PolicyDenied)
        }));
        assert!(
            staged_states
                .iter()
                .any(|state| state.stop_state.last_batch_total == 0)
        );
    }

    #[tokio::test]
    async fn last_batch_total_counts_only_visible_invoked_calls() {
        let host = MockHost::new(vec![mixed_surface_calls_response()]).with_batch_outcomes(vec![
            ironclaw_turns::run_profile::CapabilityBatchOutcome {
                outcomes: vec![CapabilityOutcome::Completed(CapabilityResultMessage {
                    result_ref: LoopResultRef::new("result:visible").expect("valid"),
                    safe_summary: "visible call completed".to_string(),
                    terminate_hint: true,
                })],
                stopped_on_suspension: false,
            },
        ]);
        let executor = CanonicalAgentLoopExecutor;
        let state = LoopExecutionState::initial_for_run(host.run_context());

        let exit = executor
            .execute_family(&crate::families::default(), &host, state)
            .await
            .expect("execute");

        assert!(matches!(exit, LoopExit::Completed(_)));
        assert_eq!(host.model_requests().len(), 1);

        let batch_invocations = host.batch_invocations();
        assert_eq!(batch_invocations.len(), 1);
        assert_eq!(batch_invocations[0].invocations.len(), 1);
        assert_eq!(
            batch_invocations[0].invocations[0].surface_version,
            surface_version()
        );
    }

    #[tokio::test]
    async fn checkpoint_payload_rehydrates_with_written_marker() {
        let host = MockHost::new(vec![reply_response()]);
        let executor = CanonicalAgentLoopExecutor;
        let state = LoopExecutionState::initial_for_run(host.run_context());

        let exit = executor
            .execute_family(&crate::families::default(), &host, state)
            .await
            .expect("execute");

        assert!(matches!(exit, LoopExit::Completed(_)));
        let staged_payloads = host.staged_payloads();
        let final_payload = staged_payloads
            .iter()
            .rev()
            .find(|request| request.kind == LoopCheckpointKind::Final)
            .expect("final checkpoint payload");
        let rehydrated = LoopExecutionState::from_checkpoint_payload(
            &final_payload.payload,
            CheckpointKind::Final,
        )
        .expect("checkpoint payload");

        assert_eq!(
            rehydrated.last_checkpoint,
            Some(crate::state::CheckpointMarker {
                kind: CheckpointKind::Final,
                iteration_at_checkpoint: rehydrated.iteration,
            })
        );
    }

    #[tokio::test]
    async fn retry_uses_single_call_invocation() {
        let host = MockHost::new(vec![calls_response()])
            .with_batch_outcomes(vec![ironclaw_turns::run_profile::CapabilityBatchOutcome {
                outcomes: vec![CapabilityOutcome::Failed(
                    ironclaw_turns::run_profile::CapabilityFailure {
                        error_kind: "transient".to_string(),
                        safe_summary: "temporary failure".to_string(),
                    },
                )],
                stopped_on_suspension: false,
            }])
            .with_single_outcomes(vec![CapabilityOutcome::Completed(
                CapabilityResultMessage {
                    result_ref: LoopResultRef::new("result:retry").expect("valid"),
                    safe_summary: "retry completed".to_string(),
                    terminate_hint: true,
                },
            )]);
        let executor = CanonicalAgentLoopExecutor;
        let state = LoopExecutionState::initial_for_run(host.run_context());

        let exit = executor
            .execute_family(&crate::families::default(), &host, state)
            .await
            .expect("execute");

        assert!(matches!(exit, LoopExit::Completed(_)));
    }

    fn reply_response() -> LoopModelResponse {
        LoopModelResponse {
            chunks: vec![ModelStreamChunk {
                safe_text_delta: "hello".to_string(),
            }],
            output: ParentLoopOutput::AssistantReply(ironclaw_turns::run_profile::AssistantReply {
                content: "hello".to_string(),
            }),
            effective_model_profile_id: ModelProfileId::new("model").expect("valid"),
        }
    }

    fn calls_response() -> LoopModelResponse {
        LoopModelResponse {
            chunks: Vec::new(),
            output: ParentLoopOutput::CapabilityCalls(vec![CapabilityCallCandidate {
                surface_version: surface_version(),
                capability_id: capability_id(),
                input_ref: CapabilityInputRef::new("input:demo").expect("valid"),
            }]),
            effective_model_profile_id: ModelProfileId::new("model").expect("valid"),
        }
    }

    fn stale_surface_calls_response() -> LoopModelResponse {
        LoopModelResponse {
            chunks: Vec::new(),
            output: ParentLoopOutput::CapabilityCalls(vec![CapabilityCallCandidate {
                surface_version: stale_surface_version(),
                capability_id: capability_id(),
                input_ref: CapabilityInputRef::new("input:demo").expect("valid"),
            }]),
            effective_model_profile_id: ModelProfileId::new("model").expect("valid"),
        }
    }

    fn mixed_surface_calls_response() -> LoopModelResponse {
        LoopModelResponse {
            chunks: Vec::new(),
            output: ParentLoopOutput::CapabilityCalls(vec![
                CapabilityCallCandidate {
                    surface_version: stale_surface_version(),
                    capability_id: capability_id(),
                    input_ref: CapabilityInputRef::new("input:stale").expect("valid"),
                },
                CapabilityCallCandidate {
                    surface_version: surface_version(),
                    capability_id: capability_id(),
                    input_ref: CapabilityInputRef::new("input:visible").expect("valid"),
                },
            ]),
            effective_model_profile_id: ModelProfileId::new("model").expect("valid"),
        }
    }

    fn capability_id() -> CapabilityId {
        CapabilityId::new("demo.echo").expect("valid")
    }

    fn surface_version() -> CapabilitySurfaceVersion {
        CapabilitySurfaceVersion::new("surface:v1").expect("valid")
    }

    fn stale_surface_version() -> CapabilitySurfaceVersion {
        CapabilitySurfaceVersion::new("surface:stale").expect("valid")
    }

    fn family_with_capability_filter(filter: CapabilityFilter) -> LoopFamily {
        let planner = DefaultPlanner::compose_default()
            .with_capability(Arc::new(FixedCapabilityStrategy { filter }));
        let id = LoopFamilyId::new("executor-filter-test");
        let version =
            ComponentIdentity::from_static("executor-filter-test", ComponentDigest([1; 32]));
        LoopFamily::new(id, version, Arc::new(planner))
    }

    fn checkpoint_kind_from_host(kind: LoopCheckpointKind) -> CheckpointKind {
        match kind {
            LoopCheckpointKind::BeforeModel => CheckpointKind::BeforeModel,
            LoopCheckpointKind::BeforeSideEffect => CheckpointKind::BeforeSideEffect,
            LoopCheckpointKind::BeforeBlock => CheckpointKind::BeforeBlock,
            LoopCheckpointKind::Final => CheckpointKind::Final,
        }
    }

    fn test_run_context() -> LoopRunContext {
        let scope = TurnScope::new(
            TenantId::new("tenant-executor").expect("valid"),
            None,
            None,
            ThreadId::new("thread-executor").expect("valid"),
        );
        let descriptor = AgentLoopDriverDescriptor {
            id: LoopDriverId::new("executor_test_driver").expect("valid"),
            version: RunProfileVersion::new(1),
            checkpoint_schema_id: Some(
                CheckpointSchemaId::new("executor_test_checkpoint").expect("valid"),
            ),
            checkpoint_schema_version: Some(RunProfileVersion::new(1)),
        };
        let resolved_run_profile = ResolvedRunProfile {
            run_class_id: RunClassId::new("executor_test_class").expect("valid"),
            profile_id: RunProfileId::default_profile(),
            profile_version: RunProfileVersion::new(1),
            loop_driver: descriptor.clone(),
            checkpoint_schema_id: descriptor
                .checkpoint_schema_id
                .clone()
                .expect("descriptor checkpoint id"),
            checkpoint_schema_version: descriptor
                .checkpoint_schema_version
                .expect("descriptor checkpoint version"),
            model_profile_id: ModelProfileId::new("executor_test_model").expect("valid"),
            capability_surface_profile_id: CapabilitySurfaceProfileId::new(
                "executor_test_capabilities",
            )
            .expect("valid"),
            context_profile_id: ContextProfileId::new("executor_test_context").expect("valid"),
            steering_policy: SteeringPolicy {
                allow_steering: false,
                allow_interrupt: true,
                allow_driver_specific_nudges: false,
            },
            cancellation_policy: CancellationPolicy {
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
                tier: ResourceBudgetTier::new("executor_test_tier").expect("valid"),
                max_model_calls: 32,
                max_capability_invocations: 64,
            },
            runtime_constraints: RuntimeProfileConstraints {
                allow_raw_runtime_backend_selection: false,
                allow_broad_capability_surface: false,
            },
            runner_pool_id: None,
            scheduling_class: SchedulingClass::new("interactive").expect("valid"),
            concurrency_class: ConcurrencyClass::new("thread_serial").expect("valid"),
            resolution_fingerprint: RunProfileFingerprint::new("executor-test-fingerprint")
                .expect("valid"),
            provenance: RedactedRunProfileProvenance {
                sources: vec![],
                effective_privileges: vec![],
            },
        };
        LoopRunContext::new(scope, TurnId::new(), TurnRunId::new(), resolved_run_profile)
    }
}
