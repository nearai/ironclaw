use ironclaw_agent_loop::executor::HostStage;
use ironclaw_loop_contracts::{AgentLoopHostErrorKind, AgentLoopHostErrorReasonKind};

use ironclaw_host_api::failure::categories::{
    BUDGET_ACCOUNTING_FAILED_CATEGORY, MODEL_CREDENTIALS_UNAVAILABLE_CATEGORY,
    MODEL_CREDITS_EXHAUSTED_CATEGORY, MODEL_PROVIDER_SESSION_UNAVAILABLE_CATEGORY,
    MODEL_PROVIDER_UNCONFIGURED_CATEGORY, MODEL_SPEND_BUDGET_EXHAUSTED_CATEGORY,
    MODEL_STAGE_POLICY_DENIED_CATEGORY, MODEL_STAGE_REQUEST_INVALID_CATEGORY,
    MODEL_STAGE_SCOPE_MISMATCH_CATEGORY, TRANSCRIPT_WRITE_FAILED_CATEGORY,
};

pub(crate) fn host_stage_failure_category(
    stage: HostStage,
    kind: AgentLoopHostErrorKind,
    reason_kind: Option<AgentLoopHostErrorReasonKind>,
) -> Option<&'static str> {
    if stage == HostStage::Transcript && kind == AgentLoopHostErrorKind::TranscriptWriteFailed {
        return Some(TRANSCRIPT_WRITE_FAILED_CATEGORY);
    }
    if stage != HostStage::Model {
        return None;
    }

    // A reason kind narrows a kind whose user-facing cause is narrower than the
    // kind itself, so it wins over the kind table below. Exhaustive on purpose:
    // a new reason kind must be given a category here, or it silently inherits
    // the kind's category and the operator is told the wrong cause.
    if let Some(reason_kind) = reason_kind {
        return Some(match reason_kind {
            AgentLoopHostErrorReasonKind::ModelCreditsExhausted => MODEL_CREDITS_EXHAUSTED_CATEGORY,
            AgentLoopHostErrorReasonKind::ModelProviderUnconfigured => {
                MODEL_PROVIDER_UNCONFIGURED_CATEGORY
            }
            AgentLoopHostErrorReasonKind::ModelProviderSessionUnavailable => {
                MODEL_PROVIDER_SESSION_UNAVAILABLE_CATEGORY
            }
        });
    }

    if kind == AgentLoopHostErrorKind::BudgetAccountingFailed {
        return Some(BUDGET_ACCOUNTING_FAILED_CATEGORY);
    }

    // Permanent for an identical retry. Without these, all four fell through to
    // `host_stage_unavailable_model` — an auto-retriable transient outage — so
    // the run re-drove a call that could not succeed and named the wrong cause.
    // `executor/mapping.rs` already documented this as handled; it was not.
    match kind {
        AgentLoopHostErrorKind::CredentialUnavailable => {
            Some(MODEL_CREDENTIALS_UNAVAILABLE_CATEGORY)
        }
        AgentLoopHostErrorKind::SpendBudgetExceeded => Some(MODEL_SPEND_BUDGET_EXHAUSTED_CATEGORY),
        AgentLoopHostErrorKind::InvalidInvocation | AgentLoopHostErrorKind::Invalid => {
            Some(MODEL_STAGE_REQUEST_INVALID_CATEGORY)
        }
        AgentLoopHostErrorKind::PolicyDenied => Some(MODEL_STAGE_POLICY_DENIED_CATEGORY),
        AgentLoopHostErrorKind::ScopeMismatch => Some(MODEL_STAGE_SCOPE_MISMATCH_CATEGORY),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A permanent model-stage failure must not be retried as a transient
    /// host outage.
    ///
    /// `InvalidInvocation`, `Invalid`, `ScopeMismatch` and `PolicyDenied` all
    /// returned `None` here, so they fell through to
    /// `host_stage_unavailable_model` — which `is_auto_retriable_category`
    /// lists as a transient outage that "re-drives cleanly on a silent retry".
    /// None of the four can succeed on an identical retry: policy does not
    /// change between attempts, and a malformed request stays malformed. The
    /// run burned retries on a call that could not work and told the operator
    /// "host stage unavailable" instead of naming the real cause.
    ///
    /// `executor/mapping.rs` already claimed this was handled — "the runner
    /// preserves the original kind when categorizing the failure". It did not.
    /// This pins the claim.
    #[test]
    fn permanent_model_stage_failures_are_not_categorized_as_transient_outages() {
        use crate::retry_disposition::is_auto_retriable_category;
        use AgentLoopHostErrorKind as K;

        for kind in [
            K::InvalidInvocation,
            K::Invalid,
            K::ScopeMismatch,
            K::PolicyDenied,
        ] {
            let category = host_stage_failure_category(HostStage::Model, kind, None)
                .unwrap_or_else(|| {
                    panic!(
                        "{kind:?} has no model-stage category, so it falls through to the generic \
                     host-stage outage and is silently auto-retried"
                    )
                });
            assert!(
                !is_auto_retriable_category(category),
                "{kind:?} -> {category:?} is auto-retriable, but an identical retry cannot succeed"
            );
            assert_ne!(
                category,
                ironclaw_host_api::failure::categories::HOST_STAGE_UNAVAILABLE_MODEL_CATEGORY,
                "{kind:?} must name its own cause, not a generic host outage"
            );
        }
    }

    #[test]
    fn model_stage_host_error_kind_category_matrix_is_exhaustive() {
        use AgentLoopHostErrorKind as K;

        let expected_without_reason = |kind| match kind {
            K::CredentialUnavailable => Some(MODEL_CREDENTIALS_UNAVAILABLE_CATEGORY),
            K::BudgetAccountingFailed => Some(BUDGET_ACCOUNTING_FAILED_CATEGORY),
            K::SpendBudgetExceeded => Some(MODEL_SPEND_BUDGET_EXHAUSTED_CATEGORY),
            // Permanent for an identical retry — each names its own cause
            // instead of falling through to the auto-retriable generic
            // host-stage outage. Inverted from `None`, which pinned the bug
            // `permanent_model_stage_failures_are_not_categorized_as_transient_outages`
            // now guards.
            K::InvalidInvocation | K::Invalid => Some(MODEL_STAGE_REQUEST_INVALID_CATEGORY),
            K::PolicyDenied => Some(MODEL_STAGE_POLICY_DENIED_CATEGORY),
            K::ScopeMismatch => Some(MODEL_STAGE_SCOPE_MISMATCH_CATEGORY),
            K::Unauthorized
            | K::StaleSurface
            | K::InvalidOutput
            | K::ContentFiltered
            | K::BudgetExceeded
            | K::ContextOverflow
            | K::OutputTruncated
            | K::BudgetApprovalRequired
            | K::RateLimited
            | K::Unavailable
            | K::Cancelled
            | K::CheckpointRejected
            | K::TranscriptWriteFailed
            | K::Internal => None,
        };

        for kind in [
            K::Unauthorized,
            K::CredentialUnavailable,
            K::ScopeMismatch,
            K::StaleSurface,
            K::InvalidInvocation,
            K::Invalid,
            K::InvalidOutput,
            K::ContentFiltered,
            K::PolicyDenied,
            K::BudgetExceeded,
            K::SpendBudgetExceeded,
            K::ContextOverflow,
            K::OutputTruncated,
            K::BudgetApprovalRequired,
            K::BudgetAccountingFailed,
            K::RateLimited,
            K::Unavailable,
            K::Cancelled,
            K::CheckpointRejected,
            K::TranscriptWriteFailed,
            K::Internal,
        ] {
            assert_eq!(
                host_stage_failure_category(HostStage::Model, kind, None),
                expected_without_reason(kind),
                "model-stage category for {kind:?} changed"
            );
            assert_eq!(
                host_stage_failure_category(HostStage::Prompt, kind, None),
                None,
                "non-model stage must not produce model-specific category for {kind:?}"
            );
        }
    }

    /// `CredentialUnavailable` is shared by three faults with three different
    /// fixes: a genuine credential rejection (edit the key), a deployment with
    /// no provider configured (add one), and an unreadable saved provider
    /// session (sign in again). The reason kind is the only thing separating
    /// them downstream, so a variant that is not routed here silently rejoins
    /// the bad-key bucket and the operator is told to check a key that is not
    /// the problem.
    ///
    /// The `expected` match is exhaustive on purpose: a new reason kind fails
    /// to compile until it is given a category.
    #[test]
    fn every_model_stage_reason_kind_routes_to_its_own_category() {
        use AgentLoopHostErrorReasonKind as R;

        let expected = |reason| match reason {
            R::ModelCreditsExhausted => MODEL_CREDITS_EXHAUSTED_CATEGORY,
            R::ModelProviderUnconfigured => MODEL_PROVIDER_UNCONFIGURED_CATEGORY,
            R::ModelProviderSessionUnavailable => MODEL_PROVIDER_SESSION_UNAVAILABLE_CATEGORY,
        };

        for reason in [
            R::ModelCreditsExhausted,
            R::ModelProviderUnconfigured,
            R::ModelProviderSessionUnavailable,
        ] {
            let category = host_stage_failure_category(
                HostStage::Model,
                AgentLoopHostErrorKind::CredentialUnavailable,
                Some(reason),
            );
            assert_eq!(
                category,
                Some(expected(reason)),
                "{reason:?} must name its own cause"
            );
            assert!(
                !crate::retry_disposition::is_auto_retriable_category(expected(reason)),
                "{reason:?} cannot succeed on an identical retry"
            );
        }

        // Unqualified, the shared category is still correct: a genuine
        // credential rejection is the one fault its pinned sentence describes.
        assert_eq!(
            host_stage_failure_category(
                HostStage::Model,
                AgentLoopHostErrorKind::CredentialUnavailable,
                None
            ),
            Some(MODEL_CREDENTIALS_UNAVAILABLE_CATEGORY)
        );
    }

    #[test]
    fn model_credits_reason_overrides_model_stage_error_kind() {
        let reason = Some(AgentLoopHostErrorReasonKind::ModelCreditsExhausted);

        assert_eq!(
            host_stage_failure_category(
                HostStage::Model,
                AgentLoopHostErrorKind::CredentialUnavailable,
                reason
            ),
            Some(MODEL_CREDITS_EXHAUSTED_CATEGORY)
        );
        assert_eq!(
            host_stage_failure_category(HostStage::Model, AgentLoopHostErrorKind::Internal, reason),
            Some(MODEL_CREDITS_EXHAUSTED_CATEGORY)
        );
        assert_eq!(
            host_stage_failure_category(
                HostStage::Prompt,
                AgentLoopHostErrorKind::CredentialUnavailable,
                reason
            ),
            None
        );
    }

    #[test]
    fn transcript_write_failure_has_a_typed_terminal_category() {
        assert_eq!(
            host_stage_failure_category(
                HostStage::Transcript,
                AgentLoopHostErrorKind::TranscriptWriteFailed,
                None,
            ),
            Some(TRANSCRIPT_WRITE_FAILED_CATEGORY)
        );
        assert_eq!(
            host_stage_failure_category(
                HostStage::Prompt,
                AgentLoopHostErrorKind::TranscriptWriteFailed,
                None,
            ),
            None
        );
    }
}
