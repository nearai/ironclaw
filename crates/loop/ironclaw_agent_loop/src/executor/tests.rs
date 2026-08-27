use std::{collections::VecDeque, sync::Arc};

use ironclaw_host_api::turn::{
    CapabilityActivityId, GateResumeDisposition, LoopGateRef, LoopResultRef, TurnRunId,
};
use ironclaw_host_api::{
    decision::DenyReason,
    dispatch::DispatchInputIssueCode,
    ids::{ApprovalRequestId, CorrelationId, DenyRef, ProviderToolName},
    resolution::Denial,
    result_meta::{CapabilityRecoveryHint, FailureKind, SameCallRetryConstraint},
};
use ironclaw_loop_contracts::{
    AgentLoopHostError, AgentLoopHostErrorKind, CapabilityApprovalResume, CapabilityAuthResume,
    CapabilityCallCandidate, CapabilityFailureDetail, CapabilityInputIssue, CapabilityInputRef,
    CapabilityInputRepair, CapabilityResultIntrinsicOutcome, CapabilityResultMessage,
    CapabilityResumeToken, LoopCancelReasonKind, LoopCancellationSignal, LoopCancelledReasonKind,
    LoopCheckpointKind, LoopCompactionError, LoopCompactionMode, LoopCompactionOutcome,
    LoopCompactionResponse, LoopCompletionKind, LoopContextCompactionKind,
    LoopContextWindowTruncation, LoopExit, LoopFailureKind, LoopInput, LoopInputAckToken,
    LoopInputBatch, LoopInputCursor, LoopInterruptKind, LoopModelCapabilityView, LoopProcessRef,
    LoopProgressEvent, LoopRecoveryClass, LoopRecoveryDisposition, LoopRecoveryStage,
    LoopRunInfoPort, LoopSafeSummary, LoopSummaryArtifactId,
    MODEL_VISIBLE_TOOL_OBSERVATION_SCHEMA_VERSION, ModelVisibleToolObservation, ObservationTrust,
    ParentLoopOutput, PromptMode, ProviderToolCallReplay, ToolObservationDetail,
    ToolObservationStatus, VisibleCapabilityRequest, resolution,
};

use crate::state::{
    CapabilityCallSignature, CheckpointKind, DeferredCompactionWatermark, IndexedMessageKind,
    LoopExecutionState, MessageIndexEntry, ModelErrorObservationClass,
    ModelErrorRecoveryObservation, PendingApprovalResume, PendingAuthResume,
    PendingExternalToolResume, PendingModelRetryDirective, RepeatedCallWarningPhase,
    RepeatedCallWarningState, TerminalWarningObservation,
};
use crate::strategies::{
    CapabilityBatchTurnSummary, CapabilityFilter, DefaultCompactionStrategy, GateKind, GateOutcome,
    StopKind, TurnSummary, capability_error_to_failure_kind,
};
use crate::test_support::compaction::{
    active_task_preserving_compaction_index, compaction_metadata,
};
use crate::test_support::{
    MockAgentLoopDriverHost as DriverMockHost, MockHostCall, ScenarioScript,
    ScriptedCapabilityCall, ScriptedCapabilityOutcome, ScriptedModelResponse,
};
use crate::{
    default_planner::DefaultPlanner,
    family::{ComponentDigest, ComponentIdentity, LoopFamily, LoopFamilyId},
};

use super::{
    AgentLoopExecutor, AgentLoopExecutorError, AssistantReplyInput, AssistantReplyStage, BatchStep,
    BudgetInput, BudgetStage, BudgetStep, CanonicalAgentLoopExecutor, CapabilityInput,
    CapabilityStage, DrainInput, ExecutorStage, ExitInput, ExitStage, GateInput, GateStage,
    HostStage, InputStage, InputStep, ModelInput, ModelStage, ModelStep, PromptInput, PromptStage,
    PromptStep, StageContext, TurnCompletedStep, UserFacingInputDrainMode,
    append_capability_result_ref, completed_exit, consume_drainable_inputs,
    sanitize_result_ref_suffix, synthetic_provider_error_result_ref,
};

#[allow(dead_code)]
fn _check(_: &dyn AgentLoopExecutor) {}

mod support;
use support::*;

mod auth_resume;
mod auth_resume_identity;
mod budget;
mod cancellation;
mod capability_results;
mod compaction;
mod completion_exit;
mod denied_resume;
mod failure_matrix;
mod gates;
mod model_recovery;
mod parallel_batch;
mod prompt_stage;
mod provider_replay;
mod reply_input;

fn diagnostic_failure_detail(text: &str) -> CapabilityFailureDetail {
    CapabilityFailureDetail::Diagnostic {
        text: text.to_string(),
    }
}

fn continuation_observation(
    result_ref: &LoopResultRef,
    byte_len: u64,
) -> ModelVisibleToolObservation {
    ModelVisibleToolObservation {
        schema_version: 1,
        status: ToolObservationStatus::Success,
        summary: "Use result_read to continue this child result.".to_string(),
        detail: ToolObservationDetail::ResultReference {
            result_ref: result_ref.as_str().to_string(),
            byte_len,
            preview: Some("first bounded chunk".to_string()),
            structured_json_view: false,
            total_bytes: Some(byte_len * 2),
            next_offset: Some(byte_len),
            item_count: None,
        },
        artifacts: Vec::new(),
        recovery: None,
        trust: ObservationTrust::UntrustedToolOutput,
    }
}

fn family_with_budget_strategy(
    strategy: crate::strategies::DefaultBudgetStrategy,
) -> crate::family::LoopFamily {
    use crate::family::{ComponentDigest, ComponentIdentity, LoopFamily, LoopFamilyId};
    let planner = crate::default_planner::DefaultPlanner::compose_default()
        .with_budget(std::sync::Arc::new(strategy));
    let id = LoopFamilyId::new("executor-budget-test").expect("valid test family id");
    let version = ComponentIdentity::from_static("executor-budget-test", ComponentDigest([6; 32]));
    LoopFamily::new(id, version, std::sync::Arc::new(planner))
}

// ---------------------------------------------------------------------------
// WU-A Step 9 — caller-level executor tests for PostCapabilityStage + SkipModel
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// F12 — CompactionStarted event carries CapabilityResultOverflow initiator
// ---------------------------------------------------------------------------

// ── Approval-then-auth resume: invocation_id preserved ───────────────────────

// ── auth-resume slot follows activity identity within a duplicate-capability batch ──

// ── Resume-origin Backend failure must not die as scope_mismatch ─────────────
