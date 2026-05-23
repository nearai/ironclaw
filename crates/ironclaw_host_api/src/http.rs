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
            Self::Network { .. } => RuntimeHttpEgressReasonCode::NetworkError,
            Self::Response { .. } => RuntimeHttpEgressReasonCode::ResponseError,
        }
    }

    /// Stable reason token safe to expose to runtime/plugin callers.
    pub fn stable_runtime_reason(&self) -> &'static str {
        match self.reason_code() {
            RuntimeHttpEgressReasonCode::CredentialUnavailable => "credential_unavailable",
            RuntimeHttpEgressReasonCode::RequestDenied => "request_denied",
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
        "authorization",
        "authenticate",
        "authentication",
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_header_sensitivity_trims_and_ignores_case() {
        for header in [
            " authorization ",
            "PROXY-AUTHORIZATION",
            "cookie",
            "X-Api-Key",
            "api-key",
            "x-auth-token",
            "x-token",
            "x-access-token",
            "x-session-token",
            "x-csrf-token",
            "x-secret",
            "x-api-secret",
        ] {
            assert!(
                is_sensitive_runtime_request_header(header),
                "{header:?} should be treated as sensitive"
            );
        }

        assert!(!is_sensitive_runtime_request_header("x-request-id"));
        assert!(!is_sensitive_runtime_request_header("authorization-extra"));
        assert!(!is_sensitive_runtime_request_header(" "));
    }

    #[test]
    fn response_header_sensitivity_catches_auth_markers() {
        for header in [
            " WWW-Authenticate ",
            "set-cookie",
            "X-Refresh-Token",
            "x-service-credential-id",
            "x-password-reset",
            "x-authentication-token",
        ] {
            assert!(
                is_sensitive_runtime_response_header(header),
                "{header:?} should be treated as sensitive"
            );
        }

        assert!(!is_sensitive_runtime_response_header("content-type"));
        assert!(!is_sensitive_runtime_response_header("x-request-id"));
        assert!(!is_sensitive_runtime_response_header("x-author"));
        assert!(!is_sensitive_runtime_response_header("x-authored-by"));
    }

    #[test]
    fn runtime_http_errors_expose_only_stable_reason_tokens_and_byte_counts() {
        let errors = [
            RuntimeHttpEgressError::Credential {
                reason: "missing secret: real-name".to_string(),
            },
            RuntimeHttpEgressError::Request {
                reason: "denied".to_string(),
                request_bytes: 5,
                response_bytes: 0,
            },
            RuntimeHttpEgressError::Network {
                reason: "dns".to_string(),
                request_bytes: 6,
                response_bytes: 7,
            },
            RuntimeHttpEgressError::Response {
                reason: "limit".to_string(),
                request_bytes: 8,
                response_bytes: 9,
            },
        ];

        assert_eq!(errors[0].stable_runtime_reason(), "credential_unavailable");
        assert_eq!(errors[1].stable_runtime_reason(), "request_denied");
        assert_eq!(errors[2].stable_runtime_reason(), "network_error");
        assert_eq!(errors[3].stable_runtime_reason(), "response_error");
        assert_eq!(errors[0].request_bytes(), 0);
        assert_eq!(errors[0].response_bytes(), 0);
        assert_eq!(errors[1].request_bytes(), 5);
        assert_eq!(errors[1].response_bytes(), 0);
        assert_eq!(errors[2].request_bytes(), 6);
        assert_eq!(errors[2].response_bytes(), 7);
        assert_eq!(errors[3].request_bytes(), 8);
        assert_eq!(errors[3].response_bytes(), 9);
    }
}
