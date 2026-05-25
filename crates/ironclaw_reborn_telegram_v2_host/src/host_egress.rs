//! Host-mediated Telegram egress.
//!
//! This is the production [`ProtocolHttpEgress`] impl for the Telegram v2
//! adapter. Unlike the previous bespoke `TelegramHttpEgress` (which owned
//! a reqwest client + a credential resolver), this implementation
//! delegates every outbound HTTP call to [`RuntimeHttpEgress`] — the
//! host-api egress contract — so network policy, redaction, byte
//! accounting, and credential leasing all flow through one host-managed
//! pipeline rather than a per-adapter shim.
//!
//! ## Credential injection for path-embedded tokens
//!
//! Telegram's Bot API embeds the bot token in the URL path
//! (`https://api.telegram.org/bot<TOKEN>/sendMessage`). Standard host-api
//! injection variants
//! ([`RuntimeCredentialTarget::Header`][header],
//! [`RuntimeCredentialTarget::QueryParam`][query_param]) cannot express
//! that. This crate uses
//! [`RuntimeCredentialTarget::UrlPath`][url_path] — added in the same
//! audit pass that produced this shim — to let the host substitute the
//! placeholder one-shot from a [`SecretStore`] lease. The placeholder
//! string ([`URL_PATH_PLACEHOLDER`]) is constant and known to be absent
//! from any literal Telegram URL we emit, so substitution is
//! unambiguous.
//!
//! [header]: ironclaw_host_api::RuntimeCredentialTarget::Header
//! [query_param]: ironclaw_host_api::RuntimeCredentialTarget::QueryParam
//! [url_path]: ironclaw_host_api::RuntimeCredentialTarget::UrlPath
//! [`SecretStore`]: ironclaw_secrets::SecretStore
//!
//! ## Async / sync bridging
//!
//! [`RuntimeHttpEgress::execute`] is synchronous. The adapter calls
//! [`ProtocolHttpEgress::send`] from async context (axum handler →
//! workflow → adapter). The bridge is [`tokio::task::spawn_blocking`] so
//! the sync executor (which itself blocks-on-async for `SecretStore`
//! leases) does not stall the async runtime.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::{
    CapabilityId, NetworkMethod, NetworkPolicy, NetworkTargetPattern, ResourceScope,
    RuntimeCredentialInjection, RuntimeCredentialSource, RuntimeCredentialTarget,
    RuntimeHttpEgress, RuntimeHttpEgressError, RuntimeHttpEgressReasonCode,
    RuntimeHttpEgressRequest, RuntimeKind, SecretHandle,
};
use ironclaw_product_adapters::{
    DeclaredEgressTarget, EgressRequest, EgressResponse, ProtocolHttpEgress,
    ProtocolHttpEgressError, RedactedString,
};

use crate::error::HostError;

/// Placeholder we embed in Telegram URLs for the bot token. The host
/// substitutes this exactly once via
/// [`RuntimeCredentialTarget::UrlPath`]. The value is a constant string
/// chosen to be impossible in any literal Telegram URL the adapter
/// emits (Telegram paths are `/bot<TOKEN>/sendMessage` etc. — no curly
/// braces).
const URL_PATH_PLACEHOLDER: &str = "{telegram_bot_token}";

/// Host-mediated [`ProtocolHttpEgress`] for the Telegram Bot API.
///
/// Construct with a real [`RuntimeHttpEgress`] (typically
/// `HostHttpEgressService<PolicyNetworkHttpEgress<ReqwestNetworkTransport>,
/// InMemorySecretStore>`), the static declared-host allowlist the adapter
/// ships, and the [`NetworkPolicy`] the host should apply to outbound
/// calls. The shim handles URL construction with a placeholder + credential
/// injection plan; the host's network/secret pipeline does the rest.
pub struct HostMediatedTelegramEgress<E> {
    egress: Arc<E>,
    declared: Vec<DeclaredEgressTarget>,
    /// Network policy applied to every outbound Telegram call. Built once
    /// at composition time from `telegram_declared_egress_hosts()` (only
    /// `api.telegram.org` is on the allowlist).
    network_policy: NetworkPolicy,
    /// Scope under which the secret store was seeded at boot.
    scope: ResourceScope,
    /// Capability id stamped on every emitted [`RuntimeHttpEgressRequest`].
    /// Tracer slice uses a fixed value; once approvals/capabilities land
    /// for product adapters this becomes per-request.
    capability_id: CapabilityId,
}

impl<E> HostMediatedTelegramEgress<E>
where
    E: RuntimeHttpEgress + 'static,
{
    pub fn new(
        egress: Arc<E>,
        declared: Vec<DeclaredEgressTarget>,
        scope: ResourceScope,
        capability_id: CapabilityId,
    ) -> Result<Self, HostError> {
        let network_policy = build_network_policy(&declared)?;
        Ok(Self {
            egress,
            declared,
            network_policy,
            scope,
            capability_id,
        })
    }
}

fn build_network_policy(declared: &[DeclaredEgressTarget]) -> Result<NetworkPolicy, HostError> {
    if declared.is_empty() {
        return Err(HostError::Startup(
            "telegram declared egress list is empty; refusing to build an open network policy"
                .to_string(),
        ));
    }
    let allowed_targets: Vec<NetworkTargetPattern> = declared
        .iter()
        .map(|target| NetworkTargetPattern {
            scheme: None,
            host_pattern: target.host.as_str().to_string(),
            port: None,
        })
        .collect();
    Ok(NetworkPolicy {
        allowed_targets,
        deny_private_ip_ranges: true,
        max_egress_bytes: None,
    })
}

#[async_trait]
impl<E> ProtocolHttpEgress for HostMediatedTelegramEgress<E>
where
    E: RuntimeHttpEgress + Send + Sync + 'static,
{
    async fn send(
        &self,
        request: EgressRequest,
    ) -> Result<EgressResponse, ProtocolHttpEgressError> {
        // 1. Declared-host allowlist. The host's network policy enforcer
        // will also reject undeclared hosts, but checking here lets us
        // surface the typed `UndeclaredHost` error the adapter contract
        // expects (the host-api egress errors collapse to opaque
        // `Network`/`Request`).
        let host_str = request.host().as_str();
        let matched = self
            .declared
            .iter()
            .find(|d| d.host.as_str() == host_str)
            .cloned()
            .ok_or_else(|| ProtocolHttpEgressError::UndeclaredHost {
                host: host_str.to_string(),
            })?;

        // 2. Pick a credential handle. The adapter may supply one
        // per-request; otherwise fall back to the declared default.
        let handle = request
            .credential_handle()
            .cloned()
            .or(matched.credential_handle.clone())
            .ok_or_else(|| ProtocolHttpEgressError::UnauthorizedCredentialHandle {
                handle: "(missing)".to_string(),
            })?;
        let secret_handle = SecretHandle::new(handle.as_str()).map_err(|_| {
            ProtocolHttpEgressError::UnknownCredentialHandle {
                handle: handle.as_str().to_string(),
            }
        })?;

        // 3. Construct URL with placeholder. The host substitutes
        // `URL_PATH_PLACEHOLDER` one-shot via `RuntimeCredentialTarget::UrlPath`.
        let path = request.path().as_str();
        let url = format!(
            "https://{host}/bot{token}{path}",
            host = host_str,
            token = URL_PATH_PLACEHOLDER,
            path = path,
        );

        // 4. Translate method + headers + body.
        let method = match request.method().as_str() {
            "POST" => NetworkMethod::Post,
            "GET" => NetworkMethod::Get,
            other => {
                return Err(ProtocolHttpEgressError::PolicyDenied {
                    reason: RedactedString::new(format!("unsupported method {other}")),
                });
            }
        };
        let headers: Vec<(String, String)> = request
            .headers()
            .iter()
            .map(|h| (h.name().to_string(), h.value().to_string()))
            .collect();

        // 5. Build the host-api request. The credential injection plan
        // names the placeholder we left in the URL; the host substitutes
        // it one-shot from a `SecretStore` lease and adds the substituted
        // value to its redaction-token set.
        let runtime_request = RuntimeHttpEgressRequest {
            runtime: RuntimeKind::Script,
            scope: self.scope.clone(),
            capability_id: self.capability_id.clone(),
            method,
            url,
            headers,
            body: request.body().to_vec(),
            network_policy: self.network_policy.clone(),
            credential_injections: vec![RuntimeCredentialInjection {
                handle: secret_handle,
                source: RuntimeCredentialSource::SecretStoreLease,
                target: RuntimeCredentialTarget::UrlPath {
                    placeholder: URL_PATH_PLACEHOLDER.to_string(),
                },
                required: true,
            }],
            response_body_limit: Some(4 * 1024 * 1024),
            timeout_ms: Some(30_000),
        };

        // 6. Cross the async ↔ sync boundary. `RuntimeHttpEgress::execute`
        // is sync (it internally blocks-on-async for `SecretStore`
        // leases); `ProtocolHttpEgress::send` is async. `spawn_blocking`
        // keeps the async runtime unblocked while the host executes the
        // call.
        let egress = Arc::clone(&self.egress);
        let response = tokio::task::spawn_blocking(move || egress.execute(runtime_request))
            .await
            .map_err(|_| {
                ProtocolHttpEgressError::Network(RedactedString::new(
                    "host egress task panicked".to_string(),
                ))
            })?
            .map_err(map_runtime_error)?;

        Ok(EgressResponse::new(response.status, response.body))
    }
}

/// Map a [`RuntimeHttpEgressError`] to the adapter-visible
/// [`ProtocolHttpEgressError`]. The host has already redacted credential
/// material from the rendered reason; this translation re-labels the
/// shape so the adapter retry classifier (`is_retryable` in
/// `ironclaw_product_adapters::error`) sees the right verdict.
///
/// Retry contract:
/// - `Network` → retryable (transport-layer failure, may succeed on retry).
/// - everything else → permanent `PolicyDenied`. Credential/Request/Response
///   failures (including leak-detector matches and response-body-limit
///   exceeded) are not made transient by retrying.
///
/// We deliberately surface only the runtime's `stable_runtime_reason()` —
/// a short, allow-listed string — into the adapter-visible reason, instead
/// of forwarding the raw `reason` text. This keeps host-internal diagnostic
/// strings out of audit logs and out of fields like `UnknownCredentialHandle.handle`
/// where they previously misled downstream consumers that treated the field
/// as a credential identity.
fn map_runtime_error(err: RuntimeHttpEgressError) -> ProtocolHttpEgressError {
    let stable = err.stable_runtime_reason();
    match err.reason_code() {
        RuntimeHttpEgressReasonCode::NetworkError => {
            ProtocolHttpEgressError::Network(RedactedString::new(stable.to_string()))
        }
        RuntimeHttpEgressReasonCode::CredentialUnavailable
        | RuntimeHttpEgressReasonCode::RequestDenied
        | RuntimeHttpEgressReasonCode::PolicyDenied
        | RuntimeHttpEgressReasonCode::ResponseError
        | RuntimeHttpEgressReasonCode::ResponseBodyLimitExceeded => {
            ProtocolHttpEgressError::PolicyDenied {
                reason: RedactedString::new(stable.to_string()),
            }
        }
    }
}
