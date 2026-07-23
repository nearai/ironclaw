use ironclaw_turns::run_profile::{AgentLoopHostErrorKind, AgentLoopHostErrorReasonKind};

use crate::failure_categories::{
    BUDGET_ACCOUNTING_FAILED_CATEGORY, MODEL_CREDENTIALS_UNAVAILABLE_CATEGORY,
    MODEL_CREDITS_EXHAUSTED_CATEGORY, MODEL_CREDITS_EXHAUSTED_REASON_KIND,
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

    (kind == AgentLoopHostErrorKind::CredentialUnavailable)
        .then_some(MODEL_CREDENTIALS_UNAVAILABLE_CATEGORY)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_stage_host_error_kind_category_matrix_is_exhaustive() {
        use AgentLoopHostErrorKind as K;

        let expected_without_reason = |kind| match kind {
            K::CredentialUnavailable => Some(MODEL_CREDENTIALS_UNAVAILABLE_CATEGORY),
            K::BudgetAccountingFailed => Some(BUDGET_ACCOUNTING_FAILED_CATEGORY),
            K::Unauthorized
            | K::ScopeMismatch
            | K::StaleSurface
            | K::InvalidInvocation
            | K::Invalid
            | K::InvalidOutput
            | K::ContentFiltered
            | K::PolicyDenied
            | K::BudgetExceeded
            | K::BudgetApprovalRequired
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
