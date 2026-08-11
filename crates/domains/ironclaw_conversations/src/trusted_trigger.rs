use crate::InboundTurnError;
use crate::turn_submission::TurnSubmissionRetry;

/// Shared classification for trusted trigger paths that encounter
/// conversation inbound failures.
///
/// This stays in `ironclaw_conversations` because it is the crate that owns
/// `InboundTurnError`. Callers keep their own local `TriggerError` wording and
/// logging, so this module does not become a generic trusted-ingress service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TrustedTriggerInboundFailureKind {
    RetryableBackend,
    SubmitRejected,
    InboundRequestRejected,
}

pub(crate) fn classify_inbound_error(error: &InboundTurnError) -> TrustedTriggerInboundFailureKind {
    match error {
        // A submission failure classifies by the port error's own retry class.
        // Both retryable classes are the same thing to a trigger fire — the
        // fire is re-polled — while a permanent rejection is a submit
        // rejection. Which host failure lands in which class is the port
        // implementor's total mapping, pinned at that seam.
        InboundTurnError::TurnSubmissionFailed { error } => match error.retry() {
            TurnSubmissionRetry::RetryableAfterKeyRotation
            | TurnSubmissionRetry::RetryableWithSameKey => {
                TrustedTriggerInboundFailureKind::RetryableBackend
            }
            TurnSubmissionRetry::Permanent => TrustedTriggerInboundFailureKind::SubmitRejected,
        },
        InboundTurnError::BindingRequired { .. }
        | InboundTurnError::InvalidExternalRef { .. }
        | InboundTurnError::AccessDenied { .. }
        | InboundTurnError::BindingConflict { .. }
        | InboundTurnError::ThreadNotFound { .. }
        | InboundTurnError::StatePoisoned
        | InboundTurnError::InvalidCanonicalRef { .. } => {
            TrustedTriggerInboundFailureKind::InboundRequestRejected
        }
        InboundTurnError::DurableState { .. } => TrustedTriggerInboundFailureKind::RetryableBackend,
    }
}
