use ironclaw_turns::run_profile::{AgentLoopHostErrorKind, AgentLoopHostErrorReasonKind};

use crate::failure_categories::{
    BUDGET_ACCOUNTING_FAILED_CATEGORY, MODEL_CREDENTIALS_UNAVAILABLE_CATEGORY,
    MODEL_CREDITS_EXHAUSTED_CATEGORY, MODEL_CREDITS_EXHAUSTED_REASON_KIND,
    MODEL_STAGE_POLICY_DENIED_CATEGORY, MODEL_STAGE_REQUEST_INVALID_CATEGORY,
    MODEL_STAGE_SCOPE_MISMATCH_CATEGORY,
};

pub(crate) fn model_stage_failure_category(
    is_model_stage: bool,
    kind: AgentLoopHostErrorKind,
    reason_kind: Option<AgentLoopHostErrorReasonKind>,
) -> Option<&'static str> {
    if !is_model_stage {
        return None;
    }

    if reason_kind == Some(MODEL_CREDITS_EXHAUSTED_REASON_KIND) {
        return Some(MODEL_CREDITS_EXHAUSTED_CATEGORY);
    }

    if kind == AgentLoopHostErrorKind::BudgetAccountingFailed {
        return Some(BUDGET_ACCOUNTING_FAILED_CATEGORY);
    }

    if kind == AgentLoopHostErrorKind::CredentialUnavailable {
        return Some(MODEL_CREDENTIALS_UNAVAILABLE_CATEGORY);
    }

    // Permanent for an identical retry. Without these, all four fell through to
    // `host_stage_unavailable_model` — an auto-retriable transient outage — so
    // the run re-drove a call that could not succeed and named the wrong cause.
    // `executor/mapping.rs` already documented this as handled; it was not.
    match kind {
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
            let category = model_stage_failure_category(true, kind, None).unwrap_or_else(|| {
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
                crate::failure_categories::HOST_STAGE_UNAVAILABLE_MODEL_CATEGORY,
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
                model_stage_failure_category(true, kind, None),
                expected_without_reason(kind),
                "model-stage category for {kind:?} changed"
            );
            assert_eq!(
                model_stage_failure_category(false, kind, None),
                None,
                "non-model stage must not produce model-specific category for {kind:?}"
            );
        }
    }

    #[test]
    fn model_credits_reason_overrides_model_stage_error_kind() {
        let reason = Some(AgentLoopHostErrorReasonKind::ModelCreditsExhausted);

        assert_eq!(
            model_stage_failure_category(
                true,
                AgentLoopHostErrorKind::CredentialUnavailable,
                reason
            ),
            Some(MODEL_CREDITS_EXHAUSTED_CATEGORY)
        );
        assert_eq!(
            model_stage_failure_category(true, AgentLoopHostErrorKind::Internal, reason),
            Some(MODEL_CREDITS_EXHAUSTED_CATEGORY)
        );
        assert_eq!(
            model_stage_failure_category(
                false,
                AgentLoopHostErrorKind::CredentialUnavailable,
                reason
            ),
            None
        );
    }
}
