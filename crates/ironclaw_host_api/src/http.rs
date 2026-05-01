//! Shared runtime HTTP egress contracts.
//!
//! Runtime lanes translate their native HTTP surfaces into these shapes and
//! delegate to one host-owned egress service. The service composes network
//! policy/transport with scoped secret leases; runtime crates must not perform
//! their own outbound HTTP, DNS, private-IP checks, or credential injection.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{CapabilityId, NetworkMethod, NetworkPolicy, ResourceScope, RuntimeKind, SecretHandle};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeHttpEgressRequest {
    pub runtime: RuntimeKind,
    pub scope: ResourceScope,
    pub capability_id: CapabilityId,
    pub method: NetworkMethod,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub network_policy: NetworkPolicy,
    /// Host-derived credential injection plan.
    ///
    /// This field is authority-bearing: runtime lanes and guest/plugin code
    /// must not invent it from untrusted input. Upstream capability/obligation
    /// composition is responsible for deriving it from declared credentials,
    /// authorization/approval, destination policy, and host-approved injection
    /// shape before this request reaches [`RuntimeHttpEgress`].
    pub credential_injections: Vec<RuntimeCredentialInjection>,
    pub response_body_limit: Option<u64>,
    /// Host-call timeout in milliseconds, already capped by the invoking
    /// runtime to its remaining execution deadline when applicable.
    pub timeout_ms: Option<u32>,
}

/// One host-approved credential injection.
///
/// The handle and target describe what the host has already authorized for this
/// runtime HTTP call. The egress service only leases, injects, redacts, and
/// enforces fail-closed required/optional behavior; it does not grant authority
/// to use arbitrary secrets by itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeCredentialInjection {
    pub handle: SecretHandle,
    #[serde(default)]
    pub source: RuntimeCredentialSource,
    pub target: RuntimeCredentialTarget,
    pub required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum RuntimeCredentialSource {
    /// Lease and consume material directly from the scoped secret store.
    ///
    /// This remains the compatibility path for host-derived credentials that
    /// are not backed by an already-satisfied authorization obligation.
    #[default]
    SecretStoreLease,
    /// Consume material staged by an `InjectSecretOnce` obligation handler.
    ///
    /// The host egress service must call `RuntimeSecretInjectionStore::take`
    /// with the request scope, this capability id, and the credential handle;
    /// it must not lease the same secret independently from the secret store.
    StagedObligation { capability_id: CapabilityId },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum RuntimeCredentialTarget {
    Header {
        name: String,
        prefix: Option<String>,
    },
    QueryParam {
        name: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeHttpEgressResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub request_bytes: u64,
    pub response_bytes: u64,
    pub redaction_applied: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RuntimeHttpEgressError {
    #[error("runtime HTTP credential error: {reason}")]
    Credential { reason: String },
    #[error("runtime HTTP request error: {reason}")]
    Request {
        reason: String,
        request_bytes: u64,
        response_bytes: u64,
    },
    #[error("runtime HTTP network error: {reason}")]
    Network {
        reason: String,
        request_bytes: u64,
        response_bytes: u64,
    },
    #[error("runtime HTTP response error: {reason}")]
    Response {
        reason: String,
        request_bytes: u64,
        response_bytes: u64,
    },
}

impl RuntimeHttpEgressError {
    pub fn request_bytes(&self) -> u64 {
        match self {
            Self::Credential { .. } => 0,
            Self::Request { request_bytes, .. }
            | Self::Network { request_bytes, .. }
            | Self::Response { request_bytes, .. } => *request_bytes,
        }
    }

    pub fn response_bytes(&self) -> u64 {
        match self {
            Self::Credential { .. } => 0,
            Self::Request { response_bytes, .. }
            | Self::Network { response_bytes, .. }
            | Self::Response { response_bytes, .. } => *response_bytes,
        }
    }

    /// Stable reason token safe to expose to runtime/plugin callers.
    pub fn stable_runtime_reason(&self) -> &'static str {
        match self {
            Self::Credential { .. } => "credential_unavailable",
            Self::Request { .. } => "request_denied",
            Self::Network { .. } => "network_error",
            Self::Response { .. } => "response_error",
        }
    }
}

pub fn is_sensitive_runtime_request_header(name: &str) -> bool {
    const SENSITIVE_REQUEST_HEADERS: &[&str] = &[
        "authorization",
        "proxy-authorization",
        "cookie",
        "x-api-key",
        "api-key",
        "x-auth-token",
        "x-token",
        "x-access-token",
        "x-session-token",
        "x-csrf-token",
        "x-secret",
        "x-api-secret",
    ];
    SENSITIVE_REQUEST_HEADERS
        .iter()
        .any(|header| name.trim().eq_ignore_ascii_case(header))
}

pub fn is_sensitive_runtime_response_header(name: &str) -> bool {
    const SENSITIVE_RESPONSE_HEADERS: &[&str] = &[
        "authorization",
        "www-authenticate",
        "set-cookie",
        "cookie",
        "x-api-key",
        "api-key",
        "x-auth-token",
        "x-token",
        "x-access-token",
        "x-session-token",
        "x-csrf-token",
        "x-secret",
        "x-api-secret",
        "proxy-authenticate",
        "proxy-authorization",
    ];
    const SENSITIVE_RESPONSE_HEADER_MARKERS: &[&str] = &[
        "auth",
        "token",
        "secret",
        "credential",
        "password",
        "cookie",
        "api-key",
        "apikey",
        "api_key",
    ];
    let normalized = name.trim().to_ascii_lowercase();
    SENSITIVE_RESPONSE_HEADERS
        .iter()
        .any(|header| normalized == *header)
        || SENSITIVE_RESPONSE_HEADER_MARKERS
            .iter()
            .any(|marker| normalized.contains(marker))
}

pub trait RuntimeHttpEgress: Send + Sync {
    fn execute(
        &self,
        request: RuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError>;
}

impl<T> RuntimeHttpEgress for std::sync::Arc<T>
where
    T: RuntimeHttpEgress + ?Sized,
{
    fn execute(
        &self,
        request: RuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        self.as_ref().execute(request)
    }
}
