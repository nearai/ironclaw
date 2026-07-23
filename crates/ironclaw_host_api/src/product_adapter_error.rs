//! Shared product-adapter error and redaction vocabulary.

use serde::{Deserialize, Serialize, Serializer};
use std::fmt;
use thiserror::Error;

pub const REDACTED_PLACEHOLDER: &str = "<redacted>";

#[derive(Clone, PartialEq, Eq)]
pub struct RedactedString(String);

impl RedactedString {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn placeholder() -> &'static str {
        REDACTED_PLACEHOLDER
    }
}

impl fmt::Debug for RedactedString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(REDACTED_PLACEHOLDER)
    }
}

impl fmt::Display for RedactedString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(REDACTED_PLACEHOLDER)
    }
}

impl Serialize for RedactedString {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(REDACTED_PLACEHOLDER)
    }
}

impl<'de> Deserialize<'de> for RedactedString {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Ok(Self::new(value))
    }
}

pub trait RedactedDebug {
    fn debug_does_not_contain(&self, needle: &str) -> bool;
}

impl<T: fmt::Debug> RedactedDebug for T {
    fn debug_does_not_contain(&self, needle: &str) -> bool {
        !format!("{self:?}").contains(needle)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error, Serialize, Deserialize)]
pub enum ProtocolAuthFailure {
    #[error("missing authentication header or token")]
    Missing,
    #[error("authentication header present but malformed")]
    Malformed,
    #[error("signature did not match expected digest")]
    SignatureMismatch,
    #[error("token did not match expected shared secret")]
    SharedSecretMismatch,
    #[error("session was not authenticated or expired")]
    SessionUnauthenticated,
    #[error("bearer token did not match")]
    BearerTokenMismatch,
    #[error("authentication failed: {detail}")]
    Other { detail: RedactedString },
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProtocolHttpEgressError {
    #[error("egress to undeclared host {host}")]
    UndeclaredHost { host: String },
    #[error("egress credential handle {handle} is unknown")]
    UnknownCredentialHandle { handle: String },
    #[error("egress credential handle {handle} is unauthorized for this adapter")]
    UnauthorizedCredentialHandle { handle: String },
    #[error("egress denied by host policy: {reason}")]
    PolicyDenied { reason: RedactedString },
    #[error("egress timed out")]
    Timeout,
    #[error("egress failed at network layer: {0}")]
    Network(RedactedString),
    #[error("egress response leak detector matched")]
    LeakDetected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProductWorkflowRejectionKind {
    ThreadBusy,
    AdmissionRejected,
    ScopeNotFound,
    Unauthorized,
    InvalidRequest,
    Unavailable,
    Conflict,
    Ambiguous,
}

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
