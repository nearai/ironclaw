//! Workflow-layer error vocabulary.
//!
//! [`ProductWorkflowError`] is the internal error type used within the workflow
//! crate. It converts to [`ProductAdapterError`] at the facade boundary so
//! adapters never see host-layer details.

use ironclaw_product_adapters::{
    ProductAdapterError, ProductWorkflowRejectionKind, RedactedString,
};
use ironclaw_turns::{TurnError, TurnErrorCategory};
use thiserror::Error;

/// Stable reasons for rejecting an auth continuation before or during turn resume.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthContinuationRejectionKind {
    NotTurnGateResume,
    MissingThreadScope,
    InvalidTurnRunRef,
    InvalidGateRef,
    InvalidIdempotencyKey,
    InvalidBindingRef,
    UnauthorizedBlockedGate,
}

impl AuthContinuationRejectionKind {
    pub fn sanitized_reason(self) -> &'static str {
        match self {
            Self::NotTurnGateResume => "auth continuation is not a turn-gate resume",
            Self::MissingThreadScope => "invalid auth continuation scope",
            Self::InvalidTurnRunRef => "invalid auth continuation run reference",
            Self::InvalidGateRef => "invalid auth continuation gate reference",
            Self::InvalidIdempotencyKey => "invalid auth continuation idempotency key",
            Self::InvalidBindingRef => "invalid auth continuation binding ref",
            Self::UnauthorizedBlockedGate => {
                "auth continuation does not match an authorized blocked auth gate"
            }
        }
    }
}

/// Internal error type for the product workflow facade.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProductWorkflowError {
    /// The adapter installation is not mapped to a tenant.
    #[error("unknown adapter installation")]
    UnknownInstallation,

    /// The conversation binding could not be resolved for the given external refs.
    #[error("binding resolution failed: {reason}")]
    BindingResolutionFailed { reason: String },

    /// The external actor has no trusted binding to a canonical user.
    #[error("binding required: {reason}")]
    BindingRequired { reason: String },

    /// The actor or route is not allowed to use the resolved thread.
    #[error("binding access denied")]
    BindingAccessDenied,

    /// The binding request is invalid and should not be retried unchanged.
    #[error("invalid binding request: {reason}")]
    InvalidBindingRequest { reason: String },

    /// Turn coordinator rejected the submission before typed turn errors were available.
    #[error("turn submission rejected: {reason}")]
    TurnSubmissionRejected { reason: String },

    /// Turn coordinator rejected the submission with typed category/status information.
    #[error("turn submission failed: {error}")]
    TurnSubmissionFailed { error: TurnError },

    /// Turn coordinator resume rejected.
    #[error("turn resume rejected: {reason}")]
    TurnResumeRejected { reason: String },

    /// Auth continuation was rejected with a stable sanitized reason.
    #[error("auth continuation rejected: {kind:?}")]
    AuthContinuationRejected { kind: AuthContinuationRejectionKind },

    /// Turn coordinator rejected a resume with typed category/status information.
    #[error("turn resume denied: {error}")]
    TurnResumeDenied { error: TurnError },

    /// A transient store or service failure.
    #[error("transient workflow failure: {reason}")]
    Transient { reason: String },

    /// Before-inbound policy failed before it could produce an allow/rewrite/reject outcome.
    #[error("before-inbound policy failed: {reason}")]
    BeforeInboundPolicyFailed { reason: String, permanent: bool },

    /// The action was identified as a duplicate and the prior outcome should be replayed.
    #[error("duplicate action")]
    DuplicateAction {
        prior_outcome: ironclaw_product_adapters::ProductInboundAck,
    },

    /// Command routing is not yet implemented.
    #[error("command routing unavailable: {command}")]
    CommandRoutingUnavailable { command: String },

    /// The requested action kind is not supported by this workflow version.
    #[error("unsupported action kind: {kind}")]
    UnsupportedActionKind { kind: String },
}

fn workflow_rejection_kind(category: TurnErrorCategory) -> ProductWorkflowRejectionKind {
    match category {
        TurnErrorCategory::ThreadBusy => ProductWorkflowRejectionKind::ThreadBusy,
        TurnErrorCategory::AdmissionRejected => ProductWorkflowRejectionKind::AdmissionRejected,
        TurnErrorCategory::ScopeNotFound => ProductWorkflowRejectionKind::ScopeNotFound,
        TurnErrorCategory::Unauthorized => ProductWorkflowRejectionKind::Unauthorized,
        TurnErrorCategory::InvalidRequest => ProductWorkflowRejectionKind::InvalidRequest,
        TurnErrorCategory::Unavailable => ProductWorkflowRejectionKind::Unavailable,
        TurnErrorCategory::Conflict => ProductWorkflowRejectionKind::Conflict,
    }
}

impl From<ProductWorkflowError> for ProductAdapterError {
    fn from(value: ProductWorkflowError) -> Self {
        match value {
            ProductWorkflowError::UnknownInstallation => ProductAdapterError::WorkflowRejected {
                kind: ProductWorkflowRejectionKind::Unauthorized,
                status_code: 403,
                retryable: false,
                reason: RedactedString::new("unknown adapter installation"),
            },
            ProductWorkflowError::BindingResolutionFailed { reason } => {
                ProductAdapterError::Internal {
                    detail: RedactedString::new(reason),
                }
            }
            ProductWorkflowError::BindingRequired { reason } => {
                ProductAdapterError::WorkflowRejected {
                    kind: ProductWorkflowRejectionKind::ScopeNotFound,
                    status_code: 404,
                    retryable: false,
                    reason: RedactedString::new(reason),
                }
            }
            ProductWorkflowError::BindingAccessDenied => ProductAdapterError::WorkflowRejected {
                kind: ProductWorkflowRejectionKind::Unauthorized,
                status_code: 403,
                retryable: false,
                reason: RedactedString::new("binding access denied"),
            },
            ProductWorkflowError::InvalidBindingRequest { reason } => {
                ProductAdapterError::WorkflowRejected {
                    kind: ProductWorkflowRejectionKind::InvalidRequest,
                    status_code: 400,
                    retryable: false,
                    reason: RedactedString::new(reason),
                }
            }
            ProductWorkflowError::TurnSubmissionRejected { reason } => {
                ProductAdapterError::Internal {
                    detail: RedactedString::new(reason),
                }
            }
            ProductWorkflowError::TurnSubmissionFailed { error } => {
                let status_code = error.adapter_status_code();
                ProductAdapterError::WorkflowRejected {
                    kind: workflow_rejection_kind(error.category()),
                    status_code,
                    retryable: matches!(status_code, 429 | 503),
                    reason: RedactedString::new(error.to_string()),
                }
            }
            ProductWorkflowError::TurnResumeRejected { reason } => ProductAdapterError::Internal {
                detail: RedactedString::new(reason),
            },
            ProductWorkflowError::AuthContinuationRejected { kind } => {
                ProductAdapterError::WorkflowRejected {
                    kind: ProductWorkflowRejectionKind::InvalidRequest,
                    status_code: 400,
                    retryable: false,
                    reason: RedactedString::new(kind.sanitized_reason()),
                }
            }
            ProductWorkflowError::TurnResumeDenied { error } => {
                let status_code = error.adapter_status_code();
                ProductAdapterError::WorkflowRejected {
                    kind: workflow_rejection_kind(error.category()),
                    status_code,
                    retryable: matches!(status_code, 429 | 503),
                    reason: RedactedString::new(error.to_string()),
                }
            }
            ProductWorkflowError::Transient { reason } => ProductAdapterError::WorkflowTransient {
                reason: RedactedString::new(reason),
            },
            ProductWorkflowError::BeforeInboundPolicyFailed { reason, permanent } => {
                // Adapter error surfaces wrap the reason in RedactedString, so
                // diagnostics remain available internally without leaking to
                // public protocol output.
                if permanent {
                    ProductAdapterError::WorkflowRejected {
                        kind: ProductWorkflowRejectionKind::AdmissionRejected,
                        status_code: 403,
                        retryable: false,
                        reason: RedactedString::new(reason),
                    }
                } else {
                    ProductAdapterError::WorkflowTransient {
                        reason: RedactedString::new(reason),
                    }
                }
            }
            ProductWorkflowError::DuplicateAction { .. } => ProductAdapterError::Internal {
                detail: RedactedString::new("duplicate action escaped workflow layer"),
            },
            ProductWorkflowError::CommandRoutingUnavailable { command } => {
                ProductAdapterError::Internal {
                    detail: RedactedString::new(format!("command routing unavailable: {command}")),
                }
            }
            ProductWorkflowError::UnsupportedActionKind { kind } => ProductAdapterError::Internal {
                detail: RedactedString::new(format!("unsupported action kind: {kind}")),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transient_maps_to_retryable() {
        let err: ProductAdapterError = ProductWorkflowError::Transient {
            reason: "db timeout".into(),
        }
        .into();
        assert!(err.is_retryable());
    }

    #[test]
    fn binding_failure_maps_to_internal() {
        let err: ProductAdapterError = ProductWorkflowError::BindingResolutionFailed {
            reason: "no tenant".into(),
        }
        .into();
        assert!(!err.is_retryable());
    }

    #[test]
    fn permanent_before_inbound_policy_failure_maps_to_rejection() {
        let err: ProductAdapterError = ProductWorkflowError::BeforeInboundPolicyFailed {
            reason: "classifier misconfigured".into(),
            permanent: true,
        }
        .into();
        assert!(!err.is_retryable());
        assert!(matches!(err, ProductAdapterError::WorkflowRejected { .. }));
    }

    #[test]
    fn turn_resume_denied_maps_to_workflow_rejected() {
        for (error, expected_kind, expected_status, expected_retryable) in [
            (
                TurnError::Unauthorized,
                ProductWorkflowRejectionKind::Unauthorized,
                403,
                false,
            ),
            (
                TurnError::ScopeNotFound,
                ProductWorkflowRejectionKind::ScopeNotFound,
                404,
                false,
            ),
            (
                TurnError::Unavailable {
                    reason: "turn store offline".to_string(),
                },
                ProductWorkflowRejectionKind::Unavailable,
                503,
                true,
            ),
        ] {
            let err: ProductAdapterError = ProductWorkflowError::TurnResumeDenied { error }.into();

            match err {
                ProductAdapterError::WorkflowRejected {
                    kind,
                    status_code,
                    retryable,
                    ..
                } => {
                    assert_eq!(kind, expected_kind);
                    assert_eq!(status_code, expected_status);
                    assert_eq!(retryable, expected_retryable);
                }
                other => panic!("expected typed workflow rejection, got {other:?}"),
            }
        }
    }
}
