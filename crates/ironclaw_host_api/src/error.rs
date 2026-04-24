use thiserror::Error;

/// Contract validation failures for host API value types.
///
/// Service crates should wrap this in their own error types for runtime
/// failures. This error is only about invalid contract values.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum HostApiError {
    #[error("invalid {kind} id '{value}': {reason}")]
    InvalidId {
        kind: &'static str,
        value: String,
        reason: String,
    },
    #[error("invalid path '{value}': {reason}")]
    InvalidPath { value: String, reason: String },
    #[error("invalid capability '{value}': {reason}")]
    InvalidCapability { value: String, reason: String },
    #[error("invalid mount '{value}': {reason}")]
    InvalidMount { value: String, reason: String },
    #[error("invalid network target '{value}': {reason}")]
    InvalidNetworkTarget { value: String, reason: String },
    #[error("host API invariant violation: {reason}")]
    InvariantViolation { reason: String },
}

impl HostApiError {
    pub(crate) fn invalid_id(
        kind: &'static str,
        value: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        Self::InvalidId {
            kind,
            value: value.into(),
            reason: reason.into(),
        }
    }

    pub(crate) fn invalid_path(value: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::InvalidPath {
            value: value.into(),
            reason: reason.into(),
        }
    }

    pub(crate) fn invalid_mount(value: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::InvalidMount {
            value: value.into(),
            reason: reason.into(),
        }
    }

    pub(crate) fn invariant(reason: impl Into<String>) -> Self {
        Self::InvariantViolation {
            reason: reason.into(),
        }
    }
}
