//! Canonical agent-loop executor.
//!
//! The executor owns loop mechanics. Loop families own strategy composition.

use std::collections::HashSet;

use async_trait::async_trait;
use ironclaw_turns::{
    LoopBlocked, LoopBlockedKind, LoopCancelled, LoopCancelledReasonKind, LoopCompleted,
    LoopCompletionKind, LoopExit, LoopExitId, LoopFailed, LoopFailureKind, LoopResultRef,
    run_profile::{
        AgentLoopDriverHost, AgentLoopHostError, AgentLoopHostErrorKind, AppendCapabilityResultRef,
        BatchPolicyKind, CapabilityBatchInvocation, CapabilityCallCandidate, CapabilityFailureKind,
        CapabilityInvocation, CapabilityOutcome, CapabilityResultMessage, FinalizeAssistantMessage,
        LoopCancelReasonKind, LoopCancellationSignal, LoopCheckpointKind, LoopCheckpointRequest,
        LoopDriverNoteKind, LoopGateKind, LoopInput, LoopInputAckToken, LoopInputBatch,
        LoopModelCapabilityView, LoopModelRequest, LoopProgressEvent, ParentLoopOutput,
        ProviderToolCallReference, StageCheckpointPayloadRequest, VisibleCapabilityRequest,
        VisibleCapabilitySurface,
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
    /// Constructed when a model or capability call returns a cancelled outcome
    /// (i.e. `AgentLoopHostErrorKind::Cancelled` or `CapabilityFailureKind::Cancelled`
    /// surfaces from an in-flight external call). Between-call boundary cancellation
    /// — detected cooperatively by `checkpoint_and_exit_if_cancelled` — returns
    /// `LoopExit::Cancelled` directly and never constructs this variant.
    /// WS16 will build further on this split when product adapters are wired.
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
    Continue(Box<LoopExecutionState>),
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

mod input;
mod steps;

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
    let completion_kind = if !state.assistant_refs.is_empty() {
        LoopCompletionKind::FinalReply
    } else if !state.result_refs.is_empty() {
        LoopCompletionKind::ResultOnly
    } else {
        LoopCompletionKind::NoReply
    };
    Ok(LoopExit::Completed(LoopCompleted {
        completion_kind,
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

fn cancelled_reason_from_signal(signal: &LoopCancellationSignal) -> LoopCancelledReasonKind {
    // LoopCancelReasonKind preserves host/input detail; LoopExit currently exposes
    // the coarser terminal taxonomy, so every observed signal maps explicitly here.
    //
    // Reason coarsened to HostCancellation intentionally: the loop exit taxonomy
    // does not expose raw reason_kind to the product layer at this WS boundary.
    // WS16/WS17 can map finer-grained reasons when the product adapter is wired.
    match signal.reason_kind {
        LoopCancelReasonKind::UserRequested
        | LoopCancelReasonKind::Superseded
        | LoopCancelReasonKind::Policy => LoopCancelledReasonKind::HostCancellation,
    }
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
        AgentLoopHostErrorKind::BudgetAccountingFailed => Some(ModelErrorClass::Unavailable),
        // Budget approval requirement is a gate, not a transient model
        // error — pass it through unclassified so the loop's gate handling
        // path takes over rather than the recovery strategy.
        AgentLoopHostErrorKind::BudgetApprovalRequired => None,
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
    surface
        .descriptors
        .retain(|descriptor| filter.permits(&descriptor.capability_id));
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

async fn append_capability_result_ref(
    host: &(dyn AgentLoopDriverHost + Send + Sync),
    call: &CapabilityCallCandidate,
    result: &CapabilityResultMessage,
) -> Result<(), AgentLoopExecutorError> {
    host.append_capability_result_ref(AppendCapabilityResultRef {
        result_ref: result.result_ref.clone(),
        safe_summary: result.safe_summary.clone(),
        provider_call: provider_tool_call_reference(call),
    })
    .await
    .map_err(capability_host_error)?;
    Ok(())
}

fn provider_tool_call_reference(
    call: &CapabilityCallCandidate,
) -> Option<ProviderToolCallReference> {
    let provider_replay = call.provider_replay.as_ref()?;
    Some(ProviderToolCallReference {
        provider_id: provider_replay.provider_id.clone(),
        provider_model_id: provider_replay.provider_model_id.clone(),
        provider_turn_id: provider_replay.provider_turn_id.clone(),
        provider_call_id: provider_replay.provider_call_id.clone(),
        provider_tool_name: provider_replay.provider_tool_name.clone(),
        capability_id: call.capability_id.clone(),
        arguments: provider_replay.arguments.clone(),
        response_reasoning: provider_replay.response_reasoning.clone(),
        reasoning: provider_replay.reasoning.clone(),
        signature: provider_replay.signature.clone(),
    })
}

async fn append_capability_error_ref(
    host: &(dyn AgentLoopDriverHost + Send + Sync),
    state: &mut LoopExecutionState,
    call: &CapabilityCallCandidate,
    summary: &CapabilityErrorSummary,
) -> Result<(), AgentLoopExecutorError> {
    append_capability_safe_summary_ref(host, state, call, summary.safe_summary.as_str().to_string())
        .await
}

async fn append_capability_safe_summary_ref(
    host: &(dyn AgentLoopDriverHost + Send + Sync),
    state: &mut LoopExecutionState,
    call: &CapabilityCallCandidate,
    safe_summary: String,
) -> Result<(), AgentLoopExecutorError> {
    if call.provider_replay.is_none() {
        return Ok(());
    }
    let result_ref = synthetic_provider_error_result_ref(call)?;
    host.append_capability_result_ref(AppendCapabilityResultRef {
        result_ref: result_ref.clone(),
        safe_summary,
        provider_call: provider_tool_call_reference(call),
    })
    .await
    .map_err(capability_host_error)?;
    state.result_refs.push(result_ref);
    Ok(())
}

fn synthetic_provider_error_result_ref(
    call: &CapabilityCallCandidate,
) -> Result<LoopResultRef, AgentLoopExecutorError> {
    let provider_replay =
        call.provider_replay
            .as_ref()
            .ok_or(AgentLoopExecutorError::PlannerContract {
                detail: "provider replay metadata is required for provider error result ref",
            })?;
    let mut suffix = format!(
        "provider-error-{}-{}",
        sanitize_result_ref_suffix(&provider_replay.provider_turn_id),
        sanitize_result_ref_suffix(&provider_replay.provider_call_id)
    );
    suffix.truncate(240);
    LoopResultRef::new(format!("result:{suffix}")).map_err(|_| {
        AgentLoopExecutorError::PlannerContract {
            detail: "provider error result ref was invalid",
        }
    })
}

fn sanitize_result_ref_suffix(value: &str) -> String {
    let mut sanitized = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.') {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    if sanitized.is_empty() {
        sanitized.push_str("unknown");
    }
    sanitized
}

fn gate_tool_result_summary(kind: GateKind, outcome: &'static str) -> String {
    let gate = match kind {
        GateKind::Approval => "approval",
        GateKind::Auth => "auth",
        GateKind::Resource => "resource",
    };
    format!("{gate} gate {outcome}")
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
mod tests;
