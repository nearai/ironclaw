//! Hosted MCP package-definition admission and installation preparation.
//!
//! The generic lifecycle knows nothing about discovery: it calls
//! [`HostedMcpPreparationService::prepare_if_pending`] and reads readiness off
//! the package itself. OAuth metadata, discovery, and catalog safety remain
//! private to this concrete strategy.

use std::sync::Arc;

use ironclaw_extension_contracts::hosted_mcp::{HostedMcpAuthSelection, RegisterHostedMcpRequest};
use ironclaw_extensions::{
    ExtensionInstallation, ExtensionInstallationId, ExtensionInstallationStorePort,
    ExtensionLifecycleService, ExtensionManifestRecord, ExtensionPackage, ManifestSource,
};
use ironclaw_host_api::{
    dispatch::CredentialStageError,
    ids::{CapabilityId, UserId},
    resource::ResourceScope,
};
use ironclaw_product_contracts::error::ProductOperationFailure;
use ironclaw_product_contracts::package_lifecycle::{
    LifecyclePackageRef, LifecycleProductResponse,
};
use tokio::sync::{Mutex, RwLock};

use crate::{
    AvailableExtensionCatalog, ExtensionActivationCredentialGate,
    ExtensionActivationCredentialReadiness, package_runtime_credential_auth_requirements,
};
use ironclaw_product_contracts::package_lifecycle::LifecyclePackageKind;

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
        let package_ref = ironclaw_product_contracts::package_lifecycle::LifecyclePackageRef::new(
            LifecyclePackageKind::Extension,
            extension_id.as_str(),
        )?;
        let existing = self
            .installation_store
            .get_registered_package_definition(&extension_id)
            .await
            .map_err(crate::product_lifecycle::map_extension_installation_error)?;
        let definition = if let Some(existing) = existing {
            if !registration_request_matches(&existing, &request, &endpoint) {
                return Err(crate::hosted_mcp_manifest::name_unavailable());
            }
            existing
        } else {
            match request.auth_selection.as_ref() {
                Some(
                    selection @ (HostedMcpAuthSelection::Auto
                    | HostedMcpAuthSelection::OAuth { .. }),
                ) => {
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
                            HostedMcpAuthSelection::Auto => auth_selection_required_response(
                                package_ref,
                                "Hosted MCP requires authentication but did not expose usable OAuth metadata; choose OAuth or Bearer token to finish registration.",
                            ),
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
            }
        };
        // Lock order invariant: catalog write guard BEFORE operation_lock,
        // matching `ExtensionLifecycleManager::import_bundle` /
        // `install` (see product_lifecycle.rs). Both paths share the same
        // catalog and operation_lock, so acquiring them in a consistent
        // order prevents an AB-BA deadlock across concurrent callers.
        let mut catalog = self.catalog.write().await;
        let _guard = self.operation_lock.lock().await;
        self.installation_store
            .admit_package_definition(definition.clone())
            .await
            .map_err(crate::product_lifecycle::map_extension_installation_error)?;
        let available = crate::hosted_mcp_manifest::available_package(&definition)?;
        catalog.extend(AvailableExtensionCatalog::from_packages(vec![available]));
        Ok(crate::hosted_mcp_manifest::registration_response(
            package_ref,
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
        let ports = self.discovery_runtime_ports.as_ref().ok_or_else(|| {
            ProductOperationFailure::Transient {
                reason: "hosted MCP registration runtime is unavailable".to_string(),
            }
        })?;
        let package = crate::hosted_mcp_manifest::available_package(&seed)?.package;
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
                let client_profile_id = match selection {
                    HostedMcpAuthSelection::OAuth { client_profile_id } => {
                        client_profile_id.clone()
                    }
                    HostedMcpAuthSelection::Auto => None,
                    _ => return Err(crate::hosted_mcp_manifest::name_unavailable()),
                };
                self.prepare_oauth_manifest(
                    seed,
                    &challenge,
                    client_profile_id,
                    &scope,
                    &capability_id,
                    ports,
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
            let current_package = crate::hosted_mcp_manifest::available_package(&manifest)?;
            if !package_runtime_credential_auth_requirements(&current_package.package).is_empty() {
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
            .map_err(|_| crate::hosted_mcp_manifest::name_unavailable())?;
            let endpoint =
                crate::hosted_mcp_admission::CanonicalHostedMcpEndpoint::parse(&endpoint)
                    .map_err(|_| crate::hosted_mcp_manifest::name_unavailable())?;
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
                if manifest.resolved().auth.is_empty()
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
                        let Some(enriched) = self
                            .prepare_oauth_manifest(
                                manifest.clone(),
                                &challenge,
                                client_profile_id,
                                &scope,
                                &capability_id,
                                ports,
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
                        let Some(enriched) = self.prepare_oauth_manifest(
                            manifest.clone(),
                            &challenge,
                            None,
                            &scope,
                            &capability_id,
                            ports,
                        )
                        .await? else {
                            return auth_selection_required_response(
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
                if matches!(
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
        if finalized.manifest().source == ManifestSource::UserRegistered {
            self.catalog
                .write()
                .await
                .extend(AvailableExtensionCatalog::from_packages(vec![
                    crate::hosted_mcp_manifest::available_package(&finalized)?,
                ]));
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
                .map_err(|_| crate::hosted_mcp_manifest::name_unavailable())?;
        let endpoint = crate::hosted_mcp_admission::CanonicalHostedMcpEndpoint::parse(&endpoint)
            .map_err(|_| crate::hosted_mcp_manifest::name_unavailable())?;
        let unresolved = crate::hosted_mcp_manifest::pending_manifest(
            extension_id,
            &manifest.resolved().name,
            &endpoint,
            &HostedMcpAuthSelection::Auto,
        )?;
        self.checkpoint_prepared_manifest(extension_id, installation, unresolved)
            .await?;
        auth_selection_required_response(package_ref.clone(), message).map(Some)
    }

    async fn checkpoint_prepared_manifest(
        &self,
        extension_id: &ironclaw_host_api::ids::ExtensionId,
        installation: &ExtensionInstallation,
        enriched: ExtensionManifestRecord,
    ) -> Result<ExtensionPackage, ProductOperationFailure> {
        let incarnation = installation
            .incarnation_id()
            .ok_or_else(crate::hosted_mcp_manifest::name_unavailable)?;
        self.installation_store
            .checkpoint_preparation(
                installation.installation_id(),
                incarnation,
                installation.manifest_ref(),
                enriched.clone(),
            )
            .await
            .map_err(crate::product_lifecycle::map_extension_installation_error)?;
        let enriched_package = crate::hosted_mcp_manifest::available_package(&enriched)?;
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

    async fn prepare_oauth_manifest(
        &self,
        seed: ExtensionManifestRecord,
        challenge: &ironclaw_extension_contracts::hosted_mcp::McpAuthChallenge,
        client_profile_id: Option<String>,
        scope: &ResourceScope,
        capability_id: &CapabilityId,
        ports: &ironclaw_host_runtime::ProductAuthProviderRuntimePorts,
    ) -> Result<Option<ExtensionManifestRecord>, ProductOperationFailure> {
        let endpoint = seed
            .resolved()
            .mcp
            .as_ref()
            .map(|mcp| mcp.server.as_str())
            .ok_or_else(crate::hosted_mcp_manifest::name_unavailable)?;
        let resource_fetches = ironclaw_auth::OAuthRecipeAdmission::<
            ironclaw_auth::EmptyOAuthClientProfileRegistry,
        >::preflight_protected_resource_candidates(
            endpoint, challenge
        )
        .map_err(crate::hosted_mcp_manifest::oauth_admission_error)?;
        let derived = challenge.www_authenticate_metadata.is_empty()
            && challenge.protected_resource_metadata.is_empty();
        for resource_fetch in resource_fetches {
            let Some(resource_metadata): Option<ironclaw_auth::ProtectedResourceAdmissionMetadata> =
                self.fetch_optional_oauth_metadata(
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
                self.fetch_oauth_metadata(
                    ports,
                    scope,
                    capability_id,
                    authorization_server_fetch.metadata_url(),
                )
                .await?;
            let endpoint = crate::hosted_mcp_admission::CanonicalHostedMcpEndpoint::parse(
                &ironclaw_extension_contracts::hosted_mcp::HostedMcpEndpoint::new(endpoint)
                    .map_err(|_| crate::hosted_mcp_manifest::name_unavailable())?,
            )
            .map_err(|_| crate::hosted_mcp_manifest::name_unavailable())?;
            let vendor = crate::hosted_mcp_admission::hosted_mcp_vendor_id(&endpoint)
                .map_err(|_| crate::hosted_mcp_manifest::name_unavailable())?;
            let profiles = Arc::clone(&self.oauth_client_profiles);
            let admitted = ironclaw_auth::OAuthRecipeAdmission::new(SharedOAuthProfiles(profiles))
                .admit(ironclaw_auth::OAuthRecipeAdmissionRequest {
                    vendor: vendor.as_str().to_string(),
                    authorization_server_fetch,
                    authorization_server_metadata,
                    scopes: Vec::new(),
                    client_profile_id,
                })
                .await
                .map_err(crate::hosted_mcp_manifest::oauth_admission_error)?;
            return crate::hosted_mcp_manifest::manifest_with_admitted_oauth(
                seed, &endpoint, admitted,
            )
            .map(Some);
        }
        Ok(None)
    }

    async fn fetch_oauth_metadata<T: serde::de::DeserializeOwned>(
        &self,
        ports: &ironclaw_host_runtime::ProductAuthProviderRuntimePorts,
        scope: &ResourceScope,
        capability_id: &CapabilityId,
        url: &str,
    ) -> Result<T, ProductOperationFailure> {
        self.fetch_optional_oauth_metadata(ports, scope, capability_id, url)
            .await?
            .ok_or_else(invalid_oauth_metadata_response)
    }

    async fn fetch_optional_oauth_metadata<T: serde::de::DeserializeOwned>(
        &self,
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
                // Same shape as the fetch arm above: the `reason` crossing the
                // boundary is fixed text (the document came from a third-party
                // server and must not be echoed), so the parse error is the only
                // thing that says *why* it was malformed. Logged, never dropped.
                tracing::debug!(%error, "hosted MCP OAuth metadata document was malformed");
                ProductOperationFailure::InvalidBindingRequest {
                    reason: "hosted MCP OAuth metadata document was malformed".to_string(),
                }
            })
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

fn invalid_oauth_metadata_response() -> ProductOperationFailure {
    ProductOperationFailure::InvalidBindingRequest {
        reason: "hosted MCP OAuth metadata response was invalid".to_string(),
    }
}

fn auth_selection_required_response(
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
