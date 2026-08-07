//! Hosted MCP installation preparation.
//!
//! The generic lifecycle knows nothing about discovery: it calls
//! [`HostedMcpPreparationService::prepare_if_pending`] and reads readiness off
//! the package itself. OAuth metadata, discovery, and catalog safety remain
//! private to this concrete strategy.

use std::sync::Arc;

use ironclaw_extension_contracts::hosted_mcp::HostedMcpAuthSelection;
use ironclaw_extension_registry::{
    ExtensionInstallation, ExtensionInstallationId, ExtensionInstallationStorePort,
    ExtensionLifecycleService, ExtensionManifestRecord, ExtensionPackage, ManifestSource,
    PackageDefinitionAudience, RegisteredPackageDefinition,
};
use ironclaw_host_api::{dispatch::CredentialStageError, ids::UserId, resource::ResourceScope};
use ironclaw_product_contracts::error::ProductOperationFailure;
use ironclaw_product_contracts::package_lifecycle::{
    LifecyclePackageRef, LifecycleProductResponse,
};
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

    pub async fn prepare_if_pending(
        &self,
        package_ref: &LifecyclePackageRef,
        scope: ResourceScope,
        credential_gate: &dyn ExtensionActivationCredentialGate,
        caller: &UserId,
    ) -> Result<Option<LifecycleProductResponse>, ProductOperationFailure> {
        let (extension_id, installation_id) =
            crate::product_lifecycle::extension_ids_from_package_ref(package_ref)?;
        let (installation, manifest, package, max_tools, best_effort) = {
            let _guard = self.operation_lock.lock().await;
            let installation = self
                .load_installation(&extension_id, &installation_id)
                .await?;
            crate::ensure_caller_may_operate(&installation, caller)?;
            let manifest = self
                .installation_store
                .get_manifest(&extension_id)
                .await
                .map_err(crate::product_lifecycle::map_extension_installation_error)?
                .ok_or_else(crate::hosted_mcp_manifest::name_unavailable)?;
            // No `[mcp]` declaration at all: nothing to discover, ever. This
            // holds for every such package regardless of what it publishes —
            // a channel-only extension declares no model-visible capability
            // and still has nothing to prepare, so it must not fall through
            // to the hosted-MCP lookup below.
            if manifest.resolved().mcp.is_none() {
                self.sync_lifecycle_package(&extension_id).await?;
                return Ok(None);
            }
            // A package that already publishes something model-visible is not
            // waiting on discovery to become usable: any discovery run for it
            // is a refresh, so failure must stay non-fatal.
            let best_effort = manifest.resolved().has_model_visible_capabilities();
            // `best_effort` here means `Ready` with `[mcp]` still declared
            // (e.g. NEAR AI, which ships a static model-visible template tool
            // so activation never depends on live discovery). It gets the
            // same discovery attempt below as a `Required` package; only the
            // handling of a failed attempt differs (see below).
            let max_tools = manifest
                .resolved()
                .mcp
                .as_ref()
                .map(|mcp| mcp.max_tools)
                .ok_or_else(|| ProductOperationFailure::InvalidBindingRequest {
                    reason: "no preparation strategy is composed for this package".to_string(),
                })?;
            let package = self.lifecycle_package(&extension_id).await?;
            (installation, manifest, package, max_tools, best_effort)
        };

        let outcome = self
            .attempt_hosted_mcp_preparation(
                package_ref,
                &extension_id,
                scope,
                credential_gate,
                &installation,
                &manifest,
                &package,
                max_tools,
                best_effort,
            )
            .await;
        if !best_effort {
            return outcome;
        }
        if let Err(error) = outcome {
            // silent-ok: a `Ready` package (NEAR AI-shaped: `[mcp]` plus a
            // static visible template tool) ships its static catalog
            // precisely so activation does not depend on a reachable MCP
            // server. Treating a failed discovery attempt as fatal here
            // reintroduced a real production regression — hosted MCP
            // preparation failing with a transient `network_error` escalated
            // to a fatal `RebornBuildError::InvalidConfig` and broke offline
            // composition builds. `activate_inner`'s own credential check
            // still blocks activation if credentials are genuinely missing,
            // so swallowing here only forgoes the *live* catalog refresh, not
            // the safety checks.
            tracing::debug!(
                extension_id = extension_id.as_str(),
                %error,
                "hosted MCP opportunistic discovery failed; continuing with the static catalog"
            );
        }
        // A `Some(response)` outcome (e.g. missing credentials, OAuth setup
        // needed) is likewise not surfaced here for the same reason: it is
        // "discovery didn't happen," not "activation must stop."
        self.sync_lifecycle_package(&extension_id).await?;
        Ok(None)
    }

    pub async fn select_auth(
        &self,
        package_ref: &LifecyclePackageRef,
        auth_selection: HostedMcpAuthSelection,
        caller: &UserId,
        tenant_operator: &UserId,
    ) -> Result<(), ProductOperationFailure> {
        if matches!(auth_selection, HostedMcpAuthSelection::Auto) {
            return Err(ProductOperationFailure::InvalidBindingRequest {
                reason: "hosted MCP auth recovery requires an explicit selection".to_string(),
            });
        }
        let (extension_id, installation_id) =
            crate::product_lifecycle::extension_ids_from_package_ref(package_ref)?;
        let (installation, enriched) = {
            let _guard = self.operation_lock.lock().await;
            let installation = self
                .load_installation(&extension_id, &installation_id)
                .await?;
            crate::ensure_caller_may_operate(&installation, caller)?;
            crate::product_lifecycle::ensure_caller_may_mutate_tenant_installation(
                &installation,
                caller,
                tenant_operator,
                "configure",
            )?;
            let manifest = self
                .installation_store
                .get_manifest(&extension_id)
                .await
                .map_err(crate::product_lifecycle::map_extension_installation_error)?
                .ok_or_else(crate::hosted_mcp_manifest::name_unavailable)?;
            if manifest.manifest().source != ManifestSource::UserRegistered
                || manifest.resolved().has_model_visible_capabilities()
            {
                return Err(ProductOperationFailure::InvalidBindingRequest {
                    reason: "hosted MCP authentication can no longer be changed".to_string(),
                });
            }
            let current_package =
                crate::hosted_mcp_manifest::registered_extension_package(&manifest)?;
            if !package_runtime_credential_auth_requirements(&current_package).is_empty() {
                return Err(ProductOperationFailure::InvalidBindingRequest {
                    reason: "hosted MCP authentication is already configured".to_string(),
                });
            }
            let mcp = manifest
                .resolved()
                .mcp
                .as_ref()
                .ok_or_else(crate::hosted_mcp_manifest::name_unavailable)?;
            if matches!(mcp.registration_auth, HostedMcpAuthSelection::Bearer) {
                return Err(ProductOperationFailure::InvalidBindingRequest {
                    reason: "hosted MCP bearer setup is already selected".to_string(),
                });
            }
            let endpoint = ironclaw_extension_contracts::hosted_mcp::HostedMcpEndpoint::new(
                mcp.server.clone(),
            )
            .map_err(crate::hosted_mcp_manifest::endpoint_input_error)?;
            let endpoint =
                crate::hosted_mcp_admission::CanonicalHostedMcpEndpoint::parse(&endpoint)
                    .map_err(crate::hosted_mcp_manifest::endpoint_admission_error)?;
            let enriched = crate::hosted_mcp_manifest::pending_manifest(
                &extension_id,
                &manifest.resolved().name,
                &endpoint,
                &auth_selection,
            )?;
            (installation, enriched)
        };
        self.checkpoint_prepared_manifest(&extension_id, &installation, enriched)
            .await?;
        Ok(())
    }

    // arch-exempt: too_many_args, mirrors the parameter set `prepare_if_pending`
    // already gathered under its operation-lock guard; missing a
    // HostedMcpPreparationAttempt context struct to own
    // (package_ref, extension_id, scope, installation, manifest), plan #6329
    #[allow(clippy::too_many_arguments)]
    async fn attempt_hosted_mcp_preparation(
        &self,
        package_ref: &LifecyclePackageRef,
        extension_id: &ironclaw_host_api::ids::ExtensionId,
        scope: ResourceScope,
        credential_gate: &dyn ExtensionActivationCredentialGate,
        installation: &ExtensionInstallation,
        manifest: &ExtensionManifestRecord,
        package: &ExtensionPackage,
        max_tools: u32,
        best_effort: bool,
    ) -> Result<Option<LifecycleProductResponse>, ProductOperationFailure> {
        let requirements = package_runtime_credential_auth_requirements(package);
        if let ExtensionActivationCredentialReadiness::Missing(missing) =
            credential_gate.credential_readiness(package).await?
        {
            return Ok(Some(
                crate::product_lifecycle::activation_credentials_incomplete_response(
                    package_ref.clone(),
                    missing,
                )?,
            ));
        }
        let ports = self.discovery_runtime_ports.as_ref().ok_or_else(|| {
            ProductOperationFailure::Transient {
                reason: "hosted MCP catalog preparation runtime is unavailable".to_string(),
            }
        })?;
        let safety = &self.catalog_safety;
        let capability_id = package
            .manifest
            .capabilities
            .first()
            .map(|capability| capability.id.clone())
            .ok_or_else(|| ProductOperationFailure::InvalidBindingRequest {
                reason: "hosted MCP registration has no discovery capability".to_string(),
            })?;
        let network_policy = crate::mcp::hosted_mcp_network_policy(package).ok_or_else(|| {
            ProductOperationFailure::InvalidBindingRequest {
                reason: "hosted MCP discovery endpoint is invalid".to_string(),
            }
        })?;
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
                    extension_id,
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
                    CredentialStageError::Backend => Err(ProductOperationFailure::Transient {
                        reason: "hosted MCP credential staging is temporarily unavailable"
                            .to_string(),
                    }),
                };
            }
        }
        let discovered = match crate::discover_hosted_mcp_package_with_policy(
            package,
            max_tools,
            scope.clone(),
            ports.runtime_http_egress(),
            Some(safety),
        )
        .await
        {
            Ok(discovered) => discovered,
            Err(crate::HostedMcpDiscoveryError::CredentialsRejected(challenge))
                if manifest.manifest().source == ManifestSource::UserRegistered
                    && manifest.resolved().auth.is_empty()
                    && matches!(
                        manifest
                            .resolved()
                            .mcp
                            .as_ref()
                            .map(|mcp| &mcp.registration_auth),
                        Some(ironclaw_extension_contracts::hosted_mcp::HostedMcpAuthSelection::OAuth { .. })
                            | Some(ironclaw_extension_contracts::hosted_mcp::HostedMcpAuthSelection::Auto)
                    ) =>
            {
                let registration_auth = manifest
                    .resolved()
                    .mcp
                    .as_ref()
                    .map(|mcp| mcp.registration_auth.clone());
                let enriched = match registration_auth {
                    Some(ironclaw_extension_contracts::hosted_mcp::HostedMcpAuthSelection::OAuth {
                        client_profile_id,
                    }) => {
                        let Some(enriched) = crate::hosted_mcp_auth_admission::prepare_oauth_manifest(
                                manifest.clone(),
                                &challenge,
                                HostedMcpAuthSelection::OAuth { client_profile_id },
                                &scope,
                                &capability_id,
                                ports,
                                Arc::clone(&self.oauth_client_profiles),
                            )
                            .await?
                        else {
                            return self
                                .reset_auth_selection(
                                    package_ref,
                                    extension_id,
                                    installation,
                                    manifest,
                                    "Hosted MCP OAuth metadata could not be discovered; choose a different authentication method or fix the server metadata.",
                                )
                                .await;
                        };
                        enriched
                    }
                    Some(ironclaw_extension_contracts::hosted_mcp::HostedMcpAuthSelection::Auto) => {
                        let Some(enriched) = crate::hosted_mcp_auth_admission::prepare_oauth_manifest(
                            manifest.clone(),
                            &challenge,
                            HostedMcpAuthSelection::Auto,
                            &scope,
                            &capability_id,
                            ports,
                            Arc::clone(&self.oauth_client_profiles),
                        )
                        .await? else {
                            return crate::hosted_mcp_auth_admission::auth_selection_required_response(
                                package_ref.clone(),
                                "Hosted MCP requires authentication but did not expose usable OAuth metadata; choose OAuth, Bearer token, or No authentication in extension setup.",
                            )
                            .map(Some);
                        };
                        enriched
                    }
                    _ => return Err(crate::hosted_mcp_manifest::name_unavailable()),
                };
                return self
                    .checkpoint_auth_preparation(package_ref, extension_id, installation, enriched)
                    .await;
            }
            Err(crate::HostedMcpDiscoveryError::CredentialsRejected(_))
                if manifest.manifest().source == ManifestSource::UserRegistered
                    && matches!(
                    manifest
                        .resolved()
                        .mcp
                        .as_ref()
                        .map(|mcp| &mcp.registration_auth),
                    Some(ironclaw_extension_contracts::hosted_mcp::HostedMcpAuthSelection::NoAuth)
                ) =>
            {
                return self
                    .reset_auth_selection(
                        package_ref,
                        extension_id,
                        installation,
                        manifest,
                        "Hosted MCP rejected unauthenticated access; choose OAuth or Bearer token in extension setup.",
                    )
                    .await;
            }
            Err(crate::HostedMcpDiscoveryError::CredentialsRejected(_)) => {
                let mut response =
                    crate::product_lifecycle::activation_credentials_incomplete_response(
                        package_ref.clone(),
                        requirements,
                    )?;
                response.message = Some(
                    "Hosted MCP rejected the saved credentials; update or reconnect them and retry activation."
                        .to_string(),
                );
                return Ok(Some(response));
            }
            Err(error) => {
                return Err(crate::hosted_mcp_manifest::discovery_error(error));
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
        // The finalized record carries the discovered model-visible
        // capabilities, so it reads as resolved without a stored flag.
        .map(|record| record.with_definition_retention(manifest.definition_retention()))
        .map_err(crate::product_lifecycle::map_extension_installation_error)?;
        let available = if finalized.manifest().source == ManifestSource::UserRegistered {
            Some(
                self.user_registered_available_package(&finalized, installation)
                    .await?,
            )
        } else {
            None
        };
        if best_effort {
            self.installation_store
                .upsert_manifest_only(
                    installation.installation_id(),
                    installation.incarnation_id(),
                    installation.manifest_ref(),
                    installation.updated_at(),
                    finalized.clone(),
                )
                .await
                .map_err(crate::product_lifecycle::map_extension_installation_error)?;
        } else {
            let incarnation = installation
                .incarnation_id()
                .ok_or_else(crate::hosted_mcp_manifest::name_unavailable)?;
            self.installation_store
                .finalize_preparation(
                    installation.installation_id(),
                    incarnation,
                    installation.manifest_ref(),
                    finalized.clone(),
                )
                .await
                .map_err(crate::product_lifecycle::map_extension_installation_error)?;
        }
        if let Some(available) = available {
            self.catalog
                .write()
                .await
                .extend(AvailableExtensionCatalog::from_packages(vec![available]));
        }
        self.sync_lifecycle_package(extension_id).await?;
        Ok(None)
    }

    async fn checkpoint_auth_preparation(
        &self,
        package_ref: &LifecyclePackageRef,
        extension_id: &ironclaw_host_api::ids::ExtensionId,
        installation: &ExtensionInstallation,
        enriched: ExtensionManifestRecord,
    ) -> Result<Option<LifecycleProductResponse>, ProductOperationFailure> {
        let enriched_package = self
            .checkpoint_prepared_manifest(extension_id, installation, enriched)
            .await?;
        let requirements = package_runtime_credential_auth_requirements(&enriched_package);
        Ok(Some(
            crate::product_lifecycle::activation_credentials_incomplete_response(
                package_ref.clone(),
                requirements,
            )?,
        ))
    }

    async fn reset_auth_selection(
        &self,
        package_ref: &LifecyclePackageRef,
        extension_id: &ironclaw_host_api::ids::ExtensionId,
        installation: &ExtensionInstallation,
        manifest: &ExtensionManifestRecord,
        message: &str,
    ) -> Result<Option<LifecycleProductResponse>, ProductOperationFailure> {
        let mcp = manifest
            .resolved()
            .mcp
            .as_ref()
            .ok_or_else(crate::hosted_mcp_manifest::name_unavailable)?;
        let endpoint =
            ironclaw_extension_contracts::hosted_mcp::HostedMcpEndpoint::new(mcp.server.clone())
                .map_err(crate::hosted_mcp_manifest::endpoint_input_error)?;
        let endpoint = crate::hosted_mcp_admission::CanonicalHostedMcpEndpoint::parse(&endpoint)
            .map_err(crate::hosted_mcp_manifest::endpoint_admission_error)?;
        let unresolved = crate::hosted_mcp_manifest::pending_manifest(
            extension_id,
            &manifest.resolved().name,
            &endpoint,
            &HostedMcpAuthSelection::Auto,
        )?;
        self.checkpoint_prepared_manifest(extension_id, installation, unresolved)
            .await?;
        crate::hosted_mcp_auth_admission::auth_selection_required_response(
            package_ref.clone(),
            message,
        )
        .map(Some)
    }

    async fn checkpoint_prepared_manifest(
        &self,
        extension_id: &ironclaw_host_api::ids::ExtensionId,
        installation: &ExtensionInstallation,
        enriched: ExtensionManifestRecord,
    ) -> Result<ExtensionPackage, ProductOperationFailure> {
        if enriched.manifest().source != ManifestSource::UserRegistered {
            return Err(ProductOperationFailure::InvalidBindingRequest {
                reason: "hosted MCP registration checkpoint requires user-registered provenance"
                    .to_string(),
            });
        }
        let incarnation = installation
            .incarnation_id()
            .ok_or_else(crate::hosted_mcp_manifest::name_unavailable)?;
        let enriched_package = self
            .user_registered_available_package(&enriched, installation)
            .await?;
        self.installation_store
            .checkpoint_preparation(
                installation.installation_id(),
                incarnation,
                installation.manifest_ref(),
                enriched.clone(),
            )
            .await
            .map_err(crate::product_lifecycle::map_extension_installation_error)?;
        let package = enriched_package.package.clone();
        self.catalog
            .write()
            .await
            .extend(AvailableExtensionCatalog::from_packages(vec![
                enriched_package,
            ]));
        self.sync_lifecycle_package(extension_id).await?;
        Ok(package)
    }

    /// Rebuild a user-registered catalog projection from its durable audience.
    ///
    /// Manifest enrichment replaces the package projection during installation
    /// preparation. Reapplying the persisted definition audience here keeps
    /// that installation-only transition from widening definition visibility.
    /// Legacy installations created before definition rows existed derive the
    /// same narrow audience from their installation membership.
    async fn user_registered_available_package(
        &self,
        manifest: &ExtensionManifestRecord,
        installation: &ExtensionInstallation,
    ) -> Result<crate::AvailableExtensionPackage, ProductOperationFailure> {
        let registered = self
            .installation_store
            .get_registered_package_definition(manifest.extension_id())
            .await
            .map_err(crate::product_lifecycle::map_extension_installation_error)?;
        if registered.as_ref().is_some_and(|registered| {
            registered.definition().extension_id() != manifest.extension_id()
                || registered.definition().manifest().source != ManifestSource::UserRegistered
        }) {
            return Err(ProductOperationFailure::InvalidBindingRequest {
                reason: "registered hosted MCP definition does not match its prepared manifest"
                    .to_string(),
            });
        }
        let audience = match registered
            .as_ref()
            .map(RegisteredPackageDefinition::audience)
        {
            Some(PackageDefinitionAudience::Managed(membership)) => {
                PackageDefinitionAudience::Managed(membership.clone())
            }
            Some(PackageDefinitionAudience::LegacyOwnerless) | None => {
                crate::lifecycle_restore::legacy_installed_definition_audience(installation)?
            }
        };
        crate::hosted_mcp_manifest::available_registered_package(manifest, &audience)
    }

    async fn load_installation(
        &self,
        extension_id: &ironclaw_host_api::ids::ExtensionId,
        installation_id: &ExtensionInstallationId,
    ) -> Result<ExtensionInstallation, ProductOperationFailure> {
        let installation = self
            .installation_store
            .get_installation(installation_id)
            .await
            .map_err(crate::product_lifecycle::map_extension_installation_error)?
            .ok_or_else(|| ProductOperationFailure::InvalidBindingRequest {
                reason: format!("extension {} is not installed", extension_id.as_str()),
            })?;
        if installation.extension_id() != extension_id {
            return Err(ProductOperationFailure::InvalidBindingRequest {
                reason: "extension installation identity mismatch".to_string(),
            });
        }
        Ok(installation)
    }

    async fn lifecycle_package(
        &self,
        extension_id: &ironclaw_host_api::ids::ExtensionId,
    ) -> Result<ExtensionPackage, ProductOperationFailure> {
        crate::product_lifecycle::lifecycle_package_from(&self.lifecycle_service, extension_id)
            .await
    }

    async fn sync_lifecycle_package(
        &self,
        extension_id: &ironclaw_host_api::ids::ExtensionId,
    ) -> Result<(), ProductOperationFailure> {
        crate::product_lifecycle::ensure_lifecycle_package_registered(
            &self.installation_store,
            &self.lifecycle_service,
            extension_id,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use chrono::Utc;
    use ironclaw_extension_contracts::hosted_mcp::{HostedMcpAuthSelection, HostedMcpEndpoint};
    use ironclaw_extension_registry::{
        ExtensionInstallation, ExtensionInstallationId, ExtensionInstallationStore,
        ExtensionInstallationStorePort as _, ExtensionLifecycleService, ExtensionManifestRecord,
        ExtensionManifestRef, ExtensionRegistry, InstallationOwner, ManifestSource,
        RegisteredPackageDefinition,
    };
    use ironclaw_filesystem::{InMemoryBackend, RootFilesystem};
    use ironclaw_host_api::{
        ids::{ExtensionId, UserId},
        path::VirtualPath,
    };
    use tokio::sync::{Mutex, RwLock};

    use super::{HostedMcpPreparationDependencies, HostedMcpPreparationService};
    use crate::AvailableExtensionCatalog;

    #[tokio::test]
    async fn projection_failure_does_not_checkpoint_prepared_manifest() {
        let filesystem: Arc<dyn RootFilesystem> = Arc::new(InMemoryBackend::new());
        let store = Arc::new(
            ExtensionInstallationStore::load_at(
                filesystem,
                VirtualPath::new("/system/extensions/.installations/projection-failure")
                    .expect("valid store path"),
                ironclaw_host_api::host_port::default_host_port_catalog()
                    .expect("host port catalog"),
                crate::product_extension_host_api_contract_registry().expect("host API contracts"),
            )
            .await
            .expect("installation store"),
        );
        let extension_id = ExtensionId::new("projection-failure").expect("valid extension id");
        let endpoint = crate::hosted_mcp_admission::CanonicalHostedMcpEndpoint::parse(
            &HostedMcpEndpoint::new("https://mcp.example.test/rpc").expect("valid endpoint"),
        )
        .expect("canonical endpoint");
        let original = crate::hosted_mcp_manifest::pending_manifest(
            &extension_id,
            "Original package",
            &endpoint,
            &HostedMcpAuthSelection::NoAuth,
        )
        .expect("original pending manifest");
        let enriched = crate::hosted_mcp_manifest::pending_manifest(
            &extension_id,
            "Enriched package",
            &endpoint,
            &HostedMcpAuthSelection::NoAuth,
        )
        .expect("enriched pending manifest");
        let conflicting_definition = ExtensionManifestRecord::from_resolved(
            original.raw_toml(),
            ManifestSource::HostBundled,
            original.resolved().clone(),
            original.manifest_hash().cloned(),
        )
        .expect("conflicting definition");
        store
            .admit_package_definition(RegisteredPackageDefinition::managed_by(
                conflicting_definition,
                UserId::new("projection-owner").expect("valid owner"),
            ))
            .await
            .expect("seed conflicting definition");
        let installation = ExtensionInstallation::new(
            ExtensionInstallationId::new(extension_id.as_str()).expect("valid installation id"),
            extension_id.clone(),
            ExtensionManifestRef::new(extension_id.clone(), original.manifest_hash().cloned()),
            Vec::new(),
            Utc::now(),
            InstallationOwner::Tenant,
        )
        .expect("pending installation");
        store
            .upsert_manifest_and_installation(original.clone(), installation.clone())
            .await
            .expect("seed pending aggregate");
        let service = HostedMcpPreparationService::new(
            Arc::clone(&store)
                as Arc<dyn ironclaw_extension_registry::ExtensionInstallationStorePort>,
            Arc::new(RwLock::new(AvailableExtensionCatalog::from_packages(
                Vec::new(),
            ))),
            Arc::new(Mutex::new(ExtensionLifecycleService::new(
                ExtensionRegistry::new(),
            ))),
            Arc::new(Mutex::new(())),
            HostedMcpPreparationDependencies {
                runtime_ports: None,
                catalog_safety: crate::McpCatalogAdmissionPolicy::new(Arc::new(
                    ironclaw_safety::Sanitizer::new(),
                )),
                oauth_client_profiles: Arc::new(ironclaw_auth::EmptyOAuthClientProfileRegistry),
            },
        );

        service
            .checkpoint_prepared_manifest(&extension_id, &installation, enriched)
            .await
            .expect_err("conflicting definition must reject catalog projection");

        let persisted = store
            .get_manifest(&extension_id)
            .await
            .expect("manifest readback")
            .expect("original manifest remains");
        assert_eq!(
            persisted, original,
            "projection failure must not checkpoint durable preparation state"
        );
    }
}
