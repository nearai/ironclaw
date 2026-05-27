//! Canonical agent-loop executor.
//!
//! The executor owns loop mechanics. Loop families own strategy composition.

mod assistant_reply;
mod budget;
mod canonical;
mod capabilities;
mod capability_helpers;
mod checkpoint;
mod exit_helpers;
mod gates;
mod input;
mod loop_exit;
mod mapping;
mod model;
mod pipeline;
mod prompt;
mod turn_stop;

use assistant_reply::{AssistantReplyInput, AssistantReplyStage};
use budget::{BudgetInput, BudgetStage, BudgetStep};
use capabilities::{CapabilityInput, CapabilityStage};
use capability_helpers::{
    CapabilitySurfaceIndex, append_capability_error_ref, append_capability_result_ref,
    append_capability_safe_summary_ref, apply_capability_filter,
    capability_invocation_from_candidate, capability_is_visible, capability_summary,
    gate_tool_result_summary, push_call_signature_once, push_completed_result,
};
#[cfg(test)]
use capability_helpers::{sanitize_result_ref_suffix, synthetic_provider_error_result_ref};
use checkpoint::{CheckpointInput, CheckpointStage};
use exit_helpers::{
    cancelled_exit, cancelled_exit_with_reason, cancelled_reason_from_signal, completed_exit,
    exit_id, failed_exit,
};
use gates::{AwaitDependentRunGateInput, AwaitDependentRunGateStage, GateInput, GateStage};
#[cfg(test)]
use input::consume_drainable_inputs;
use input::{DrainInput, InputStage, InputStep, UserFacingInputDrainMode};
use loop_exit::{ExitInput, ExitStage};
use mapping::{
    batch_policy_kind, blocked_kind, capability_batch_counts, capability_error_class,
    capability_failure_kind, capability_host_error, checkpoint_kind_to_host,
    honor_retry_alteration, loop_gate_kind, model_error_class, model_preference_to_host,
    sanitized_strategy_summary,
};
use model::{ModelInput, ModelStage, ModelStep};
use pipeline::{DefaultExecutorPipeline, ExecutorStage, StageContext};
use prompt::{PromptInput, PromptStage, PromptStep};
use turn_stop::{StopInput, StopStage, StopStep};

use async_trait::async_trait;
use ironclaw_turns::{
    LoopCancelledReasonKind, LoopDiagnosticRef, LoopExit,
    run_profile::{
        AgentLoopDriverHost, AgentLoopHostErrorKind, LoopInputAckToken, LoopSafeSummary,
    },
};

use crate::{
    family::LoopFamily,
    state::{CheckpointKind, LoopExecutionState},
    strategies::TurnSummary,
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
    #[error("host port returned an unrecoverable error: {stage:?} ({kind:?}: {safe_summary})")]
    HostUnavailableWithDiagnostics {
        stage: HostStage,
        kind: AgentLoopHostErrorKind,
        safe_summary: LoopSafeSummary,
        diagnostic_ref: Option<LoopDiagnosticRef>,
    },
    #[error("planner returned a contract violation: {detail}")]
    PlannerContract { detail: &'static str },
    #[error("checkpoint write failed at {stage:?}")]
    CheckpointFailed { stage: CheckpointKind },
    /// Constructed when a model or capability call returns a cancelled outcome
    /// (i.e. `AgentLoopHostErrorKind::Cancelled` or `CapabilityFailureKind::Cancelled`
    /// surfaces from an in-flight external call). Between-call boundary cancellation
    /// — detected cooperatively by `CheckpointStage::cancel_if_requested` — returns
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
        DefaultExecutorPipeline::default()
            .execute(family, host, initial_state)
            .await
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

#[derive(Debug)]
enum TurnCompletedStep {
    Continue {
        state: Box<LoopExecutionState>,
        summary: TurnSummary,
    },
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

#[cfg(test)]
mod tests;
