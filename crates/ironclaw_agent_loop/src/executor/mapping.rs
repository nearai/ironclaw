use ironclaw_turns::{
    LoopBlockedKind, LoopFailureKind,
    run_profile::{
        AgentLoopHostError, AgentLoopHostErrorKind, BatchPolicyKind, CapabilityFailureKind,
        CapabilityOutcome, LoopCheckpointKind, LoopGateKind,
    },
};

use crate::{
    state::CheckpointKind,
    strategies::{
        BatchPolicy, CapabilityErrorClass, GateKind, ModelErrorClass, ModelPreference,
        RetryAlteration, SanitizedStrategySummary,
    },
};

use super::{AgentLoopExecutorError, HostStage};

pub(super) fn checkpoint_kind_to_host(kind: CheckpointKind) -> LoopCheckpointKind {
    match kind {
        CheckpointKind::BeforeModel => LoopCheckpointKind::BeforeModel,
        CheckpointKind::BeforeSideEffect => LoopCheckpointKind::BeforeSideEffect,
        CheckpointKind::BeforeBlock => LoopCheckpointKind::BeforeBlock,
        CheckpointKind::Final => LoopCheckpointKind::Final,
    }
}

pub(super) fn blocked_kind(kind: GateKind) -> LoopBlockedKind {
    match kind {
        GateKind::Approval => LoopBlockedKind::Approval,
        GateKind::Auth => LoopBlockedKind::Auth,
        GateKind::Resource => LoopBlockedKind::Resource,
        GateKind::AwaitDependentRun => LoopBlockedKind::AwaitDependentRun,
    }
}

pub(super) fn loop_gate_kind(kind: GateKind) -> LoopGateKind {
    match kind {
        GateKind::Approval => LoopGateKind::Approval,
        GateKind::Auth => LoopGateKind::Auth,
        GateKind::Resource => LoopGateKind::ResourceWait,
        GateKind::AwaitDependentRun => LoopGateKind::AwaitDependentRun,
    }
}

pub(super) fn batch_policy_kind(policy: BatchPolicy) -> BatchPolicyKind {
    match policy {
        BatchPolicy::Sequential => BatchPolicyKind::Sequential,
        BatchPolicy::Parallel => BatchPolicyKind::Parallel,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CapabilityOutcomeBucket {
    Result,
    Denied,
    Gated,
    Failed,
}

pub(super) fn capability_outcome_bucket(outcome: &CapabilityOutcome) -> CapabilityOutcomeBucket {
    match outcome {
        CapabilityOutcome::Completed(_) | CapabilityOutcome::SpawnedChildRun { .. } => {
            CapabilityOutcomeBucket::Result
        }
        CapabilityOutcome::Denied(_) => CapabilityOutcomeBucket::Denied,
        CapabilityOutcome::ApprovalRequired { .. }
        | CapabilityOutcome::AuthRequired { .. }
        | CapabilityOutcome::ResourceBlocked { .. }
        | CapabilityOutcome::AwaitDependentRun { .. }
        // SpawnedProcess: treated as gated — it is a non-completing, non-failing, non-denied
        // outcome that defers completion to a background process. Grouped with gated to avoid
        // treating it as completed or failed in batch accounting.
        | CapabilityOutcome::SpawnedProcess(_) => CapabilityOutcomeBucket::Gated,
        CapabilityOutcome::Failed(_) => CapabilityOutcomeBucket::Failed,
    }
}

pub(super) fn capability_batch_counts(outcomes: &[CapabilityOutcome]) -> (u32, u32, u32, u32) {
    let mut result_count = 0;
    let mut denied_count = 0;
    let mut gated_count = 0;
    let mut failed_count = 0;
    for outcome in outcomes {
        match capability_outcome_bucket(outcome) {
            CapabilityOutcomeBucket::Result => result_count += 1,
            CapabilityOutcomeBucket::Denied => denied_count += 1,
            CapabilityOutcomeBucket::Gated => gated_count += 1,
            CapabilityOutcomeBucket::Failed => failed_count += 1,
        }
    }
    (result_count, denied_count, gated_count, failed_count)
}

pub(super) fn model_preference_to_host(
    preference: ModelPreference,
) -> Result<Option<ironclaw_turns::ModelProfileId>, AgentLoopExecutorError> {
    match preference {
        ModelPreference::Primary => Ok(None),
        ModelPreference::Fallback { .. } => Err(AgentLoopExecutorError::PlannerContract {
            detail: "fallback model preference requires model route chain support",
        }),
    }
}

pub(super) fn model_error_class(error: &AgentLoopHostError) -> Option<ModelErrorClass> {
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

pub(super) fn capability_host_error(error: AgentLoopHostError) -> AgentLoopExecutorError {
    if error.kind == AgentLoopHostErrorKind::Cancelled {
        return AgentLoopExecutorError::Cancelled;
    }
    AgentLoopExecutorError::HostUnavailable {
        stage: HostStage::Capability,
    }
}

pub(super) fn capability_error_class(kind: &CapabilityFailureKind) -> CapabilityErrorClass {
    // Runtime capability failures are first dispositioned in
    // `ironclaw_host_runtime` and adapted by `ironclaw_loop_support`.
    // Keep this recovery class mapping aligned with that adapter: retryable
    // runtime kinds must arrive here as Transient/Unavailable/Internal,
    // model-visible kinds as OperationFailed/InputInvalid/PolicyDenied, and
    // run-ending protocol/cancellation kinds as Permanent or Cancelled.
    match kind {
        CapabilityFailureKind::Network | CapabilityFailureKind::Transient => {
            CapabilityErrorClass::Transient
        }
        CapabilityFailureKind::Backend | CapabilityFailureKind::Unavailable => {
            CapabilityErrorClass::Unavailable
        }
        CapabilityFailureKind::InvalidInput => CapabilityErrorClass::InputInvalid,
        CapabilityFailureKind::MissingRuntime
        | CapabilityFailureKind::OperationFailed
        | CapabilityFailureKind::OutputTooLarge
        | CapabilityFailureKind::Process
        | CapabilityFailureKind::Resource => CapabilityErrorClass::OperationFailed,
        CapabilityFailureKind::Authorization | CapabilityFailureKind::PolicyDenied => {
            CapabilityErrorClass::PolicyDenied
        }
        CapabilityFailureKind::Internal => CapabilityErrorClass::Internal,
        CapabilityFailureKind::Dispatcher => CapabilityErrorClass::Permanent,
        CapabilityFailureKind::Cancelled => CapabilityErrorClass::Permanent,
        CapabilityFailureKind::InvalidOutput
        | CapabilityFailureKind::Permanent
        | CapabilityFailureKind::Unknown(_) => CapabilityErrorClass::Permanent,
        // CapabilityFailureKind is #[non_exhaustive]; treat unrecognised future variants as
        // permanent failures so callers do not retry indefinitely on unknown error kinds.
        &_ => CapabilityErrorClass::Permanent,
    }
}

pub(super) fn capability_failure_kind(kind: &CapabilityFailureKind) -> LoopFailureKind {
    match kind {
        CapabilityFailureKind::Authorization | CapabilityFailureKind::PolicyDenied => {
            LoopFailureKind::PolicyDenied
        }
        _ => LoopFailureKind::CapabilityProtocolError,
    }
}

pub(super) fn sanitized_strategy_summary(
    summary: String,
) -> Result<SanitizedStrategySummary, AgentLoopExecutorError> {
    SanitizedStrategySummary::new(summary).map_err(|_| AgentLoopExecutorError::PlannerContract {
        detail: "host returned unsafe strategy summary",
    })
}

pub(super) fn honor_retry_alteration(
    alteration: Option<&RetryAlteration>,
) -> Result<(), AgentLoopExecutorError> {
    if matches!(alteration, Some(RetryAlteration::AdvanceFallback)) {
        return Err(AgentLoopExecutorError::PlannerContract {
            detail: "fallback model route alteration requires model route chain support",
        });
    }
    Ok(())
}
