//! Constrained protocol HTTP egress and outbound delivery sink.
//!
//! Adapters reach external networks ONLY via [`ProtocolHttpEgress`]. The host
//! resolves credential handles at request time, scans the response for leaks
//! before returning to the adapter, and reports per-attempt delivery status
//! through [`OutboundDeliverySink`].

use std::collections::BTreeMap;

use async_trait::async_trait;
use ironclaw_turns::{ReplyTargetBindingRef, TurnRunId};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::error::ProductAdapterError;
use crate::redaction::RedactedString;

/// Stable name of an external host an adapter has declared in its manifest
/// (e.g. `api.telegram.org`). The host enforces that egress targets only a
/// declared host; undeclared hosts fail closed.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DeclaredEgressHost(String);

impl DeclaredEgressHost {
    pub fn new(value: impl Into<String>) -> Result<Self, ProductAdapterError> {
        let value = value.into();
        if value.is_empty() {
            return Err(ProductAdapterError::InvalidIdentifier {
                kind: "declared_egress_host",
                reason: "must not be empty".into(),
            });
        }
        if value.len() > 253 {
            // RFC 1035 hostname bound.
            return Err(ProductAdapterError::InvalidIdentifier {
                kind: "declared_egress_host",
                reason: "must be at most 253 bytes".into(),
            });
        }
        if value
            .chars()
            .any(|c| !(c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_'))
        {
            return Err(ProductAdapterError::InvalidIdentifier {
                kind: "declared_egress_host",
                reason: "must contain only ASCII alphanumeric, '.', '-' or '_'".into(),
            });
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Opaque host-side credential handle. The adapter knows the handle id; the
/// raw secret is never exposed to the adapter.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EgressCredentialHandle(String);

impl EgressCredentialHandle {
    pub fn new(value: impl Into<String>) -> Result<Self, ProductAdapterError> {
        let value = value.into();
        if value.is_empty() {
            return Err(ProductAdapterError::InvalidIdentifier {
                kind: "egress_credential_handle",
                reason: "must not be empty".into(),
            });
        }
        if value.len() > 256 {
            return Err(ProductAdapterError::InvalidIdentifier {
                kind: "egress_credential_handle",
                reason: "must be at most 256 bytes".into(),
            });
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Outbound HTTP request. Body is bytes; the host enforces size and content
/// limits and applies the credential handle.
#[derive(Debug, Clone)]
pub struct EgressRequest {
    pub host: DeclaredEgressHost,
    pub method: String,
    pub path: String,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
    pub credential_handle: Option<EgressCredentialHandle>,
}

/// Response surface visible to the adapter. Body is already leak-scanned by
/// the host before this struct is constructed.
#[derive(Debug, Clone)]
pub struct EgressResponse {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
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
    PolicyDenied { reason: String },
    #[error("egress timed out")]
    Timeout,
    #[error("egress failed at network layer: {0}")]
    Network(RedactedString),
    #[error("egress response leak detector matched")]
    LeakDetected,
}

#[async_trait]
pub trait ProtocolHttpEgress: Send + Sync {
    async fn send(&self, request: EgressRequest)
    -> Result<EgressResponse, ProtocolHttpEgressError>;
}

/// Stable identifier for one delivery attempt. Sinks use this to dedupe.
pub type DeliveryAttemptId = Uuid;

/// Per-attempt delivery outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryStatus {
    Delivered {
        attempt_id: DeliveryAttemptId,
        target: ReplyTargetBindingRef,
        run_id: Option<TurnRunId>,
    },
    /// Failure that the host SHOULD retry (transient network, 5xx, 429).
    FailedRetryable {
        attempt_id: DeliveryAttemptId,
        target: ReplyTargetBindingRef,
        run_id: Option<TurnRunId>,
        reason: String,
    },
    /// Failure that should NOT be retried (binding revoked, permission
    /// denied, blocked-by-user).
    FailedUnauthorized {
        attempt_id: DeliveryAttemptId,
        target: ReplyTargetBindingRef,
        run_id: Option<TurnRunId>,
        reason: String,
    },
    /// Adapter chose to defer — for example, when the canonical thread access
    /// policy revoked the binding mid-flight.
    Deferred {
        attempt_id: DeliveryAttemptId,
        target: ReplyTargetBindingRef,
        run_id: Option<TurnRunId>,
        reason: String,
    },
}

impl DeliveryStatus {
    pub fn attempt_id(&self) -> DeliveryAttemptId {
        match self {
            Self::Delivered { attempt_id, .. }
            | Self::FailedRetryable { attempt_id, .. }
            | Self::FailedUnauthorized { attempt_id, .. }
            | Self::Deferred { attempt_id, .. } => *attempt_id,
        }
    }
}

#[async_trait]
pub trait OutboundDeliverySink: Send + Sync {
    async fn record(&self, status: DeliveryStatus);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declared_host_rejects_invalid_chars() {
        assert!(DeclaredEgressHost::new("example.com/path").is_err());
        assert!(DeclaredEgressHost::new("ex ample.com").is_err());
        assert!(DeclaredEgressHost::new("ex@ample.com").is_err());
        assert!(DeclaredEgressHost::new("api.telegram.org").is_ok());
    }

    #[test]
    fn credential_handle_round_trips() {
        let h = EgressCredentialHandle::new("telegram_bot_token").expect("valid");
        let json = serde_json::to_string(&h).expect("serialize");
        let parsed: EgressCredentialHandle = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(h, parsed);
    }

    #[test]
    fn undeclared_host_error_renders_safely() {
        let err = ProtocolHttpEgressError::UndeclaredHost {
            host: "evil.example.com".into(),
        };
        let rendered = err.to_string();
        assert!(rendered.contains("evil.example.com"));
        assert!(!rendered.contains("token"));
    }

    #[test]
    fn network_error_does_not_leak_inner_string() {
        let err = ProtocolHttpEgressError::Network(RedactedString::new(
            "connection refused at 10.0.0.1:443 with token=secret",
        ));
        let rendered = err.to_string();
        assert!(!rendered.contains("10.0.0.1"));
        assert!(!rendered.contains("secret"));
    }
}
