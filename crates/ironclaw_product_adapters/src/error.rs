//! Product-adapter error vocabulary.
//!
//! Errors here are user-safe and structured. They never carry raw secrets,
//! host paths, raw provider/runtime internals, or backend diagnostics; if a
//! protocol layer surfaces such a string, the adapter must redact it before
//! returning a [`ProductAdapterError`].

use thiserror::Error;

use crate::ProtocolAuthFailure;
use crate::redaction::RedactedString;

/// Public error surface for product adapters and the workflow facade.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProductAdapterError {
    #[error("invalid {kind} identifier: {reason}")]
    InvalidIdentifier { kind: &'static str, reason: String },

    #[error("inbound payload is malformed: {reason}")]
    MalformedInboundPayload { reason: String },

    #[error("protocol authentication failed: {0}")]
    Authentication(#[from] ProtocolAuthFailure),

    #[error("egress denied: {reason}")]
    EgressDenied { reason: String },

    #[error("egress to undeclared host {host}")]
    EgressUndeclaredHost { host: String },

    #[error("workflow rejected inbound: {reason}")]
    WorkflowRejected { reason: String },

    #[error("workflow transient failure: {reason}")]
    WorkflowTransient { reason: String },

    #[error("internal adapter error: {detail}")]
    Internal { detail: RedactedString },
}

impl ProductAdapterError {
    /// True when the protocol layer should surface a retryable response (5xx
    /// / 429 for webhooks). Used by host glue to map errors to status codes.
    pub fn is_retryable(&self) -> bool {
        matches!(self, ProductAdapterError::WorkflowTransient { .. })
    }

    /// True when the failure should fail-closed at the protocol surface
    /// (401/403 for webhook auth).
    pub fn is_auth_failure(&self) -> bool {
        matches!(self, ProductAdapterError::Authentication(_))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_failure_classified() {
        let err = ProductAdapterError::Authentication(ProtocolAuthFailure::SignatureMismatch);
        assert!(err.is_auth_failure());
        assert!(!err.is_retryable());
    }

    #[test]
    fn transient_classified() {
        let err = ProductAdapterError::WorkflowTransient {
            reason: "store unavailable".into(),
        };
        assert!(err.is_retryable());
        assert!(!err.is_auth_failure());
    }

    #[test]
    fn internal_error_does_not_leak_detail_in_display() {
        let err = ProductAdapterError::Internal {
            detail: RedactedString::new("super-secret-token"),
        };
        let rendered = err.to_string();
        assert!(!rendered.contains("super-secret-token"));
    }
}
