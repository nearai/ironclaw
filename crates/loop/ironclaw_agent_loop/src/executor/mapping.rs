use ironclaw_host_api::turn::SanitizedFailure;
use ironclaw_host_api::{
    resolution::{Resolution, ToolVerdict},
    result_meta::FailureKind,
};
use ironclaw_loop_contracts::{
    AgentLoopHostError, AgentLoopHostErrorKind, BatchPolicyKind, LoopBlockedKind,
    LoopCheckpointKind, LoopGateKind, LoopRecoveryClass, LoopSafeSummary,
    sanitize_model_visible_text,
};

use crate::{
    state::CheckpointKind,
    strategies::{
        BatchPolicy, GateKind, ModelErrorClass, ModelErrorSummary, ModelPreference,
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

pub(super) fn capability_batch_counts<'a>(
    resolutions: impl IntoIterator<Item = &'a Resolution>,
) -> (u32, u32, u32, u32) {
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
) -> Result<(Option<ironclaw_loop_contracts::ModelProfileId>, u32), AgentLoopExecutorError> {
    match preference {
        ModelPreference::Primary => Ok((None, 0)),
        ModelPreference::Fallback { index } if index > 0 => Ok((None, index)),
        ModelPreference::Fallback { .. } => Err(AgentLoopExecutorError::PlannerContract {
            detail: "fallback model preference index must be nonzero",
        }),
    }
}

pub(super) fn model_error_class(error: &AgentLoopHostError) -> Option<ModelErrorClass> {
    match error.kind {
        AgentLoopHostErrorKind::RateLimited => Some(ModelErrorClass::Transient),
        AgentLoopHostErrorKind::Unavailable => Some(ModelErrorClass::Unavailable),
        AgentLoopHostErrorKind::Internal => Some(ModelErrorClass::Internal),
        AgentLoopHostErrorKind::InvalidOutput => Some(ModelErrorClass::InvalidOutput),
        AgentLoopHostErrorKind::ContentFiltered => Some(ModelErrorClass::ContentFiltered),
        // Legacy generic capacity errors keep the historic context-overflow
        // projection. Live model-call producers use the precise variants below.
        AgentLoopHostErrorKind::BudgetExceeded => Some(ModelErrorClass::ContextOverflow),
        AgentLoopHostErrorKind::ContextOverflow => Some(ModelErrorClass::ContextOverflow),
        AgentLoopHostErrorKind::OutputTruncated => Some(ModelErrorClass::OutputTruncated),
        // Exhausted configured spend is terminal: another model call cannot
        // succeed until the budget changes, and shrinking context is irrelevant.
        AgentLoopHostErrorKind::SpendBudgetExceeded => None,
        // Accounting storage failed before the host could establish a
        // trustworthy budget outcome. Preserve the typed host error instead
        // of retrying it as a provider availability failure.
        AgentLoopHostErrorKind::BudgetAccountingFailed => None,
        // Budget approval requirement is a gate, not a transient model
        // error — pass it through unclassified so the loop's gate handling
        // path takes over rather than the recovery strategy.
        AgentLoopHostErrorKind::BudgetApprovalRequired => None,
        AgentLoopHostErrorKind::Cancelled => None,
        // Unclassified so the runner derives the precise category from
        // kind + reason_kind (`model_credits_exhausted` vs
        // `model_credentials_unavailable`); classifying here would lose the
        // reason_kind distinction. See
        // `ironclaw_turn_runner::model_failure_mapping::host_stage_failure_category`.
        AgentLoopHostErrorKind::CredentialUnavailable => None,
        // Model-fixable by rebuild: the request was built against a stale
        // surface or prompt bundle (surface refreshed mid-iteration, host
        // state moved). An iteration-scoped retry rebuilds both; exhaustion
        // fails with the precise `model_stale_request` category. Audit
        // §6.1/§7, docs/internal/plans/2026-06-28-reborn-error-recoverability-audit.md.
        AgentLoopHostErrorKind::StaleSurface => Some(ModelErrorClass::StaleRequest),
        // Precise terminal categories: immediate abort via the recovery
        // strategy so the run fails gracefully (`LoopExit::Failed` with a
        // user-actionable category) instead of hard-borking as a generic
        // model-stage unavailability.
        AgentLoopHostErrorKind::Unauthorized => Some(ModelErrorClass::Unauthorized),
        AgentLoopHostErrorKind::CheckpointRejected => Some(ModelErrorClass::CheckpointRejected),
        AgentLoopHostErrorKind::TranscriptWriteFailed => {
            Some(ModelErrorClass::TranscriptWriteFailed)
        }
        // Deliberately unclassified (terminal with diagnostics): deterministic
        // request-invalid errors must not masquerade as stale/retryable, while
        // policy denial and scope mismatch remain host/config-shaped. The
        // runner names each of these with its own failure category
        // (`model_stage_request_invalid` / `_policy_denied` / `_scope_mismatch`
        // in `ironclaw_turn_runner::failure_categories`), none of which is
        // auto-retriable.
        //
        // This comment previously claimed the runner "preserves the original
        // kind" — it did not. All four fell through to
        // `host_stage_unavailable_model`, which the runner lists as a transient
        // outage that re-drives cleanly, so a permanently-failing call was
        // silently retried and reported as a generic host outage.
        AgentLoopHostErrorKind::InvalidInvocation
        | AgentLoopHostErrorKind::Invalid
        | AgentLoopHostErrorKind::ScopeMismatch
        | AgentLoopHostErrorKind::PolicyDenied => None,
    }
}

pub(super) fn model_recovery_class(class: ModelErrorClass) -> LoopRecoveryClass {
    match class {
        ModelErrorClass::Transient => LoopRecoveryClass::ModelTransient,
        ModelErrorClass::ContextOverflow => LoopRecoveryClass::ModelContextOverflow,
        ModelErrorClass::ContentFiltered => LoopRecoveryClass::ModelContentFiltered,
        ModelErrorClass::InvalidOutput => LoopRecoveryClass::ModelInvalidOutput,
        ModelErrorClass::OutputTruncated => LoopRecoveryClass::ModelOutputTruncated,
        ModelErrorClass::Unavailable => LoopRecoveryClass::ModelUnavailable,
        ModelErrorClass::Internal => LoopRecoveryClass::ModelInternal,
        ModelErrorClass::StaleRequest => LoopRecoveryClass::ModelStaleRequest,
        ModelErrorClass::Unauthorized => LoopRecoveryClass::ModelUnauthorized,
        ModelErrorClass::CheckpointRejected => LoopRecoveryClass::ModelCheckpointRejected,
        ModelErrorClass::TranscriptWriteFailed => LoopRecoveryClass::ModelTranscriptWriteFailed,
    }
}

/// Whether a capability-stage port `Err` is a genuine host fault that must end
/// the run (`capability_host_error`), as opposed to a caller-shaped failure the
/// model can recover from (surfaced as a tool error via
/// `handle_capability_error`, routed by `FailureKind::fate`).
///
/// Exhaustive on purpose — a new port kind must decide its dispatch
/// disposition here instead of inheriting a wildcard bucket. Terminal kinds
/// are cancellation plus the Internal/Resource-shaped host faults (the host's
/// own machinery failed: budget accounting, checkpointing, transcript,
/// availability); everything else describes the *call*, which the model can
/// route around.
pub(super) fn capability_port_error_is_terminal(kind: AgentLoopHostErrorKind) -> bool {
    match kind {
        AgentLoopHostErrorKind::Cancelled
        | AgentLoopHostErrorKind::RateLimited
        | AgentLoopHostErrorKind::Unavailable
        | AgentLoopHostErrorKind::Internal
        | AgentLoopHostErrorKind::BudgetExceeded
        | AgentLoopHostErrorKind::SpendBudgetExceeded
        | AgentLoopHostErrorKind::ContextOverflow
        | AgentLoopHostErrorKind::OutputTruncated
        | AgentLoopHostErrorKind::BudgetApprovalRequired
        | AgentLoopHostErrorKind::BudgetAccountingFailed
        | AgentLoopHostErrorKind::CheckpointRejected
        | AgentLoopHostErrorKind::TranscriptWriteFailed => true,
        AgentLoopHostErrorKind::Unauthorized
        | AgentLoopHostErrorKind::CredentialUnavailable
        | AgentLoopHostErrorKind::ScopeMismatch
        | AgentLoopHostErrorKind::StaleSurface
        | AgentLoopHostErrorKind::InvalidInvocation
        | AgentLoopHostErrorKind::Invalid
        | AgentLoopHostErrorKind::InvalidOutput
        | AgentLoopHostErrorKind::ContentFiltered
        | AgentLoopHostErrorKind::PolicyDenied => false,
    }
}

pub(super) fn capability_host_error(error: AgentLoopHostError) -> AgentLoopExecutorError {
    if error.kind == AgentLoopHostErrorKind::Cancelled {
        return AgentLoopExecutorError::Cancelled;
    }
    if error.kind == AgentLoopHostErrorKind::TranscriptWriteFailed {
        return transcript_host_error(error);
    }
    // Fail soft on a malformed summary: a summary that fails strict validation
    // (e.g. contains `/`, `{`) must NOT bork the run. Degrade to a canned
    // fallback and carry the real cause on the model-visible detail channel so
    // the failure explainer/runner still sees why the call failed. debug! only
    // — info!/warn! corrupt the REPL/TUI (see repo CLAUDE.md).
    let raw_summary = error.safe_summary;
    let (safe_summary, rejected_summary_detail) = match LoopSafeSummary::new(raw_summary.clone()) {
        Ok(summary) => (summary, None),
        Err(validation_error) => {
            tracing::debug!(
                kind = error.kind.as_str(),
                validation_error = %validation_error,
                "capability host error summary rejected; using fallback"
            );
            (
                LoopSafeSummary::capability_failure_summary(raw_summary.clone()),
                Some(sanitize_model_visible_text(raw_summary)),
            )
        }
    };
    let detail = error.detail.or(rejected_summary_detail);
    if detail.is_none() && error.reason_kind.is_none() {
        return AgentLoopExecutorError::HostUnavailable {
            stage: HostStage::Capability,
        };
    }
    AgentLoopExecutorError::HostUnavailableWithDiagnostics {
        stage: HostStage::Capability,
        kind: error.kind,
        safe_summary,
        reason_kind: error.reason_kind,
        detail,
    }
}

/// Preserve the typed transcript-write cause without exposing backend detail.
///
/// Another model output would cross the same failed durability boundary, so
/// remediation is derived from the terminal category rather than model
/// inference.
pub(super) fn transcript_host_error(error: AgentLoopHostError) -> AgentLoopExecutorError {
    if error.kind == AgentLoopHostErrorKind::Cancelled {
        return AgentLoopExecutorError::Cancelled;
    }
    let error = error.sanitize_transcript_write_failure();
    let raw_summary = error.safe_summary;
    let (safe_summary, rejected_summary_detail) = match LoopSafeSummary::new(raw_summary.clone()) {
        Ok(summary) => (summary, None),
        Err(validation_error) => {
            tracing::debug!(
                kind = error.kind.as_str(),
                validation_error = %validation_error,
                "transcript host error summary rejected; using fallback"
            );
            (
                LoopSafeSummary::assistant_transcript_write_failed(),
                Some(sanitize_model_visible_text(raw_summary)),
            )
        }
    };
    AgentLoopExecutorError::HostUnavailableWithDiagnostics {
        stage: HostStage::Transcript,
        kind: error.kind,
        safe_summary,
        reason_kind: error.reason_kind,
        detail: error.detail.or(rejected_summary_detail),
    }
}

/// Sanitized failure-category wire strings for a terminal capability failure.
///
/// The seven output strings are a HARD cross-crate contract: the runner's
/// failure summaries and the product failure explanations match on them
/// byte-for-byte. This bucketing preserves the retired `CapabilityErrorClass`
/// membership for the retired kinds and assigns each precise kind to the bucket
/// its coarse ancestor used:
///
/// - `capability_permanent` survives only for `Cancelled` (the retired
///   `Permanent` *kind* merged into `OperationFailed`, so its old bucket is no
///   longer reachable through that name — the string stays pinned for the
///   cancellation abort path and for the runner's benefit).
/// - `StaleSurface` keeps `capability_policy_denied`: the retired mint sites
///   lied with `PolicyDenied` and the wire category must not change under the
///   honest rename.
pub(super) fn capability_error_failure_category(
    kind: FailureKind,
) -> Result<SanitizedFailure, AgentLoopExecutorError> {
    sanitized_failure_category(match kind {
        FailureKind::Network | FailureKind::Transient => "capability_transient",
        FailureKind::Backend | FailureKind::Unavailable => "capability_unavailable",
        FailureKind::Internal => "capability_internal",
        FailureKind::PolicyDenied
        | FailureKind::NetworkDenied
        | FailureKind::FilesystemDenied
        | FailureKind::SecretDenied
        | FailureKind::Authorization
        | FailureKind::GateDeclined
        | FailureKind::AuthRequired
        | FailureKind::StaleSurface => "capability_policy_denied",
        FailureKind::InputEncode => "capability_input_invalid",
        FailureKind::Cancelled => "capability_permanent",
        FailureKind::MethodMissing
        | FailureKind::UndeclaredCapability
        | FailureKind::UnknownCapability
        | FailureKind::UnknownProvider
        | FailureKind::OperationFailed
        | FailureKind::OutputTooLarge
        | FailureKind::Resource
        | FailureKind::Guest
        | FailureKind::ExitFailure
        | FailureKind::OutputDecode
        | FailureKind::InvalidResult
        | FailureKind::Memory
        | FailureKind::Manifest
        | FailureKind::ExtensionRuntimeMismatch
        | FailureKind::RuntimeMismatch
        | FailureKind::MissingRuntimeBackend
        | FailureKind::UnsupportedRunner
        | FailureKind::MissingRuntime
        | FailureKind::Client
        | FailureKind::Executor
        | FailureKind::Unclassified => "capability_operation_failed",
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
        ModelErrorClass::OutputTruncated => "model_output_truncated",
        ModelErrorClass::Unavailable => "model_unavailable",
        ModelErrorClass::Internal => "model_internal",
        ModelErrorClass::StaleRequest => "model_stale_request",
        // Pinned category shared with the runner's CredentialUnavailable
        // mapping (`ironclaw_turn_runner::failure_categories`): an unauthorized
        // model call is a credentials/permission problem the user must fix.
        ModelErrorClass::Unauthorized => "model_credentials_unavailable",
        ModelErrorClass::CheckpointRejected => "checkpoint_rejected",
        ModelErrorClass::TranscriptWriteFailed => "transcript_write_failed",
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

/// Sanitize a strategy summary, failing soft: a summary that fails strict
/// validation degrades to a fixed fallback instead of aborting the run, and the
/// secret-value-scrubbed raw cause is returned alongside so the caller can carry
/// it on the model-visible detail channel.
pub(super) fn sanitized_strategy_summary_or_fallback(
    summary: String,
    fallback: &'static str,
) -> (SanitizedStrategySummary, Option<String>) {
    match SanitizedStrategySummary::new(summary.clone()) {
        Ok(summary) => (summary, None),
        Err(validation_error) => {
            tracing::debug!(
                validation_error = %validation_error,
                "strategy summary rejected; using fallback"
            );
            (
                SanitizedStrategySummary::from_trusted_static(fallback),
                Some(sanitize_model_visible_text(summary)),
            )
        }
    }
}

pub(super) fn honor_capability_retry_alteration(
    alteration: Option<&RetryAlteration>,
) -> Result<(), AgentLoopExecutorError> {
    if matches!(alteration, Some(RetryAlteration::AdvanceFallback { .. })) {
        return Err(AgentLoopExecutorError::PlannerContract {
            detail: "fallback advancement is valid only for model recovery",
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_capability_input_is_model_error_not_protocol_failure() {
        use crate::strategies::capability_error_to_failure_kind;
        use ironclaw_loop_contracts::LoopFailureKind;

        assert_eq!(
            capability_error_to_failure_kind(FailureKind::InputEncode),
            LoopFailureKind::ModelError
        );
    }

    #[test]
    fn fallback_model_preference_maps_to_ordered_host_index() {
        assert_eq!(
            model_preference_to_host(ModelPreference::Primary).expect("primary"),
            (None, 0)
        );
        assert_eq!(
            model_preference_to_host(ModelPreference::Fallback { index: 2 })
                .expect("configured fallback"),
            (None, 2)
        );
        assert!(
            model_preference_to_host(ModelPreference::Fallback { index: 0 }).is_err(),
            "fallback zero aliases primary and must be rejected as a planner bug"
        );
    }

    #[test]
    fn protocol_and_policy_failure_kinds_remain_distinct() {
        use crate::strategies::capability_error_to_failure_kind;
        use ironclaw_loop_contracts::LoopFailureKind;

        assert_eq!(
            capability_error_to_failure_kind(FailureKind::OutputDecode),
            LoopFailureKind::CapabilityProtocolError
        );
        assert_eq!(
            capability_error_to_failure_kind(FailureKind::PolicyDenied),
            LoopFailureKind::PolicyDenied
        );
    }

    /// Classification lock for the seven-string failure-category contract:
    /// every unified `FailureKind` maps to a deliberate wire category, and the
    /// bucket set never grows — runner and product failure explanations match
    /// these strings byte-for-byte.
    ///
    /// This complements the compile-time guarantee (the match is exhaustive
    /// with no `_ =>` wildcard) by also catching a silent *re-bucketing* of an
    /// *existing* kind. Notable pins:
    /// - retry-fated kinds keep their retryable buckets
    ///   (transient/unavailable/internal), matching their `fate()`;
    /// - `StaleSurface` stays `capability_policy_denied` — the retired mint
    ///   sites minted `PolicyDenied` there and the wire category must not
    ///   change under the honest rename;
    /// - `capability_permanent` is reachable only through `Cancelled`, the
    ///   sole Terminal-fated kind.
    #[test]
    fn every_failure_kind_has_a_deliberate_failure_category() {
        use ironclaw_host_api::result_meta::FailureFate;

        const ALL_CATEGORIES: [&str; 7] = [
            "capability_transient",
            "capability_unavailable",
            "capability_internal",
            "capability_policy_denied",
            "capability_input_invalid",
            "capability_operation_failed",
            "capability_permanent",
        ];

        let expected: &[(FailureKind, &str)] = &[
            (FailureKind::Network, "capability_transient"),
            (FailureKind::Transient, "capability_transient"),
            (FailureKind::Backend, "capability_unavailable"),
            (FailureKind::Unavailable, "capability_unavailable"),
            (FailureKind::Internal, "capability_internal"),
            (FailureKind::InputEncode, "capability_input_invalid"),
            (FailureKind::PolicyDenied, "capability_policy_denied"),
            (FailureKind::NetworkDenied, "capability_policy_denied"),
            (FailureKind::FilesystemDenied, "capability_policy_denied"),
            (FailureKind::SecretDenied, "capability_policy_denied"),
            (FailureKind::Authorization, "capability_policy_denied"),
            (FailureKind::GateDeclined, "capability_policy_denied"),
            (FailureKind::AuthRequired, "capability_policy_denied"),
            (FailureKind::StaleSurface, "capability_policy_denied"),
            (FailureKind::Cancelled, "capability_permanent"),
        ];
        for (kind, category) in expected {
            let failure = capability_error_failure_category(*kind).expect("valid category");
            assert_eq!(
                failure.category(),
                *category,
                "failure category for {kind:?} changed — the seven strings are a \
                 cross-crate contract with the runner and product layers"
            );
        }

        for &kind in FailureKind::ALL {
            let failure = capability_error_failure_category(kind).expect("valid category");
            let category = failure.category().to_string();
            assert!(
                ALL_CATEGORIES.contains(&category.as_str()),
                "{kind:?} produced {category:?}, outside the pinned seven-string set"
            );
            if kind.fate() == FailureFate::Retry {
                assert!(
                    matches!(
                        category.as_str(),
                        "capability_transient" | "capability_unavailable" | "capability_internal"
                    ),
                    "{kind:?} is Retry-fated but categorized {category:?}"
                );
            }
            if category == "capability_permanent" {
                assert_eq!(
                    kind,
                    FailureKind::Cancelled,
                    "capability_permanent is reserved for the cancellation abort path"
                );
            }
        }
    }

    /// Classification lock for `model_error_class`: every model-path
    /// `AgentLoopHostErrorKind` maps to a deliberate outcome. `Some(class)`
    /// routes through the recovery strategy (retry / precise-category abort);
    /// `None` is reserved for kinds the executor handles structurally
    /// (`Cancelled`, gate-shaped `BudgetApprovalRequired`) or that must reach
    /// the runner as `HostUnavailableWithDiagnostics{Model}` because the
    /// runner derives their precise category from kind + reason_kind
    /// (`CredentialUnavailable` -> credits/credentials,
    /// `BudgetAccountingFailed` -> budget_accounting_failed). See
    /// `docs/internal/plans/2026-06-28-reborn-error-recoverability-audit.md` §1/§7.
    #[test]
    fn every_model_path_host_error_kind_has_a_deliberate_class() {
        use AgentLoopHostErrorKind as K;
        use ModelErrorClass as C;

        let class_for = |kind: K| model_error_class(&AgentLoopHostError::new(kind, "test"));
        let cases: &[(K, Option<C>)] = &[
            (K::RateLimited, Some(C::Transient)),
            (K::Unavailable, Some(C::Unavailable)),
            (K::Internal, Some(C::Internal)),
            (K::InvalidOutput, Some(C::InvalidOutput)),
            (K::ContentFiltered, Some(C::ContentFiltered)),
            (K::BudgetExceeded, Some(C::ContextOverflow)),
            (K::SpendBudgetExceeded, None),
            (K::ContextOverflow, Some(C::ContextOverflow)),
            (K::OutputTruncated, Some(C::OutputTruncated)),
            // Model-fixable-by-rebuild: iteration retry refreshes the surface
            // and prompt bundle; exhaustion -> `model_stale_request`.
            (K::StaleSurface, Some(C::StaleRequest)),
            // Precise terminal categories, never silently retried.
            (K::Unauthorized, Some(C::Unauthorized)),
            (K::CheckpointRejected, Some(C::CheckpointRejected)),
            (K::TranscriptWriteFailed, Some(C::TranscriptWriteFailed)),
            // Structural / runner-categorized kinds stay unclassified.
            (K::BudgetAccountingFailed, None),
            (K::BudgetApprovalRequired, None),
            (K::Cancelled, None),
            (K::CredentialUnavailable, None),
            (K::InvalidInvocation, None),
            (K::Invalid, None),
            (K::PolicyDenied, None),
            (K::ScopeMismatch, None),
        ];

        for (kind, expected) in cases {
            assert_eq!(
                class_for(*kind),
                *expected,
                "model error class for {kind:?} changed — re-confirm the audit lane \
                 (recoverable vs precise-terminal vs runner-categorized) is deliberate"
            );
        }
    }

    #[test]
    fn stale_request_and_precise_terminal_classes_have_precise_categories() {
        for (class, category) in [
            (ModelErrorClass::StaleRequest, "model_stale_request"),
            (ModelErrorClass::OutputTruncated, "model_output_truncated"),
            (
                ModelErrorClass::Unauthorized,
                "model_credentials_unavailable",
            ),
            (ModelErrorClass::CheckpointRejected, "checkpoint_rejected"),
            (
                ModelErrorClass::TranscriptWriteFailed,
                "transcript_write_failed",
            ),
        ] {
            let failure = model_error_failure_category(class).expect("valid category");
            assert_eq!(failure.category(), category);
        }
    }

    #[test]
    fn output_truncation_preserves_its_recovery_identity() {
        assert_eq!(
            model_recovery_class(ModelErrorClass::OutputTruncated).as_str(),
            "model_output_truncated"
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

    #[test]
    fn capability_result_transcript_failure_uses_terminal_transcript_lane() {
        let mapped = capability_host_error(
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::TranscriptWriteFailed,
                "raw tool result",
            )
            .with_detail("storage credential sk-secret"),
        );

        assert_eq!(
            mapped,
            AgentLoopExecutorError::HostUnavailableWithDiagnostics {
                stage: HostStage::Transcript,
                kind: AgentLoopHostErrorKind::TranscriptWriteFailed,
                safe_summary: LoopSafeSummary::assistant_transcript_write_failed(),
                reason_kind: None,
                detail: None,
            }
        );
    }

    #[test]
    fn non_transcript_finalization_error_preserves_its_sanitized_diagnostics() {
        let mapped = transcript_host_error(
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::ScopeMismatch,
                "thread scope did not match",
            )
            .with_detail("expected tenant scope"),
        );

        assert_eq!(
            mapped,
            AgentLoopExecutorError::HostUnavailableWithDiagnostics {
                stage: HostStage::Transcript,
                kind: AgentLoopHostErrorKind::ScopeMismatch,
                safe_summary: LoopSafeSummary::new("thread scope did not match").expect("safe"),
                reason_kind: None,
                detail: Some("expected tenant scope".to_string()),
            }
        );
    }

    #[test]
    fn invalid_transcript_stage_summary_uses_transcript_specific_fallback() {
        let mapped = transcript_host_error(AgentLoopHostError::new(
            AgentLoopHostErrorKind::ScopeMismatch,
            "invalid\0summary",
        ));

        let AgentLoopExecutorError::HostUnavailableWithDiagnostics {
            stage,
            safe_summary,
            ..
        } = mapped
        else {
            panic!("transcript host error must retain diagnostic structure");
        };
        assert_eq!(stage, HostStage::Transcript);
        assert_eq!(
            safe_summary,
            LoopSafeSummary::assistant_transcript_write_failed()
        );
    }
}
