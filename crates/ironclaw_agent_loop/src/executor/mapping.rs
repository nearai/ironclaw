use ironclaw_turns::{
    LoopBlockedKind, LoopFailureKind, SanitizedFailure,
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
        GateKind::ExternalTool => LoopBlockedKind::ExternalTool,
    }
}

pub(super) fn loop_gate_kind(kind: GateKind) -> LoopGateKind {
    match kind {
        GateKind::Approval => LoopGateKind::Approval,
        GateKind::Auth => LoopGateKind::Auth,
        GateKind::Resource => LoopGateKind::ResourceWait,
        GateKind::AwaitDependentRun => LoopGateKind::AwaitDependentRun,
        GateKind::ExternalTool => LoopGateKind::ExternalTool,
    }
}

pub(super) fn batch_policy_kind(policy: BatchPolicy) -> BatchPolicyKind {
    match policy {
        BatchPolicy::Sequential => BatchPolicyKind::Sequential,
        BatchPolicy::Parallel => BatchPolicyKind::Parallel,
    }
}

pub(super) fn capability_batch_counts(outcomes: &[CapabilityOutcome]) -> (u32, u32, u32, u32) {
    let mut result_count = 0;
    let mut denied_count = 0;
    let mut gated_count = 0;
    let mut failed_count = 0;
    for outcome in outcomes {
        match outcome {
            CapabilityOutcome::Completed(_) | CapabilityOutcome::SpawnedChildRun { .. } => {
                result_count += 1
            }
            CapabilityOutcome::Denied(_) => denied_count += 1,
            CapabilityOutcome::ApprovalRequired { .. }
            | CapabilityOutcome::AuthRequired { .. }
            | CapabilityOutcome::ResourceBlocked { .. }
            // ExternalToolPending: the run parks waiting for the client to submit
            // tool output — a non-completing, non-failing, non-denied gate.
            | CapabilityOutcome::ExternalToolPending { .. }
            | CapabilityOutcome::AwaitDependentRun { .. }
            // SpawnedProcess: treated as gated — it is a non-completing, non-failing, non-denied
            // outcome that defers completion to a background process. Grouped with gated to avoid
            // treating it as completed or failed in batch accounting.
            | CapabilityOutcome::SpawnedProcess(_) => gated_count += 1,
            CapabilityOutcome::Failed(_) => failed_count += 1,
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
        AgentLoopHostErrorKind::InvalidOutput => Some(ModelErrorClass::InvalidOutput),
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
    tracing::warn!(
        kind = error.kind.as_str(),
        safe_summary = error.safe_summary.as_str(),
        "capability host error mapped to HostUnavailable"
    );
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
        | CapabilityFailureKind::Resource
        // Dispatcher/InvalidOutput/Unknown are dispositioned as model-visible
        // (run-continuing) by the host_runtime layer
        // (`capability_failure_disposition`). Map them to OperationFailed so the
        // recovery strategy turns them into a model-visible ToolErrorResult
        // instead of aborting the run — e.g. the model calling a nonexistent
        // tool (UnknownCapability/UnknownProvider -> InvalidOutput) becomes a
        // recoverable tool error the model can correct.
        | CapabilityFailureKind::Dispatcher
        | CapabilityFailureKind::InvalidOutput
        | CapabilityFailureKind::Unknown(_) => CapabilityErrorClass::OperationFailed,
        CapabilityFailureKind::Authorization
        | CapabilityFailureKind::GateDeclined
        | CapabilityFailureKind::PolicyDenied => CapabilityErrorClass::PolicyDenied,
        CapabilityFailureKind::Internal => CapabilityErrorClass::Internal,
        // Cancelled is intercepted upstream as cancellation; Permanent is an
        // explicit non-retryable signal. Both stay terminal.
        CapabilityFailureKind::Cancelled | CapabilityFailureKind::Permanent => {
            CapabilityErrorClass::Permanent
        }
        // CapabilityFailureKind is #[non_exhaustive]. Treat unrecognised future
        // variants as recoverable (OperationFailed) to match the host_runtime
        // disposition layer's recoverable default: by design no capability
        // failure should abort the run, so an unknown kind becomes a
        // model-visible tool error rather than killing the run.
        &_ => CapabilityErrorClass::OperationFailed,
    }
}

pub(super) fn capability_failure_kind(kind: &CapabilityFailureKind) -> LoopFailureKind {
    match kind {
        CapabilityFailureKind::InvalidInput => LoopFailureKind::ModelError,
        CapabilityFailureKind::Authorization
        | CapabilityFailureKind::GateDeclined
        | CapabilityFailureKind::PolicyDenied => LoopFailureKind::PolicyDenied,
        _ => LoopFailureKind::CapabilityProtocolError,
    }
}

pub(super) fn capability_error_failure_category(
    class: CapabilityErrorClass,
) -> Result<SanitizedFailure, AgentLoopExecutorError> {
    sanitized_failure_category(match class {
        CapabilityErrorClass::Transient => "capability_transient",
        CapabilityErrorClass::Permanent => "capability_permanent",
        CapabilityErrorClass::InputInvalid => "capability_input_invalid",
        CapabilityErrorClass::OperationFailed => "capability_operation_failed",
        CapabilityErrorClass::PolicyDenied => "capability_policy_denied",
        CapabilityErrorClass::Unavailable => "capability_unavailable",
        CapabilityErrorClass::Internal => "capability_internal",
    })
}

pub(super) fn model_error_failure_category(
    class: ModelErrorClass,
) -> Result<SanitizedFailure, AgentLoopExecutorError> {
    sanitized_failure_category(match class {
        ModelErrorClass::Transient => "model_transient",
        ModelErrorClass::ContextOverflow => "model_context_overflow",
        ModelErrorClass::ContentFiltered => "model_content_filtered",
        ModelErrorClass::InvalidOutput => "model_invalid_output",
        ModelErrorClass::Unavailable => "model_unavailable",
        ModelErrorClass::Internal => "model_internal",
    })
}

fn sanitized_failure_category(
    category: &'static str,
) -> Result<SanitizedFailure, AgentLoopExecutorError> {
    SanitizedFailure::new(category).map_err(|_| AgentLoopExecutorError::PlannerContract {
        detail: "static failure category was invalid",
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_capability_input_is_model_error_not_protocol_failure() {
        assert_eq!(
            capability_error_class(&CapabilityFailureKind::InvalidInput),
            CapabilityErrorClass::InputInvalid
        );
        assert_eq!(
            capability_failure_kind(&CapabilityFailureKind::InvalidInput),
            LoopFailureKind::ModelError
        );
    }

    #[test]
    fn protocol_and_policy_failure_kinds_remain_distinct() {
        assert_eq!(
            capability_failure_kind(&CapabilityFailureKind::InvalidOutput),
            LoopFailureKind::CapabilityProtocolError
        );
        assert_eq!(
            capability_failure_kind(&CapabilityFailureKind::PolicyDenied),
            LoopFailureKind::PolicyDenied
        );
    }

    #[test]
    fn model_recoverable_capability_failures_are_operation_failed_not_permanent() {
        // The host_runtime disposition layer treats Dispatcher, InvalidOutput,
        // and Unknown as model-visible (run-continuing) errors. The recovery
        // class mapping must agree: these become OperationFailed (a
        // model-visible tool error) rather than Permanent (which aborts the
        // run). E.g. the model calling a nonexistent tool becomes a recoverable
        // tool error, not a run-ending protocol failure.
        assert_eq!(
            capability_error_class(&CapabilityFailureKind::Dispatcher),
            CapabilityErrorClass::OperationFailed
        );
        assert_eq!(
            capability_error_class(&CapabilityFailureKind::InvalidOutput),
            CapabilityErrorClass::OperationFailed
        );
        let unknown = CapabilityFailureKind::unknown("some_future_kind".to_string())
            .expect("valid unknown kind");
        assert_eq!(
            capability_error_class(&unknown),
            CapabilityErrorClass::OperationFailed
        );
    }

    #[test]
    fn terminal_capability_failures_remain_permanent() {
        // Cancelled is intercepted upstream as cancellation; Permanent is an
        // explicit non-retryable signal. Both must stay terminal.
        assert_eq!(
            capability_error_class(&CapabilityFailureKind::Cancelled),
            CapabilityErrorClass::Permanent
        );
        assert_eq!(
            capability_error_class(&CapabilityFailureKind::Permanent),
            CapabilityErrorClass::Permanent
        );
    }

    #[test]
    fn invalid_model_output_is_distinct_from_unavailable() {
        let error = AgentLoopHostError::new(
            AgentLoopHostErrorKind::InvalidOutput,
            "model output was structurally invalid",
        );

        assert_eq!(
            model_error_class(&error),
            Some(ModelErrorClass::InvalidOutput)
        );
    }
}
