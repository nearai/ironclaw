//! Host API validation errors.
//!
//! [`HostApiError`] reports invalid contract values: malformed identifiers,
//! paths, mounts, network targets, and invariant violations. It is deliberately
//! not a service/runtime error type. Filesystem, resources, auth, network, and
//! runtime crates should wrap these errors when validation failures surface
//! through their APIs.

use serde::{Deserialize, Serialize};
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

/// Host-safe, redacted error classification string.
///
/// `ErrorKind` intentionally accepts only short symbolic values suitable for
/// current state records and events. Detail-like strings are collapsed to
/// `Unclassified` so host paths, secrets, stderr, and guest messages do not leak
/// through control-plane status fields.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ErrorKind(String);

impl ErrorKind {
    pub fn new(value: impl Into<String>) -> Self {
        let value = value.into();
        let is_safe = !value.is_empty()
            && value.len() <= 128
            && value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':')
            });
        if is_safe {
            Self(value)
        } else {
            Self("Unclassified".to_string())
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<String> for ErrorKind {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl From<&str> for ErrorKind {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
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

    pub(crate) fn invalid_network_target(
        value: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        Self::InvalidNetworkTarget {
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
