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
    action::{NetworkMethod, NetworkPolicy},
    error::HostApiError,
    ids::{CapabilityId, SecretHandle},
    mount::MountGrant,
    path::ScopedPath,
    resource::ResourceScope,
    runtime::RuntimeKind,
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
    /// Insert the resolved secret as a JSON string at the RFC 6901 pointer in
    /// the request's JSON body (e.g. a vendor webhook-registration call whose
    /// API takes the shared secret as a body field). The host parses the
    /// body, inserts the value at the pointer, and re-serializes; a non-JSON
    /// body, a missing parent object, or an already-present field fails the
    /// request closed.
    BodyJsonPointer {
        pointer: String,
        /// Host-derived cap checked after every credential has been injected
        /// and the JSON body has been re-serialized. Callers that do not own a
        /// narrower request-body contract leave this unset.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        post_injection_body_limit_bytes: Option<u64>,
    },
    /// Compute and inject an RFC 8292 `Authorization: vapid t=<jwt>,k=<pub>`
    /// header for a Web Push request. The resolved secret material must be a
    /// serialized [`VapidCredentialMaterialV1`]; the host signs an ES256 JWT
    /// whose `aud` is the origin of the request URL being sent, so the token
    /// is valid only for the push service this request targets. Carries no
    /// declaration fields: everything request-dependent is derived host-side
    /// at the injection chokepoint, and the adapter never sees key bytes.
    VapidAuthorization,
}

/// The credential-material schema behind
/// [`RuntimeCredentialTarget::VapidAuthorization`] (schema `vapid.v1`): a
/// deployment's Web Push application-server identity per RFC 8292.
///
/// Stored as one JSON blob under the channel's VAPID credential handle.
/// `es256_private_key_pkcs8_b64url` is secret; the public key and subject
/// are not, but travel inside the same material so the injector needs one
/// resolution. Generation lives in `ironclaw_web_push`; parsing/signing at
/// the host egress credential boundary.
///
/// `Debug` is hand-written to redact the private key: this type is serialized
/// as a channel credential, and the safety boundary forbids raw secret
/// material in any debug output, log, event, or snapshot. A derived `Debug`
/// would render `es256_private_key_pkcs8_b64url` verbatim.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VapidCredentialMaterialV1 {
    /// PKCS#8 P-256 private key, base64url (unpadded).
    pub es256_private_key_pkcs8_b64url: String,
    /// Uncompressed P-256 public key (65 bytes), base64url (unpadded) — the
    /// browser-facing `applicationServerKey` and the `k=` parameter.
    pub public_key_b64url: String,
    /// RFC 8292 `sub` claim: a `mailto:` or `https:` operator contact URI.
    pub subject: String,
}

impl std::fmt::Debug for VapidCredentialMaterialV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("VapidCredentialMaterialV1")
            .field("es256_private_key_pkcs8_b64url", &"<redacted>")
            .field("public_key_b64url", &self.public_key_b64url)
            .field("subject", &self.subject)
            .finish()
    }
}

impl VapidCredentialMaterialV1 {
    /// Validate the material's cryptographic shape: the private key must be a
    /// parseable PKCS#8 P-256 signing key and the public key a 65-byte
    /// uncompressed point whose base64url decodes cleanly. `deny_unknown_fields`
    /// only rejects extra JSON keys; this rejects a structurally corrupt blob
    /// so a bad persisted credential fails at composition rather than surfacing
    /// later as a push-service rejection on every delivery. Kept dependency-free
    /// (length/prefix + base64url checks only) so the contracts crate stays a
    /// leaf; full keypair parsing happens at the signing boundary.
    pub fn validate_shape(&self) -> Result<(), HostApiError> {
        let public_key = decode_b64url_no_pad(&self.public_key_b64url).ok_or_else(|| {
            HostApiError::invalid_runtime_credential_target(
                "vapid_public_key",
                "must be base64url (unpadded)",
            )
        })?;
        if public_key.len() != 65 || public_key.first() != Some(&0x04) {
            return Err(HostApiError::invalid_runtime_credential_target(
                "vapid_public_key",
                "must decode to a 65-byte uncompressed P-256 point",
            ));
        }
        if decode_b64url_no_pad(&self.es256_private_key_pkcs8_b64url).is_none() {
            return Err(HostApiError::invalid_runtime_credential_target(
                "vapid_private_key",
                "must be base64url (unpadded)",
            ));
        }
        let subject_ok = (self.subject.starts_with("mailto:")
            && self.subject.len() > "mailto:".len())
            || (self.subject.starts_with("https://") && self.subject.len() > "https://".len());
        if !subject_ok || self.subject.len() > 256 || self.subject.chars().any(char::is_control) {
            return Err(HostApiError::invalid_runtime_credential_target(
                "vapid_subject",
                "must be a short control-free mailto: or https: URI",
            ));
        }
        Ok(())
    }
}

/// Minimal unpadded-base64url decode, dependency-free so this stays in the
/// contracts leaf. Accepts the RFC 4648 URL-safe alphabet without padding.
fn decode_b64url_no_pad(value: &str) -> Option<Vec<u8>> {
    fn sextet(byte: u8) -> Option<u8> {
        match byte {
            b'A'..=b'Z' => Some(byte - b'A'),
            b'a'..=b'z' => Some(byte - b'a' + 26),
            b'0'..=b'9' => Some(byte - b'0' + 52),
            b'-' => Some(62),
            b'_' => Some(63),
            _ => None,
        }
    }
    let bytes = value.as_bytes();
    if bytes.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3 + 2);
    for chunk in bytes.chunks(4) {
        let mut acc = 0u32;
        for &byte in chunk {
            acc = (acc << 6) | u32::from(sextet(byte)?);
        }
        let pad = 4 - chunk.len();
        acc <<= 6 * pad as u32;
        let take = 3 - pad;
        for index in 0..take {
            out.push((acc >> (16 - 8 * index)) as u8);
        }
    }
    Some(out)
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
            Self::BodyJsonPointer { pointer, .. } => {
                validate_runtime_credential_body_pointer(pointer)?;
            }
            // Field-free: audience, expiry, and header value are derived
            // host-side from the request being sent and the resolved
            // material; there is nothing declared to validate.
            Self::VapidAuthorization => {}
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

fn validate_runtime_credential_body_pointer(pointer: &str) -> Result<(), HostApiError> {
    if pointer.is_empty()
        || !pointer.starts_with('/')
        || pointer.contains('\0')
        || pointer.chars().any(char::is_control)
    {
        return Err(HostApiError::invalid_runtime_credential_target(
            "body_json_pointer",
            "must be a non-empty RFC 6901 pointer starting with '/' without control characters",
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

    /// Egress for a **host OAuth token exchange** — an OAuth/OIDC token
    /// endpoint such as Slack `oauth.v2.access`. This entry point is reserved
    /// for the host auth system's OAuth provider client; no tool, plugin, or
    /// general runtime caller may use it.
    ///
    /// Unlike [`RuntimeHttpEgress::execute`], the response is intentionally
    /// **not** leak-sanitized. A token-endpoint response legitimately carries
    /// credential material (e.g. `xoxp-`/`xoxb-` tokens) that the host auth
    /// system consumes directly — parsed and re-stored as a secret handle — and
    /// never surfaces to the model. Running the response sanitizer here would
    /// redact or hard-block the very token this call exists to retrieve.
    /// Request-side leak validation and host credential injection still apply,
    /// exactly as on [`RuntimeHttpEgress::execute`]; only the response
    /// sanitizer is bypassed, and only because such requests carry no injected
    /// credentials to redact.
    ///
    /// The default forwards to [`RuntimeHttpEgress::execute`], which is correct
    /// for implementations that never sanitize responses (e.g. test fakes).
    /// The production host egress service overrides this to run the transport
    /// pipeline with the response sanitizer skipped.
    async fn execute_credential_exchange(
        &self,
        request: RuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        self.execute(request).await
    }
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

    async fn execute_credential_exchange(
        &self,
        request: RuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        self.as_ref().execute_credential_exchange(request).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{InvocationId, UserId};

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
