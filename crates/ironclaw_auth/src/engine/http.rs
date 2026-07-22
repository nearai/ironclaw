//! Engine vendor-HTTP helpers: bounded, allowlisted requests through the
//! injected [`RuntimeHttpEgress`] port. Every request pins a network policy
//! to the endpoint's host, caps the response body, and never logs bodies.

use ironclaw_host_api::{
    CapabilityId, NetworkMethod, NetworkPolicy, NetworkScheme, NetworkTargetPattern, ResourceScope,
    RuntimeCredentialInjection, RuntimeHttpEgressRequest, RuntimeHttpEgressResponse, RuntimeKind,
};
use secrecy::{ExposeSecret, SecretString};

use crate::{AuthProductError, OAuthClientId};

use super::AuthEngine;

/// Default vendor-call timeout.
pub(super) const VENDOR_TIMEOUT_MS: u32 = 30_000;
/// Vendor response bodies are size-capped (AUTH-6): token/identity/probe
/// responses are small JSON documents.
pub(super) const VENDOR_RESPONSE_BODY_LIMIT: u64 = 32 * 1024;

/// Capability id engine egress runs under. One id for every vendor — the
/// vendor is data on the request, never a code path.
pub(super) const ENGINE_CAPABILITY_ID: &str = "ironclaw_auth.vendor_recipe";

pub(super) struct VendorHttpResponse {
    pub(super) status: u16,
    pub(super) body: Vec<u8>,
}

impl AuthEngine {
    /// POST to a vendor token-class endpoint (exchange, refresh, revoke).
    /// Uses the credential-exchange egress entry point: the response
    /// legitimately carries token material that the engine consumes directly
    /// and re-stores behind secret handles.
    pub(super) async fn execute_credential_post(
        &self,
        scope: &ResourceScope,
        url: &str,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    ) -> Result<VendorHttpResponse, AuthProductError> {
        let request = self.vendor_request(scope, NetworkMethod::Post, url, headers, body)?;
        let response = self
            .egress
            .execute_credential_exchange(request)
            .await
            .map_err(map_egress_error)?;
        Ok(sanitize_response(response))
    }

    /// GET a vendor JSON endpoint (identity endpoint, api-key probe, DCR
    /// metadata). Responses carry no token material.
    pub(super) async fn execute_vendor_get(
        &self,
        scope: &ResourceScope,
        url: &str,
        headers: Vec<(String, String)>,
    ) -> Result<VendorHttpResponse, AuthProductError> {
        let request = self.vendor_request(scope, NetworkMethod::Get, url, headers, Vec::new())?;
        let response = self
            .egress
            .execute_credential_exchange(request)
            .await
            .map_err(map_egress_error)?;
        Ok(sanitize_response(response))
    }

    /// POST a vendor JSON endpoint (DCR registration).
    pub(super) async fn execute_vendor_post_json(
        &self,
        scope: &ResourceScope,
        url: &str,
        body: Vec<u8>,
    ) -> Result<VendorHttpResponse, AuthProductError> {
        let headers = vec![
            ("content-type".to_string(), "application/json".to_string()),
            ("accept".to_string(), "application/json".to_string()),
        ];
        let request = self.vendor_request(scope, NetworkMethod::Post, url, headers, body)?;
        let response = self
            .egress
            .execute_credential_exchange(request)
            .await
            .map_err(map_egress_error)?;
        Ok(sanitize_response(response))
    }

    fn vendor_request(
        &self,
        scope: &ResourceScope,
        method: NetworkMethod,
        url: &str,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    ) -> Result<RuntimeHttpEgressRequest, AuthProductError> {
        let host = https_endpoint_host(url)?;
        let capability_id = CapabilityId::new(ENGINE_CAPABILITY_ID)
            .map_err(|_| AuthProductError::BackendUnavailable)?;
        Ok(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::System,
            scope: scope.clone(),
            capability_id,
            method,
            url: url.to_string(),
            headers,
            body,
            network_policy: vendor_network_policy(&host),
            credential_injections: Vec::<RuntimeCredentialInjection>::new(),
            response_body_limit: Some(VENDOR_RESPONSE_BODY_LIMIT),
            save_body_to: None,
            timeout_ms: Some(VENDOR_TIMEOUT_MS),
        })
    }
}

/// Cap the buffered body defensively even when the egress implementation
/// ignored `response_body_limit`.
fn sanitize_response(response: RuntimeHttpEgressResponse) -> VendorHttpResponse {
    let mut body = response.body;
    body.truncate(VENDOR_RESPONSE_BODY_LIMIT as usize);
    VendorHttpResponse {
        status: response.status,
        body,
    }
}

pub(super) fn vendor_network_policy(host: &str) -> NetworkPolicy {
    NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: Some(NetworkScheme::Https),
            host_pattern: host.to_string(),
            port: None,
        }],
        deny_private_ip_ranges: true,
        max_egress_bytes: Some(VENDOR_RESPONSE_BODY_LIMIT),
    }
}

/// Host of an `https` endpoint; anything else fails closed.
pub(super) fn https_endpoint_host(endpoint: &str) -> Result<String, AuthProductError> {
    let url = url::Url::parse(endpoint).map_err(|_| AuthProductError::MalformedConfig)?;
    if url.scheme() != "https" {
        return Err(AuthProductError::MalformedConfig);
    }
    url.host_str()
        .filter(|host| !host.trim().is_empty())
        .map(str::to_string)
        .ok_or(AuthProductError::MalformedConfig)
}

/// HTTP Basic client authentication header (RFC 6749 §2.3.1).
pub(super) fn basic_auth_header(
    client_id: &OAuthClientId,
    client_secret: Option<&SecretString>,
) -> (String, String) {
    use base64::Engine as _;
    let encode =
        |value: &str| url::form_urlencoded::byte_serialize(value.as_bytes()).collect::<String>();
    let credentials = format!(
        "{}:{}",
        encode(client_id.as_str()),
        encode(
            client_secret
                .map(|secret| secret.expose_secret())
                .unwrap_or_default()
        )
    );
    (
        "authorization".to_string(),
        format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(credentials)
        ),
    )
}

fn map_egress_error(error: ironclaw_host_api::RuntimeHttpEgressError) -> AuthProductError {
    tracing::debug!(
        egress_reason = error.stable_runtime_reason(),
        "auth engine vendor egress failed"
    );
    AuthProductError::BackendUnavailable
}

pub(super) fn map_secret_store_error(
    error: ironclaw_secrets::SecretStoreError,
) -> AuthProductError {
    tracing::debug!(
        secret_store_reason = error.stable_reason(),
        "auth engine secret store operation failed"
    );
    AuthProductError::BackendUnavailable
}

pub(super) fn map_refresh_secret_error(
    error: ironclaw_secrets::SecretStoreError,
) -> AuthProductError {
    if error.is_unknown_secret()
        || error.is_unknown_lease()
        || error.is_consumed()
        || error.is_revoked()
        || error.is_expired()
    {
        AuthProductError::RefreshFailed
    } else {
        map_secret_store_error(error)
    }
}
