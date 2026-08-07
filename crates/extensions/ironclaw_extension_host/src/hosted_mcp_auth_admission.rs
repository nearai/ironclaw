//! Lifecycle-neutral hosted-MCP authentication admission.
//!
//! Registration and installed-extension preparation both need to validate
//! OAuth metadata. This module owns only that network admission work; it has no
//! installation store, lifecycle service, catalog, or operation lock.

use std::sync::Arc;

use ironclaw_extension_contracts::hosted_mcp::HostedMcpAuthSelection;
use ironclaw_extension_registry::ExtensionManifestRecord;
use ironclaw_host_api::{ids::CapabilityId, resource::ResourceScope};
use ironclaw_product_contracts::{
    error::ProductOperationFailure,
    package_lifecycle::{LifecyclePackageRef, LifecycleProductResponse},
};

pub(crate) async fn prepare_oauth_manifest(
    seed: ExtensionManifestRecord,
    challenge: &ironclaw_extension_contracts::hosted_mcp::McpAuthChallenge,
    selection: HostedMcpAuthSelection,
    scope: &ResourceScope,
    capability_id: &CapabilityId,
    ports: &ironclaw_host_runtime::ProductAuthProviderRuntimePorts,
    oauth_client_profiles: Arc<dyn ironclaw_auth::OAuthClientProfileRegistry>,
) -> Result<Option<ExtensionManifestRecord>, ProductOperationFailure> {
    let client_profile_id = match &selection {
        HostedMcpAuthSelection::OAuth { client_profile_id } => client_profile_id.clone(),
        HostedMcpAuthSelection::Auto => None,
        _ => return Err(crate::hosted_mcp_manifest::name_unavailable()),
    };
    let endpoint = seed
        .resolved()
        .mcp
        .as_ref()
        .map(|mcp| mcp.server.as_str())
        .ok_or_else(crate::hosted_mcp_manifest::name_unavailable)?;
    let resource_fetches = ironclaw_auth::OAuthRecipeAdmission::<
        ironclaw_auth::EmptyOAuthClientProfileRegistry,
    >::preflight_protected_resource_candidates(endpoint, challenge)
    .map_err(crate::hosted_mcp_manifest::oauth_admission_error)?;
    let derived = challenge.www_authenticate_metadata.is_empty()
        && challenge.protected_resource_metadata.is_empty();
    for resource_fetch in resource_fetches {
        let Some(resource_metadata): Option<ironclaw_auth::ProtectedResourceAdmissionMetadata> =
            fetch_optional_oauth_metadata(
                ports,
                scope,
                capability_id,
                resource_fetch.metadata_url(),
            )
            .await?
        else {
            if derived {
                continue;
            }
            return Err(invalid_oauth_metadata_response());
        };
        let authorization_server_fetch = ironclaw_auth::OAuthRecipeAdmission::<
            ironclaw_auth::EmptyOAuthClientProfileRegistry,
        >::preflight_authorization_server(
            resource_fetch, &resource_metadata
        )
        .map_err(crate::hosted_mcp_manifest::oauth_admission_error)?;
        let authorization_server_metadata: ironclaw_auth::AuthorizationServerAdmissionMetadata =
            fetch_oauth_metadata(
                ports,
                scope,
                capability_id,
                authorization_server_fetch.metadata_url(),
            )
            .await?;
        if matches!(selection, HostedMcpAuthSelection::Auto)
            && authorization_server_metadata
                .registration_endpoint
                .is_none()
        {
            tracing::debug!(
                authorization_server = authorization_server_fetch.issuer(),
                "hosted MCP OAuth metadata does not support automatic client registration"
            );
            return Ok(None);
        }
        let endpoint = crate::hosted_mcp_admission::CanonicalHostedMcpEndpoint::parse(
            &ironclaw_extension_contracts::hosted_mcp::HostedMcpEndpoint::new(endpoint)
                .map_err(crate::hosted_mcp_manifest::endpoint_input_error)?,
        )
        .map_err(crate::hosted_mcp_manifest::endpoint_admission_error)?;
        let vendor = crate::hosted_mcp_admission::hosted_mcp_vendor_id(&endpoint)
            .map_err(crate::hosted_mcp_manifest::endpoint_admission_error)?;
        let admitted =
            ironclaw_auth::OAuthRecipeAdmission::new(SharedOAuthProfiles(oauth_client_profiles))
                .admit(ironclaw_auth::OAuthRecipeAdmissionRequest {
                    vendor: vendor.as_str().to_string(),
                    authorization_server_fetch,
                    authorization_server_metadata,
                    scopes: Vec::new(),
                    client_profile_id,
                })
                .await
                .map_err(crate::hosted_mcp_manifest::oauth_admission_error)?;
        return crate::hosted_mcp_manifest::manifest_with_admitted_oauth(seed, &endpoint, admitted)
            .map(Some);
    }
    Ok(None)
}

pub(crate) fn auth_selection_required_response(
    package_ref: LifecyclePackageRef,
    message: &str,
) -> Result<LifecycleProductResponse, ProductOperationFailure> {
    let mut response = crate::product_lifecycle::activation_credentials_incomplete_response(
        package_ref,
        Vec::new(),
    )?;
    response.blockers = vec![
        ironclaw_product_contracts::package_lifecycle::LifecycleReadinessBlocker::Setup {
            ref_id: Some(
                ironclaw_extension_contracts::lifecycle_id::LifecycleBlockerRef::new(
                    ironclaw_product_contracts::package_lifecycle::HOSTED_MCP_AUTH_SELECTION_BLOCKER_REF,
                )?,
            ),
        },
    ];
    response.message = Some(message.to_string());
    Ok(response)
}

async fn fetch_oauth_metadata<T: serde::de::DeserializeOwned>(
    ports: &ironclaw_host_runtime::ProductAuthProviderRuntimePorts,
    scope: &ResourceScope,
    capability_id: &CapabilityId,
    url: &str,
) -> Result<T, ProductOperationFailure> {
    fetch_optional_oauth_metadata(ports, scope, capability_id, url)
        .await?
        .ok_or_else(invalid_oauth_metadata_response)
}

async fn fetch_optional_oauth_metadata<T: serde::de::DeserializeOwned>(
    ports: &ironclaw_host_runtime::ProductAuthProviderRuntimePorts,
    scope: &ResourceScope,
    capability_id: &CapabilityId,
    url: &str,
) -> Result<Option<T>, ProductOperationFailure> {
    const BODY_LIMIT: u64 = 64 * 1024;
    let policy = crate::hosted_mcp_manifest::metadata_network_policy(url)?;
    ports.stage_network_policy_once(scope, capability_id, policy.clone());
    let response = ports
        .runtime_http_egress()
        .execute(ironclaw_host_api::http::RuntimeHttpEgressRequest {
            runtime: ironclaw_host_api::runtime::RuntimeKind::Mcp,
            scope: scope.clone(),
            capability_id: capability_id.clone(),
            method: ironclaw_host_api::action::NetworkMethod::Get,
            url: url.to_string(),
            headers: vec![("accept".to_string(), "application/json".to_string())],
            body: Vec::new(),
            network_policy: policy,
            credential_injections: Vec::new(),
            response_body_limit: Some(BODY_LIMIT),
            save_body_to: None,
            timeout_ms: Some(10_000),
        })
        .await
        .map_err(|error| {
            tracing::debug!(%error, "hosted MCP OAuth metadata fetch failed");
            ProductOperationFailure::Transient {
                reason: "hosted MCP OAuth metadata fetch failed".to_string(),
            }
        })?;
    if response.status == 404 {
        return Ok(None);
    }
    if response.status != 200 || response.body.len() as u64 > BODY_LIMIT {
        return Err(invalid_oauth_metadata_response());
    }
    serde_json::from_slice(&response.body)
        .map(Some)
        .map_err(|error| {
            tracing::debug!(%error, "hosted MCP OAuth metadata document was malformed");
            ProductOperationFailure::InvalidBindingRequest {
                reason: "hosted MCP OAuth metadata document was malformed".to_string(),
            }
        })
}

fn invalid_oauth_metadata_response() -> ProductOperationFailure {
    ProductOperationFailure::InvalidBindingRequest {
        reason: "hosted MCP OAuth metadata response was invalid".to_string(),
    }
}

#[derive(Clone)]
struct SharedOAuthProfiles(Arc<dyn ironclaw_auth::OAuthClientProfileRegistry>);

impl std::fmt::Debug for SharedOAuthProfiles {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SharedOAuthProfiles")
    }
}

#[async_trait::async_trait]
impl ironclaw_auth::OAuthClientProfileRegistry for SharedOAuthProfiles {
    async fn resolve(&self, profile_id: &str) -> Option<ironclaw_auth::AdmissionClientProfile> {
        self.0.resolve(profile_id).await
    }
}
