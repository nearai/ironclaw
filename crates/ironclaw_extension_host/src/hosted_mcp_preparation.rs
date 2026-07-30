//! Hosted MCP package-definition admission and installation preparation.
//!
//! The generic lifecycle sees only `PreparationRequirement::Required` and
//! calls [`HostedMcpPreparationService::prepare_if_pending`]. OAuth metadata,
//! discovery, and catalog safety remain private to this concrete strategy.

use std::sync::Arc;

use ironclaw_extensions::{
    ExtensionInstallation, ExtensionInstallationId, ExtensionInstallationStorePort,
    ExtensionLifecycleService, ExtensionManifestRecord, ExtensionPackage, ManifestSource,
};
use ironclaw_host_api::{
    CapabilityId, CredentialStageError, RegisterHostedMcpRequest, ResourceScope, UserId,
};
use ironclaw_product::{LifecyclePackageRef, LifecycleProductResponse, ProductSurfaceFailure};
use tokio::sync::{Mutex, RwLock};

use crate::{
    AvailableExtensionCatalog, ExtensionActivationCredentialGate,
    ExtensionActivationCredentialReadiness, package_runtime_credential_auth_requirements,
};

pub struct HostedMcpPreparationService {
    installation_store: Arc<dyn ExtensionInstallationStorePort>,
    catalog: Arc<RwLock<AvailableExtensionCatalog>>,
    lifecycle_service: Arc<Mutex<ExtensionLifecycleService>>,
    operation_lock: Arc<Mutex<()>>,
    discovery_runtime_ports: Option<ironclaw_host_runtime::ProductAuthProviderRuntimePorts>,
    catalog_safety: crate::McpCatalogAdmissionPolicy,
    oauth_client_profiles: Arc<dyn ironclaw_auth::OAuthClientProfileRegistry>,
}

pub struct HostedMcpPreparationDependencies {
    pub runtime_ports: Option<ironclaw_host_runtime::ProductAuthProviderRuntimePorts>,
    pub catalog_safety: crate::McpCatalogAdmissionPolicy,
    pub oauth_client_profiles: Arc<dyn ironclaw_auth::OAuthClientProfileRegistry>,
}

impl HostedMcpPreparationService {
    pub fn new(
        installation_store: Arc<dyn ExtensionInstallationStorePort>,
        catalog: Arc<RwLock<AvailableExtensionCatalog>>,
        lifecycle_service: Arc<Mutex<ExtensionLifecycleService>>,
        operation_lock: Arc<Mutex<()>>,
        dependencies: HostedMcpPreparationDependencies,
    ) -> Self {
        Self {
            installation_store,
            catalog,
            lifecycle_service,
            operation_lock,
            discovery_runtime_ports: dependencies.runtime_ports,
            catalog_safety: dependencies.catalog_safety,
            oauth_client_profiles: dependencies.oauth_client_profiles,
        }
    }

    pub async fn register(
        &self,
        request: RegisterHostedMcpRequest,
    ) -> Result<LifecycleProductResponse, ProductSurfaceFailure> {
        let endpoint =
            crate::hosted_mcp_admission::CanonicalHostedMcpEndpoint::parse(&request.endpoint)
                .map_err(|_| crate::product_lifecycle::hosted_mcp_name_unavailable())?;
        let extension_id = ironclaw_host_api::hosted_mcp_extension_id(&request.desired_id)
            .map_err(|_| crate::product_lifecycle::hosted_mcp_name_unavailable())?;
        let package_ref = ironclaw_product::LifecyclePackageRef::new(
            ironclaw_product::LifecyclePackageKind::Extension,
            extension_id.as_str(),
        )?;
        let _guard = self.operation_lock.lock().await;
        let definition = match request.auth_selection.as_ref() {
            Some(selection) => crate::product_lifecycle::pending_hosted_mcp_manifest(
                &extension_id,
                &request.desired_name,
                &endpoint,
                selection,
            )?,
            None => self
                .installation_store
                .get_registered_package_definition(&extension_id)
                .await
                .map_err(crate::product_lifecycle::map_extension_installation_error)?
                .ok_or_else(crate::product_lifecycle::hosted_mcp_name_unavailable)?,
        };
        self.installation_store
            .admit_package_definition(definition.clone())
            .await
            .map_err(|_| crate::product_lifecycle::hosted_mcp_name_unavailable())?;
        let available = crate::product_lifecycle::available_user_registered_package(&definition)?;
        self.catalog
            .write()
            .await
            .extend(AvailableExtensionCatalog::from_packages(vec![available]));
        Ok(crate::product_lifecycle::hosted_mcp_registration_response(
            package_ref,
        ))
    }

    pub async fn prepare_if_pending(
        &self,
        package_ref: &LifecyclePackageRef,
        scope: ResourceScope,
        credential_gate: &dyn ExtensionActivationCredentialGate,
        caller: &UserId,
    ) -> Result<Option<LifecycleProductResponse>, ProductSurfaceFailure> {
        let (extension_id, installation_id) =
            crate::product_lifecycle::extension_ids_from_package_ref(package_ref)?;
        let (installation, manifest, package) = {
            let _guard = self.operation_lock.lock().await;
            let installation = self
                .load_installation(&extension_id, &installation_id)
                .await?;
            crate::ensure_caller_may_operate(&installation, caller)?;
            if installation.preparation_state().is_ready() {
                self.sync_lifecycle_package(&extension_id).await?;
                return Ok(None);
            }
            let manifest = self
                .installation_store
                .get_manifest(&extension_id)
                .await
                .map_err(crate::product_lifecycle::map_extension_installation_error)?
                .ok_or_else(crate::product_lifecycle::hosted_mcp_name_unavailable)?;
            if manifest.resolved().mcp.is_none() {
                return Err(ProductSurfaceFailure::InvalidBindingRequest {
                    reason: "no preparation strategy is composed for this package".to_string(),
                });
            }
            let package = self.lifecycle_package(&extension_id).await?;
            (installation, manifest, package)
        };

        let requirements = package_runtime_credential_auth_requirements(&package);
        if let ExtensionActivationCredentialReadiness::Missing(missing) =
            credential_gate.credential_readiness(&package).await?
        {
            return Ok(Some(
                crate::product_lifecycle::activation_credentials_incomplete_response(
                    package_ref.clone(),
                    missing,
                )?,
            ));
        }
        let ports = self.discovery_runtime_ports.as_ref().ok_or_else(|| {
            ProductSurfaceFailure::Transient {
                reason: "hosted MCP catalog preparation runtime is unavailable".to_string(),
            }
        })?;
        let safety = &self.catalog_safety;
        let capability_id = package
            .manifest
            .capabilities
            .first()
            .map(|capability| capability.id.clone())
            .ok_or_else(|| ProductSurfaceFailure::InvalidBindingRequest {
                reason: "hosted MCP registration has no discovery capability".to_string(),
            })?;
        let network_policy =
            crate::activation_transaction::hosted_mcp_discovery_network_policy(&package)
                .map_err(crate::product_lifecycle::map_extension_installation_error)?;
        let _handoff_guard = ports.staged_handoff_guard(scope.clone(), capability_id.clone());
        ports.stage_network_policy_once(&scope, &capability_id, network_policy);
        for requirement in package
            .manifest
            .capabilities
            .iter()
            .flat_map(|capability| capability.runtime_credentials.iter())
            .filter(|requirement| requirement.required)
        {
            if let Err(error) = ports
                .stage_credential_requirement_once(
                    &scope,
                    &capability_id,
                    requirement,
                    &extension_id,
                )
                .await
            {
                return match error {
                    CredentialStageError::AuthRequired => Ok(Some(
                        crate::product_lifecycle::activation_credentials_incomplete_response(
                            package_ref.clone(),
                            requirements,
                        )?,
                    )),
                    CredentialStageError::Backend => Err(ProductSurfaceFailure::Transient {
                        reason: "hosted MCP credential staging is temporarily unavailable"
                            .to_string(),
                    }),
                };
            }
        }
        let discovered = match crate::discover_hosted_mcp_package_with_policy(
            &package,
            scope.clone(),
            ports.runtime_http_egress(),
            Some(safety),
        )
        .await
        {
            Ok(discovered) => discovered,
            Err(crate::HostedMcpDiscoveryError::CredentialsRejected(challenge))
                if manifest.resolved().auth.is_empty()
                    && matches!(
                        manifest
                            .resolved()
                            .mcp
                            .as_ref()
                            .map(|mcp| &mcp.registration_auth),
                        Some(ironclaw_host_api::HostedMcpAuthSelection::OAuth { .. })
                    ) =>
            {
                let client_profile_id = match manifest
                    .resolved()
                    .mcp
                    .as_ref()
                    .map(|mcp| mcp.registration_auth.clone())
                {
                    Some(ironclaw_host_api::HostedMcpAuthSelection::OAuth {
                        client_profile_id,
                    }) => client_profile_id,
                    _ => None,
                };
                let enriched = self
                    .prepare_oauth_manifest(
                        manifest.clone(),
                        &challenge,
                        client_profile_id,
                        &scope,
                        &capability_id,
                        ports,
                    )
                    .await?;
                let incarnation = installation
                    .incarnation_id()
                    .ok_or_else(crate::product_lifecycle::hosted_mcp_name_unavailable)?;
                self.installation_store
                    .checkpoint_preparation(
                        installation.installation_id(),
                        incarnation,
                        installation.manifest_ref(),
                        enriched.clone(),
                    )
                    .await
                    .map_err(crate::product_lifecycle::map_extension_installation_error)?;
                let enriched_package =
                    crate::product_lifecycle::available_user_registered_package(&enriched)?;
                let requirements =
                    package_runtime_credential_auth_requirements(&enriched_package.package);
                self.catalog
                    .write()
                    .await
                    .extend(AvailableExtensionCatalog::from_packages(vec![
                        enriched_package,
                    ]));
                self.sync_lifecycle_package(&extension_id).await?;
                return Ok(Some(
                    crate::product_lifecycle::activation_credentials_incomplete_response(
                        package_ref.clone(),
                        requirements,
                    )?,
                ));
            }
            Err(crate::HostedMcpDiscoveryError::CredentialsRejected(_)) => {
                return Ok(Some(
                    crate::product_lifecycle::activation_credentials_incomplete_response(
                        package_ref.clone(),
                        requirements,
                    )?,
                ));
            }
            Err(error) => {
                return Err(crate::product_lifecycle::hosted_mcp_discovery_error(error));
            }
        };
        let finalized_resolved =
            crate::effective_resolved_for_package(manifest.resolved(), &discovered);
        let finalized = ExtensionManifestRecord::from_resolved(
            manifest.raw_toml(),
            manifest.manifest().source,
            finalized_resolved,
            manifest.manifest_hash().cloned(),
        )
        .map(|record| record.with_definition_retention(manifest.definition_retention()))
        .map_err(crate::product_lifecycle::map_extension_installation_error)?;
        let incarnation = installation
            .incarnation_id()
            .ok_or_else(crate::product_lifecycle::hosted_mcp_name_unavailable)?;
        self.installation_store
            .finalize_preparation(
                installation.installation_id(),
                incarnation,
                installation.manifest_ref(),
                finalized.clone(),
            )
            .await
            .map_err(crate::product_lifecycle::map_extension_installation_error)?;
        if finalized.manifest().source == ManifestSource::UserRegistered {
            self.catalog
                .write()
                .await
                .extend(AvailableExtensionCatalog::from_packages(vec![
                    crate::product_lifecycle::available_user_registered_package(&finalized)?,
                ]));
        }
        self.sync_lifecycle_package(&extension_id).await?;
        Ok(None)
    }

    async fn prepare_oauth_manifest(
        &self,
        seed: ExtensionManifestRecord,
        challenge: &ironclaw_host_api::McpAuthChallenge,
        client_profile_id: Option<String>,
        scope: &ResourceScope,
        capability_id: &CapabilityId,
        ports: &ironclaw_host_runtime::ProductAuthProviderRuntimePorts,
    ) -> Result<ExtensionManifestRecord, ProductSurfaceFailure> {
        let endpoint = seed
            .resolved()
            .mcp
            .as_ref()
            .map(|mcp| mcp.server.as_str())
            .ok_or_else(crate::product_lifecycle::hosted_mcp_name_unavailable)?;
        let resource_fetch = ironclaw_auth::OAuthRecipeAdmission::<
            ironclaw_auth::EmptyOAuthClientProfileRegistry,
        >::preflight_protected_resource(endpoint, challenge)
        .map_err(crate::product_lifecycle::hosted_mcp_oauth_admission_error)?;
        let resource_metadata: ironclaw_auth::ProtectedResourceAdmissionMetadata = self
            .fetch_oauth_metadata(ports, scope, capability_id, resource_fetch.metadata_url())
            .await?;
        let authorization_server_fetch = ironclaw_auth::OAuthRecipeAdmission::<
            ironclaw_auth::EmptyOAuthClientProfileRegistry,
        >::preflight_authorization_server(
            resource_fetch, &resource_metadata
        )
        .map_err(crate::product_lifecycle::hosted_mcp_oauth_admission_error)?;
        let authorization_server_metadata: ironclaw_auth::AuthorizationServerAdmissionMetadata =
            self.fetch_oauth_metadata(
                ports,
                scope,
                capability_id,
                authorization_server_fetch.metadata_url(),
            )
            .await?;
        let endpoint = crate::hosted_mcp_admission::CanonicalHostedMcpEndpoint::parse(
            &ironclaw_host_api::HostedMcpEndpoint::new(endpoint)
                .map_err(|_| crate::product_lifecycle::hosted_mcp_name_unavailable())?,
        )
        .map_err(|_| crate::product_lifecycle::hosted_mcp_name_unavailable())?;
        let vendor = crate::hosted_mcp_admission::hosted_mcp_vendor_id(&endpoint)
            .map_err(|_| crate::product_lifecycle::hosted_mcp_name_unavailable())?;
        let profiles = Arc::clone(&self.oauth_client_profiles);
        let admitted = ironclaw_auth::OAuthRecipeAdmission::new(SharedOAuthProfiles(profiles))
            .admit(ironclaw_auth::OAuthRecipeAdmissionRequest {
                vendor: vendor.as_str().to_string(),
                authorization_server_fetch,
                authorization_server_metadata,
                scopes: Vec::new(),
                client_profile_id,
                dcr_policy_allowed: true,
            })
            .await
            .map_err(crate::product_lifecycle::hosted_mcp_oauth_admission_error)?;
        crate::product_lifecycle::hosted_mcp_manifest_with_admitted_oauth(seed, &endpoint, admitted)
    }

    async fn fetch_oauth_metadata<T: serde::de::DeserializeOwned>(
        &self,
        ports: &ironclaw_host_runtime::ProductAuthProviderRuntimePorts,
        scope: &ResourceScope,
        capability_id: &CapabilityId,
        url: &str,
    ) -> Result<T, ProductSurfaceFailure> {
        const BODY_LIMIT: u64 = 64 * 1024;
        let policy = crate::product_lifecycle::hosted_mcp_metadata_network_policy(url)?;
        ports.stage_network_policy_once(scope, capability_id, policy.clone());
        let response = ports
            .runtime_http_egress()
            .execute(ironclaw_host_api::RuntimeHttpEgressRequest {
                runtime: ironclaw_host_api::RuntimeKind::Mcp,
                scope: scope.clone(),
                capability_id: capability_id.clone(),
                method: ironclaw_host_api::NetworkMethod::Get,
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
            .map_err(|_| ProductSurfaceFailure::Transient {
                reason: "hosted MCP OAuth metadata fetch failed".to_string(),
            })?;
        if response.status != 200 || response.body.len() as u64 > BODY_LIMIT {
            return Err(ProductSurfaceFailure::InvalidBindingRequest {
                reason: "hosted MCP OAuth metadata response was invalid".to_string(),
            });
        }
        serde_json::from_slice(&response.body).map_err(|_| {
            ProductSurfaceFailure::InvalidBindingRequest {
                reason: "hosted MCP OAuth metadata document was malformed".to_string(),
            }
        })
    }

    async fn load_installation(
        &self,
        extension_id: &ironclaw_host_api::ExtensionId,
        installation_id: &ExtensionInstallationId,
    ) -> Result<ExtensionInstallation, ProductSurfaceFailure> {
        let installation = self
            .installation_store
            .get_installation(installation_id)
            .await
            .map_err(crate::product_lifecycle::map_extension_installation_error)?
            .ok_or_else(|| ProductSurfaceFailure::InvalidBindingRequest {
                reason: format!("extension {} is not installed", extension_id.as_str()),
            })?;
        if installation.extension_id() != extension_id {
            return Err(ProductSurfaceFailure::InvalidBindingRequest {
                reason: "extension installation identity mismatch".to_string(),
            });
        }
        Ok(installation)
    }

    async fn lifecycle_package(
        &self,
        extension_id: &ironclaw_host_api::ExtensionId,
    ) -> Result<ExtensionPackage, ProductSurfaceFailure> {
        self.lifecycle_service
            .lock()
            .await
            .registry()
            .get_extension(extension_id)
            .cloned()
            .ok_or_else(|| ProductSurfaceFailure::InvalidBindingRequest {
                reason: format!("extension {} is not installed", extension_id.as_str()),
            })
    }

    async fn sync_lifecycle_package(
        &self,
        extension_id: &ironclaw_host_api::ExtensionId,
    ) -> Result<(), ProductSurfaceFailure> {
        let record = self
            .installation_store
            .get_manifest(extension_id)
            .await
            .map_err(crate::product_lifecycle::map_extension_installation_error)?
            .ok_or_else(|| ProductSurfaceFailure::InvalidBindingRequest {
                reason: format!(
                    "extension {} has no installed manifest",
                    extension_id.as_str()
                ),
            })?;
        let manifest: ironclaw_extensions::ExtensionManifest = record
            .manifest()
            .clone()
            .try_into()
            .map_err(crate::product_lifecycle::map_extension_error)?;
        let package = crate::generic_host::rebuild_package_from_resolved(
            manifest,
            record.resolved(),
            extension_id.as_str(),
        )
        .map_err(|reason| ProductSurfaceFailure::InvalidBindingRequest { reason })?;
        let mut lifecycle = self.lifecycle_service.lock().await;
        match lifecycle.registry().get_extension(extension_id) {
            None => lifecycle
                .install(package)
                .await
                .map_err(crate::product_lifecycle::map_extension_error),
            Some(current) if current == &package => Ok(()),
            Some(_) => lifecycle
                .update(package)
                .await
                .map_err(crate::product_lifecycle::map_extension_error),
        }
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
