use std::fmt;

use ironclaw_auth::{AuthFlowId, AuthProductError, OAuthClientId, OAuthRedirectUri, ProviderScope};
use ironclaw_common::hashing::sha256_hex;
use ironclaw_host_api::SecretHandle;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

use crate::oauth_provider_client::{HostOAuthProviderSpec, OAuthClientMaterial};

#[derive(Debug, Deserialize)]
pub(super) struct ProtectedResourceMetadata {
    pub(super) authorization_servers: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct AuthorizationServerMetadata {
    pub(super) authorization_endpoint: String,
    pub(super) token_endpoint: String,
    pub(super) registration_endpoint: String,
}

#[derive(Debug, Serialize)]
pub(super) struct DcrRegistrationRequest<'a> {
    pub(super) client_name: &'a str,
    pub(super) redirect_uris: Vec<&'a str>,
    pub(super) grant_types: Vec<&'a str>,
    pub(super) response_types: Vec<&'a str>,
    pub(super) token_endpoint_auth_method: &'a str,
}

#[derive(Debug, Deserialize)]
pub(super) struct DcrRegistrationResponse {
    pub(super) client_id: String,
    #[serde(default)]
    pub(super) client_secret: Option<SecretString>,
}

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct StoredDcrClientMaterial {
    pub(super) client_id: String,
    #[serde(default)]
    pub(super) client_secret: Option<String>,
    pub(super) redirect_uri: String,
    pub(super) token_endpoint: String,
}

impl fmt::Debug for StoredDcrClientMaterial {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StoredDcrClientMaterial")
            .field("client_id", &self.client_id)
            .field(
                "client_secret",
                &self.client_secret.as_ref().map(|_| "[REDACTED]"),
            )
            .field("redirect_uri", &self.redirect_uri)
            .field("token_endpoint", &self.token_endpoint)
            .finish()
    }
}

impl StoredDcrClientMaterial {
    pub(super) fn into_client_material(self) -> Result<OAuthClientMaterial, AuthProductError> {
        Ok(OAuthClientMaterial {
            client_id: OAuthClientId::new(self.client_id)?,
            client_secret: self.client_secret.map(SecretString::from),
            redirect_uri: OAuthRedirectUri::new(self.redirect_uri)?,
            token_endpoint: self.token_endpoint,
        })
    }
}

pub(super) fn protected_resource_metadata_url(resource: &str) -> Result<String, AuthProductError> {
    let resource = url::Url::parse(resource).map_err(|_| AuthProductError::BackendUnavailable)?;
    if resource.scheme() != "https" {
        return Err(AuthProductError::BackendUnavailable);
    }
    let mut metadata = resource.clone();
    let resource_path = resource.path().trim_end_matches('/');
    metadata.set_path(&format!(
        "{resource_path}/.well-known/oauth-protected-resource"
    ));
    metadata.set_query(None);
    metadata.set_fragment(None);
    Ok(metadata.to_string())
}

pub(super) fn authorization_server_metadata_url(
    resource: &str,
) -> Result<String, AuthProductError> {
    let resource = url::Url::parse(resource).map_err(|_| AuthProductError::BackendUnavailable)?;
    authorization_server_metadata_url_from_issuer(resource.origin().ascii_serialization().as_str())
}

pub(super) fn authorization_server_metadata_url_from_issuer(
    issuer: &str,
) -> Result<String, AuthProductError> {
    let mut metadata = url::Url::parse(issuer).map_err(|_| AuthProductError::BackendUnavailable)?;
    if metadata.scheme() != "https" {
        return Err(AuthProductError::BackendUnavailable);
    }
    metadata.set_path("/.well-known/oauth-authorization-server");
    metadata.set_query(None);
    metadata.set_fragment(None);
    Ok(metadata.to_string())
}

pub(super) fn callback_base_url(
    origin: &str,
    flow_id: AuthFlowId,
) -> Result<url::Url, AuthProductError> {
    let origin = origin.trim_end_matches('/');
    let callback = format!("{origin}/api/reborn/product-auth/oauth/callback/{flow_id}");
    url::Url::parse(&callback).map_err(|_| AuthProductError::BackendUnavailable)
}

pub(super) fn validate_callback_origin(origin: &str) -> Result<(), AuthProductError> {
    let parsed = url::Url::parse(origin).map_err(|_| AuthProductError::BackendUnavailable)?;
    let is_loopback_http = parsed.scheme() == "http"
        && parsed
            .host_str()
            .is_some_and(|host| matches!(host, "localhost" | "127.0.0.1" | "[::1]"));
    if parsed.scheme() != "https" && !is_loopback_http {
        return Err(AuthProductError::BackendUnavailable);
    }
    if parsed.path() != "/" || parsed.query().is_some() || parsed.fragment().is_some() {
        return Err(AuthProductError::BackendUnavailable);
    }
    Ok(())
}

pub(super) fn flow_secret_handle(
    spec: &HostOAuthProviderSpec,
    flow_id: AuthFlowId,
    kind: &'static str,
) -> Result<SecretHandle, AuthProductError> {
    SecretHandle::new(format!(
        "{}-oauth-dcr-flow-{kind}-{flow_id}",
        spec.secret_handle_prefix
    ))
    .map_err(|_| AuthProductError::BackendUnavailable)
}

pub(super) fn refresh_secret_handle(
    spec: &HostOAuthProviderSpec,
    refresh_secret: &SecretHandle,
) -> Result<SecretHandle, AuthProductError> {
    SecretHandle::new(format!(
        "{}-oauth-dcr-refresh-client-{}",
        spec.secret_handle_prefix,
        sha256_hex(refresh_secret.as_str().as_bytes())
    ))
    .map_err(|_| AuthProductError::BackendUnavailable)
}

pub(super) fn scope_text(scopes: &[ProviderScope]) -> String {
    scopes
        .iter()
        .map(ProviderScope::as_str)
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protected_resource_metadata_url_preserves_resource_path() {
        assert_eq!(
            protected_resource_metadata_url("https://mcp.example.com/mcp").unwrap(),
            "https://mcp.example.com/mcp/.well-known/oauth-protected-resource"
        );
    }

    #[test]
    fn stored_dcr_client_material_debug_redacts_client_secret() {
        let material = StoredDcrClientMaterial {
            client_id: "client".to_string(),
            client_secret: Some("super-secret".to_string()),
            redirect_uri: "https://app.example/callback".to_string(),
            token_endpoint: "https://issuer.example/token".to_string(),
        };

        let debug = format!("{material:?}");

        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("super-secret"));
    }
}
