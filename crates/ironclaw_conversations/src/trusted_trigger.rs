use ironclaw_turns::{AdmissionRejectionReason, TurnError};

use crate::InboundTurnError;

/// Shared classification for trusted trigger paths that encounter
/// conversation inbound failures.
///
/// This stays in `ironclaw_conversations` because it is the crate that owns
/// `InboundTurnError`. Callers keep their own local `TriggerError` wording and
/// logging, so this module does not become a generic trusted-ingress facade.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TrustedTriggerInboundFailureKind {
    RetryableBackend,
    SubmitRejected,
    InboundRequestRejected,
}

pub(crate) fn classify_inbound_error(error: &InboundTurnError) -> TrustedTriggerInboundFailureKind {
    match error {
        InboundTurnError::TurnSubmissionFailed {
            error: TurnError::ThreadBusy(_),
        } => TrustedTriggerInboundFailureKind::RetryableBackend,
        InboundTurnError::TurnSubmissionFailed {
            error: TurnError::AdmissionRejected(rejection),
        } => match rejection.reason {
            AdmissionRejectionReason::TenantLimit | AdmissionRejectionReason::Unavailable => {
                TrustedTriggerInboundFailureKind::RetryableBackend
            }
            AdmissionRejectionReason::ProfileRejected
            | AdmissionRejectionReason::Policy
            | AdmissionRejectionReason::Unauthorized => {
                TrustedTriggerInboundFailureKind::SubmitRejected
            }
        },
        InboundTurnError::TurnSubmissionFailed {
            error:
                TurnError::Unavailable { .. }
                | TurnError::CapacityExceeded { .. }
                | TurnError::Conflict { .. },
        } => TrustedTriggerInboundFailureKind::RetryableBackend,
        InboundTurnError::TurnSubmissionFailed {
            error:
                TurnError::ScopeNotFound
                | TurnError::Unauthorized
                | TurnError::InvalidRequest { .. }
                | TurnError::InvalidTransition { .. }
                | TurnError::LeaseMismatch
                | TurnError::InvalidRunOriginAdapter,
        } => TrustedTriggerInboundFailureKind::SubmitRejected,
        InboundTurnError::InvalidExternalRef { .. }
        | InboundTurnError::BindingRequired { .. }
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
