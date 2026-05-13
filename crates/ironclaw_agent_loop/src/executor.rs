//! Canonical agent-loop executor.
//!
//! The executor owns loop mechanics. Loop families own strategy composition.
//! See `docs/reborn/agent-loop-skeleton.md` section 8 for the canonical tick.

use std::collections::HashSet;

use async_trait::async_trait;
use ironclaw_turns::{
    LoopBlocked, LoopBlockedKind, LoopCancelled, LoopCancelledReasonKind, LoopCompleted,
    LoopCompletionKind, LoopExit, LoopExitId, LoopFailed, LoopFailureKind,
    run_profile::{
        AgentLoopDriverHost, AgentLoopHostError, AgentLoopHostErrorKind, BatchPolicyKind,
        CapabilityBatchInvocation, CapabilityCallCandidate, CapabilityFailureKind,
        CapabilityInvocation, CapabilityOutcome, CapabilityResultMessage, FinalizeAssistantMessage,
        LoopCheckpointKind, LoopCheckpointRequest, LoopDriverNoteKind, LoopGateKind, LoopInput,
        LoopInputAckToken, LoopInputBatch, LoopModelRequest, LoopProgressEvent, ParentLoopOutput,
        StageCheckpointPayloadRequest, VisibleCapabilityRequest, VisibleCapabilitySurface,
    },
};

use crate::{
    family::LoopFamily,
    planner::AgentLoopPlannerInternal,
    state::{CapabilityCallSignature, CheckpointKind, LoopExecutionState},
    strategies::{
        BatchPolicy, CapabilityCallSummary, CapabilityErrorClass, CapabilityErrorSummary,
        CapabilityFilter, GateKind, GateOutcome, ModelErrorClass, ModelErrorSummary,
        ModelPreference, RecoveryOutcome, RetryAlteration, SanitizedStrategySummary, StopKind,
        StopOutcome, TurnEndKind, TurnSummary,
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
    Continue(Box<LoopExecutionState>),
    Exit(LoopExit),
}

#[derive(Debug, Default)]
struct PendingInputAck {
    tokens: Vec<LoopInputAckToken>,
}

impl PendingInputAck {
    fn is_empty(&self) -> bool {
        self.tokens.is_empty()
    }

    fn replace(&mut self, tokens: Vec<LoopInputAckToken>) -> Result<(), AgentLoopExecutorError> {
        if !tokens.is_empty() && !self.tokens.is_empty() {
            return Err(AgentLoopExecutorError::PlannerContract {
                detail: "input ack was advanced before prior ack became durable",
            });
        }
        if !tokens.is_empty() {
            self.tokens = tokens;
        }
        Ok(())
    }

    async fn ack(
        &mut self,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
    ) -> Result<(), AgentLoopExecutorError> {
        if self.tokens.is_empty() {
            return Ok(());
        }
        let tokens = std::mem::take(&mut self.tokens);
        host.ack_inputs(tokens)
            .await
            .map_err(|_| AgentLoopExecutorError::HostUnavailable {
                stage: HostStage::Input,
            })
    }
}

#[derive(Debug)]
struct DrainedInputs {
    state: LoopExecutionState,
    drained: bool,
    ack_tokens: Vec<LoopInputAckToken>,
    cancelled_reason_kind: Option<LoopCancelledReasonKind>,
}

#[derive(Debug)]
enum CancelCheck {
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
        let mut pending_input_ack = PendingInputAck::default();

        loop {
            state = match self.checkpoint_and_exit_if_cancelled(host, state).await? {
                CancelCheck::Continue(state) => state,
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
                    CancelCheck::Continue(state) => state,
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
            state = match self.checkpoint_and_exit_if_cancelled(host, state).await? {
                CancelCheck::Continue(state) => state,
                CancelCheck::Exit(exit) => return Ok(exit),
            };

            let context_request = planner.context().plan_context_request(&state).await;
            let prompt_mode = context_request.mode;
            state = match self.checkpoint_and_exit_if_cancelled(host, state).await? {
                CancelCheck::Continue(state) => state,
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
            state = match self.checkpoint_and_exit_if_cancelled(host, state).await? {
                CancelCheck::Continue(state) => state,
                CancelCheck::Exit(exit) => return Ok(exit),
            };

            let surface_filter = planner.capability().filter(&state).await;
            state = match self.checkpoint_and_exit_if_cancelled(host, state).await? {
                CancelCheck::Continue(state) => state,
                CancelCheck::Exit(exit) => return Ok(exit),
            };
            let mut surface = host
                .visible_capabilities(VisibleCapabilityRequest)
                .await
                .map_err(|_| AgentLoopExecutorError::HostUnavailable {
                    stage: HostStage::Capability,
                })?;
            apply_capability_filter(&mut surface, &surface_filter);
            state.surface_version = Some(surface.version.clone());
            state = match self.checkpoint_and_exit_if_cancelled(host, state).await? {
                CancelCheck::Continue(state) => state,
                CancelCheck::Exit(exit) => return Ok(exit),
            };

            state = self
                .checkpoint(host, state, CheckpointKind::BeforeModel)
                .await?
                .state;
            pending_input_ack.ack(host).await?;
            state = match self.checkpoint_and_exit_if_cancelled(host, state).await? {
                CancelCheck::Continue(state) => state,
                CancelCheck::Exit(exit) => return Ok(exit),
            };

            let model_preference =
                model_preference_to_host(planner.model().preference(&state).await)?;
            state = match self.checkpoint_and_exit_if_cancelled(host, state).await? {
                CancelCheck::Continue(state) => state,
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
            state = match self.checkpoint_and_exit_if_cancelled(host, state).await? {
                CancelCheck::Continue(state) => state,
                CancelCheck::Exit(exit) => return Ok(exit),
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
                        CancelCheck::Continue(state) => state,
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
                                CancelCheck::Continue(state) => state,
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
                                CancelCheck::Continue(state) => state,
                                CancelCheck::Exit(exit) => return Ok(exit),
                            };
                            if planner.drain().drain_followup(&state).await {
                                state =
                                    match self.checkpoint_and_exit_if_cancelled(host, state).await?
                                    {
                                        CancelCheck::Continue(state) => state,
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
                                state =
                                    match self.checkpoint_and_exit_if_cancelled(host, state).await?
                                    {
                                        CancelCheck::Continue(state) => state,
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
                        CancelCheck::Continue(state) => state,
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
                                CancelCheck::Continue(state) => state,
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
                                CancelCheck::Continue(state) => state,
                                CancelCheck::Exit(exit) => return Ok(exit),
                            };
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
                                CancelCheck::Continue(next) => state = next,
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
                                CancelCheck::Continue(next) => state = next,
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
            CancelCheck::Continue(next) => state = next,
            CancelCheck::Exit(exit) => return Ok(BatchStep::Exit(exit)),
        }

        state = self
            .checkpoint(host, state, CheckpointKind::BeforeSideEffect)
            .await?
            .state;

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
        match self.checkpoint_and_exit_if_cancelled(host, state).await? {
            CancelCheck::Continue(next) => state = next,
            CancelCheck::Exit(exit) => return Ok(BatchStep::Exit(exit)),
        }

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
                    state = match self.checkpoint_and_exit_if_cancelled(host, *next).await? {
                        CancelCheck::Continue(next) => next,
                        CancelCheck::Exit(exit) => return Ok(BatchStep::Exit(exit)),
                    };
                }
                BatchStep::Exit(exit) => return Ok(BatchStep::Exit(exit)),
            }
        }

        Ok(BatchStep::Continue(Box::new(state)))
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
                Ok(BatchStep::Continue(Box::new(state)))
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
                self.fail_unsupported_process_wait(host, state, &handle.process_ref)
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
                    match self.checkpoint_and_exit_if_cancelled(host, state).await? {
                        CancelCheck::Continue(next) => state = next,
                        CancelCheck::Exit(exit) => return Ok(BatchStep::Exit(exit)),
                    }
                    return Ok(BatchStep::Continue(Box::new(state)));
                }
                RecoveryOutcome::Abort {
                    recovery,
                    failure_kind,
                } => {
                    state.recovery_state = recovery;
                    match self.checkpoint_and_exit_if_cancelled(host, state).await? {
                        CancelCheck::Continue(next) => state = next,
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
                        match self.checkpoint_and_exit_if_cancelled(host, state).await? {
                            CancelCheck::Continue(next) => state = next,
                            CancelCheck::Exit(exit) => return Ok(BatchStep::Exit(exit)),
                        }
                        return Ok(BatchStep::Continue(Box::new(state)));
                    }
                    state.recovery_state = recovery;
                    match self.checkpoint_and_exit_if_cancelled(host, state).await? {
                        CancelCheck::Continue(next) => state = next,
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
                    match self.checkpoint_and_exit_if_cancelled(host, state).await? {
                        CancelCheck::Continue(next) => state = next,
                        CancelCheck::Exit(exit) => return Ok(BatchStep::Exit(exit)),
                    }
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
                                push_completed_result(&mut state, result);
                                return Ok(BatchStep::Continue(Box::new(state)));
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
                                return self
                                    .fail_unsupported_process_wait(host, state, &handle.process_ref)
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

        let checked = self.checkpoint(host, state, CheckpointKind::Final).await?;
        Ok(BatchStep::Exit(failed_exit(
            host,
            checked.state,
            LoopFailureKind::DriverBug,
            Some(checked.checkpoint_id),
        )?))
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
                match self.checkpoint_and_exit_if_cancelled(host, state).await? {
                    CancelCheck::Continue(next) => state = next,
                    CancelCheck::Exit(exit) => return Ok(BatchStep::Exit(exit)),
                }
                state.last_gate = Some(gate_ref.clone());
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
                match self.checkpoint_and_exit_if_cancelled(host, state).await? {
                    CancelCheck::Continue(next) => state = next,
                    CancelCheck::Exit(exit) => return Ok(BatchStep::Exit(exit)),
                }
                Ok(BatchStep::Continue(Box::new(state)))
            }
            GateOutcome::Abort { gate, failure_kind } => {
                state.gate_state = gate;
                match self.checkpoint_and_exit_if_cancelled(host, state).await? {
                    CancelCheck::Continue(next) => state = next,
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

    async fn fail_unsupported_process_wait(
        &self,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        state: LoopExecutionState,
        _process_ref: &ironclaw_turns::run_profile::LoopProcessRef,
    ) -> Result<BatchStep, AgentLoopExecutorError> {
        let checked = self.checkpoint(host, state, CheckpointKind::Final).await?;
        Ok(BatchStep::Exit(failed_exit(
            host,
            checked.state,
            LoopFailureKind::CapabilityProtocolError,
            Some(checked.checkpoint_id),
        )?))
    }

    async fn cancelled_after_checkpoint(
        &self,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        state: LoopExecutionState,
    ) -> Result<BatchStep, AgentLoopExecutorError> {
        let checked = self.checkpoint(host, state, CheckpointKind::Final).await?;
        Ok(BatchStep::Exit(cancelled_exit(
            host,
            checked.state,
            Some(checked.checkpoint_id),
        )?))
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

    async fn emit_progress(
        &self,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        event: LoopProgressEvent,
    ) {
        let _ = host.emit_loop_progress(event).await;
    }

    async fn checkpoint_and_exit_if_cancelled(
        &self,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        state: LoopExecutionState,
    ) -> Result<CancelCheck, AgentLoopExecutorError> {
        let Some(_signal) = host.observe_cancellation() else {
            return Ok(CancelCheck::Continue(state));
        };

        let fallback_state = state.clone();
        match self.checkpoint(host, state, CheckpointKind::Final).await {
            Ok(checked) => Ok(CancelCheck::Exit(cancelled_exit(
                host,
                checked.state,
                Some(checked.checkpoint_id),
            )?)),
            Err(_)
                if !host
                    .run_context()
                    .resolved_run_profile
                    .checkpoint_policy
                    .require_final_checkpoint =>
            {
                Ok(CancelCheck::Exit(cancelled_exit(host, fallback_state, None)?))
            }
            Err(_) => Ok(CancelCheck::Exit(LoopExit::failed(
                LoopFailureKind::CheckpointRejected,
                exit_id(host, "failed")?,
            ))),
        }
    }

    async fn drain_user_inputs(
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

    async fn drain_followup(
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

#[derive(Debug, Clone, Copy)]
enum UserFacingInputDrainMode {
    Steering,
    FollowUp,
}

fn consume_drainable_inputs(
    batch: &LoopInputBatch,
    mode: UserFacingInputDrainMode,
    state: &mut LoopExecutionState,
) -> Result<
    (
        bool,
        Vec<LoopInputAckToken>,
        Option<LoopCancelledReasonKind>,
    ),
    AgentLoopExecutorError,
> {
    let mut consumed_len = 0;
    let mut drained = false;
    let mut cancelled_reason_kind = None;
    for input in &batch.inputs {
        if user_facing_input_matches_drain_mode(input, mode) {
            consumed_len += 1;
            drained = true;
            continue;
        }
        match input {
            LoopInput::Cancel { .. } => {
                consumed_len += 1;
                cancelled_reason_kind = Some(LoopCancelledReasonKind::HostCancellation);
                break;
            }
            LoopInput::Interrupt { .. } => {
                consumed_len += 1;
                cancelled_reason_kind = Some(LoopCancelledReasonKind::HostInterrupt);
                break;
            }
            LoopInput::GateResolved { .. } | LoopInput::CapabilitySurfaceChanged { .. } => break,
            LoopInput::UserMessage { .. }
            | LoopInput::FollowUp { .. }
            | LoopInput::Steering { .. } => {
                break;
            }
        }
    }
    if consumed_len == 0 {
        return Ok((false, Vec::new(), None));
    }
    if batch.input_acks.len() < consumed_len {
        return Err(AgentLoopExecutorError::PlannerContract {
            detail: "input batch omitted ack metadata for consumed inputs",
        });
    }
    let last_ack = &batch.input_acks[consumed_len - 1];
    state.input_cursor = last_ack.cursor.clone();
    let ack_tokens = batch
        .input_acks
        .iter()
        .take(consumed_len)
        .map(|ack| ack.token.clone())
        .collect();
    Ok((drained, ack_tokens, cancelled_reason_kind))
}

fn user_facing_input_matches_drain_mode(input: &LoopInput, mode: UserFacingInputDrainMode) -> bool {
    match mode {
        UserFacingInputDrainMode::Steering => {
            matches!(
                input,
                LoopInput::UserMessage { .. } | LoopInput::Steering { .. }
            )
        }
        UserFacingInputDrainMode::FollowUp => {
            matches!(
                input,
                LoopInput::FollowUp { .. } | LoopInput::UserMessage { .. }
            )
        }
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
) -> Result<LoopExit, AgentLoopExecutorError> {
    Ok(LoopExit::Completed(LoopCompleted {
        completion_kind: if state.assistant_refs.is_empty() {
            LoopCompletionKind::NoReply
        } else {
            LoopCompletionKind::FinalReply
        },
        reply_message_refs: state.assistant_refs,
        result_refs: state.result_refs,
        final_checkpoint_id,
        usage_summary_ref: None,
        exit_id: exit_id(host, "completed")?,
    }))
}

fn failed_exit(
    host: &(dyn AgentLoopDriverHost + Send + Sync),
    _state: LoopExecutionState,
    reason_kind: LoopFailureKind,
    checkpoint_id: Option<ironclaw_turns::TurnCheckpointId>,
) -> Result<LoopExit, AgentLoopExecutorError> {
    Ok(LoopExit::Failed(LoopFailed {
        reason_kind,
        checkpoint_id,
        usage_summary_ref: None,
        diagnostic_ref: None,
        exit_id: exit_id(host, "failed")?,
    }))
}

fn cancelled_exit(
    host: &(dyn AgentLoopDriverHost + Send + Sync),
    state: LoopExecutionState,
    checkpoint_id: Option<ironclaw_turns::TurnCheckpointId>,
) -> Result<LoopExit, AgentLoopExecutorError> {
    cancelled_exit_with_reason(
        host,
        state,
        LoopCancelledReasonKind::HostCancellation,
        checkpoint_id,
    )
}

fn cancelled_exit_with_reason(
    host: &(dyn AgentLoopDriverHost + Send + Sync),
    state: LoopExecutionState,
    reason_kind: LoopCancelledReasonKind,
    checkpoint_id: Option<ironclaw_turns::TurnCheckpointId>,
) -> Result<LoopExit, AgentLoopExecutorError> {
    Ok(LoopExit::Cancelled(LoopCancelled {
        reason_kind,
        checkpoint_id,
        interrupted_message_refs: state.assistant_refs,
        exit_id: exit_id(host, "cancelled")?,
    }))
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

fn loop_gate_kind(kind: GateKind) -> LoopGateKind {
    match kind {
        GateKind::Approval => LoopGateKind::Approval,
        GateKind::Auth => LoopGateKind::Auth,
        GateKind::Resource => LoopGateKind::ResourceWait,
    }
}

fn batch_policy_kind(policy: BatchPolicy) -> BatchPolicyKind {
    match policy {
        BatchPolicy::Sequential => BatchPolicyKind::Sequential,
        BatchPolicy::Parallel => BatchPolicyKind::Parallel,
    }
}

fn capability_batch_counts(outcomes: &[CapabilityOutcome]) -> (u32, u32, u32, u32) {
    let mut result_count = 0;
    let mut denied_count = 0;
    let mut gated_count = 0;
    let mut failed_count = 0;
    for outcome in outcomes {
        match outcome {
            CapabilityOutcome::Completed(_) => result_count += 1,
            CapabilityOutcome::Denied(_) => denied_count += 1,
            CapabilityOutcome::ApprovalRequired { .. }
            | CapabilityOutcome::AuthRequired { .. }
            | CapabilityOutcome::ResourceBlocked { .. }
            // SpawnedProcess: treated as gated — it is a non-completing, non-failing, non-denied
            // outcome that defers completion to a background process. Grouped with gated to avoid
            // treating it as completed or failed in batch accounting.
            | CapabilityOutcome::SpawnedProcess(_) => gated_count += 1,
            CapabilityOutcome::Failed(_) => failed_count += 1,
        }
    }
    (result_count, denied_count, gated_count, failed_count)
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
        AgentLoopHostErrorKind::Cancelled => None,
        AgentLoopHostErrorKind::CredentialUnavailable => None,
        AgentLoopHostErrorKind::Unauthorized
        | AgentLoopHostErrorKind::ScopeMismatch
        | AgentLoopHostErrorKind::StaleSurface
        | AgentLoopHostErrorKind::InvalidInvocation
        | AgentLoopHostErrorKind::Invalid
        | AgentLoopHostErrorKind::PolicyDenied
        | AgentLoopHostErrorKind::CheckpointRejected
        | AgentLoopHostErrorKind::TranscriptWriteFailed => None,
    }
}

fn capability_host_error(error: AgentLoopHostError) -> AgentLoopExecutorError {
    if error.kind == AgentLoopHostErrorKind::Cancelled {
        return AgentLoopExecutorError::Cancelled;
    }
    AgentLoopExecutorError::HostUnavailable {
        stage: HostStage::Capability,
    }
}

fn capability_error_class(kind: &CapabilityFailureKind) -> CapabilityErrorClass {
    match kind {
        CapabilityFailureKind::Network | CapabilityFailureKind::Transient => {
            CapabilityErrorClass::Transient
        }
        CapabilityFailureKind::Backend
        | CapabilityFailureKind::MissingRuntime
        | CapabilityFailureKind::Unavailable => CapabilityErrorClass::Unavailable,
        CapabilityFailureKind::InvalidInput => CapabilityErrorClass::InputInvalid,
        CapabilityFailureKind::Authorization | CapabilityFailureKind::PolicyDenied => {
            CapabilityErrorClass::PolicyDenied
        }
        CapabilityFailureKind::Dispatcher | CapabilityFailureKind::Internal => {
            CapabilityErrorClass::Internal
        }
        CapabilityFailureKind::Cancelled => CapabilityErrorClass::Permanent,
        CapabilityFailureKind::OutputTooLarge
        | CapabilityFailureKind::Process
        | CapabilityFailureKind::Resource
        | CapabilityFailureKind::Permanent
        | CapabilityFailureKind::Unknown(_) => CapabilityErrorClass::Permanent,
        // CapabilityFailureKind is #[non_exhaustive]; treat unrecognised future variants as
        // permanent failures so callers do not retry indefinitely on unknown error kinds.
        &_ => CapabilityErrorClass::Permanent,
    }
}

fn capability_failure_kind(kind: &CapabilityFailureKind) -> LoopFailureKind {
    match kind {
        CapabilityFailureKind::Authorization | CapabilityFailureKind::PolicyDenied => {
            LoopFailureKind::PolicyDenied
        }
        _ => LoopFailureKind::CapabilityProtocolError,
    }
}

fn sanitized_strategy_summary(
    summary: String,
) -> Result<SanitizedStrategySummary, AgentLoopExecutorError> {
    SanitizedStrategySummary::new(summary).map_err(|_| AgentLoopExecutorError::PlannerContract {
        detail: "host returned unsafe strategy summary",
    })
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
    state.recovery_state = state.recovery_state.cleared_attempts();
    state.result_refs.push(result.result_ref);
    if result.terminate_hint {
        state.stop_state.terminate_hints_in_last_batch = state
            .stop_state
            .terminate_hints_in_last_batch
            .saturating_add(1);
    }
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
            ContextProfileId, LoopCancelReasonKind, LoopCancellationPort, LoopCancellationSignal,
            LoopCheckpointRequest, LoopCheckpointStateRef, LoopContextBundle, LoopContextRequest,
            LoopDriverId, LoopInputAck, LoopInputAckToken, LoopInputBatch, LoopInputCursor,
            LoopInputCursorToken, LoopInterruptKind, LoopModelMessage, LoopModelResponse,
            LoopProcessRef, LoopPromptBundle, LoopPromptBundleRef, LoopPromptBundleRequest,
            LoopRunContext, LoopRunInfoPort, ModelProfileId, ModelStreamChunk,
            ProcessHandleSummary, RedactedRunProfileProvenance, ResolvedRunProfile,
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
        model_errors: Arc<Mutex<VecDeque<AgentLoopHostError>>>,
        model_requests: Arc<Mutex<Vec<LoopModelRequest>>>,
        input_batches: Arc<Mutex<VecDeque<LoopInputBatch>>>,
        acked_input_tokens: Arc<Mutex<Vec<LoopInputAckToken>>>,
        batch_outcomes: Arc<Mutex<VecDeque<ironclaw_turns::run_profile::CapabilityBatchOutcome>>>,
        single_outcomes: Arc<Mutex<VecDeque<CapabilityOutcome>>>,
        checkpoints: Arc<Mutex<Vec<LoopCheckpointKind>>>,
        batch_invocations: Arc<Mutex<Vec<CapabilityBatchInvocation>>>,
        single_invocations: Arc<Mutex<Vec<CapabilityInvocation>>>,
        staged_payloads: Arc<Mutex<Vec<StageCheckpointPayloadRequest>>>,
        events: Arc<Mutex<Vec<String>>>,
        prompt_surface_version: Option<CapabilitySurfaceVersion>,
        visible_surface_version: CapabilitySurfaceVersion,
        progress_events: Arc<Mutex<Vec<ironclaw_turns::run_profile::LoopProgressEvent>>>,
        fail_progress_port: bool,
        cancellation: Arc<Mutex<Option<LoopCancellationSignal>>>,
        cancel_after_checkpoint: Arc<Mutex<Option<LoopCheckpointKind>>>,
        fail_checkpoint: Arc<Mutex<Option<LoopCheckpointKind>>>,
    }

    impl MockHost {
        fn new(model_responses: Vec<LoopModelResponse>) -> Self {
            Self {
                context: test_run_context(),
                model_responses: Arc::new(Mutex::new(model_responses.into())),
                model_errors: Arc::new(Mutex::new(VecDeque::new())),
                model_requests: Arc::new(Mutex::new(Vec::new())),
                input_batches: Arc::new(Mutex::new(VecDeque::new())),
                acked_input_tokens: Arc::new(Mutex::new(Vec::new())),
                batch_outcomes: Arc::new(Mutex::new(VecDeque::new())),
                single_outcomes: Arc::new(Mutex::new(VecDeque::new())),
                checkpoints: Arc::new(Mutex::new(Vec::new())),
                batch_invocations: Arc::new(Mutex::new(Vec::new())),
                single_invocations: Arc::new(Mutex::new(Vec::new())),
                staged_payloads: Arc::new(Mutex::new(Vec::new())),
                events: Arc::new(Mutex::new(Vec::new())),
                prompt_surface_version: Some(surface_version()),
                visible_surface_version: surface_version(),
                progress_events: Arc::new(Mutex::new(Vec::new())),
                fail_progress_port: false,
                cancellation: Arc::new(Mutex::new(None)),
                cancel_after_checkpoint: Arc::new(Mutex::new(None)),
                fail_checkpoint: Arc::new(Mutex::new(None)),
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

        fn with_model_errors(self, errors: Vec<AgentLoopHostError>) -> Self {
            *self.model_errors.lock().expect("lock") = errors.into();
            self
        }

        fn with_failing_progress_port(mut self) -> Self {
            self.fail_progress_port = true;
            self
        }

        fn with_input_batches(self, batches: Vec<LoopInputBatch>) -> Self {
            *self.input_batches.lock().expect("lock") = batches.into();
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

        fn acked_input_tokens(&self) -> Vec<LoopInputAckToken> {
            self.acked_input_tokens.lock().expect("lock").clone()
        }

        fn staged_payloads(&self) -> Vec<StageCheckpointPayloadRequest> {
            self.staged_payloads.lock().expect("lock").clone()
        }

        fn events(&self) -> Vec<String> {
            self.events.lock().expect("lock").clone()
        }

        fn progress_events(&self) -> Vec<ironclaw_turns::run_profile::LoopProgressEvent> {
            self.progress_events.lock().expect("lock").clone()
        }

        fn progress_event_names(&self) -> Vec<&'static str> {
            self.progress_events()
                .iter()
                .map(|event| event.kind_name())
                .collect()
        }

        fn request_cancellation(&self, reason_kind: LoopCancelReasonKind) {
            *self.cancellation.lock().expect("lock") = Some(LoopCancellationSignal {
                reason_kind,
                requested_at: chrono::Utc::now(),
            });
        }

        fn cancel_after_checkpoint(self, kind: LoopCheckpointKind) -> Self {
            *self.cancel_after_checkpoint.lock().expect("lock") = Some(kind);
            self
        }

        fn fail_checkpoint(self, kind: LoopCheckpointKind) -> Self {
            *self.fail_checkpoint.lock().expect("lock") = Some(kind);
            self
        }

        fn with_require_final_checkpoint(mut self, require_final_checkpoint: bool) -> Self {
            self.context
                .resolved_run_profile
                .checkpoint_policy
                .require_final_checkpoint = require_final_checkpoint;
            self
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
                instruction_fingerprint: None,
                identity_message_count: 0,
                instruction_snippet_count: 0,
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
            if let Some(batch) = self.input_batches.lock().expect("lock").pop_front() {
                return Ok(batch);
            }
            Ok(LoopInputBatch {
                inputs: Vec::new(),
                input_acks: Vec::new(),
                next_cursor: after,
            })
        }

        async fn ack_inputs(
            &self,
            tokens: Vec<LoopInputAckToken>,
        ) -> Result<(), AgentLoopHostError> {
            self.events
                .lock()
                .expect("lock")
                .push("ack_inputs".to_string());
            self.acked_input_tokens.lock().expect("lock").extend(tokens);
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
            if let Some(error) = self.model_errors.lock().expect("lock").pop_front() {
                return Err(error);
            }
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
            self.events
                .lock()
                .expect("lock")
                .push(format!("checkpoint:{}", request.kind.as_str()));
            self.checkpoints.lock().expect("lock").push(request.kind);
            if self
                .fail_checkpoint
                .lock()
                .expect("lock")
                .is_some_and(|kind| kind == request.kind)
            {
                return Err(AgentLoopHostError::new(
                    AgentLoopHostErrorKind::CheckpointRejected,
                    "scripted checkpoint failure",
                ));
            }
            if self
                .cancel_after_checkpoint
                .lock()
                .expect("lock")
                .is_some_and(|kind| kind == request.kind)
            {
                self.request_cancellation(LoopCancelReasonKind::UserRequested);
            }
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
            event: ironclaw_turns::run_profile::LoopProgressEvent,
        ) -> Result<(), AgentLoopHostError> {
            if self.fail_progress_port {
                return Err(AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Unavailable,
                    "progress sink unavailable",
                ));
            }
            self.progress_events.lock().expect("lock").push(event);
            Ok(())
        }
    }

    impl LoopCancellationPort for MockHost {
        fn observe_cancellation(&self) -> Option<LoopCancellationSignal> {
            self.cancellation.lock().expect("lock").clone()
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
        assert_eq!(
            host.progress_event_names(),
            vec![
                "iteration_started",
                "prompt_bundle_built",
                "checkpoint_written",
                "checkpoint_written",
            ]
        );
    }

    #[tokio::test]
    async fn progress_port_failure_does_not_abort_reply_only_run() {
        let host = MockHost::new(vec![reply_response()]).with_failing_progress_port();
        let executor = CanonicalAgentLoopExecutor;
        let state = LoopExecutionState::initial_for_run(host.run_context());

        let exit = executor
            .execute_family(&crate::families::default(), &host, state)
            .await
            .expect("execute");

        match exit {
            LoopExit::Completed(completed) => {
                assert_eq!(
                    completed.reply_message_refs,
                    vec![message_ref("msg:assistant")]
                );
                assert!(completed.final_checkpoint_id.is_some());
            }
            other => panic!("expected completed, got {other:?}"),
        }
        assert_eq!(
            host.checkpoint_kinds(),
            vec![LoopCheckpointKind::BeforeModel, LoopCheckpointKind::Final]
        );
        assert!(host.progress_events().is_empty());

        let final_state = final_staged_state(&host);
        assert_eq!(
            final_state.assistant_refs,
            vec![message_ref("msg:assistant")]
        );
        assert_eq!(
            final_state.last_checkpoint,
            Some(crate::state::CheckpointMarker {
                kind: CheckpointKind::Final,
                iteration_at_checkpoint: final_state.iteration,
            })
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
        assert_eq!(
            host.progress_event_names(),
            vec![
                "iteration_started",
                "prompt_bundle_built",
                "checkpoint_written",
                "checkpoint_written",
                "capability_batch_started",
                "capability_batch_completed",
                "checkpoint_written",
            ]
        );
        let completed = host
            .progress_events()
            .into_iter()
            .find_map(|event| match event {
                ironclaw_turns::run_profile::LoopProgressEvent::CapabilityBatchCompleted {
                    result_count,
                    denied_count,
                    gated_count,
                    failed_count,
                    ..
                } => Some((result_count, denied_count, gated_count, failed_count)),
                _ => None,
            })
            .expect("batch completed progress event");
        assert_eq!(completed, (1, 0, 0, 0));
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
        assert_eq!(
            host.progress_event_names(),
            vec![
                "iteration_started",
                "prompt_bundle_built",
                "checkpoint_written",
                "checkpoint_written",
                "capability_batch_started",
                "capability_batch_completed",
                "gate_blocked",
                "checkpoint_written",
            ]
        );
        let completed = host
            .progress_events()
            .into_iter()
            .find_map(|event| match event {
                ironclaw_turns::run_profile::LoopProgressEvent::CapabilityBatchCompleted {
                    result_count,
                    denied_count,
                    gated_count,
                    failed_count,
                    ..
                } => Some((result_count, denied_count, gated_count, failed_count)),
                _ => None,
            })
            .expect("batch completed progress event");
        assert_eq!(completed, (0, 0, 1, 0));
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
    async fn steering_drain_does_not_ack_cancel_before_user_message() {
        let host = MockHost::new(Vec::new());
        let run_context = host.run_context().clone();
        let next_cursor = input_cursor(&run_context, "input-cursor:after-cancel");
        let host = host.with_input_batches(vec![LoopInputBatch {
            inputs: vec![
                LoopInput::Cancel {
                    reason_kind: LoopCancelReasonKind::UserRequested,
                },
                LoopInput::UserMessage {
                    message_ref: message_ref("msg:after-cancel"),
                },
            ],
            input_acks: vec![
                input_ack(&run_context, "input-cursor:cancel", "input-ack:cancel"),
                input_ack(
                    &run_context,
                    "input-cursor:after-cancel",
                    "input-ack:after-cancel",
                ),
            ],
            next_cursor,
        }]);
        let executor = CanonicalAgentLoopExecutor;
        let state = LoopExecutionState::initial_for_run(host.run_context());

        let next = executor
            .drain_user_inputs(&host, state)
            .await
            .expect("drain");

        assert_eq!(
            next.state.input_cursor,
            input_cursor(&run_context, "input-cursor:cancel")
        );
        assert!(!next.drained);
        assert_eq!(
            next.ack_tokens,
            vec![LoopInputAckToken::new("input-ack:cancel").expect("valid ack token")]
        );
        assert_eq!(
            next.cancelled_reason_kind,
            Some(LoopCancelledReasonKind::HostCancellation)
        );
        assert!(host.acked_input_tokens().is_empty());
    }

    #[tokio::test]
    async fn queued_cancel_exits_before_prompt_or_model_call() {
        let host = MockHost::new(Vec::new());
        let run_context = host.run_context().clone();
        let host = host.with_input_batches(vec![LoopInputBatch {
            inputs: vec![
                LoopInput::Cancel {
                    reason_kind: LoopCancelReasonKind::UserRequested,
                },
                LoopInput::UserMessage {
                    message_ref: message_ref("msg:after-cancel"),
                },
            ],
            input_acks: vec![
                input_ack(&run_context, "input-cursor:cancel", "input-ack:cancel"),
                input_ack(
                    &run_context,
                    "input-cursor:after-cancel",
                    "input-ack:after-cancel",
                ),
            ],
            next_cursor: input_cursor(&run_context, "input-cursor:after-cancel"),
        }]);
        let executor = CanonicalAgentLoopExecutor;
        let state = LoopExecutionState::initial_for_run(host.run_context());

        let exit = executor
            .execute_family(&crate::families::default(), &host, state)
            .await
            .expect("queued cancel should produce a loop exit");

        match exit {
            LoopExit::Cancelled(cancelled) => {
                assert_eq!(
                    cancelled.reason_kind,
                    LoopCancelledReasonKind::HostCancellation
                );
                assert!(cancelled.checkpoint_id.is_some());
            }
            other => panic!("expected queued cancel to return Cancelled, got {other:?}"),
        }
        assert!(host.model_requests().is_empty());
        assert_eq!(host.checkpoint_kinds(), vec![LoopCheckpointKind::Final]);
        assert_eq!(
            host.acked_input_tokens(),
            vec![LoopInputAckToken::new("input-ack:cancel").expect("valid ack token")]
        );
        assert_eq!(
            host.events(),
            vec!["checkpoint:final".to_string(), "ack_inputs".to_string()]
        );
    }

    #[tokio::test]
    async fn queued_cancel_after_user_prefix_exits_before_model_call() {
        let host = MockHost::new(Vec::new());
        let run_context = host.run_context().clone();
        let host = host.with_input_batches(vec![LoopInputBatch {
            inputs: vec![
                LoopInput::UserMessage {
                    message_ref: message_ref("msg:before-cancel"),
                },
                LoopInput::Cancel {
                    reason_kind: LoopCancelReasonKind::UserRequested,
                },
                LoopInput::UserMessage {
                    message_ref: message_ref("msg:after-cancel"),
                },
            ],
            input_acks: vec![
                input_ack(
                    &run_context,
                    "input-cursor:before-cancel",
                    "input-ack:before-cancel",
                ),
                input_ack(&run_context, "input-cursor:cancel", "input-ack:cancel"),
                input_ack(
                    &run_context,
                    "input-cursor:after-cancel",
                    "input-ack:after-cancel",
                ),
            ],
            next_cursor: input_cursor(&run_context, "input-cursor:after-cancel"),
        }]);
        let executor = CanonicalAgentLoopExecutor;
        let state = LoopExecutionState::initial_for_run(host.run_context());

        let exit = executor
            .execute_family(&crate::families::default(), &host, state)
            .await
            .expect("queued cancel should produce a loop exit");

        assert!(matches!(exit, LoopExit::Cancelled(_)));
        assert!(host.model_requests().is_empty());
        assert_eq!(host.checkpoint_kinds(), vec![LoopCheckpointKind::Final]);
        assert_eq!(
            host.acked_input_tokens(),
            vec![
                LoopInputAckToken::new("input-ack:before-cancel").expect("valid"),
                LoopInputAckToken::new("input-ack:cancel").expect("valid"),
            ]
        );
        assert_eq!(
            host.events(),
            vec!["checkpoint:final".to_string(), "ack_inputs".to_string()]
        );
    }

    #[tokio::test]
    async fn steering_drain_leaves_unhandled_control_at_head_unacked() {
        let host = MockHost::new(Vec::new());
        let run_context = host.run_context().clone();
        let host = host.with_input_batches(vec![LoopInputBatch {
            inputs: vec![
                LoopInput::GateResolved {
                    gate_ref: LoopGateRef::new("gate:resolved").expect("valid"),
                },
                LoopInput::CapabilitySurfaceChanged {
                    version: surface_version(),
                },
                LoopInput::UserMessage {
                    message_ref: message_ref("msg:after-control"),
                },
            ],
            input_acks: vec![
                input_ack(&run_context, "input-cursor:gate", "input-ack:gate"),
                input_ack(&run_context, "input-cursor:surface", "input-ack:surface"),
                input_ack(
                    &run_context,
                    "input-cursor:after-control",
                    "input-ack:after-control",
                ),
            ],
            next_cursor: input_cursor(&run_context, "input-cursor:after-control"),
        }]);
        let executor = CanonicalAgentLoopExecutor;
        let state = LoopExecutionState::initial_for_run(host.run_context());

        let next = executor
            .drain_user_inputs(&host, state)
            .await
            .expect("drain");

        assert!(!next.drained);
        assert!(next.ack_tokens.is_empty());
        assert!(next.cancelled_reason_kind.is_none());
        assert_eq!(
            next.state.input_cursor,
            LoopInputCursor::origin_for_run(&run_context)
        );
        assert!(host.acked_input_tokens().is_empty());
    }

    #[tokio::test]
    async fn followup_drain_does_not_ack_interrupt_before_followup() {
        let host = MockHost::new(Vec::new());
        let run_context = host.run_context().clone();
        let next_cursor = input_cursor(&run_context, "input-cursor:after-interrupt");
        let host = host.with_input_batches(vec![LoopInputBatch {
            inputs: vec![
                LoopInput::Interrupt {
                    kind: LoopInterruptKind::UserInterrupt,
                },
                LoopInput::FollowUp {
                    message_ref: message_ref("msg:after-interrupt"),
                },
            ],
            input_acks: vec![
                input_ack(
                    &run_context,
                    "input-cursor:interrupt",
                    "input-ack:interrupt",
                ),
                input_ack(
                    &run_context,
                    "input-cursor:after-interrupt",
                    "input-ack:after-interrupt",
                ),
            ],
            next_cursor,
        }]);
        let executor = CanonicalAgentLoopExecutor;
        let state = LoopExecutionState::initial_for_run(host.run_context());

        let next = executor.drain_followup(&host, state).await.expect("drain");

        assert!(!next.drained);
        assert_eq!(
            next.state.input_cursor,
            input_cursor(&run_context, "input-cursor:interrupt")
        );
        assert_eq!(
            next.ack_tokens,
            vec![LoopInputAckToken::new("input-ack:interrupt").expect("valid ack token")]
        );
        assert_eq!(
            next.cancelled_reason_kind,
            Some(LoopCancelledReasonKind::HostInterrupt)
        );
        assert!(host.acked_input_tokens().is_empty());
    }

    #[tokio::test]
    async fn steering_drain_acks_only_after_cursor_checkpoint_is_durable() {
        let host = MockHost::new(vec![reply_response()]);
        let run_context = host.run_context().clone();
        let next_cursor = input_cursor(&run_context, "input-cursor:after-user");
        let host = host.with_input_batches(vec![LoopInputBatch {
            inputs: vec![LoopInput::UserMessage {
                message_ref: message_ref("msg:user-drained"),
            }],
            input_acks: vec![input_ack(
                &run_context,
                "input-cursor:after-user",
                "input-ack:after-user",
            )],
            next_cursor: next_cursor.clone(),
        }]);
        let executor = CanonicalAgentLoopExecutor;
        let state = LoopExecutionState::initial_for_run(host.run_context());

        let exit = executor
            .execute_family(&crate::families::default(), &host, state)
            .await
            .expect("execute");

        assert!(matches!(exit, LoopExit::Completed(_)));
        assert_eq!(
            host.acked_input_tokens(),
            vec![LoopInputAckToken::new("input-ack:after-user").expect("valid")]
        );
        assert_eq!(
            host.events(),
            vec![
                "checkpoint:before_model".to_string(),
                "ack_inputs".to_string(),
                "checkpoint:final".to_string(),
            ]
        );
    }

    #[tokio::test]
    async fn model_cancelled_returns_cancelled_without_retry() {
        let host = MockHost::new(Vec::new()).with_model_errors(vec![AgentLoopHostError::new(
            AgentLoopHostErrorKind::Cancelled,
            "model cancelled",
        )]);
        let executor = CanonicalAgentLoopExecutor;
        let state = LoopExecutionState::initial_for_run(host.run_context());

        let result = executor
            .execute_family(&crate::families::default(), &host, state)
            .await;

        assert!(matches!(result, Err(AgentLoopExecutorError::Cancelled)));
        assert_eq!(host.model_requests().len(), 1);
    }

    #[tokio::test]
    async fn capability_cancelled_returns_cancelled_exit_without_retry() {
        let host = MockHost::new(vec![calls_response()]).with_batch_outcomes(vec![
            ironclaw_turns::run_profile::CapabilityBatchOutcome {
                outcomes: vec![CapabilityOutcome::Failed(
                    ironclaw_turns::run_profile::CapabilityFailure {
                        error_kind: CapabilityFailureKind::Cancelled,
                        safe_summary: "capability cancelled".to_string(),
                    },
                )],
                stopped_on_suspension: false,
            },
        ]);
        let executor = CanonicalAgentLoopExecutor;
        let state = LoopExecutionState::initial_for_run(host.run_context());

        let exit = executor
            .execute_family(&crate::families::default(), &host, state)
            .await
            .expect("execute");

        match exit {
            LoopExit::Cancelled(cancelled) => {
                assert_eq!(
                    cancelled.reason_kind,
                    LoopCancelledReasonKind::HostCancellation
                );
                assert!(cancelled.checkpoint_id.is_some());
            }
            other => panic!("expected cancelled exit, got {other:?}"),
        }
        assert!(host.single_invocations().is_empty());
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
    async fn model_retry_success_clears_recovery_state() {
        let host =
            MockHost::new(vec![reply_response()]).with_model_errors(vec![AgentLoopHostError::new(
                AgentLoopHostErrorKind::Unavailable,
                "model unavailable",
            )]);
        let executor = CanonicalAgentLoopExecutor;
        let state = LoopExecutionState::initial_for_run(host.run_context());

        let exit = executor
            .execute_family(&crate::families::default(), &host, state)
            .await
            .expect("execute");

        assert!(matches!(exit, LoopExit::Completed(_)));
        assert_eq!(host.model_requests().len(), 2);
        assert_eq!(final_staged_state(&host).recovery_state, Default::default());
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
        assert!(!batch_invocations[0].stop_on_first_suspension);
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
                        error_kind: CapabilityFailureKind::Transient,
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
        assert_eq!(final_staged_state(&host).recovery_state, Default::default());
    }

    #[tokio::test]
    async fn spawned_process_fails_closed_until_process_wait_contract_exists() {
        let host = MockHost::new(vec![calls_response()]).with_batch_outcomes(vec![
            ironclaw_turns::run_profile::CapabilityBatchOutcome {
                outcomes: vec![CapabilityOutcome::SpawnedProcess(ProcessHandleSummary {
                    process_ref: LoopProcessRef::new("process:alpha").expect("valid"),
                    safe_summary: "spawned".to_string(),
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

        match exit {
            LoopExit::Failed(failed) => {
                assert_eq!(failed.reason_kind, LoopFailureKind::CapabilityProtocolError);
                assert!(failed.checkpoint_id.is_some());
            }
            other => panic!("expected failed exit, got {other:?}"),
        }
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
    async fn cancellation_before_first_iteration_exits_with_final_checkpoint() {
        let host = MockHost::new(vec![reply_response()]);
        host.request_cancellation(LoopCancelReasonKind::UserRequested);
        let executor = CanonicalAgentLoopExecutor;
        let state = LoopExecutionState::initial_for_run(host.run_context());

        let exit = executor
            .execute_family(&crate::families::default(), &host, state)
            .await
            .expect("execute");

        match exit {
            LoopExit::Cancelled(cancelled) => {
                assert_eq!(
                    cancelled.reason_kind,
                    LoopCancelledReasonKind::HostCancellation
                );
                assert!(cancelled.checkpoint_id.is_some());
            }
            other => panic!("expected cancelled, got {other:?}"),
        }
        assert_eq!(host.checkpoint_kinds(), vec![LoopCheckpointKind::Final]);
    }

    #[tokio::test]
    async fn cancellation_after_boundary_skips_next_model_call() {
        let host = MockHost::new(vec![reply_response()])
            .cancel_after_checkpoint(LoopCheckpointKind::BeforeModel);
        let executor = CanonicalAgentLoopExecutor;
        let state = LoopExecutionState::initial_for_run(host.run_context());

        let exit = executor
            .execute_family(&crate::families::default(), &host, state)
            .await
            .expect("execute");

        assert!(matches!(exit, LoopExit::Cancelled(_)));
        assert_eq!(
            host.checkpoint_kinds(),
            vec![LoopCheckpointKind::BeforeModel, LoopCheckpointKind::Final]
        );
    }

    #[tokio::test]
    async fn cancellation_checkpoint_failure_still_cancels_for_permissive_profile() {
        let host = MockHost::new(vec![reply_response()]).fail_checkpoint(LoopCheckpointKind::Final);
        host.request_cancellation(LoopCancelReasonKind::UserRequested);
        let executor = CanonicalAgentLoopExecutor;
        let state = LoopExecutionState::initial_for_run(host.run_context());

        let exit = executor
            .execute_family(&crate::families::default(), &host, state)
            .await
            .expect("execute");

        match exit {
            LoopExit::Cancelled(cancelled) => assert!(cancelled.checkpoint_id.is_none()),
            other => panic!("expected cancelled, got {other:?}"),
        }
        assert_eq!(host.checkpoint_kinds(), vec![LoopCheckpointKind::Final]);
    }

    #[tokio::test]
    async fn cancellation_checkpoint_failure_maps_to_failed_for_strict_profile() {
        let host = MockHost::new(vec![reply_response()])
            .with_require_final_checkpoint(true)
            .fail_checkpoint(LoopCheckpointKind::Final);
        host.request_cancellation(LoopCancelReasonKind::UserRequested);
        let executor = CanonicalAgentLoopExecutor;
        let state = LoopExecutionState::initial_for_run(host.run_context());

        let exit = executor
            .execute_family(&crate::families::default(), &host, state)
            .await
            .expect("execute");

        match exit {
            LoopExit::Failed(failed) => {
                assert_eq!(failed.reason_kind, LoopFailureKind::CheckpointRejected);
                assert!(failed.checkpoint_id.is_none());
            }
            other => panic!("expected failed, got {other:?}"),
        }
        assert_eq!(host.checkpoint_kinds(), vec![LoopCheckpointKind::Final]);
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

    fn input_cursor(context: &LoopRunContext, token: &str) -> LoopInputCursor {
        LoopInputCursor::from_host_token(
            context,
            LoopInputCursorToken::new(token).expect("valid input cursor token"),
        )
    }

    fn input_ack(context: &LoopRunContext, cursor_token: &str, ack_token: &str) -> LoopInputAck {
        LoopInputAck {
            cursor: input_cursor(context, cursor_token),
            token: LoopInputAckToken::new(ack_token).expect("valid input ack token"),
        }
    }

    fn message_ref(value: &str) -> LoopMessageRef {
        LoopMessageRef::new(value).expect("valid message ref")
    }

    fn family_with_capability_filter(filter: CapabilityFilter) -> LoopFamily {
        let planner = DefaultPlanner::compose_default()
            .with_capability(Arc::new(FixedCapabilityStrategy { filter }));
        let id = LoopFamilyId::new("executor-filter-test").expect("valid test family id");
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

    fn final_staged_state(host: &MockHost) -> LoopExecutionState {
        let staged_payloads = host.staged_payloads();
        let final_payload = staged_payloads
            .iter()
            .rev()
            .find(|request| request.kind == LoopCheckpointKind::Final)
            .expect("final checkpoint payload");
        LoopExecutionState::from_checkpoint_payload(&final_payload.payload, CheckpointKind::Final)
            .expect("checkpoint payload")
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
