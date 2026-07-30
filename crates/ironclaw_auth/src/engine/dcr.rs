//! RFC 7591 dynamic client registration — implemented ONCE, as generic
//! hosted-MCP behavior. A recipe without `client_credentials` declares that
//! the vendor's client is discovered and registered dynamically:
//!
//! 1. resolve the authorization server from RFC 9728 protected-resource
//!    metadata for the declared canonical resource,
//! 2. fetch RFC 8414 authorization-server metadata (authorize/token/
//!    registration endpoints; the recipe's endpoints are static placeholders),
//! 3. register a client with the static vendor callback as its redirect URI,
//! 4. persist the registered client and reuse it for every later flow.

use ironclaw_host_api::{HttpsEndpoint, ResourceScope, SecretHandle};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};

use crate::{
    AuthProductError, AuthorizationServerAdmissionMetadata, OAuthClientId,
    ProtectedResourceAdmissionMetadata,
};

use super::exchange::EffectiveOAuthClient;
use super::{AuthEngine, http};

/// Prefix of the per-vendor persisted registered-client handle.
pub const DCR_CLIENT_HANDLE_PREFIX: &str = "oauth-dcr-client";

#[derive(Debug, Serialize)]
struct DcrRegistrationRequest<'a> {
    client_name: &'a str,
    redirect_uris: Vec<&'a str>,
    grant_types: Vec<&'a str>,
    response_types: Vec<&'a str>,
    token_endpoint_auth_method: &'a str,
}

#[derive(Debug, Deserialize)]
struct DcrRegistrationResponse {
    client_id: String,
    #[serde(default)]
    client_secret: Option<String>,
}

/// The persisted registered client (stored as JSON secret material under the
/// per-vendor handle).
#[derive(Serialize, Deserialize)]
struct StoredDcrClient {
    client_id: String,
    #[serde(default)]
    client_secret: Option<String>,
    authorization_endpoint: String,
    token_endpoint: String,
    redirect_uri: String,
}

impl AuthEngine {
    /// Resolve (and on flow preparation, register) the dynamic client for a
    /// vendor whose recipe carries no deployment client credentials.
    pub(super) async fn dcr_client(
        &self,
        scope: &ResourceScope,
        vendor: &str,
        resource: Option<&str>,
        admitted_protected_resource_metadata_url: Option<&HttpsEndpoint>,
        register_if_missing: bool,
    ) -> Result<EffectiveOAuthClient, AuthProductError> {
        if let Some(stored) = self.load_dcr_client(scope, vendor).await? {
            return stored_to_effective(stored);
        }
        if !register_if_missing {
            // Exchange/refresh must never register: the client had to exist
            // before the vendor could redirect back.
            return Err(AuthProductError::MalformedConfig);
        }
        // Serialize registrations so concurrent first flows register one
        // client, not several.
        let _registration_guard = self.dcr_registration_lock.lock().await;
        if let Some(stored) = self.load_dcr_client(scope, vendor).await? {
            return stored_to_effective(stored);
        }
        let stored = self
            .discover_and_register(
                scope,
                vendor,
                resource,
                admitted_protected_resource_metadata_url,
            )
            .await?;
        self.persist_dcr_client(scope, vendor, &stored).await?;
        stored_to_effective(stored)
    }

    async fn discover_and_register(
        &self,
        scope: &ResourceScope,
        vendor: &str,
        resource: Option<&str>,
        admitted_protected_resource_metadata_url: Option<&HttpsEndpoint>,
    ) -> Result<StoredDcrClient, AuthProductError> {
        // 1. Resolve the authorization-server issuer.
        let issuer = match resource {
            Some(resource) => {
                let metadata_url = match admitted_protected_resource_metadata_url {
                    Some(url) => url.as_str().to_string(),
                    None => protected_resource_metadata_url(resource)?,
                };
                let response = self
                    .execute_vendor_get(scope, &metadata_url, Vec::new())
                    .await?;
                if !(200..300).contains(&response.status) {
                    // Dynamic registration is opt-in through protected
                    // resource metadata. Never invent an issuer from the
                    // resource when the server did not advertise one.
                    return Err(AuthProductError::MalformedConfig);
                }
                let metadata: ProtectedResourceAdmissionMetadata =
                    serde_json::from_slice(&response.body)
                        .map_err(|_| AuthProductError::BackendUnavailable)?;
                if metadata.resource.as_str() != resource
                    || metadata.authorization_servers.len() != 1
                {
                    return Err(AuthProductError::MalformedConfig);
                }
                metadata.authorization_servers[0].as_str().to_string()
            }
            None => return Err(AuthProductError::MalformedConfig),
        };

        // 2. Authorization-server metadata (RFC 8414).
        let metadata_url = authorization_server_metadata_url(&issuer)?;
        let response = self
            .execute_vendor_get(scope, &metadata_url, Vec::new())
            .await?;
        if !(200..300).contains(&response.status) {
            tracing::debug!(vendor, status = response.status, "AS metadata fetch failed");
            return Err(AuthProductError::BackendUnavailable);
        }
        let metadata: AuthorizationServerAdmissionMetadata = serde_json::from_slice(&response.body)
            .map_err(|_| AuthProductError::BackendUnavailable)?;
        if metadata.issuer.as_str() != issuer {
            return Err(AuthProductError::MalformedConfig);
        }
        let registration_endpoint = metadata
            .registration_endpoint
            .as_ref()
            .ok_or(AuthProductError::MalformedConfig)?;
        validate_endpoint_origin(registration_endpoint.as_str(), &metadata_url)?;

        // 3. Register (RFC 7591) with the static vendor callback.
        let redirect_uri = self.callback_base.redirect_uri_for(vendor)?;
        let registration = DcrRegistrationRequest {
            client_name: &self.dcr_client_name,
            redirect_uris: vec![redirect_uri.as_str()],
            grant_types: vec!["authorization_code", "refresh_token"],
            response_types: vec!["code"],
            token_endpoint_auth_method: "none",
        };
        let body =
            serde_json::to_vec(&registration).map_err(|_| AuthProductError::BackendUnavailable)?;
        let response = self
            .execute_vendor_post_json(scope, registration_endpoint.as_str(), body)
            .await?;
        if !(200..300).contains(&response.status) {
            tracing::warn!(
                vendor,
                status = response.status,
                "dynamic client registration rejected"
            );
            return Err(AuthProductError::BackendUnavailable);
        }
        let registered: DcrRegistrationResponse = serde_json::from_slice(&response.body)
            .map_err(|_| AuthProductError::BackendUnavailable)?;
        if registered.client_id.trim().is_empty() {
            return Err(AuthProductError::BackendUnavailable);
        }

        Ok(StoredDcrClient {
            client_id: registered.client_id,
            client_secret: registered.client_secret,
            authorization_endpoint: metadata.authorization_endpoint.as_str().to_string(),
            token_endpoint: metadata.token_endpoint.as_str().to_string(),
            redirect_uri: redirect_uri.as_str().to_string(),
        })
    }

    async fn load_dcr_client(
        &self,
        scope: &ResourceScope,
        vendor: &str,
    ) -> Result<Option<StoredDcrClient>, AuthProductError> {
        let handle = dcr_client_handle(vendor)?;
        let lease = match self.secret_store.lease_once(scope, &handle).await {
            Ok(lease) => lease,
            Err(error) if error.is_unknown_secret() || error.is_expired() => return Ok(None),
            Err(error) => return Err(http::map_secret_store_error(error)),
        };
        let material = self
            .secret_store
            .consume(scope, lease.id)
            .await
            .map_err(http::map_secret_store_error)?;
        let stored: StoredDcrClient = serde_json::from_str(material.expose_secret())
            .map_err(|_| AuthProductError::BackendUnavailable)?;
        Ok(Some(stored))
    }

    async fn persist_dcr_client(
        &self,
        scope: &ResourceScope,
        vendor: &str,
        client: &StoredDcrClient,
    ) -> Result<(), AuthProductError> {
        let handle = dcr_client_handle(vendor)?;
        let material =
            serde_json::to_string(client).map_err(|_| AuthProductError::BackendUnavailable)?;
        self.secret_store
            .put(scope.clone(), handle, SecretString::from(material), None)
            .await
            .map(|_| ())
            .map_err(http::map_secret_store_error)
    }
}

fn stored_to_effective(stored: StoredDcrClient) -> Result<EffectiveOAuthClient, AuthProductError> {
    Ok(EffectiveOAuthClient {
        client_id: OAuthClientId::new(stored.client_id)?,
        client_secret: stored.client_secret.map(SecretString::from),
        authorization_endpoint: stored.authorization_endpoint,
        token_endpoint: stored.token_endpoint,
    })
}

fn dcr_client_handle(vendor: &str) -> Result<SecretHandle, AuthProductError> {
    SecretHandle::new(format!("{DCR_CLIENT_HANDLE_PREFIX}-{vendor}"))
        .map_err(|_| AuthProductError::BackendUnavailable)
}

pub(crate) fn protected_resource_metadata_url(resource: &str) -> Result<String, AuthProductError> {
    let parsed = url::Url::parse(resource).map_err(|_| AuthProductError::MalformedConfig)?;
    if parsed.scheme() != "https" {
        return Err(AuthProductError::MalformedConfig);
    }
    let mut metadata = parsed.clone();
    let resource_path = parsed.path().trim_end_matches('/');
    metadata.set_path(&format!(
        "{resource_path}/.well-known/oauth-protected-resource"
    ));
    metadata.set_query(None);
    metadata.set_fragment(None);
    Ok(metadata.to_string())
}

pub(crate) fn authorization_server_metadata_url(issuer: &str) -> Result<String, AuthProductError> {
    let mut metadata = url::Url::parse(issuer).map_err(|_| AuthProductError::BackendUnavailable)?;
    if metadata.scheme() != "https" {
        return Err(AuthProductError::BackendUnavailable);
    }
    metadata.set_path("/.well-known/oauth-authorization-server");
    metadata.set_query(None);
    metadata.set_fragment(None);
    Ok(metadata.to_string())
}

pub(crate) fn validate_endpoint_origin(
    endpoint: &str,
    expected: &str,
) -> Result<(), AuthProductError> {
    let endpoint = url::Url::parse(endpoint).map_err(|_| AuthProductError::BackendUnavailable)?;
    let expected = url::Url::parse(expected).map_err(|_| AuthProductError::BackendUnavailable)?;
    if endpoint.origin() != expected.origin() {
        return Err(AuthProductError::BackendUnavailable);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_urls_are_well_formed() {
        assert_eq!(
            protected_resource_metadata_url("https://mcp.example.com/mcp").unwrap(),
            "https://mcp.example.com/mcp/.well-known/oauth-protected-resource"
        );
        assert_eq!(
            authorization_server_metadata_url("https://auth.example.com").unwrap(),
            "https://auth.example.com/.well-known/oauth-authorization-server"
        );
        assert!(protected_resource_metadata_url("http://mcp.example.com/mcp").is_err());
    }

    #[test]
    fn registration_endpoint_must_share_metadata_origin() {
        validate_endpoint_origin(
            "https://auth.example.com/register",
            "https://auth.example.com/.well-known/oauth-authorization-server",
        )
        .unwrap();
        assert!(
            validate_endpoint_origin(
                "https://attacker.invalid/register",
                "https://auth.example.com/.well-known/oauth-authorization-server",
            )
            .is_err()
        );
    }
}
