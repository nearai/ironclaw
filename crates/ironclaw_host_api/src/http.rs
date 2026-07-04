//! Shared runtime HTTP egress contracts.
//!
//! Runtime lanes translate their native HTTP surfaces into these shapes and
//! delegate to one host-owned egress service. The service composes network
//! policy/transport with scoped secret leases; runtime crates must not perform
//! their own outbound HTTP, DNS, private-IP checks, or credential injection.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::{
    CapabilityId, HostApiError, MountGrant, NetworkMethod, NetworkPolicy, ResourceScope,
    RuntimeKind, ScopedPath, SecretHandle,
};

/// Runtime HTTP request accepted by the host-owned egress service.
///
/// URL and header values may contain host-injected credential material after
/// the service resolves approved credential injections. Those buffers are
/// zeroized when the request is dropped; transport code may still need
/// plaintext while dispatching the request.
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_body_limit: Option<u64>,
    /// Optional scoped destination for storing the sanitized response body.
    ///
    /// This is a scoped path, not a host path. Host composition must provide the
    /// body store that resolves the scoped destination through filesystem
    /// authority for the invocation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub save_body_to: Option<RuntimeHttpSaveTarget>,
    /// Host-call timeout in milliseconds, already capped by the invoking
    /// runtime to its remaining execution deadline when applicable.
    pub timeout_ms: Option<u32>,
}

impl Drop for RuntimeHttpEgressRequest {
    fn drop(&mut self) {
        self.scrub_sensitive_url_and_headers();
    }
}

impl RuntimeHttpEgressRequest {
    fn scrub_sensitive_url_and_headers(&mut self) {
        // Host credential injection currently writes secrets into URL components
        // and header values. Header names and body payloads are separate
        // caller-controlled data and need an explicit threat-model decision
        // before broadening this carrier scrub scope.
        self.url.zeroize();
        for (_, value) in &mut self.headers {
            value.zeroize();
        }
    }
}

impl ZeroizeOnDrop for RuntimeHttpEgressRequest {}

const _: fn(&RuntimeHttpEgressRequest) = |request| {
    fn require_zeroize_on_drop<T: ?Sized + ZeroizeOnDrop>(_: &T) {}
    require_zeroize_on_drop(request);
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeHttpSaveTarget {
    pub path: ScopedPath,
    /// Host-derived write authority for `path`.
    ///
    /// This is skipped on the wire so guest/runtime-provided requests cannot
    /// grant themselves filesystem authority by serializing a custom mount.
    /// Host translators that already resolved the destination may attach a
    /// narrowed single-path grant before dispatching to the host egress service.
    #[serde(skip)]
    pub mount_grant: Option<MountGrant>,
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
    PathPlaceholder {
        placeholder: String,
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
            Self::PathPlaceholder { placeholder } => {
                validate_runtime_credential_path_placeholder(placeholder)?;
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

fn validate_runtime_credential_path_placeholder(placeholder: &str) -> Result<(), HostApiError> {
    if placeholder.is_empty()
        || placeholder == "."
        || placeholder == ".."
        || !placeholder
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~'))
    {
        return Err(HostApiError::invalid_runtime_credential_target(
            "path_placeholder",
            "must be a non-empty unreserved path segment other than . or ..",
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub saved_body: Option<RuntimeHttpSavedBody>,
    pub request_bytes: u64,
    pub response_bytes: u64,
    pub redaction_applied: bool,
}

/// Runtime-lane host HTTP request shared by MCP, scripts, and other capability
/// hosts.
///
/// This is the pre-translation shape a runtime lane hands to its
/// `RuntimeHttpEgress` adapter; the adapter fills in `runtime`/`save_body_to`
/// when building the [`RuntimeHttpEgressRequest`]. Non-serde by design — an
/// in-process host-call value, not a wire/persistence type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityHostHttpRequest {
    pub scope: ResourceScope,
    pub capability_id: CapabilityId,
    pub method: NetworkMethod,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub network_policy: NetworkPolicy,
    /// Host-derived credential injection plan (authority-bearing).
    ///
    /// Same contract as [`RuntimeHttpEgressRequest::credential_injections`]:
    /// runtime lanes and guest/plugin code must not invent it from untrusted
    /// input — it is derived by upstream capability/obligation composition.
    pub credential_injections: Vec<RuntimeCredentialInjection>,
    pub response_body_limit: Option<u64>,
    pub timeout_ms: Option<u32>,
}

impl CapabilityHostHttpRequest {
    /// Translate into the runtime-lane [`RuntimeHttpEgressRequest`] for the given
    /// `runtime`. Single owner of this mapping so a field added to either type is
    /// updated in one place rather than in each capability host by hand.
    pub fn into_runtime_request(self, runtime: RuntimeKind) -> RuntimeHttpEgressRequest {
        RuntimeHttpEgressRequest {
            runtime,
            scope: self.scope,
            capability_id: self.capability_id,
            method: self.method,
            url: self.url,
            headers: self.headers,
            body: self.body,
            network_policy: self.network_policy,
            credential_injections: self.credential_injections,
            response_body_limit: self.response_body_limit,
            save_body_to: None,
            timeout_ms: self.timeout_ms,
        }
    }
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeHttpSavedBody {
    pub path: ScopedPath,
    pub bytes_written: u64,
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

#[async_trait]
pub trait RuntimeHttpEgress: Send + Sync {
    async fn execute(
        &self,
        request: RuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError>;
}

#[async_trait]
impl<T> RuntimeHttpEgress for std::sync::Arc<T>
where
    T: RuntimeHttpEgress + ?Sized,
{
    async fn execute(
        &self,
        request: RuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        self.as_ref().execute(request).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{InvocationId, UserId};

    #[test]
    fn runtime_http_egress_request_scrubs_url_and_header_values() {
        let mut request = RuntimeHttpEgressRequest {
            runtime: RuntimeKind::Script,
            scope: ResourceScope::local_default(UserId::new("user1").unwrap(), InvocationId::new())
                .unwrap(),
            capability_id: CapabilityId::new("runtime.http").unwrap(),
            method: NetworkMethod::Post,
            url: "https://api.example.test/v1?token=sk-query-secret".to_string(),
            headers: vec![(
                "authorization".to_string(),
                "Bearer sk-header-secret".to_string(),
            )],
            body: b"hello".to_vec(),
            network_policy: NetworkPolicy {
                allowed_targets: vec![],
                deny_private_ip_ranges: true,
                max_egress_bytes: Some(4096),
            },
            credential_injections: vec![],
            response_body_limit: Some(4096),
            save_body_to: None,
            timeout_ms: None,
        };

        request.scrub_sensitive_url_and_headers();

        assert!(request.url.is_empty());
        assert_eq!(request.headers[0].0, "authorization");
        assert!(request.headers[0].1.is_empty());
    }
}
