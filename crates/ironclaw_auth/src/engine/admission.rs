//! Auth-owned admission policy for OAuth-protected hosted MCP resources.
//!
//! This deliberately admits only already-validated metadata and opaque client
//! handles. The caller supplies metadata fetched through mediated egress; raw
//! tokens and client secrets never enter this boundary.

use std::collections::BTreeSet;
use std::fmt;

use async_trait::async_trait;
use ironclaw_extension_contracts::hosted_mcp::McpAuthChallenge;
use ironclaw_extension_contracts::recipe::{
    BoundedJsonPointer, HttpsEndpoint, OAuth2CodeRecipe, RecipeClientCredentials, TokenResponseMap,
    VendorAuthRecipe,
};

use crate::{AuthProductError, ResolvedVendorAuthRecipe};

/// Bounded, decoded RFC 9728 document supplied by mediated egress. Keeping
/// this typed prevents arbitrary provider bodies from entering admission.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct ProtectedResourceAdmissionMetadata {
    pub resource: HttpsEndpoint,
    pub authorization_servers: Vec<HttpsEndpoint>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct AuthorizationServerAdmissionMetadata {
    pub issuer: HttpsEndpoint,
    pub authorization_endpoint: HttpsEndpoint,
    pub token_endpoint: HttpsEndpoint,
    #[serde(default)]
    pub registration_endpoint: Option<HttpsEndpoint>,
}

/// First preflight result: the only protected-resource metadata URL that the
/// caller is authorized to fetch. Construction validates HTTPS before I/O.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtectedResourceMetadataFetch {
    canonical_resource: HttpsEndpoint,
    metadata_url: HttpsEndpoint,
}

impl ProtectedResourceMetadataFetch {
    pub fn canonical_resource(&self) -> &str {
        self.canonical_resource.as_str()
    }

    pub fn metadata_url(&self) -> &str {
        self.metadata_url.as_str()
    }
}

/// Second preflight result: the exact issuer and canonical HTTPS metadata URL
/// that mediated egress may fetch after the resource document is validated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizationServerMetadataFetch {
    resource: ProtectedResourceMetadataFetch,
    issuer: HttpsEndpoint,
    metadata_url: HttpsEndpoint,
}

impl AuthorizationServerMetadataFetch {
    pub fn issuer(&self) -> &str {
        self.issuer.as_str()
    }

    pub fn metadata_url(&self) -> &str {
        self.metadata_url.as_str()
    }
}

/// An operator-managed client profile. It is usable only for its exact
/// canonical resource and issuer, and carries handles rather than secrets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmissionClientProfile {
    pub id: String,
    pub resource: HttpsEndpoint,
    pub issuer: HttpsEndpoint,
    pub credentials: RecipeClientCredentials,
}

/// Allowlisted operator/admin client-profile lookup. The empty implementation
/// is fail-closed, so admission never invents a client.
#[async_trait]
pub trait OAuthClientProfileRegistry: Send + Sync + fmt::Debug {
    async fn resolve(&self, profile_id: &str) -> Option<AdmissionClientProfile>;
}

#[derive(Debug, Default)]
pub struct EmptyOAuthClientProfileRegistry;

#[async_trait]
impl OAuthClientProfileRegistry for EmptyOAuthClientProfileRegistry {
    async fn resolve(&self, _profile_id: &str) -> Option<AdmissionClientProfile> {
        None
    }
}

/// Input after the MCP protocol adapter has parsed a redacted challenge and
/// fetched the two standards metadata documents through mediated egress.
#[derive(Debug, Clone)]
pub struct OAuthRecipeAdmissionRequest {
    pub vendor: String,
    pub authorization_server_fetch: AuthorizationServerMetadataFetch,
    pub authorization_server_metadata: AuthorizationServerAdmissionMetadata,
    pub scopes: Vec<String>,
    pub client_profile_id: Option<String>,
}

/// Narrow, policy-only admission. Network discovery remains mediated by the
/// caller; this service validates the discovered document chain and produces
/// only recipe/profile references suitable for manifest persistence.
#[derive(Debug)]
pub struct OAuthRecipeAdmission<R> {
    profiles: R,
}

impl<R> OAuthRecipeAdmission<R>
where
    R: OAuthClientProfileRegistry,
{
    pub fn new(profiles: R) -> Self {
        Self { profiles }
    }

    /// Select one advertised protected-resource metadata URL and validate it
    /// as canonical HTTPS before the caller performs any network fetch.
    pub fn preflight_protected_resource(
        canonical_resource: &str,
        challenge: &McpAuthChallenge,
    ) -> Result<ProtectedResourceMetadataFetch, AuthProductError> {
        if !matches!(challenge.status, 401 | 403) {
            return Err(AuthProductError::MalformedConfig);
        }
        let canonical_resource = HttpsEndpoint::new(canonical_resource.to_string())
            .map_err(|_| AuthProductError::MalformedConfig)?;
        let mut advertised = BTreeSet::new();
        for location in challenge
            .www_authenticate_metadata
            .iter()
            .chain(challenge.protected_resource_metadata.iter())
        {
            let parsed = url::Url::parse(location.as_str())
                .map_err(|_| AuthProductError::MalformedConfig)?;
            if parsed.scheme() != "https"
                || parsed.host_str().is_none()
                || !parsed.username().is_empty()
                || parsed.password().is_some()
                || parsed.fragment().is_some()
            {
                return Err(AuthProductError::MalformedConfig);
            }
            let normalized = HttpsEndpoint::new(parsed.to_string())
                .map_err(|_| AuthProductError::MalformedConfig)?;
            advertised.insert(normalized.as_str().to_string());
        }
        if advertised.len() != 1 {
            return Err(AuthProductError::MalformedConfig);
        }
        let metadata_url = HttpsEndpoint::new(
            advertised
                .into_iter()
                .next()
                .ok_or(AuthProductError::MalformedConfig)?,
        )
        .map_err(|_| AuthProductError::MalformedConfig)?;
        Ok(ProtectedResourceMetadataFetch {
            canonical_resource,
            metadata_url,
        })
    }

    /// Validate the fetched RFC 9728 document and produce the only issuer
    /// metadata URL that may be fetched next.
    pub fn preflight_authorization_server(
        resource: ProtectedResourceMetadataFetch,
        metadata: &ProtectedResourceAdmissionMetadata,
    ) -> Result<AuthorizationServerMetadataFetch, AuthProductError> {
        if metadata.resource.as_str() != resource.canonical_resource.as_str()
            || metadata.authorization_servers.len() != 1
        {
            return Err(AuthProductError::MalformedConfig);
        }
        let issuer = metadata.authorization_servers[0].clone();
        let metadata_url = HttpsEndpoint::new(super::dcr::authorization_server_metadata_url(
            issuer.as_str(),
        )?)
        .map_err(|_| AuthProductError::MalformedConfig)?;
        Ok(AuthorizationServerMetadataFetch {
            resource,
            issuer,
            metadata_url,
        })
    }

    pub async fn admit(
        &self,
        request: OAuthRecipeAdmissionRequest,
    ) -> Result<ResolvedVendorAuthRecipe, AuthProductError> {
        let issuer = request.authorization_server_fetch.issuer.as_str();
        let canonical_resource = request
            .authorization_server_fetch
            .resource
            .canonical_resource
            .as_str();
        let expected_as_metadata = request.authorization_server_fetch.metadata_url.as_str();
        if request.authorization_server_metadata.issuer.as_str() != issuer {
            return Err(AuthProductError::MalformedConfig);
        }
        let authorization_endpoint = request
            .authorization_server_metadata
            .authorization_endpoint
            .as_str();
        let token_endpoint = request
            .authorization_server_metadata
            .token_endpoint
            .as_str();
        super::http::https_endpoint_host(authorization_endpoint)?;
        super::http::https_endpoint_host(token_endpoint)?;
        let registration_endpoint = request
            .authorization_server_metadata
            .registration_endpoint
            .as_ref()
            .map(HttpsEndpoint::as_str);
        let credentials = match request.client_profile_id {
            Some(profile_id) => {
                let profile = self
                    .profiles
                    .resolve(&profile_id)
                    .await
                    .ok_or(AuthProductError::MalformedConfig)?;
                if profile.resource.as_str() != canonical_resource
                    || profile.issuer.as_str() != issuer
                {
                    return Err(AuthProductError::MalformedConfig);
                }
                Some(profile.credentials)
            }
            None => {
                let registration =
                    registration_endpoint.ok_or(AuthProductError::MalformedConfig)?;
                super::dcr::validate_endpoint_origin(registration, expected_as_metadata)?;
                super::http::https_endpoint_host(registration)?;
                None
            }
        };
        let recipe = OAuth2CodeRecipe {
            display_name: request.vendor.clone(),
            authorization_endpoint: ironclaw_extension_contracts::recipe::HttpsEndpoint::new(
                authorization_endpoint.to_string(),
            )
            .map_err(|_| AuthProductError::MalformedConfig)?,
            token_endpoint: ironclaw_extension_contracts::recipe::HttpsEndpoint::new(
                token_endpoint.to_string(),
            )
            .map_err(|_| AuthProductError::MalformedConfig)?,
            scope_param: None,
            scope_join: Default::default(),
            pkce: Default::default(),
            scopes: request.scopes,
            extra_authorize_params: Default::default(),
            client_credentials: credentials,
            exchange_auth: Default::default(),
            token_response: TokenResponseMap {
                access_token: BoundedJsonPointer::new("/access_token")
                    .map_err(|_| AuthProductError::MalformedConfig)?,
                refresh_token: Some(
                    BoundedJsonPointer::new("/refresh_token")
                        .map_err(|_| AuthProductError::MalformedConfig)?,
                ),
                expires_in: Some(
                    BoundedJsonPointer::new("/expires_in")
                        .map_err(|_| AuthProductError::MalformedConfig)?,
                ),
                scope: None,
            },
            identity: None,
            refresh: None,
            revoke: None,
            instructions: None,
            setup_url: None,
        };
        recipe
            .validate()
            .map_err(|_| AuthProductError::MalformedConfig)?;
        Ok(ResolvedVendorAuthRecipe {
            vendor: request.vendor,
            recipe: VendorAuthRecipe::Oauth2Code(Box::new(recipe)),
            token_exchange_resource: Some(canonical_resource.to_string()),
            protected_resource_metadata_url: Some(
                request.authorization_server_fetch.resource.metadata_url,
            ),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_extension_contracts::hosted_mcp::McpAuthMetadataLocation;

    #[derive(Debug)]
    struct Profiles(Option<AdmissionClientProfile>);
    #[async_trait]
    impl OAuthClientProfileRegistry for Profiles {
        async fn resolve(&self, id: &str) -> Option<AdmissionClientProfile> {
            self.0.clone().filter(|profile| profile.id == id)
        }
    }

    fn https(value: &str) -> HttpsEndpoint {
        HttpsEndpoint::new(value.to_string()).unwrap()
    }

    fn request() -> OAuthRecipeAdmissionRequest {
        let location = "https://mcp.example.test/metadata";
        let challenge = McpAuthChallenge {
            status: 401,
            www_authenticate_metadata: vec![McpAuthMetadataLocation::new(location).unwrap()],
            protected_resource_metadata: vec![],
        };
        let resource_fetch = OAuthRecipeAdmission::<Profiles>::preflight_protected_resource(
            "https://mcp.example.test/mcp",
            &challenge,
        )
        .unwrap();
        let protected = ProtectedResourceAdmissionMetadata {
            resource: https("https://mcp.example.test/mcp"),
            authorization_servers: vec![https("https://auth.example.test")],
        };
        let authorization_server_fetch =
            OAuthRecipeAdmission::<Profiles>::preflight_authorization_server(
                resource_fetch,
                &protected,
            )
            .unwrap();
        OAuthRecipeAdmissionRequest {
            vendor: "mcp-test".into(),
            authorization_server_fetch,
            authorization_server_metadata: AuthorizationServerAdmissionMetadata {
                issuer: https("https://auth.example.test"),
                authorization_endpoint: https("https://auth.example.test/authorize"),
                token_endpoint: https("https://auth.example.test/token"),
                registration_endpoint: Some(https("https://auth.example.test/register")),
            },
            scopes: vec!["read".into()],
            client_profile_id: None,
        }
    }
    async fn admit(
        request: OAuthRecipeAdmissionRequest,
    ) -> Result<ResolvedVendorAuthRecipe, AuthProductError> {
        OAuthRecipeAdmission::new(Profiles(None))
            .admit(request)
            .await
    }
    #[tokio::test]
    async fn valid_dcr_and_www_advertisement_admit() {
        let resolved = admit(request()).await.expect("valid request admits");
        // Regression pin: a hosted-MCP OAuth account must be able to refresh
        // after the initial exchange, not just authenticate once. The
        // admitted recipe's token_response map is the only place that
        // capability is declared — if it dropped `refresh_token`/
        // `expires_in`, the exchange path would have nothing to persist as
        // the account's refresh secret and every later refresh would fail
        // closed forever with no admission-time error.
        let VendorAuthRecipe::Oauth2Code(recipe) = &resolved.recipe else {
            panic!("DCR admission always produces an oauth2_code recipe");
        };
        assert!(
            recipe.token_response.refresh_token.is_some(),
            "admitted recipe must capture refresh_token so the account can be kept alive"
        );
        assert!(
            recipe.token_response.expires_in.is_some(),
            "admitted recipe must capture expires_in so keepalive/expiry accounting works"
        );
    }
    #[tokio::test]
    async fn dedicated_header_advertisement_admits() {
        let challenge = McpAuthChallenge {
            status: 401,
            www_authenticate_metadata: vec![],
            protected_resource_metadata: vec![
                McpAuthMetadataLocation::new("https://mcp.example.test/metadata").unwrap(),
            ],
        };
        assert!(
            OAuthRecipeAdmission::<Profiles>::preflight_protected_resource(
                "https://mcp.example.test/mcp",
                &challenge
            )
            .is_ok()
        );
    }
    #[tokio::test]
    async fn missing_advertisement_rejects() {
        let challenge = McpAuthChallenge {
            status: 401,
            www_authenticate_metadata: vec![],
            protected_resource_metadata: vec![],
        };
        assert!(
            OAuthRecipeAdmission::<Profiles>::preflight_protected_resource(
                "https://mcp.example.test/mcp",
                &challenge
            )
            .is_err()
        );
    }
    #[tokio::test]
    async fn http_metadata_location_rejects_before_fetch() {
        let challenge = McpAuthChallenge {
            status: 401,
            www_authenticate_metadata: vec![
                McpAuthMetadataLocation::new("http://mcp.example.test/metadata").unwrap(),
            ],
            protected_resource_metadata: vec![],
        };
        assert!(
            OAuthRecipeAdmission::<Profiles>::preflight_protected_resource(
                "https://mcp.example.test/mcp",
                &challenge
            )
            .is_err()
        );
    }
    #[tokio::test]
    async fn mixed_http_and_https_advertisements_reject() {
        let challenge = McpAuthChallenge {
            status: 401,
            www_authenticate_metadata: vec![
                McpAuthMetadataLocation::new("http://mcp.example.test/metadata").unwrap(),
            ],
            protected_resource_metadata: vec![
                McpAuthMetadataLocation::new("https://mcp.example.test/metadata").unwrap(),
            ],
        };
        assert!(
            OAuthRecipeAdmission::<Profiles>::preflight_protected_resource(
                "https://mcp.example.test/mcp",
                &challenge
            )
            .is_err()
        );
    }
    #[tokio::test]
    async fn two_distinct_https_advertisements_reject_as_ambiguous() {
        let challenge = McpAuthChallenge {
            status: 401,
            www_authenticate_metadata: vec![
                McpAuthMetadataLocation::new("https://mcp.example.test/one").unwrap(),
            ],
            protected_resource_metadata: vec![
                McpAuthMetadataLocation::new("https://mcp.example.test/two").unwrap(),
            ],
        };
        assert!(
            OAuthRecipeAdmission::<Profiles>::preflight_protected_resource(
                "https://mcp.example.test/mcp",
                &challenge
            )
            .is_err()
        );
    }
    #[tokio::test]
    async fn duplicate_advertisement_across_headers_is_accepted_once() {
        let location = McpAuthMetadataLocation::new("https://mcp.example.test/metadata").unwrap();
        let challenge = McpAuthChallenge {
            status: 401,
            www_authenticate_metadata: vec![location.clone()],
            protected_resource_metadata: vec![location],
        };
        assert!(
            OAuthRecipeAdmission::<Profiles>::preflight_protected_resource(
                "https://mcp.example.test/mcp",
                &challenge
            )
            .is_ok()
        );
    }
    #[tokio::test]
    async fn wrong_resource_rejects() {
        let challenge = McpAuthChallenge {
            status: 401,
            www_authenticate_metadata: vec![
                McpAuthMetadataLocation::new("https://mcp.example.test/metadata").unwrap(),
            ],
            protected_resource_metadata: vec![],
        };
        let fetch = OAuthRecipeAdmission::<Profiles>::preflight_protected_resource(
            "https://mcp.example.test/mcp",
            &challenge,
        )
        .unwrap();
        let metadata = ProtectedResourceAdmissionMetadata {
            resource: https("https://other.example/mcp"),
            authorization_servers: vec![https("https://auth.example.test")],
        };
        assert!(
            OAuthRecipeAdmission::<Profiles>::preflight_authorization_server(fetch, &metadata)
                .is_err()
        );
    }
    #[tokio::test]
    async fn wrong_issuer_rejects() {
        let challenge = McpAuthChallenge {
            status: 401,
            www_authenticate_metadata: vec![
                McpAuthMetadataLocation::new("https://mcp.example.test/metadata").unwrap(),
            ],
            protected_resource_metadata: vec![],
        };
        let fetch = OAuthRecipeAdmission::<Profiles>::preflight_protected_resource(
            "https://mcp.example.test/mcp",
            &challenge,
        )
        .unwrap();
        let metadata = ProtectedResourceAdmissionMetadata {
            resource: https("https://mcp.example.test/mcp"),
            authorization_servers: vec![https("https://login.other.test")],
        };
        let stage =
            OAuthRecipeAdmission::<Profiles>::preflight_authorization_server(fetch, &metadata)
                .unwrap();
        let mut r = request();
        r.authorization_server_fetch = stage;
        assert!(admit(r).await.is_err());
    }
    #[tokio::test]
    async fn multiple_issuers_reject_without_fallback() {
        let challenge = McpAuthChallenge {
            status: 401,
            www_authenticate_metadata: vec![
                McpAuthMetadataLocation::new("https://mcp.example.test/metadata").unwrap(),
            ],
            protected_resource_metadata: vec![],
        };
        let fetch = OAuthRecipeAdmission::<Profiles>::preflight_protected_resource(
            "https://mcp.example.test/mcp",
            &challenge,
        )
        .unwrap();
        let metadata = ProtectedResourceAdmissionMetadata {
            resource: https("https://mcp.example.test/mcp"),
            authorization_servers: vec![
                https("https://auth.example.test"),
                https("https://other.example.test"),
            ],
        };
        assert!(
            OAuthRecipeAdmission::<Profiles>::preflight_authorization_server(fetch, &metadata)
                .is_err()
        );
    }
    #[tokio::test]
    async fn cross_origin_advertised_issuer_is_valid() {
        let challenge = McpAuthChallenge {
            status: 401,
            www_authenticate_metadata: vec![
                McpAuthMetadataLocation::new("https://mcp.example.test/metadata").unwrap(),
            ],
            protected_resource_metadata: vec![],
        };
        let fetch = OAuthRecipeAdmission::<Profiles>::preflight_protected_resource(
            "https://mcp.example.test/mcp",
            &challenge,
        )
        .unwrap();
        let metadata = ProtectedResourceAdmissionMetadata {
            resource: https("https://mcp.example.test/mcp"),
            authorization_servers: vec![https("https://login.identity.test")],
        };
        let stage =
            OAuthRecipeAdmission::<Profiles>::preflight_authorization_server(fetch, &metadata)
                .unwrap();
        assert_eq!(stage.issuer(), "https://login.identity.test");
    }
    #[tokio::test]
    async fn invalid_endpoints_reject() {
        assert!(
            serde_json::from_value::<AuthorizationServerAdmissionMetadata>(serde_json::json!({
                "issuer": "https://auth.example.test",
                "authorization_endpoint": "https://auth.example.test/authorize",
                "token_endpoint": "http://auth.example.test/token"
            }))
            .is_err()
        );
        assert!(
            serde_json::from_value::<AuthorizationServerAdmissionMetadata>(serde_json::json!({
                "issuer": "https://auth.example.test",
                "authorization_endpoint": "https://auth.example.test/authorize",
                "token_endpoint": "https://auth.example.test/token",
                "registration_endpoint": "http://auth.example.test/register"
            }))
            .is_err()
        );
    }

    #[tokio::test]
    async fn standard_metadata_extensions_are_ignored() {
        let protected =
            serde_json::from_value::<ProtectedResourceAdmissionMetadata>(serde_json::json!({
                "resource": "https://mcp.notion.com/mcp",
                "authorization_servers": ["https://mcp.notion.com"],
                "scopes_supported": ["default"],
                "bearer_methods_supported": ["header"],
                "resource_name": "Notion MCP (Beta)"
            }))
            .expect("standard protected-resource extensions should be ignored");
        assert_eq!(protected.resource.as_str(), "https://mcp.notion.com/mcp");
        assert_eq!(
            protected.authorization_servers[0].as_str(),
            "https://mcp.notion.com"
        );

        let authorization = serde_json::from_value::<AuthorizationServerAdmissionMetadata>(
            serde_json::json!({
                "issuer": "https://mcp.notion.com",
                "authorization_endpoint": "https://mcp.notion.com/authorize",
                "token_endpoint": "https://mcp.notion.com/token",
                "registration_endpoint": "https://mcp.notion.com/register",
                "scopes_supported": ["default"],
                "response_types_supported": ["code"],
                "response_modes_supported": ["query"],
                "grant_types_supported": ["authorization_code", "refresh_token", "urn:ietf:params:oauth:grant-type:jwt-bearer"],
                "authorization_grant_profiles_supported": ["urn:ietf:params:oauth:grant-profile:id-jag"],
                "token_endpoint_auth_methods_supported": ["client_secret_basic", "client_secret_post", "none"],
                "revocation_endpoint": "https://mcp.notion.com/token",
                "code_challenge_methods_supported": ["plain", "S256"],
                "client_id_metadata_document_supported": true,
                "introspection_endpoint": "https://mcp.notion.com/introspect",
                "introspection_endpoint_auth_methods_supported": ["client_secret_basic", "client_secret_post", "none"]
            }),
        )
        .expect("standard authorization-server extensions should be ignored");
        assert_eq!(
            authorization
                .registration_endpoint
                .as_ref()
                .map(HttpsEndpoint::as_str),
            Some("https://mcp.notion.com/register")
        );

        let challenge = McpAuthChallenge {
            status: 401,
            www_authenticate_metadata: vec![
                McpAuthMetadataLocation::new(
                    "https://mcp.notion.com/.well-known/oauth-protected-resource/mcp",
                )
                .expect("metadata location"),
            ],
            protected_resource_metadata: vec![],
        };
        let resource_fetch = OAuthRecipeAdmission::<Profiles>::preflight_protected_resource(
            "https://mcp.notion.com/mcp",
            &challenge,
        )
        .expect("protected resource preflight");
        let authorization_server_fetch =
            OAuthRecipeAdmission::<Profiles>::preflight_authorization_server(
                resource_fetch,
                &protected,
            )
            .expect("authorization server preflight");
        let admitted = admit(OAuthRecipeAdmissionRequest {
            vendor: "mcp-notion".to_string(),
            authorization_server_fetch,
            authorization_server_metadata: authorization,
            scopes: Vec::new(),
            client_profile_id: None,
        })
        .await
        .expect("Notion-compatible OAuth metadata should be admitted");
        assert_eq!(
            admitted.token_exchange_resource.as_deref(),
            Some("https://mcp.notion.com/mcp")
        );
    }

    #[tokio::test]
    async fn malformed_metadata_shapes_reject() {
        assert!(
            serde_json::from_value::<ProtectedResourceAdmissionMetadata>(serde_json::json!({
                "resource": "https://mcp.example.test/mcp\n",
                "authorization_servers": ["https://auth.example.test"]
            }))
            .is_err()
        );
        let challenge = McpAuthChallenge {
            status: 401,
            www_authenticate_metadata: vec![
                McpAuthMetadataLocation::new("https://mcp.example.test/metadata").unwrap(),
            ],
            protected_resource_metadata: vec![],
        };
        let fetch = OAuthRecipeAdmission::<Profiles>::preflight_protected_resource(
            "https://mcp.example.test/mcp",
            &challenge,
        )
        .unwrap();
        let empty = ProtectedResourceAdmissionMetadata {
            resource: https("https://mcp.example.test/mcp"),
            authorization_servers: vec![],
        };
        assert!(
            OAuthRecipeAdmission::<Profiles>::preflight_authorization_server(fetch, &empty)
                .is_err()
        );
    }
    #[tokio::test]
    async fn invalid_registration_rejects() {
        let mut r = request();
        r.authorization_server_metadata.registration_endpoint =
            Some(https("https://evil.example/register"));
        assert!(admit(r).await.is_err());
    }
    #[tokio::test]
    async fn unknown_profile_rejects() {
        let mut r = request();
        r.client_profile_id = Some("unknown".into());
        assert!(admit(r).await.is_err());
    }
    #[tokio::test]
    async fn mismatched_profile_rejects() {
        let mut r = request();
        r.client_profile_id = Some("known".into());
        let profile = AdmissionClientProfile {
            id: "known".into(),
            resource: https("https://other.example/mcp"),
            issuer: https("https://auth.example.test"),
            credentials: RecipeClientCredentials {
                client_id_handle: ironclaw_host_api::ids::SecretHandle::new("client-id").unwrap(),
                client_secret_handle: None,
            },
        };
        assert!(
            OAuthRecipeAdmission::new(Profiles(Some(profile)))
                .admit(r)
                .await
                .is_err()
        );
    }
    #[tokio::test]
    async fn exact_allowlisted_profile_admits_handles_only() {
        let mut r = request();
        r.client_profile_id = Some("known".into());
        let profile = AdmissionClientProfile {
            id: "known".into(),
            resource: https("https://mcp.example.test/mcp"),
            issuer: https("https://auth.example.test"),
            credentials: RecipeClientCredentials {
                client_id_handle: ironclaw_host_api::ids::SecretHandle::new("client-id").unwrap(),
                client_secret_handle: Some(
                    ironclaw_host_api::ids::SecretHandle::new("client-secret").unwrap(),
                ),
            },
        };
        let resolved = OAuthRecipeAdmission::new(Profiles(Some(profile)))
            .admit(r)
            .await
            .unwrap();
        let VendorAuthRecipe::Oauth2Code(recipe) = resolved.recipe else {
            panic!("oauth recipe")
        };
        assert_eq!(
            recipe.client_credentials.unwrap().client_id_handle.as_str(),
            "client-id"
        );
    }
}
