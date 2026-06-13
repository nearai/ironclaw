//! Product-adapter error vocabulary.

use thiserror::Error;

use crate::ProtocolAuthFailure;
use crate::egress::ProtocolHttpEgressError;
use crate::redaction::RedactedString;

/// Stable workflow rejection category exposed to product adapters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProductWorkflowRejectionKind {
    ThreadBusy,
    AdmissionRejected,
    ScopeNotFound,
    Unauthorized,
    InvalidRequest,
    Unavailable,
    Conflict,
    /// A bare approval/auth reply matched more than one live pending gate;
    /// the caller must disambiguate with an explicit gate reference.
    Ambiguous,
}

/// Public error surface for product adapters and the workflow facade.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProductAdapterError {
    #[error("invalid {kind} identifier: {reason}")]
    InvalidIdentifier { kind: &'static str, reason: String },

    #[error("inbound payload is malformed: {reason}")]
    MalformedInboundPayload { reason: RedactedString },

    #[error("protocol authentication failed: {0}")]
    Authentication(#[from] ProtocolAuthFailure),

    #[error("egress denied: {reason}")]
    EgressDenied { reason: RedactedString },

    #[error("egress to undeclared host {host}")]
    EgressUndeclaredHost { host: String },

    #[error("egress transient failure: {reason}")]
    EgressTransient { reason: RedactedString },

    #[error("workflow transient failure: {reason}")]
    WorkflowTransient { reason: RedactedString },

    #[error("workflow rejected request ({kind:?}, status {status_code}): {reason}")]
    WorkflowRejected {
        kind: ProductWorkflowRejectionKind,
        status_code: u16,
        retryable: bool,
        reason: RedactedString,
    },

    #[error("internal adapter error: {detail}")]
    Internal { detail: RedactedString },
}

impl ProductAdapterError {
    /// True when the protocol layer should surface a retryable response (5xx
    /// / 429 for webhooks). Used by host glue to map errors to status codes.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            ProductAdapterError::WorkflowTransient { .. }
                | ProductAdapterError::EgressTransient { .. }
                | ProductAdapterError::WorkflowRejected {
                    retryable: true,
                    ..
                }
        )
    }

    /// True when the failure should fail-closed at the protocol surface
    /// (401/403 for webhook auth).
    pub fn is_auth_failure(&self) -> bool {
        matches!(self, ProductAdapterError::Authentication(_))
    }
}

impl From<ProtocolHttpEgressError> for ProductAdapterError {
    fn from(value: ProtocolHttpEgressError) -> Self {
        match value {
            ProtocolHttpEgressError::Timeout | ProtocolHttpEgressError::Network(_) => {
                ProductAdapterError::EgressTransient {
                    reason: RedactedString::new(value.to_string()),
                }
            }
            ProtocolHttpEgressError::UndeclaredHost { host } => {
                ProductAdapterError::EgressUndeclaredHost { host }
            }
            ProtocolHttpEgressError::UnknownCredentialHandle { handle }
            | ProtocolHttpEgressError::UnauthorizedCredentialHandle { handle } => {
                ProductAdapterError::EgressDenied {
                    reason: RedactedString::new(handle),
                }
            }
            ProtocolHttpEgressError::PolicyDenied { reason } => {
                ProductAdapterError::EgressDenied { reason }
            }
            ProtocolHttpEgressError::LeakDetected => ProductAdapterError::EgressDenied {
                reason: RedactedString::new("egress response leak detector matched"),
            },
        }
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
            reason: RedactedString::new("store unavailable"),
        };
        assert!(err.is_retryable());
        assert!(!err.is_auth_failure());
    }

    #[test]
    fn egress_timeout_is_retryable() {
        let err: ProductAdapterError = ProtocolHttpEgressError::Timeout.into();
        assert!(err.is_retryable());
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
