//! Constrained protocol HTTP egress and outbound delivery sink.

use async_trait::async_trait;
use ironclaw_turns::{ReplyTargetBindingRef, TurnRunId};
use serde::{Deserialize, Deserializer, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::error::ProductAdapterError;
use crate::redaction::RedactedString;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
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

impl std::fmt::Display for DeclaredEgressHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for DeclaredEgressHost {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
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
        if value.chars().any(|c| c == '\0' || c.is_control()) {
            return Err(ProductAdapterError::InvalidIdentifier {
                kind: "egress_credential_handle",
                reason: "must not contain control characters".into(),
            });
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for EgressCredentialHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for EgressCredentialHandle {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct EgressMethod(String);

impl<'de> Deserialize<'de> for EgressMethod {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

impl EgressMethod {
    pub fn new(value: impl Into<String>) -> Result<Self, ProductAdapterError> {
        let value = value.into().to_ascii_uppercase();
        match value.as_str() {
            "GET" | "POST" | "PUT" | "PATCH" | "DELETE" => Ok(Self(value)),
            _ => Err(ProductAdapterError::InvalidIdentifier {
                kind: "egress_method",
                reason: "method is not allowed".into(),
            }),
        }
    }

    pub fn post() -> Self {
        Self("POST".into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct EgressPath(String);

impl EgressPath {
    pub fn new(value: impl Into<String>) -> Result<Self, ProductAdapterError> {
        let value = value.into();
        if !value.starts_with('/') || value.starts_with("//") {
            return Err(ProductAdapterError::InvalidIdentifier {
                kind: "egress_path",
                reason: "must be an origin-form path starting with a single '/'".into(),
            });
        }
        let lower = value.to_ascii_lowercase();
        if lower.contains("://")
            || value.contains('#')
            || value.contains('\\')
            || value.chars().any(|c| c == '\0' || c.is_control())
        {
            return Err(ProductAdapterError::InvalidIdentifier {
                kind: "egress_path",
                reason: "must not contain scheme, fragment, backslash, or control characters"
                    .into(),
            });
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for EgressPath {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EgressHeader {
    name: String,
    value: String,
}

impl EgressHeader {
    pub fn new(
        name: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<Self, ProductAdapterError> {
        let name = name.into();
        let value = value.into();
        validate_header_name(&name)?;
        if value.chars().any(|c| c == '\0' || c == '\r' || c == '\n') {
            return Err(ProductAdapterError::InvalidIdentifier {
                kind: "egress_header_value",
                reason: "must not contain NUL/CR/LF".into(),
            });
        }
        Ok(Self { name, value })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn value(&self) -> &str {
        &self.value
    }
}

#[derive(Deserialize)]
struct EgressHeaderWire {
    name: String,
    value: String,
}

impl<'de> Deserialize<'de> for EgressHeader {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = EgressHeaderWire::deserialize(deserializer)?;
        Self::new(wire.name, wire.value).map_err(serde::de::Error::custom)
    }
}

fn validate_header_name(name: &str) -> Result<(), ProductAdapterError> {
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
    {
        return Err(ProductAdapterError::InvalidIdentifier {
            kind: "egress_header_name",
            reason: "must be an ASCII token".into(),
        });
    }
    let lower = name.to_ascii_lowercase();
    // Headers the host owns. Two categories:
    //
    //   1. **Identity / proxy metadata** — `host` is set by the host
    //      from the declared egress target; auth and forwarding
    //      headers must be controlled by the host's credential
    //      injection and proxy posture, not adapter input.
    //
    //   2. **Hop-by-hop / transport-managed** — these govern the HTTP
    //      connection lifecycle and message framing (RFC 9110 §7.6.1).
    //      Letting adapters set them allows a component to influence
    //      the host's HTTP client semantics (chunked encoding,
    //      connection pinning, protocol upgrades), which is host
    //      policy. Per-message body length comes from the
    //      `EgressRequest` body itself — adapters do not get to lie
    //      about `content-length`.
    const FORBIDDEN: &[&str] = &[
        // Identity / proxy metadata.
        "host",
        "authorization",
        "proxy-authorization",
        "cookie",
        "set-cookie",
        "forwarded",
        "x-forwarded-for",
        "x-forwarded-host",
        "x-forwarded-proto",
        "x-real-ip",
        // Hop-by-hop and transport-managed (RFC 9110 §7.6.1).
        "connection",
        "content-length",
        "transfer-encoding",
        "te",
        "trailer",
        "upgrade",
        "keep-alive",
        "proxy-connection",
    ];
    if FORBIDDEN.contains(&lower.as_str()) {
        return Err(ProductAdapterError::InvalidIdentifier {
            kind: "egress_header_name",
            reason: "header is managed by the host".into(),
        });
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EgressRequest {
    host: DeclaredEgressHost,
    method: EgressMethod,
    path: EgressPath,
    headers: Vec<EgressHeader>,
    body: Vec<u8>,
    credential_handle: Option<EgressCredentialHandle>,
}

impl EgressRequest {
    pub fn new(host: DeclaredEgressHost, method: EgressMethod, path: EgressPath) -> Self {
        Self {
            host,
            method,
            path,
            headers: Vec::new(),
            body: Vec::new(),
            credential_handle: None,
        }
    }

    pub fn with_header(mut self, header: EgressHeader) -> Self {
        self.headers.push(header);
        self
    }

    pub fn with_body(mut self, body: Vec<u8>) -> Self {
        self.body = body;
        self
    }

    pub fn with_credential_handle(mut self, handle: Option<EgressCredentialHandle>) -> Self {
        self.credential_handle = handle;
        self
    }

    pub fn host(&self) -> &DeclaredEgressHost {
        &self.host
    }

    pub fn method(&self) -> &EgressMethod {
        &self.method
    }

    pub fn path(&self) -> &EgressPath {
        &self.path
    }

    pub fn headers(&self) -> &[EgressHeader] {
        &self.headers
    }

    pub fn body(&self) -> &[u8] {
        &self.body
    }

    pub fn credential_handle(&self) -> Option<&EgressCredentialHandle> {
        self.credential_handle.as_ref()
    }
}

/// Response surface visible to the adapter. Raw response headers are withheld;
/// the host may inspect them internally for leak detection and retry policy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EgressResponse {
    status: u16,
    body: Vec<u8>,
}

impl EgressResponse {
    pub fn new(status: u16, body: Vec<u8>) -> Self {
        Self { status, body }
    }

    pub fn status(&self) -> u16 {
        self.status
    }

    pub fn body(&self) -> &[u8] {
        &self.body
    }
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

#[async_trait]
pub trait ProtocolHttpEgress: Send + Sync {
    async fn send(&self, request: EgressRequest)
    -> Result<EgressResponse, ProtocolHttpEgressError>;
}

pub type DeliveryAttemptId = Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryStatus {
    Delivered {
        attempt_id: DeliveryAttemptId,
        target: ReplyTargetBindingRef,
        run_id: Option<TurnRunId>,
    },
    FailedRetryable {
        attempt_id: DeliveryAttemptId,
        target: ReplyTargetBindingRef,
        run_id: Option<TurnRunId>,
        reason: RedactedString,
    },
    FailedPermanent {
        attempt_id: DeliveryAttemptId,
        target: ReplyTargetBindingRef,
        run_id: Option<TurnRunId>,
        reason: RedactedString,
    },
    FailedUnauthorized {
        attempt_id: DeliveryAttemptId,
        target: ReplyTargetBindingRef,
        run_id: Option<TurnRunId>,
        reason: RedactedString,
    },
    Deferred {
        attempt_id: DeliveryAttemptId,
        target: ReplyTargetBindingRef,
        run_id: Option<TurnRunId>,
        reason: RedactedString,
    },
}

impl DeliveryStatus {
    pub fn attempt_id(&self) -> DeliveryAttemptId {
        match self {
            Self::Delivered { attempt_id, .. }
            | Self::FailedRetryable { attempt_id, .. }
            | Self::FailedPermanent { attempt_id, .. }
            | Self::FailedUnauthorized { attempt_id, .. }
            | Self::Deferred { attempt_id, .. } => *attempt_id,
        }
    }
}

#[async_trait]
pub trait OutboundDeliverySink: Send + Sync {
    async fn record(&self, status: DeliveryStatus);
}

/// Host-visible egress declaration advertised by an adapter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeclaredEgressTarget {
    pub host: DeclaredEgressHost,
    pub credential_handle: Option<EgressCredentialHandle>,
}

impl DeclaredEgressTarget {
    pub fn new(
        host: DeclaredEgressHost,
        credential_handle: Option<EgressCredentialHandle>,
    ) -> Self {
        Self {
            host,
            credential_handle,
        }
    }
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
    fn egress_request_rejects_unsafe_surface() {
        assert!(EgressMethod::new("CONNECT").is_err());
        assert!(EgressPath::new("//metadata").is_err());
        assert!(EgressHeader::new("Authorization", "secret").is_err());
    }

    #[test]
    fn egress_path_rejects_url_and_header_injection_shapes() {
        for path in [
            "http://metadata.local/latest",
            "https://api.example.com/send",
            "//metadata",
            "/path#fragment",
            "/redirect/http://metadata.local/latest",
            "/windows\\path",
            "/line\nbreak",
            "/carriage\rreturn",
            "/nul\0byte",
        ] {
            let res = EgressPath::new(path);
            assert!(
                res.is_err(),
                "{path:?} must not be accepted as an origin-form egress path"
            );
        }

        assert!(EgressPath::new("/v1/messages?chat_id=42").is_ok());
    }

    #[test]
    fn egress_header_rejects_host_identity_and_proxy_metadata_headers() {
        for header in [
            "host",
            "Host",
            "authorization",
            "proxy-authorization",
            "cookie",
            "set-cookie",
            "forwarded",
            "x-forwarded-for",
            "x-forwarded-host",
            "x-forwarded-proto",
            "x-real-ip",
        ] {
            let res = EgressHeader::new(header, "anything");
            assert!(
                res.is_err(),
                "{header} must be rejected because identity/proxy metadata is host-managed",
            );
        }
    }

    #[test]
    fn egress_header_rejects_invalid_header_name_syntax() {
        for header in ["", "Bad Header", "X:Bad", "X/Bad", "X.Bad", "X\r\nInjected"] {
            let res = EgressHeader::new(header, "anything");
            assert!(
                res.is_err(),
                "{header:?} must be rejected as an invalid header name token"
            );
        }
    }

    #[test]
    fn egress_header_rejects_hop_by_hop_and_transport_managed_headers() {
        // Per RFC 9110 §7.6.1 the host's HTTP client owns connection
        // lifecycle / message framing; adapters must not influence
        // them. Asserting per-name so a regression that drops one
        // entry from FORBIDDEN fails with a clear identification of
        // which header escaped. Each lookup is case-insensitive (the
        // validator lower-cases before matching) — exercise mixed
        // case to pin that too.
        for header in [
            "connection",
            "Connection",
            "content-length",
            "Content-Length",
            "transfer-encoding",
            "TE",
            "trailer",
            "upgrade",
            "Keep-Alive",
            "proxy-connection",
        ] {
            let res = EgressHeader::new(header, "anything");
            assert!(
                res.is_err(),
                "{header} must be rejected by validate_header_name as host-managed",
            );
        }
    }

    #[test]
    fn egress_header_accepts_application_headers() {
        // Defense against an over-eager FORBIDDEN list: regular
        // application headers must still pass. Pin a handful so a
        // future tightening doesn't silently break adapter egress.
        for header in [
            "x-request-id",
            "content-type",
            "accept",
            "user-agent",
            "x-custom-trace",
        ] {
            let res = EgressHeader::new(header, "value");
            assert!(
                res.is_ok(),
                "{header} is a normal application header and must be allowed",
            );
        }
    }

    #[test]
    fn credential_handle_round_trips() {
        let h = EgressCredentialHandle::new("telegram_bot_token").expect("valid");
        let json = serde_json::to_string(&h).expect("serialize");
        let parsed: EgressCredentialHandle = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(h, parsed);
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
