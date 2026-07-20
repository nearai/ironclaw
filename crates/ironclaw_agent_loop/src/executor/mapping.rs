use ironclaw_host_api::{Resolution, ToolVerdict};
use ironclaw_turns::{
    LoopBlockedKind, LoopFailureKind, SanitizedFailure,
    run_profile::{
        AgentLoopHostError, AgentLoopHostErrorKind, BatchPolicyKind, CapabilityFailureKind,
        LoopCheckpointKind, LoopGateKind,
    },
};

use crate::{
    state::CheckpointKind,
    strategies::{
        BatchPolicy, CapabilityErrorClass, GateKind, ModelErrorClass, ModelErrorSummary,
        ModelPreference, RetryAlteration, SanitizedStrategySummary,
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

pub(super) fn capability_batch_counts(resolutions: &[Resolution]) -> (u32, u32, u32, u32) {
    let mut result_count = 0;
    let mut denied_count = 0;
    let mut gated_count = 0;
    let mut failed_count = 0;
    for resolution in resolutions {
        // Exhaustive over `Resolution`, no wildcard (§11.9). `Done` splits on its
        // verdict: `Success`/`ChildSpawned` are results, a `RecoverableFailure` is
        // a model-visible failure. `Denied` is denied; every `Blocked` gate and
        // every `Suspended` (process/dependent-run/external-tool) is gated — a
        // non-completing, non-failing, non-denied outcome that defers completion.
        match resolution {
            Resolution::Done(outcome) => match &outcome.verdict {
                ToolVerdict::Success | ToolVerdict::ChildSpawned { .. } => result_count += 1,
                ToolVerdict::RecoverableFailure { .. } => failed_count += 1,
            },
            Resolution::Denied(_) => denied_count += 1,
            Resolution::Blocked(_) | Resolution::Suspended(_) => gated_count += 1,
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
        // Accounting storage failed before the host could establish a
        // trustworthy budget outcome. Preserve the typed host error instead
        // of retrying it as a provider availability failure.
        AgentLoopHostErrorKind::BudgetAccountingFailed => None,
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
    // `ironclaw_host_runtime` and adapted by `ironclaw_loop_host`.
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
    }
}

pub(super) fn capability_failure_kind(kind: &CapabilityFailureKind) -> LoopFailureKind {
    match kind {
        CapabilityFailureKind::InvalidInput => LoopFailureKind::ModelError,
        CapabilityFailureKind::Authorization
        | CapabilityFailureKind::GateDeclined
        | CapabilityFailureKind::PolicyDenied => LoopFailureKind::PolicyDenied,
        // Every remaining kind maps to the protocol-error failure. Enumerated
        // explicitly (no wildcard) so a new `CapabilityFailureKind` variant must
        // be classified here deliberately rather than silently inheriting this
        // terminal fate — `CapabilityFailureKind` is no longer `#[non_exhaustive]`.
        CapabilityFailureKind::Backend
        | CapabilityFailureKind::Cancelled
        | CapabilityFailureKind::Dispatcher
        | CapabilityFailureKind::InvalidOutput
        | CapabilityFailureKind::MissingRuntime
        | CapabilityFailureKind::Network
        | CapabilityFailureKind::OperationFailed
        | CapabilityFailureKind::OutputTooLarge
        | CapabilityFailureKind::Process
        | CapabilityFailureKind::Resource
        | CapabilityFailureKind::Transient
        | CapabilityFailureKind::Unavailable
        | CapabilityFailureKind::Internal
        | CapabilityFailureKind::Permanent
        | CapabilityFailureKind::Unknown(_) => LoopFailureKind::CapabilityProtocolError,
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

pub(super) fn model_error_failure_summary(
    summary: &ModelErrorSummary,
) -> Result<SanitizedFailure, AgentLoopExecutorError> {
    Ok(model_error_failure_category(summary.class)?
        .with_detail(summary.safe_summary.as_str().to_string()))
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

    /// Classification lock for `capability_error_class`: every
    /// `CapabilityFailureKind` variant maps to a deliberate recovery class.
    ///
    /// This complements the compile-time guarantee (the match is exhaustive with
    /// no `_ =>` wildcard, since `CapabilityFailureKind` is no longer
    /// `#[non_exhaustive]`, so a *new* variant fails to compile until classified)
    /// by also catching a silent *re-bucketing* of an *existing* variant — e.g.
    /// moving a recoverable kind into the run-aborting `Permanent` class, or vice
    /// versa. Only the genuinely-terminal kinds (`Cancelled` and `Permanent`)
    /// may map to `Permanent`; runtime-dispositioned tool failures such as
    /// `Dispatcher`, `InvalidOutput`, and the open-set `Unknown` stay
    /// model-visible. See
    /// `docs/plans/2026-06-28-reborn-error-recoverability-audit.md` §6.1.
    #[test]
    fn every_capability_failure_kind_has_a_deliberate_recovery_class() {
        use CapabilityErrorClass as C;
        use CapabilityFailureKind as K;

        let unknown = K::unknown("some_future_kind").expect("valid unknown kind");
        let cases: &[(K, C)] = &[
            (K::Network, C::Transient),
            (K::Transient, C::Transient),
            (K::Backend, C::Unavailable),
            (K::Unavailable, C::Unavailable),
            (K::InvalidInput, C::InputInvalid),
            (K::MissingRuntime, C::OperationFailed),
            (K::OperationFailed, C::OperationFailed),
            (K::OutputTooLarge, C::OperationFailed),
            (K::Process, C::OperationFailed),
            (K::Resource, C::OperationFailed),
            (K::Authorization, C::PolicyDenied),
            (K::GateDeclined, C::PolicyDenied),
            (K::PolicyDenied, C::PolicyDenied),
            (K::Internal, C::Internal),
            (K::Dispatcher, C::OperationFailed),
            (K::Cancelled, C::Permanent),
            (K::InvalidOutput, C::OperationFailed),
            (K::Permanent, C::Permanent),
            (unknown.clone(), C::OperationFailed),
        ];

        for (kind, expected) in cases {
            assert_eq!(
                capability_error_class(kind),
                *expected,
                "recovery class for {kind:?} changed — re-confirm it is deliberate \
                 and does not silently abort a recoverable failure"
            );
        }

        // Only these kinds may abort the run.
        for (kind, class) in cases {
            if *class == C::Permanent {
                assert!(
                    matches!(kind, K::Cancelled | K::Permanent),
                    "{kind:?} maps to the run-aborting Permanent class but is not a \
                     recognized terminal kind — a recoverable failure must not abort"
                );
            }
        }
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
