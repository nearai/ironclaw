//! Hosted-MCP OAuth client selection — implemented once for CIMD and RFC 7591
//! dynamic client registration. A recipe without `client_credentials`
//! discovers its authorization server and selects the advertised mechanism:
//!
//! 1. resolve the authorization server from RFC 9728 protected-resource
//!    metadata for the declared canonical resource,
//! 2. fetch RFC 8414 authorization-server metadata (authorize/token/
//!    registration endpoints; the recipe's endpoints are static placeholders),
//! 3. prefer Client ID Metadata Documents (CIMD), otherwise register a client
//!    with the static vendor callback as its redirect URI,
//! 4. persist the issuer-bound effective client for exchange and refresh.

use ironclaw_host_api::{ids::SecretHandle, resource::ResourceScope};

use ironclaw_extension_contracts::recipe::HttpsEndpoint;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};

use crate::{
    AuthFlowId, AuthProductError, AuthorizationServerAdmissionMetadata, OAuthClientId,
    ProtectedResourceAdmissionMetadata,
};

use super::exchange::EffectiveOAuthClient;
use super::{AuthEngine, http};

/// Prefix of the per-vendor persisted registered-client handle.
pub const DCR_CLIENT_HANDLE_PREFIX: &str = "oauth-dcr-client";

/// Flow-bound client snapshots only bridge the authorization redirect. The
/// refresh-token binding becomes the durable copy after a successful exchange.
const FLOW_CLIENT_SNAPSHOT_TTL_MINUTES: i64 = 15;

#[derive(Debug, Serialize)]
struct DcrRegistrationRequest<'a> {
    client_name: &'a str,
    application_type: &'a str,
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
#[derive(Clone, Serialize, Deserialize)]
struct StoredHostedClient {
    #[serde(default)]
    issuer: HostedClientIssuer,
    #[serde(default)]
    registration_method: HostedClientRegistration,
    client_id: String,
    #[serde(default)]
    client_secret: Option<String>,
    authorization_endpoint: String,
    token_endpoint: String,
    redirect_uri: String,
}

#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
enum HostedClientIssuer {
    Bound(String),
    #[default]
    Legacy,
}

impl HostedClientIssuer {
    fn matches(&self, issuer: &str) -> bool {
        matches!(self, Self::Bound(value) if value == issuer)
    }

    fn is_legacy(&self) -> bool {
        matches!(self, Self::Legacy)
    }
}

#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum HostedClientRegistration {
    #[default]
    LegacyDynamic,
    Dynamic,
    ClientMetadataDocument,
}

struct DiscoveredOAuthServer {
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
    registration_endpoint: Option<String>,
    client_id_metadata_document_supported: bool,
}

pub(super) enum DiscoveredClientUse {
    Prepare(AuthFlowId),
    Exchange(AuthFlowId),
    Refresh(SecretHandle),
}

/// Public OAuth client metadata served at the URL used as a CIMD client id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OAuthClientMetadataDocument {
    pub client_id: String,
    pub client_name: String,
    pub redirect_uris: Vec<String>,
    pub grant_types: Vec<String>,
    pub response_types: Vec<String>,
    pub token_endpoint_auth_method: String,
}

impl AuthEngine {
    /// Resolve the hosted client for a vendor whose recipe carries no
    /// deployment client credentials, registering it when DCR is selected.
    pub(super) async fn hosted_oauth_client(
        &self,
        scope: &ResourceScope,
        vendor: &str,
        resource: Option<&str>,
        admitted_protected_resource_metadata_url: Option<&HttpsEndpoint>,
        client_use: DiscoveredClientUse,
    ) -> Result<EffectiveOAuthClient, AuthProductError> {
        let flow_id = match client_use {
            DiscoveredClientUse::Prepare(flow_id) => flow_id,
            DiscoveredClientUse::Exchange(flow_id) => {
                return self
                    .load_bound_hosted_client(scope, vendor, &flow_client_handle(vendor, flow_id)?)
                    .await
                    .and_then(stored_to_effective);
            }
            DiscoveredClientUse::Refresh(refresh_secret) => {
                return self
                    .load_bound_hosted_client(
                        scope,
                        vendor,
                        &crate::oauth::hosted_oauth_refresh_client_handle(vendor, &refresh_secret)?,
                    )
                    .await
                    .and_then(stored_to_effective);
            }
        };
        let discovered = self
            .discover_oauth_server(
                scope,
                vendor,
                resource,
                admitted_protected_resource_metadata_url,
            )
            .await?;
        let redirect_uri = self.callback_base.redirect_uri_for(vendor)?;
        // Vendor discovery is network I/O and must not block unrelated OAuth
        // starts. Serialize only the selection/registration update so
        // concurrent first flows still persist one issuer-bound client.
        let _registration_guard = self.hosted_client_lock.lock().await;
        let existing = self.load_current_hosted_client(scope, vendor).await?;
        if let Some(legacy) = existing.as_ref().filter(|stored| stored.issuer.is_legacy()) {
            // Pre-issuer records may still back refresh tokens minted before
            // this release. Preserve that immutable migration source before
            // the current vendor selection is replaced.
            self.persist_hosted_client(scope, legacy_dcr_client_handle(vendor)?, legacy, None)
                .await?;
        }

        let selected = if discovered.client_id_metadata_document_supported {
            let document = self.client_metadata_document(vendor)?;
            let stored = StoredHostedClient {
                issuer: HostedClientIssuer::Bound(discovered.issuer),
                registration_method: HostedClientRegistration::ClientMetadataDocument,
                client_id: document.client_id,
                client_secret: None,
                authorization_endpoint: discovered.authorization_endpoint,
                token_endpoint: discovered.token_endpoint,
                redirect_uri: redirect_uri.as_str().to_string(),
            };
            if existing.as_ref().is_none_or(|existing| {
                existing.issuer != stored.issuer
                    || existing.registration_method != stored.registration_method
                    || existing.client_id != stored.client_id
                    || existing.authorization_endpoint != stored.authorization_endpoint
                    || existing.token_endpoint != stored.token_endpoint
                    || existing.redirect_uri != stored.redirect_uri
            }) {
                self.persist_current_hosted_client(scope, vendor, &stored)
                    .await?;
            }
            stored
        } else if let Some(stored) = existing
            && stored.issuer.matches(&discovered.issuer)
            && stored.registration_method == HostedClientRegistration::Dynamic
            && stored.redirect_uri == redirect_uri.as_str()
        {
            stored
        } else {
            let stored = self
                .register_dynamic_client(scope, vendor, discovered, &redirect_uri)
                .await?;
            self.persist_current_hosted_client(scope, vendor, &stored)
                .await?;
            stored
        };

        self.persist_hosted_client(
            scope,
            flow_client_handle(vendor, flow_id)?,
            &selected,
            Some(chrono::Utc::now() + chrono::Duration::minutes(FLOW_CLIENT_SNAPSHOT_TTL_MINUTES)),
        )
        .await?;
        stored_to_effective(selected)
    }

    async fn discover_oauth_server(
        &self,
        scope: &ResourceScope,
        vendor: &str,
        resource: Option<&str>,
        admitted_protected_resource_metadata_url: Option<&HttpsEndpoint>,
    ) -> Result<DiscoveredOAuthServer, AuthProductError> {
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
        if let Some(registration_endpoint) = metadata.registration_endpoint.as_ref() {
            validate_endpoint_origin(registration_endpoint.as_str(), &metadata_url)?;
        }

        Ok(DiscoveredOAuthServer {
            issuer,
            authorization_endpoint: metadata.authorization_endpoint.as_str().to_string(),
            token_endpoint: metadata.token_endpoint.as_str().to_string(),
            registration_endpoint: metadata
                .registration_endpoint
                .map(|endpoint| endpoint.as_str().to_string()),
            client_id_metadata_document_supported: metadata.client_id_metadata_document_supported,
        })
    }

    async fn register_dynamic_client(
        &self,
        scope: &ResourceScope,
        vendor: &str,
        discovered: DiscoveredOAuthServer,
        redirect_uri: &crate::OAuthRedirectUri,
    ) -> Result<StoredHostedClient, AuthProductError> {
        let registration_endpoint = discovered
            .registration_endpoint
            .as_deref()
            .ok_or(AuthProductError::MalformedConfig)?;
        let application_type = if redirect_uri.as_str().starts_with("http://") {
            "native"
        } else {
            "web"
        };
        let registration = DcrRegistrationRequest {
            client_name: &self.dcr_client_name,
            application_type,
            redirect_uris: vec![redirect_uri.as_str()],
            grant_types: vec!["authorization_code", "refresh_token"],
            response_types: vec!["code"],
            token_endpoint_auth_method: "none",
        };
        let body =
            serde_json::to_vec(&registration).map_err(|_| AuthProductError::BackendUnavailable)?;
        let response = self
            .execute_vendor_post_json(scope, registration_endpoint, body)
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

        Ok(StoredHostedClient {
            issuer: HostedClientIssuer::Bound(discovered.issuer),
            registration_method: HostedClientRegistration::Dynamic,
            client_id: registered.client_id,
            client_secret: registered.client_secret,
            authorization_endpoint: discovered.authorization_endpoint,
            token_endpoint: discovered.token_endpoint,
            redirect_uri: redirect_uri.as_str().to_string(),
        })
    }

    async fn load_current_hosted_client(
        &self,
        scope: &ResourceScope,
        vendor: &str,
    ) -> Result<Option<StoredHostedClient>, AuthProductError> {
        self.load_hosted_client(scope, &dcr_client_handle(vendor)?)
            .await
    }

    async fn load_hosted_client(
        &self,
        scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<Option<StoredHostedClient>, AuthProductError> {
        let lease = match self.secret_store.lease_once(scope, handle).await {
            Ok(lease) => lease,
            Err(error) if error.is_unknown_secret() || error.is_expired() => return Ok(None),
            Err(error) => return Err(http::map_secret_store_error(error)),
        };
        let material = self
            .secret_store
            .consume(scope, lease.id)
            .await
            .map_err(http::map_secret_store_error)?;
        let stored: StoredHostedClient = serde_json::from_str(material.expose_secret())
            .map_err(|_| AuthProductError::BackendUnavailable)?;
        Ok(Some(stored))
    }

    async fn load_bound_hosted_client(
        &self,
        scope: &ResourceScope,
        vendor: &str,
        handle: &SecretHandle,
    ) -> Result<StoredHostedClient, AuthProductError> {
        if let Some(stored) = self.load_hosted_client(scope, handle).await? {
            return Ok(stored);
        }
        // Before issuer binding shipped, the vendor cache was the only client
        // record. Preserve those issuerless in-flight credentials, but never
        // fall back to a mutable issuer-bound cache for a new flow/token.
        let legacy = match self
            .load_hosted_client(scope, &legacy_dcr_client_handle(vendor)?)
            .await?
        {
            Some(stored) => Some(stored),
            None => self
                .load_current_hosted_client(scope, vendor)
                .await?
                .filter(|stored| stored.issuer.is_legacy()),
        };
        let stored = legacy.ok_or(AuthProductError::MalformedConfig)?;
        self.persist_hosted_client(scope, handle.clone(), &stored, None)
            .await?;
        Ok(stored)
    }

    async fn persist_current_hosted_client(
        &self,
        scope: &ResourceScope,
        vendor: &str,
        client: &StoredHostedClient,
    ) -> Result<(), AuthProductError> {
        self.persist_hosted_client(scope, dcr_client_handle(vendor)?, client, None)
            .await
    }

    async fn persist_hosted_client(
        &self,
        scope: &ResourceScope,
        handle: SecretHandle,
        client: &StoredHostedClient,
        expires_at: Option<crate::Timestamp>,
    ) -> Result<(), AuthProductError> {
        let material =
            serde_json::to_string(client).map_err(|_| AuthProductError::BackendUnavailable)?;
        self.secret_store
            .put(
                scope.clone(),
                handle,
                SecretString::from(material),
                expires_at,
            )
            .await
            .map(|_| ())
            .map_err(http::map_secret_store_error)
    }

    pub(super) async fn bind_flow_client_to_refresh(
        &self,
        scope: &ResourceScope,
        vendor: &str,
        flow_id: AuthFlowId,
        refresh_secret: &SecretHandle,
    ) -> Result<(), AuthProductError> {
        let stored = self
            .load_bound_hosted_client(scope, vendor, &flow_client_handle(vendor, flow_id)?)
            .await?;
        self.persist_hosted_client(
            scope,
            crate::oauth::hosted_oauth_refresh_client_handle(vendor, refresh_secret)?,
            &stored,
            None,
        )
        .await
    }

    pub(super) async fn discard_refresh_client_binding(
        &self,
        scope: &ResourceScope,
        vendor: &str,
        refresh_secret: &SecretHandle,
    ) {
        let Ok(handle) = crate::oauth::hosted_oauth_refresh_client_handle(vendor, refresh_secret)
        else {
            tracing::debug!(
                vendor,
                "hosted OAuth client-binding handle derivation failed"
            );
            return;
        };
        if let Err(error) = self.secret_store.delete(scope, &handle).await
            && !error.is_unknown_secret()
        {
            tracing::debug!(
                vendor,
                secret_store_reason = error.stable_reason(),
                "best-effort hosted OAuth client-binding cleanup failed"
            );
        }
    }

    pub(super) async fn rebind_refresh_client(
        &self,
        scope: &ResourceScope,
        vendor: &str,
        previous_refresh_secret: &SecretHandle,
        next_refresh_secret: &SecretHandle,
    ) -> Result<(), AuthProductError> {
        let previous_handle =
            crate::oauth::hosted_oauth_refresh_client_handle(vendor, previous_refresh_secret)?;
        let stored = self
            .load_bound_hosted_client(scope, vendor, &previous_handle)
            .await?;
        self.persist_hosted_client(
            scope,
            crate::oauth::hosted_oauth_refresh_client_handle(vendor, next_refresh_secret)?,
            &stored,
            None,
        )
        .await
    }

    pub fn client_metadata_document(
        &self,
        vendor: &str,
    ) -> Result<OAuthClientMetadataDocument, AuthProductError> {
        let vendor = crate::AuthProviderId::new(vendor.to_string())?;
        let client_id = format!(
            "{}/{}/client-metadata.json",
            self.callback_base.base,
            vendor.as_str()
        );
        let parsed = url::Url::parse(&client_id).map_err(|error| {
            tracing::warn!(?error, "configured OAuth client metadata URL is invalid");
            AuthProductError::invalid_request(
                "CIMD requires a valid public HTTPS client metadata URL",
            )
        })?;
        if parsed.scheme() != "https" {
            tracing::warn!(
                scheme = parsed.scheme(),
                "CIMD was advertised but the configured client metadata URL is not public HTTPS"
            );
            return Err(AuthProductError::invalid_request(
                "CIMD requires a public HTTPS client metadata URL",
            ));
        }
        let redirect_uri = self.callback_base.redirect_uri_for(vendor.as_str())?;
        Ok(OAuthClientMetadataDocument {
            client_id,
            client_name: self.dcr_client_name.clone(),
            redirect_uris: vec![redirect_uri.as_str().to_string()],
            grant_types: vec![
                "authorization_code".to_string(),
                "refresh_token".to_string(),
            ],
            response_types: vec!["code".to_string()],
            token_endpoint_auth_method: "none".to_string(),
        })
    }
}

fn stored_to_effective(
    stored: StoredHostedClient,
) -> Result<EffectiveOAuthClient, AuthProductError> {
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

fn legacy_dcr_client_handle(vendor: &str) -> Result<SecretHandle, AuthProductError> {
    SecretHandle::new(format!("{DCR_CLIENT_HANDLE_PREFIX}-legacy-{vendor}"))
        .map_err(|_| AuthProductError::BackendUnavailable)
}

fn flow_client_handle(vendor: &str, flow_id: AuthFlowId) -> Result<SecretHandle, AuthProductError> {
    bound_client_handle("flow", vendor, &flow_id.to_string())
}

fn bound_client_handle(
    binding: &str,
    vendor: &str,
    discriminator: &str,
) -> Result<SecretHandle, AuthProductError> {
    let digest =
        ironclaw_common::hashing::sha256_hex(format!("{vendor}\0{discriminator}").as_bytes());
    SecretHandle::new(format!("oauth-hosted-{binding}-{digest}"))
        .map_err(|_| AuthProductError::BackendUnavailable)
}

pub(crate) fn protected_resource_metadata_url(resource: &str) -> Result<String, AuthProductError> {
    let parsed = url::Url::parse(resource).map_err(|_| AuthProductError::MalformedConfig)?;
    if parsed.scheme() != "https" {
        return Err(AuthProductError::MalformedConfig);
    }
    let mut metadata = parsed.clone();
    let resource_path = match parsed.path() {
        "/" => "",
        path => path,
    };
    metadata.set_path(&format!(
        "/.well-known/oauth-protected-resource{resource_path}"
    ));
    metadata.set_fragment(None);
    Ok(metadata.to_string())
}

pub(crate) fn protected_resource_metadata_root_url(
    resource: &str,
) -> Result<String, AuthProductError> {
    let mut metadata = url::Url::parse(resource).map_err(|_| AuthProductError::MalformedConfig)?;
    if metadata.scheme() != "https" {
        return Err(AuthProductError::MalformedConfig);
    }
    metadata.set_path("/.well-known/oauth-protected-resource");
    metadata.set_query(None);
    metadata.set_fragment(None);
    Ok(metadata.to_string())
}

pub(crate) fn authorization_server_metadata_url(issuer: &str) -> Result<String, AuthProductError> {
    let mut metadata = url::Url::parse(issuer).map_err(|_| AuthProductError::BackendUnavailable)?;
    if metadata.scheme() != "https" {
        return Err(AuthProductError::BackendUnavailable);
    }
    let issuer_path = metadata.path().trim_end_matches('/');
    metadata.set_path(&format!(
        "/.well-known/oauth-authorization-server{issuer_path}"
    ));
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
            "https://mcp.example.com/.well-known/oauth-protected-resource/mcp"
        );
        assert_eq!(
            protected_resource_metadata_url("https://mcp.example.com/mcp/?tenant=one").unwrap(),
            "https://mcp.example.com/.well-known/oauth-protected-resource/mcp/?tenant=one"
        );
        assert_eq!(
            protected_resource_metadata_root_url("https://mcp.example.com/mcp?tenant=one").unwrap(),
            "https://mcp.example.com/.well-known/oauth-protected-resource"
        );
        assert_eq!(
            authorization_server_metadata_url("https://auth.example.com").unwrap(),
            "https://auth.example.com/.well-known/oauth-authorization-server"
        );
        assert_eq!(
            authorization_server_metadata_url("https://auth.example.com/issuer/path").unwrap(),
            "https://auth.example.com/.well-known/oauth-authorization-server/issuer/path"
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
