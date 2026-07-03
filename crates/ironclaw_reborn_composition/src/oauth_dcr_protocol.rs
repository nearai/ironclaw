use std::fmt;

use ironclaw_auth::{AuthFlowId, AuthProductError, OAuthClientId, OAuthRedirectUri, ProviderScope};
use ironclaw_common::hashing::sha256_hex;
use ironclaw_host_api::SecretHandle;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::oauth_provider_client::{HostOAuthProviderSpec, OAuthClientMaterial};

#[derive(Debug, Deserialize)]
pub(super) struct ProtectedResourceMetadata {
    pub(super) authorization_servers: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
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
    pub(super) registration_client_uri: Option<String>,
    #[serde(default)]
    pub(super) registration_access_token: Option<SecretString>,
}

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct StoredDcrClientMaterial {
    pub(super) client_id: String,
    #[serde(
        default,
        serialize_with = "serialize_optional_secret",
        deserialize_with = "deserialize_optional_secret"
    )]
    pub(super) client_secret: Option<SecretString>,
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
            client_secret: self.client_secret,
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

pub(super) fn validate_issuer_related_to_resource(
    resource: &str,
    issuer: &str,
) -> Result<(), AuthProductError> {
    let resource = url::Url::parse(resource).map_err(|_| AuthProductError::BackendUnavailable)?;
    let issuer = url::Url::parse(issuer).map_err(|_| AuthProductError::BackendUnavailable)?;
    let Some(resource_domain) = registrable_domain(&resource) else {
        return Err(AuthProductError::BackendUnavailable);
    };
    let Some(issuer_domain) = registrable_domain(&issuer) else {
        return Err(AuthProductError::BackendUnavailable);
    };
    if resource_domain != issuer_domain {
        return Err(AuthProductError::BackendUnavailable);
    }
    Ok(())
}

pub(super) fn validate_endpoint_origin(
    endpoint: &str,
    expected_origin_url: &str,
) -> Result<(), AuthProductError> {
    let endpoint = url::Url::parse(endpoint).map_err(|_| AuthProductError::BackendUnavailable)?;
    let expected =
        url::Url::parse(expected_origin_url).map_err(|_| AuthProductError::BackendUnavailable)?;
    if endpoint.origin() != expected.origin() {
        return Err(AuthProductError::BackendUnavailable);
    }
    Ok(())
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
            .is_some_and(|host| matches!(host, "localhost" | "127.0.0.1" | "::1" | "[::1]"));
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

fn registrable_domain(url: &url::Url) -> Option<String> {
    let host = url.host_str()?.trim_end_matches('.');
    let labels = host.split('.').collect::<Vec<_>>();
    if labels.len() < 2 {
        return None;
    }
    Some(labels[labels.len() - 2..].join("."))
}

fn serialize_optional_secret<S>(
    secret: &Option<SecretString>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    secret
        .as_ref()
        .map(|secret| secret.expose_secret())
        .serialize(serializer)
}

fn deserialize_optional_secret<'de, D>(deserializer: D) -> Result<Option<SecretString>, D::Error>
where
    D: Deserializer<'de>,
{
    Option::<String>::deserialize(deserializer).map(|secret| secret.map(SecretString::from))
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
            client_secret: Some(SecretString::from("super-secret".to_string())),
            redirect_uri: "https://app.example/callback".to_string(),
            token_endpoint: "https://issuer.example/token".to_string(),
        };

        let debug = format!("{material:?}");

        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("super-secret"));
    }

    #[test]
    fn authorization_server_issuer_must_match_resource_registrable_domain() {
        validate_issuer_related_to_resource(
            "https://mcp.notion.com/mcp",
            "https://oauth.notion.com",
        )
        .unwrap();

        assert!(matches!(
            validate_issuer_related_to_resource(
                "https://mcp.notion.com/mcp",
                "https://attacker.example"
            ),
            Err(AuthProductError::BackendUnavailable)
        ));
    }

    #[test]
    fn registration_endpoint_must_share_authorization_server_origin() {
        validate_endpoint_origin(
            "https://oauth.notion.com/register",
            "https://oauth.notion.com/.well-known/oauth-authorization-server",
        )
        .unwrap();

        assert!(matches!(
            validate_endpoint_origin(
                "https://attacker.example/register",
                "https://oauth.notion.com/.well-known/oauth-authorization-server",
            ),
            Err(AuthProductError::BackendUnavailable)
        ));
    }

    #[test]
    fn validate_callback_origin_rejects_non_loopback_http_and_non_root_path() {
        validate_callback_origin("http://127.0.0.1:3000").unwrap();
        validate_callback_origin("http://[::1]:3000").unwrap();
        validate_callback_origin("https://app.example").unwrap();

        assert!(matches!(
            validate_callback_origin("http://app.example"),
            Err(AuthProductError::BackendUnavailable)
        ));
        assert!(matches!(
            validate_callback_origin("https://app.example/callback"),
            Err(AuthProductError::BackendUnavailable)
        ));
        assert!(matches!(
            validate_callback_origin("https://app.example?x=1"),
            Err(AuthProductError::BackendUnavailable)
        ));
    }
}
