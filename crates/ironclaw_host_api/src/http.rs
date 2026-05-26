//! Shared runtime HTTP egress contracts.
//!
//! Runtime lanes translate their native HTTP surfaces into these shapes and
//! delegate to one host-owned egress service. The service composes network
//! policy/transport with scoped secret leases; runtime crates must not perform
//! their own outbound HTTP, DNS, private-IP checks, or credential injection.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    CapabilityId, HostApiError, NetworkMethod, NetworkPolicy, ResourceScope, RuntimeKind,
    SecretHandle,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeHttpEgressRequest {
    pub runtime: RuntimeKind,
    pub scope: ResourceScope,
    pub capability_id: CapabilityId,
    pub method: NetworkMethod,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// Request-carried fallback policy used only by legacy/test egress services.
    /// Production first-party dispatch stages network policy in the host service
    /// before this request is executed, so the field is ignored on that path.
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
    pub source: RuntimeCredentialSource,
    pub target: RuntimeCredentialTarget,
    pub required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum RuntimeCredentialSource {
    /// Lease and consume material directly from the scoped secret store.
    ///
    /// This is the legacy/test compatibility path for host-derived credentials
    /// that are not backed by an already-satisfied authorization obligation.
    /// Production runtime tool egress must use [`Self::StagedObligation`].
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

pub fn valid_http_field_name(name: &str) -> bool {
    !name.is_empty()
        && name.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b'!' | b'#'
                        | b'$'
                        | b'%'
                        | b'&'
                        | b'\''
                        | b'*'
                        | b'+'
                        | b'-'
                        | b'.'
                        | b'^'
                        | b'_'
                        | b'`'
                        | b'|'
                        | b'~'
                )
        })
}

impl RuntimeCredentialTarget {
    pub fn validate_declaration(&self) -> Result<(), HostApiError> {
        match self {
            Self::Header { name, prefix } => {
                validate_runtime_credential_header_name(name)?;
                if let Some(prefix) = prefix {
                    validate_runtime_credential_fragment_no_control(
                        "header_prefix",
                        prefix,
                        "must not contain NUL/control characters",
                    )?;
                }
            }
            Self::QueryParam { name } => {
                validate_runtime_credential_fragment_non_empty_no_control(
                    "query_param_name",
                    name,
                    "must not be empty or contain NUL/control characters",
                )?;
            }
        }
        Ok(())
    }
}

fn validate_runtime_credential_header_name(name: &str) -> Result<(), HostApiError> {
    if !valid_http_field_name(name) {
        return Err(HostApiError::invalid_runtime_credential_target(
            "header_name",
            "must be an ASCII HTTP field-name token",
        ));
    }
    Ok(())
}

fn validate_runtime_credential_fragment_non_empty_no_control(
    value_kind: &'static str,
    value: &str,
    reason: &'static str,
) -> Result<(), HostApiError> {
    if value.trim().is_empty() || value.contains('\0') || value.chars().any(char::is_control) {
        return Err(HostApiError::invalid_runtime_credential_target(
            value_kind, reason,
        ));
    }
    Ok(())
}

fn validate_runtime_credential_fragment_no_control(
    value_kind: &'static str,
    value: &str,
    reason: &'static str,
) -> Result<(), HostApiError> {
    if value.contains('\0') || value.chars().any(char::is_control) {
        return Err(HostApiError::invalid_runtime_credential_target(
            value_kind, reason,
        ));
    }
    Ok(())
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

pub const RUNTIME_HTTP_REASON_RESPONSE_BODY_LIMIT_EXCEEDED: &str = "response_body_limit_exceeded";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeHttpEgressReasonCode {
    CredentialUnavailable,
    RequestDenied,
    PolicyDenied,
    NetworkError,
    ResponseError,
    ResponseBodyLimitExceeded,
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

    pub fn reason_code(&self) -> RuntimeHttpEgressReasonCode {
        match self {
            Self::Credential { .. } => RuntimeHttpEgressReasonCode::CredentialUnavailable,
            Self::Request { .. } => RuntimeHttpEgressReasonCode::RequestDenied,
            Self::Network { reason, .. } | Self::Response { reason, .. }
                if reason == RUNTIME_HTTP_REASON_RESPONSE_BODY_LIMIT_EXCEEDED =>
            {
                RuntimeHttpEgressReasonCode::ResponseBodyLimitExceeded
            }
            Self::Network { reason, .. } if reason == "policy_denied" => {
                RuntimeHttpEgressReasonCode::PolicyDenied
            }
            Self::Network { .. } => RuntimeHttpEgressReasonCode::NetworkError,
            Self::Response { .. } => RuntimeHttpEgressReasonCode::ResponseError,
        }
    }

    /// Stable reason token safe to expose to runtime/plugin callers.
    pub fn stable_runtime_reason(&self) -> &'static str {
        match self.reason_code() {
            RuntimeHttpEgressReasonCode::CredentialUnavailable => "credential_unavailable",
            RuntimeHttpEgressReasonCode::RequestDenied => "request_denied",
            RuntimeHttpEgressReasonCode::PolicyDenied => "policy_denied",
            RuntimeHttpEgressReasonCode::NetworkError => "network_error",
            RuntimeHttpEgressReasonCode::ResponseError => "response_error",
            RuntimeHttpEgressReasonCode::ResponseBodyLimitExceeded => {
                RUNTIME_HTTP_REASON_RESPONSE_BODY_LIMIT_EXCEEDED
            }
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
