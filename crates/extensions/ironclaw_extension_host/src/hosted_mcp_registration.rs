//! Hosted MCP package-definition registration.
//!
//! Registration admits a caller-scoped definition and projects it into the
//! available catalog. It deliberately has no installation lifecycle service,
//! installation operation lock, or active-capability publisher.

use std::sync::Arc;

use ironclaw_extension_contracts::hosted_mcp::{HostedMcpAuthSelection, RegisterHostedMcpRequest};
use ironclaw_extension_registry::{
    ExtensionInstallationError, ExtensionInstallationStorePort, ExtensionManifestRecord,
    ManifestSource, RegisteredPackageDefinition,
};
use ironclaw_host_api::{ids::UserId, resource::ResourceScope};
use ironclaw_product_contracts::{
    error::ProductOperationFailure,
    package_lifecycle::{LifecyclePackageKind, LifecyclePackageRef, LifecycleProductResponse},
};
use tokio::sync::{RwLock, Semaphore};

use crate::{AvailableExtensionCatalog, HostedMcpPreparationDependencies};

/// Per-process ceiling for registration-time network preflight. It bounds
/// concurrent third-party probes without queueing callers behind a stalled
/// endpoint; saturation is returned as a retryable product failure.
const MAX_CONCURRENT_HOSTED_MCP_REGISTRATION_PREFLIGHTS: usize = 8;

pub(crate) struct CustomMcpRegistrationService {
    definition_store: Arc<dyn ExtensionInstallationStorePort>,
    catalog: Arc<RwLock<AvailableExtensionCatalog>>,
    runtime_ports: Option<ironclaw_host_runtime::ProductAuthProviderRuntimePorts>,
    oauth_client_profiles: Arc<dyn ironclaw_auth::OAuthClientProfileRegistry>,
    preflight_semaphore: Arc<Semaphore>,
}

impl CustomMcpRegistrationService {
    pub(crate) fn new(
        definition_store: Arc<dyn ExtensionInstallationStorePort>,
        catalog: Arc<RwLock<AvailableExtensionCatalog>>,
        dependencies: &HostedMcpPreparationDependencies,
    ) -> Self {
        Self {
            definition_store,
            catalog,
            runtime_ports: dependencies.runtime_ports.clone(),
            oauth_client_profiles: Arc::clone(&dependencies.oauth_client_profiles),
            preflight_semaphore: Arc::new(Semaphore::new(
                MAX_CONCURRENT_HOSTED_MCP_REGISTRATION_PREFLIGHTS,
            )),
        }
    }

    pub(crate) async fn register(
        &self,
        request: RegisterHostedMcpRequest,
        scope: ResourceScope,
    ) -> Result<LifecycleProductResponse, ProductOperationFailure> {
        let endpoint =
            crate::hosted_mcp_admission::CanonicalHostedMcpEndpoint::parse(&request.endpoint)
                .map_err(|error| {
                    tracing::debug!(?error, "hosted MCP registration rejected: invalid endpoint");
                    crate::hosted_mcp_manifest::name_unavailable()
                })?;
        let extension_id =
            ironclaw_extension_contracts::hosted_mcp::hosted_mcp_extension_id(&request.desired_id)
                .map_err(|error| {
                    tracing::debug!(%error, "hosted MCP registration rejected: invalid desired id");
                    crate::hosted_mcp_manifest::name_unavailable()
                })?;
        let package_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, extension_id.as_str())?;
        let caller = scope.user_id.clone();
        if let Some(existing) = self
            .definition_store
            .get_registered_package_definition(&extension_id)
            .await
            .map_err(crate::product_lifecycle::map_extension_installation_error)?
        {
            return self
                .existing_registration_response(
                    existing,
                    &request,
                    &endpoint,
                    &package_ref,
                    &caller,
                )
                .await;
        }
        let definition = match request.auth_selection.as_ref() {
            Some(
                selection @ (HostedMcpAuthSelection::Auto | HostedMcpAuthSelection::OAuth { .. }),
            ) => {
                let _preflight_permit = self.preflight_semaphore.try_acquire().map_err(|_| {
                    ProductOperationFailure::Transient {
                        reason: "hosted MCP registration preflight is temporarily saturated"
                            .to_string(),
                    }
                })?;
                let seed = crate::hosted_mcp_manifest::pending_manifest(
                    &extension_id,
                    &request.desired_name,
                    &endpoint,
                    selection,
                )?;
                let Some(resolved) = self
                    .resolve_registration_auth(
                        &extension_id,
                        &request.desired_name,
                        &endpoint,
                        seed,
                        selection,
                        scope,
                    )
                    .await?
                else {
                    return match selection {
                        HostedMcpAuthSelection::Auto => {
                            crate::hosted_mcp_auth_admission::auth_selection_required_response(
                                package_ref,
                                "Hosted MCP requires authentication but did not expose usable OAuth metadata; choose OAuth or Bearer token to finish registration.",
                            )
                        }
                        HostedMcpAuthSelection::OAuth { .. } => {
                            Err(ProductOperationFailure::InvalidBindingRequest {
                                reason: "hosted MCP did not expose usable OAuth metadata"
                                    .to_string(),
                            })
                        }
                        _ => Err(crate::hosted_mcp_manifest::name_unavailable()),
                    };
                };
                resolved
            }
            Some(selection) => crate::hosted_mcp_manifest::pending_manifest(
                &extension_id,
                &request.desired_name,
                &endpoint,
                selection,
            )?,
            None => return Err(crate::hosted_mcp_manifest::name_unavailable()),
        };
        let registered = RegisteredPackageDefinition::managed_by(definition, caller);
        let available = crate::hosted_mcp_manifest::available_registered_package(
            registered.definition(),
            registered.audience(),
        )?;
        match self
            .definition_store
            .admit_package_definition(registered)
            .await
        {
            Ok(_) => {}
            Err(ExtensionInstallationError::PackageDefinitionConflict { .. }) => {
                return Err(crate::hosted_mcp_manifest::name_unavailable());
            }
            Err(error) => {
                return Err(crate::product_lifecycle::map_extension_installation_error(
                    error,
                ));
            }
        }
        self.catalog
            .write()
            .await
            .extend(AvailableExtensionCatalog::from_packages(vec![available]));
        Ok(crate::hosted_mcp_manifest::registration_response(
            package_ref,
        ))
    }

    async fn existing_registration_response(
        &self,
        existing: RegisteredPackageDefinition,
        request: &RegisterHostedMcpRequest,
        endpoint: &crate::hosted_mcp_admission::CanonicalHostedMcpEndpoint,
        package_ref: &LifecyclePackageRef,
        caller: &UserId,
    ) -> Result<LifecycleProductResponse, ProductOperationFailure> {
        if !existing.audience().visible_to(caller) {
            return Err(crate::hosted_mcp_manifest::name_unavailable());
        }
        let definition = existing.definition();
        if definition.manifest().source != ManifestSource::UserRegistered
            || !registration_request_matches(definition, request, endpoint)
        {
            return Err(crate::hosted_mcp_manifest::name_unavailable());
        }
        let available = crate::hosted_mcp_manifest::available_registered_package(
            definition,
            existing.audience(),
        )?;
        self.catalog
            .write()
            .await
            .extend(AvailableExtensionCatalog::from_packages(vec![available]));
        Ok(crate::hosted_mcp_manifest::registration_response(
            package_ref.clone(),
        ))
    }

    async fn resolve_registration_auth(
        &self,
        extension_id: &ironclaw_host_api::ids::ExtensionId,
        desired_name: &str,
        endpoint: &crate::hosted_mcp_admission::CanonicalHostedMcpEndpoint,
        seed: ExtensionManifestRecord,
        selection: &HostedMcpAuthSelection,
        scope: ResourceScope,
    ) -> Result<Option<ExtensionManifestRecord>, ProductOperationFailure> {
        let ports =
            self.runtime_ports
                .as_ref()
                .ok_or_else(|| ProductOperationFailure::Transient {
                    reason: "hosted MCP registration runtime is unavailable".to_string(),
                })?;
        let package = crate::hosted_mcp_manifest::registered_extension_package(&seed)?;
        let capability_id = package
            .manifest
            .capabilities
            .first()
            .map(|capability| capability.id.clone())
            .ok_or_else(crate::hosted_mcp_manifest::name_unavailable)?;
        let network_policy = crate::mcp::hosted_mcp_network_policy(&package)
            .ok_or_else(crate::hosted_mcp_manifest::name_unavailable)?;
        let _handoff_guard = ports.staged_handoff_guard(scope.clone(), capability_id.clone());
        ports.stage_network_policy_once(&scope, &capability_id, network_policy);
        match crate::mcp_discovery::probe_hosted_mcp_auth(
            &package,
            scope.clone(),
            ports.runtime_http_egress(),
        )
        .await
        {
            Ok(_) if matches!(selection, HostedMcpAuthSelection::Auto) => {
                crate::hosted_mcp_manifest::pending_manifest(
                    extension_id,
                    desired_name,
                    endpoint,
                    &HostedMcpAuthSelection::NoAuth,
                )
                .map(Some)
            }
            Ok(_) => Err(ProductOperationFailure::InvalidBindingRequest {
                reason: "hosted MCP accepted unauthenticated access instead of advertising OAuth"
                    .to_string(),
            }),
            Err(crate::HostedMcpDiscoveryError::CredentialsRejected(challenge)) => {
                crate::hosted_mcp_auth_admission::prepare_oauth_manifest(
                    seed,
                    &challenge,
                    selection.clone(),
                    &scope,
                    &capability_id,
                    ports,
                    Arc::clone(&self.oauth_client_profiles),
                )
                .await
            }
            Err(error) => {
                tracing::debug!(
                    extension_id = %extension_id,
                    ?error,
                    "hosted MCP registration preflight failed"
                );
                Err(crate::hosted_mcp_manifest::discovery_error(error))
            }
        }
    }
}

fn registration_request_matches(
    existing: &ExtensionManifestRecord,
    request: &RegisterHostedMcpRequest,
    endpoint: &crate::hosted_mcp_admission::CanonicalHostedMcpEndpoint,
) -> bool {
    let resolved = existing.resolved();
    if resolved.name != request.desired_name.trim() {
        return false;
    }
    let Some(mcp) = resolved.mcp.as_ref() else {
        return false;
    };
    if mcp.server != endpoint.as_str() {
        return false;
    }
    match request.auth_selection.as_ref() {
        None | Some(HostedMcpAuthSelection::Auto) => true,
        Some(selection) => &mcp.registration_auth == selection,
    }
}
