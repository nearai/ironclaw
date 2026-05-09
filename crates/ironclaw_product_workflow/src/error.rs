//! Workflow-layer error vocabulary.
//!
//! [`ProductWorkflowError`] is the internal error type used within the workflow
//! crate. It converts to [`ProductAdapterError`] at the facade boundary so
//! adapters never see host-layer details.

use ironclaw_product_adapters::{ProductAdapterError, RedactedString};
use thiserror::Error;

/// Internal error type for the product workflow facade.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProductWorkflowError {
    /// The conversation binding could not be resolved for the given external refs.
    #[error("binding resolution failed: {reason}")]
    BindingResolutionFailed { reason: String },

    /// Turn coordinator rejected the submission.
    #[error("turn submission rejected: {reason}")]
    TurnSubmissionRejected { reason: String },

    /// Turn coordinator resume rejected.
    #[error("turn resume rejected: {reason}")]
    TurnResumeRejected { reason: String },

    /// A transient store or service failure.
    #[error("transient workflow failure: {reason}")]
    Transient { reason: String },

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

impl From<ProductWorkflowError> for ProductAdapterError {
    fn from(value: ProductWorkflowError) -> Self {
        match value {
            ProductWorkflowError::BindingResolutionFailed { reason } => {
                ProductAdapterError::Internal {
                    detail: RedactedString::new(reason),
                }
            }
            ProductWorkflowError::TurnSubmissionRejected { reason } => {
                ProductAdapterError::Internal {
                    detail: RedactedString::new(reason),
                }
            }
            ProductWorkflowError::TurnResumeRejected { reason } => ProductAdapterError::Internal {
                detail: RedactedString::new(reason),
            },
            ProductWorkflowError::Transient { reason } => ProductAdapterError::WorkflowTransient {
                reason: RedactedString::new(reason),
            },
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
}
