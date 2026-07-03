use std::{collections::BTreeSet, sync::Arc};

use ironclaw_extensions::{
    CapabilityVisibility, ExtensionActivationState, ExtensionError, ExtensionInstallation,
    ExtensionInstallationError, ExtensionInstallationId, ExtensionInstallationStore,
    ExtensionLifecycleService, ExtensionManifestRecord, ExtensionManifestRef, ExtensionPackage,
    ManifestHash, ManifestSource,
};
use ironclaw_filesystem::RootFilesystem;
use ironclaw_host_api::{
    CapabilityDescriptor, CapabilityId, EffectKind, ExtensionId, PermissionMode, ResourceScope,
    RuntimeCredentialAuthRequirement, RuntimeCredentialRequirement, RuntimeHttpEgress, VirtualPath,
    sha256_digest_token,
};
use ironclaw_product_adapter_registry::PRODUCT_ADAPTER_HOST_API_ID;
use ironclaw_product_workflow::{
    ChannelConnectionRequirement, LifecycleExtensionSummary, LifecycleExtensionSurfaceKind,
    LifecycleInstalledExtensionSummary, LifecyclePackageKind, LifecyclePackageRef, LifecyclePhase,
    LifecycleProductPayload, LifecycleProductResponse, LifecycleSearchExtensionSummary,
    ProductWorkflowError, RebornChannelConnectStrategy,
};
use tokio::sync::Mutex;

mod active_publication;
#[cfg(test)]
mod hosted_mcp_test_support;

use crate::available_extensions::{
    AvailableExtensionCatalog, AvailableExtensionPackage, materialize_available_extension,
    visible_capability_ids,
};
use crate::extension_activation_credentials::{
    ExtensionActivationCredentialGate, RuntimeExtensionActivationCredentialGate,
    UnavailableExtensionActivationCredentialGate,
};
use crate::extension_credential_requirements::package_runtime_credential_auth_requirements;
use crate::lifecycle::response_with_payload;
use crate::mcp_discovery::{
    HostedMcpDiscoveryError, discover_hosted_mcp_package, is_hosted_http_mcp_package,
};

pub(crate) use active_publication::ActiveExtensionPublisher;
#[cfg(test)]
use active_publication::extension_trust_policy_input;

// This port is deliberately scoped to LocalSingleUser composition. The
// lifecycle service models the installed extension set, while active_registry
// is the model-visible capability surface read by host runtime dispatch.
// install/remove keep the lifecycle set durable; activate/remove are the only
// local-dev writers that should mirror lifecycle-managed packages into or out
// of active_registry. Production and multi-tenant reuse require scoped storage
// and registry ownership first; tracked in #4091.
pub(crate) struct RebornLocalExtensionManagementPort {
    filesystem: Arc<dyn RootFilesystem>,
    catalog: AvailableExtensionCatalog,
    installation_store: Arc<dyn ExtensionInstallationStore>,
    lifecycle_service: Arc<Mutex<ExtensionLifecycleService>>,
    active_extensions: ActiveExtensionPublisher,
    operation_lock: Arc<Mutex<()>>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ActiveExtensionCapability {
    pub(crate) id: CapabilityId,
    pub(crate) provider: ExtensionId,
    pub(crate) effects: Vec<EffectKind>,
    pub(crate) default_permission: PermissionMode,
    pub(crate) runtime_credentials: Vec<RuntimeCredentialRequirement>,
}

#[derive(Clone)]
pub(crate) enum ExtensionActivationMode {
    Static,
    HostedMcpDiscovery {
        scope: ResourceScope,
        runtime_http_egress: Arc<dyn RuntimeHttpEgress>,
    },
}

impl ActiveExtensionCapability {
    fn from_descriptor(descriptor: &CapabilityDescriptor) -> Self {
        Self {
            id: descriptor.id.clone(),
            provider: descriptor.provider.clone(),
            effects: descriptor.effects.clone(),
            default_permission: descriptor.default_permission,
            runtime_credentials: descriptor.runtime_credentials.clone(),
        }
    }
}

impl ExtensionActivationMode {
    pub(crate) fn from_dispatch_context(
        scope: ResourceScope,
        runtime_http_egress: Option<Arc<dyn RuntimeHttpEgress>>,
    ) -> Self {
        match runtime_http_egress {
            Some(runtime_http_egress) => Self::HostedMcpDiscovery {
                scope,
                runtime_http_egress,
            },
            None => Self::Static,
        }
    }
}

pub(crate) async fn restore_extension_lifecycle_state(
    catalog: &AvailableExtensionCatalog,
    filesystem: &Arc<dyn RootFilesystem>,
    installation_store: &Arc<dyn ExtensionInstallationStore>,
    lifecycle_service: &Arc<Mutex<ExtensionLifecycleService>>,
    active_extensions: &ActiveExtensionPublisher,
) -> Result<(), ProductWorkflowError> {
    for installation in installation_store
        .list_installations()
        .await
        .map_err(map_extension_installation_error)?
    {
        let package_ref = LifecyclePackageRef::new(
            LifecyclePackageKind::Extension,
            installation.extension_id().as_str(),
        )?;
        let available = catalog.resolve(&package_ref)?;
        if let Err(hash_error) = validate_restored_manifest_hash(&installation, available) {
            migrate_host_bundled_manifest_hash(
                installation_store,
                available,
                &installation,
                hash_error,
            )
            .await?;
        }
        materialize_available_extension(filesystem.as_ref(), available).await?;
        {
            let mut lifecycle = lifecycle_service.lock().await;
            lifecycle
                .install(available.package.clone())
                .await
                .map_err(map_extension_error)?;
            match installation.activation_state() {
                ExtensionActivationState::Enabled => {
                    lifecycle
                        .enable(&available.package.id)
                        .await
                        .map_err(map_extension_error)?;
                }
                ExtensionActivationState::Installed | ExtensionActivationState::Disabled => {
                    lifecycle
                        .disable(&available.package.id)
                        .await
                        .map_err(map_extension_error)?;
                }
            }
        }
        if installation.activation_state() == ExtensionActivationState::Enabled {
            active_extensions.publish(&available.package)?;
        }
    }
    Ok(())
}

impl RebornLocalExtensionManagementPort {
    pub(crate) fn new(
        filesystem: Arc<dyn RootFilesystem>,
        catalog: AvailableExtensionCatalog,
        installation_store: Arc<dyn ExtensionInstallationStore>,
        lifecycle_service: Arc<Mutex<ExtensionLifecycleService>>,
        active_extensions: ActiveExtensionPublisher,
    ) -> Self {
        Self {
            filesystem,
            catalog,
            installation_store,
            lifecycle_service,
            active_extensions,
            operation_lock: Arc::new(Mutex::new(())),
        }
    }

    /// Test-support access to the extension installation store.
    ///
    /// Mirrors the `installation_store` field that `build_local_runtime` wires
    /// in when constructing `RebornLocalExtensionManagementPort`. For tests
    /// only — zero bytes shipped in production builds.
    #[cfg(feature = "test-support")]
    pub(crate) fn installation_store_for_test(&self) -> Arc<dyn ExtensionInstallationStore> {
        Arc::clone(&self.installation_store)
    }

    /// C-JOURNEY: test-support access to the active-extension publisher
    /// (registry + trust policy). `activate()` ultimately delegates the
    /// model-visible-surface mutation to `self.active_extensions.publish(..)`
    /// (see `active_publication.rs`) after its own install/credential-gate
    /// bookkeeping; this accessor reaches that SAME publish step directly so a
    /// test harness can make a bundled first-party WASM package (e.g. github)
    /// genuinely dispatchable without driving the full multi-turn
    /// install→activate capability handshake through the model. For tests
    /// only — zero bytes shipped in production builds.
    #[cfg(feature = "test-support")]
    pub(crate) fn active_extensions_for_test(&self) -> &ActiveExtensionPublisher {
        &self.active_extensions
    }

    pub(crate) async fn search(
        &self,
        query: &str,
        credential_gate: Option<&RuntimeExtensionActivationCredentialGate>,
    ) -> Result<LifecycleProductResponse, ProductWorkflowError> {
        let extensions = self.catalog.search(query);
        let mut summaries = Vec::new();
        for extension in extensions {
            summaries.push(self.search_summary(extension, credential_gate).await?);
        }
        let count = summaries.len();
        let mut response = response_with_payload(
            None,
            LifecyclePhase::Discovered,
            LifecycleProductPayload::ExtensionSearch {
                extensions: summaries,
                count,
            },
        );
        if extension_search_has_installed_external_channel_result(response.payload.as_ref()) {
            response.message = Some(
                "Search found installed external channel results. Search cannot prove the calling user's channel account is personally connected, so do not treat those results as ready for delivery or message access. Call builtin.extension_activate now for the matching extension id; activation surfaces the channel-specific pairing/setup instructions and, for proof-code flows, the user must paste the code into the WebChat connection panel rather than normal chat."
                    .to_string(),
            );
        } else if extension_search_has_ready_result(response.payload.as_ref()) {
            response.message = Some(
                "Search found installed extension results that are already configured or active. Treat those results as ready for this connection request; do not ask the user for credentials unless a later tool call reports auth_required."
                    .to_string(),
            );
        }
        Ok(response)
    }

    pub(crate) async fn list_installed(
        &self,
    ) -> Result<LifecycleProductResponse, ProductWorkflowError> {
        let summaries = self.installed_summaries().await?;
        let count = summaries.len();
        Ok(response_with_payload(
            None,
            LifecyclePhase::Installed,
            LifecycleProductPayload::ExtensionList {
                extensions: summaries,
                count,
            },
        ))
    }

    pub(crate) async fn project(
        &self,
        package_ref: LifecyclePackageRef,
    ) -> Result<LifecycleProductResponse, ProductWorkflowError> {
        let (_, installation_id) = extension_ids_from_package_ref(&package_ref)?;
        let phase = self
            .installation_store
            .get_installation(&installation_id)
            .await
            .map_err(map_extension_installation_error)?
            .map(|installation| phase_for_activation_state(installation.activation_state()))
            .unwrap_or(LifecyclePhase::Discovered);
        let summary = self.catalog.resolve(&package_ref)?.summary();
        Ok(response_with_payload(
            Some(package_ref),
            phase,
            LifecycleProductPayload::ExtensionList {
                extensions: vec![LifecycleInstalledExtensionSummary { summary, phase }],
                count: 1,
            },
        ))
    }

    pub(crate) async fn active_model_visible_capabilities(
        &self,
    ) -> Result<Vec<ActiveExtensionCapability>, ProductWorkflowError> {
        let enabled_extension_ids = self
            .installation_store
            .list_enabled_installations()
            .await
            .map_err(map_extension_installation_error)?
            .into_iter()
            .map(|installation| installation.extension_id().clone())
            .collect::<BTreeSet<_>>();
        let registry = self.active_extensions.snapshot();
        Ok(registry
            .capabilities()
            .filter(|descriptor| enabled_extension_ids.contains(&descriptor.provider))
            .filter(|descriptor| {
                registry
                    .capability_visibility(&descriptor.id)
                    .unwrap_or(CapabilityVisibility::Model)
                    == CapabilityVisibility::Model
            })
            .map(ActiveExtensionCapability::from_descriptor)
            .collect())
    }

    pub(crate) async fn activation_credential_requirements(
        &self,
        package_ref: &LifecyclePackageRef,
    ) -> Result<Vec<RuntimeCredentialAuthRequirement>, ProductWorkflowError> {
        let (extension_id, installation_id) = extension_ids_from_package_ref(package_ref)?;
        let _operation_guard = self.operation_lock.lock().await;
        self.load_installation(&extension_id, &installation_id)
            .await?;
        let package = self.lifecycle_package(&extension_id).await?;
        Ok(package_runtime_credential_auth_requirements(&package))
    }

    async fn installed_summaries(
        &self,
    ) -> Result<Vec<LifecycleInstalledExtensionSummary>, ProductWorkflowError> {
        let installations = self
            .installation_store
            .list_installations()
            .await
            .map_err(map_extension_installation_error)?;
        let mut summaries = Vec::with_capacity(installations.len());
        for installation in installations {
            let Ok(package_ref) = LifecyclePackageRef::new(
                LifecyclePackageKind::Extension,
                installation.extension_id().as_str(),
            ) else {
                continue;
            };
            let Ok(available) = self.catalog.resolve(&package_ref) else {
                continue;
            };
            summaries.push(LifecycleInstalledExtensionSummary {
                summary: available.summary(),
                phase: phase_for_activation_state(installation.activation_state()),
            });
        }
        Ok(summaries)
    }

    async fn search_summary(
        &self,
        extension: &AvailableExtensionPackage,
        credential_gate: Option<&RuntimeExtensionActivationCredentialGate>,
    ) -> Result<LifecycleSearchExtensionSummary, ProductWorkflowError> {
        let mut summary = extension.summary();
        suppress_search_credential_onboarding(&mut summary);
        let Some(installation) = self.search_installation(&extension.package.id).await? else {
            return Ok(LifecycleSearchExtensionSummary {
                summary,
                installation_phase: None,
            });
        };
        let phase = search_installation_phase(extension, &installation, credential_gate).await?;
        Ok(LifecycleSearchExtensionSummary {
            summary,
            installation_phase: Some(phase),
        })
    }

    async fn search_installation(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<Option<ExtensionInstallation>, ProductWorkflowError> {
        let installation_id = ExtensionInstallationId::new(extension_id.as_str().to_string())
            .map_err(map_extension_installation_error)?;
        let installation = self
            .installation_store
            .get_installation(&installation_id)
            .await
            .map_err(map_extension_installation_error)?;
        if installation
            .as_ref()
            .is_some_and(|installation| installation.extension_id() != extension_id)
        {
            return Err(ProductWorkflowError::InvalidBindingRequest {
                reason: format!(
                    "installation {} does not belong to extension {}",
                    installation_id.as_str(),
                    extension_id.as_str()
                ),
            });
        }
        Ok(installation)
    }

    pub(crate) async fn install(
        &self,
        package_ref: LifecyclePackageRef,
    ) -> Result<LifecycleProductResponse, ProductWorkflowError> {
        let available = self.catalog.resolve(&package_ref)?;
        let plan = prepare_install(available)?;
        let _operation_guard = self.operation_lock.lock().await;
        self.ensure_not_installed(&available.package.id, plan.installation.installation_id())
            .await?;
        self.register_lifecycle_package(&available.package).await?;

        if let Err(error) =
            materialize_available_extension(self.filesystem.as_ref(), available).await
        {
            if let Err(rollback_error) =
                self.rollback_lifecycle_install(&available.package.id).await
            {
                return Err(compensation_failure(
                    "extension install materialization failed and lifecycle rollback failed",
                    error,
                    rollback_error,
                ));
            }
            return Err(error);
        }
        if let Err(error) = self.persist_install_plan(plan).await {
            let _ = self
                .delete_materialized_extension_files(&available.package.id)
                .await;
            if let Err(rollback_error) =
                self.rollback_lifecycle_install(&available.package.id).await
            {
                return Err(compensation_failure(
                    "extension install persistence failed and lifecycle rollback failed",
                    error,
                    rollback_error,
                ));
            }
            return Err(error);
        }

        Ok(response_with_payload(
            Some(package_ref.clone()),
            LifecyclePhase::Installed,
            LifecycleProductPayload::ExtensionInstall {
                installed: true,
                visible_capability_ids: visible_capability_ids(available)
                    .map(|id| id.as_str().to_string())
                    .collect(),
                next_step: format!(
                    "Call builtin.extension_activate now with input {{\"extension_id\":\"{}\"}}. Activation publishes the tools and opens the auth gate if credentials are missing.",
                    package_ref.id.as_str()
                ),
            },
        ))
    }

    pub(crate) async fn activate(
        &self,
        package_ref: LifecyclePackageRef,
        mode: ExtensionActivationMode,
    ) -> Result<LifecycleProductResponse, ProductWorkflowError> {
        let credential_gate = UnavailableExtensionActivationCredentialGate;
        self.activate_inner(package_ref, mode, &credential_gate)
            .await
    }

    pub(crate) async fn activate_with_credential_gate(
        &self,
        package_ref: LifecyclePackageRef,
        mode: ExtensionActivationMode,
        credential_gate: impl ExtensionActivationCredentialGate,
    ) -> Result<LifecycleProductResponse, ProductWorkflowError> {
        self.activate_inner(package_ref, mode, &credential_gate)
            .await
    }

    #[cfg(test)]
    pub(crate) async fn activate_with_prechecked_credentials_for_test(
        &self,
        package_ref: LifecyclePackageRef,
        mode: ExtensionActivationMode,
    ) -> Result<LifecycleProductResponse, ProductWorkflowError> {
        let credential_gate =
            crate::extension_activation_credentials::PrecheckedExtensionActivationCredentialGate;
        self.activate_inner(package_ref, mode, &credential_gate)
            .await
    }

    async fn activate_inner(
        &self,
        package_ref: LifecyclePackageRef,
        mode: ExtensionActivationMode,
        credential_gate: &dyn ExtensionActivationCredentialGate,
    ) -> Result<LifecycleProductResponse, ProductWorkflowError> {
        let (extension_id, installation_id) = extension_ids_from_package_ref(&package_ref)?;

        let discovery = {
            let _operation_guard = self.operation_lock.lock().await;
            let installation = self
                .load_installation(&extension_id, &installation_id)
                .await?;
            let package = self.lifecycle_package(&extension_id).await?;
            credential_gate.ensure_credentials(&package).await?;
            match mode {
                ExtensionActivationMode::HostedMcpDiscovery {
                    scope,
                    runtime_http_egress,
                } if is_hosted_http_mcp_package(&package) => HostedMcpDiscoveryRequest {
                    base_package: package,
                    scope,
                    runtime_http_egress,
                },
                _ => {
                    return self
                        .commit_activation(
                            package_ref,
                            &extension_id,
                            &installation_id,
                            installation.activation_state(),
                            package,
                        )
                        .await;
                }
            }
        };

        let active_package = match discover_hosted_mcp_package(
            &discovery.base_package,
            discovery.scope,
            discovery.runtime_http_egress,
        )
        .await
        {
            Ok(active_package) => active_package,
            Err(HostedMcpDiscoveryError::Transient(reason)) => {
                tracing::debug!(
                    extension_id = %extension_id.as_str(),
                    reason,
                    "hosted MCP discovery failed during activation; falling back to bundled manifest"
                );
                discovery.base_package.clone()
            }
            Err(error @ HostedMcpDiscoveryError::Permanent(_)) => {
                return Err(hosted_mcp_discovery_error(error));
            }
        };

        let _operation_guard = self.operation_lock.lock().await;
        let installation = self
            .load_installation(&extension_id, &installation_id)
            .await
            .map_err(|_| hosted_mcp_changed_during_discovery_error())?;
        let current_package = self
            .lifecycle_package(&extension_id)
            .await
            .map_err(|_| hosted_mcp_changed_during_discovery_error())?;
        if current_package != discovery.base_package {
            return Err(hosted_mcp_changed_during_discovery_error());
        };
        credential_gate.ensure_credentials(&active_package).await?;
        self.commit_activation(
            package_ref,
            &extension_id,
            &installation_id,
            installation.activation_state(),
            active_package,
        )
        .await
    }

    async fn commit_activation(
        &self,
        package_ref: LifecyclePackageRef,
        extension_id: &ExtensionId,
        installation_id: &ExtensionInstallationId,
        previous_state: ExtensionActivationState,
        active_package: ExtensionPackage,
    ) -> Result<LifecycleProductResponse, ProductWorkflowError> {
        self.enable_lifecycle_package(extension_id).await?;
        if let Err(error) = self
            .installation_store
            .set_activation_state(installation_id, ExtensionActivationState::Enabled)
            .await
        {
            self.disable_lifecycle_package(extension_id).await;
            return Err(map_extension_installation_error(error));
        }
        if let Err(error) = self.active_extensions.publish(&active_package) {
            if previous_state != ExtensionActivationState::Enabled {
                self.disable_lifecycle_package(extension_id).await;
            }
            if let Err(cleanup_error) = self
                .installation_store
                .set_activation_state(installation_id, previous_state)
                .await
            {
                return Err(compensation_failure(
                    "extension activation failed to publish active package and activation restore failed",
                    error,
                    map_extension_installation_error(cleanup_error),
                ));
            }
            return Err(error);
        }

        let visible_capability_ids = package_visible_capability_ids(&active_package);
        let message =
            activation_success_message(&package_ref, &active_package, &visible_capability_ids);
        // For an inbound-channel extension, attach the structured connect
        // requirement so WebChat can render the in-chat pairing panel from
        // structured state (the activation message is model guidance only).
        let connection_required = if package_declares_inbound_product_adapter(&active_package) {
            Some(channel_connection_requirement(
                package_ref.id.as_str(),
                active_package.manifest.name.as_str(),
            ))
        } else {
            None
        };

        let mut response = response_with_payload(
            Some(package_ref),
            LifecyclePhase::Active,
            LifecycleProductPayload::ExtensionActivate {
                activated: true,
                visible_capability_ids,
                connection_required,
            },
        );
        response.message = Some(message);
        Ok(response)
    }

    pub(crate) async fn package_requires_hosted_mcp_discovery(
        &self,
        package_ref: &LifecyclePackageRef,
    ) -> Result<bool, ProductWorkflowError> {
        let (extension_id, _) = extension_ids_from_package_ref(package_ref)?;
        let _operation_guard = self.operation_lock.lock().await;
        let package = self.lifecycle_package(&extension_id).await?;
        Ok(is_hosted_http_mcp_package(&package))
    }

    pub(crate) async fn remove(
        &self,
        package_ref: LifecyclePackageRef,
    ) -> Result<LifecycleProductResponse, ProductWorkflowError> {
        let (extension_id, installation_id) = extension_ids_from_package_ref(&package_ref)?;
        let _operation_guard = self.operation_lock.lock().await;
        let installation = self
            .load_installation(&extension_id, &installation_id)
            .await?;
        let manifest = self
            .installation_store
            .get_manifest(&extension_id)
            .await
            .map_err(map_extension_installation_error)?
            .ok_or_else(|| ProductWorkflowError::InvalidBindingRequest {
                reason: format!(
                    "extension {} manifest is not installed",
                    extension_id.as_str()
                ),
            })?;
        let previous_state = installation.activation_state();
        let lifecycle_package = self.lifecycle_package(&extension_id).await?;
        if let Err(error) = self
            .installation_store
            .set_activation_state(&installation_id, ExtensionActivationState::Disabled)
            .await
        {
            return Err(map_extension_installation_error(error));
        }
        if let Err(error) = self.remove_lifecycle_package(&extension_id).await {
            if let Err(cleanup_error) = self
                .installation_store
                .set_activation_state(&installation_id, previous_state)
                .await
            {
                return Err(compensation_failure(
                    "extension remove failed to remove lifecycle package and activation restore failed",
                    error,
                    map_extension_installation_error(cleanup_error),
                ));
            }
            return Err(error);
        }
        if let Err(error) = self.active_extensions.unpublish(&lifecycle_package) {
            if let Err(restore_error) = self
                .restore_lifecycle_package(&lifecycle_package, previous_state)
                .await
            {
                return Err(compensation_failure(
                    "extension remove failed to unpublish active package and lifecycle restore failed",
                    error,
                    restore_error,
                ));
            }
            if let Err(cleanup_error) = self
                .installation_store
                .set_activation_state(&installation_id, previous_state)
                .await
            {
                return Err(compensation_failure(
                    "extension remove failed to unpublish active package and activation restore failed",
                    error,
                    map_extension_installation_error(cleanup_error),
                ));
            }
            return Err(error);
        }

        if let Err(error) = self
            .installation_store
            .delete_installation(&installation_id)
            .await
        {
            let original_error = map_extension_installation_error(error);
            if let Err(restore_error) = self
                .restore_lifecycle_package(&lifecycle_package, previous_state)
                .await
            {
                return Err(compensation_failure(
                    "extension remove failed to delete installation and lifecycle restore failed",
                    original_error,
                    restore_error,
                ));
            }
            if let Err(restore_error) =
                self.restore_active_publication(&lifecycle_package, previous_state)
            {
                return Err(compensation_failure(
                    "extension remove failed to delete installation and active publication restore failed",
                    original_error,
                    restore_error,
                ));
            }
            if let Err(restore_error) = self
                .installation_store
                .set_activation_state(&installation_id, previous_state)
                .await
                .map_err(map_extension_installation_error)
            {
                return Err(compensation_failure(
                    "extension remove failed to delete installation and activation restore failed",
                    original_error,
                    restore_error,
                ));
            }
            return Err(original_error);
        }
        if let Err(error) = self.installation_store.delete_manifest(&extension_id).await {
            let original_error = map_extension_installation_error(error);
            if let Err(restore_error) = self
                .restore_lifecycle_package(&lifecycle_package, previous_state)
                .await
            {
                return Err(compensation_failure(
                    "extension remove failed to delete manifest and lifecycle restore failed",
                    original_error,
                    restore_error,
                ));
            }
            if let Err(restore_error) =
                self.restore_active_publication(&lifecycle_package, previous_state)
            {
                return Err(compensation_failure(
                    "extension remove failed to delete manifest and active publication restore failed",
                    original_error,
                    restore_error,
                ));
            }
            if let Err(restore_error) = self.restore_installation(&installation).await {
                return Err(compensation_failure(
                    "extension remove failed to delete manifest and installation restore failed",
                    original_error,
                    restore_error,
                ));
            }
            return Err(original_error);
        }
        if let Err(error) = self
            .delete_materialized_extension_files(&extension_id)
            .await
        {
            if let Err(restore_error) = self
                .restore_lifecycle_package(&lifecycle_package, previous_state)
                .await
            {
                return Err(compensation_failure(
                    "extension remove failed to delete files and lifecycle restore failed",
                    error,
                    restore_error,
                ));
            }
            if let Err(restore_error) =
                self.restore_active_publication(&lifecycle_package, previous_state)
            {
                return Err(compensation_failure(
                    "extension remove failed to delete files and active publication restore failed",
                    error,
                    restore_error,
                ));
            }
            if let Err(restore_error) = self
                .restore_installation_records(manifest, installation)
                .await
            {
                return Err(compensation_failure(
                    "extension remove failed to delete files and installation restore failed",
                    error,
                    restore_error,
                ));
            }
            return Err(error);
        }

        Ok(response_with_payload(
            Some(package_ref),
            LifecyclePhase::Removed,
            LifecycleProductPayload::ExtensionRemove { removed: true },
        ))
    }

    async fn register_lifecycle_package(
        &self,
        package: &ExtensionPackage,
    ) -> Result<(), ProductWorkflowError> {
        let mut lifecycle = self.lifecycle_service.lock().await;
        if lifecycle.registry().get_extension(&package.id).is_some() {
            return Err(ProductWorkflowError::InvalidBindingRequest {
                reason: format!("extension {} is already installed", package.id.as_str()),
            });
        }
        lifecycle
            .install(package.clone())
            .await
            .map_err(map_extension_error)?;
        Ok(())
    }

    async fn ensure_not_installed(
        &self,
        extension_id: &ExtensionId,
        installation_id: &ExtensionInstallationId,
    ) -> Result<(), ProductWorkflowError> {
        if self
            .installation_store
            .get_installation(installation_id)
            .await
            .map_err(map_extension_installation_error)?
            .is_some()
        {
            return Err(ProductWorkflowError::InvalidBindingRequest {
                reason: format!("extension {} is already installed", extension_id.as_str()),
            });
        }
        if self
            .installation_store
            .get_manifest(extension_id)
            .await
            .map_err(map_extension_installation_error)?
            .is_some()
        {
            return Err(ProductWorkflowError::InvalidBindingRequest {
                reason: format!("extension {} is already installed", extension_id.as_str()),
            });
        }
        Ok(())
    }

    async fn load_installation(
        &self,
        extension_id: &ExtensionId,
        installation_id: &ExtensionInstallationId,
    ) -> Result<ExtensionInstallation, ProductWorkflowError> {
        let installation = self
            .installation_store
            .get_installation(installation_id)
            .await
            .map_err(map_extension_installation_error)?
            .ok_or_else(|| ProductWorkflowError::InvalidBindingRequest {
                reason: format!("extension {} is not installed", extension_id.as_str()),
            })?;
        if installation.extension_id() != extension_id {
            return Err(ProductWorkflowError::InvalidBindingRequest {
                reason: format!(
                    "installation {} does not belong to extension {}",
                    installation_id.as_str(),
                    extension_id.as_str()
                ),
            });
        }
        Ok(installation)
    }

    async fn lifecycle_package(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<ExtensionPackage, ProductWorkflowError> {
        let lifecycle = self.lifecycle_service.lock().await;
        lifecycle
            .registry()
            .get_extension(extension_id)
            .cloned()
            .ok_or_else(|| ProductWorkflowError::InvalidBindingRequest {
                reason: format!("extension {} is not installed", extension_id.as_str()),
            })
    }

    async fn enable_lifecycle_package(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<(), ProductWorkflowError> {
        self.lifecycle_service
            .lock()
            .await
            .enable(extension_id)
            .await
            .map_err(map_extension_error)
    }

    async fn disable_lifecycle_package(&self, extension_id: &ExtensionId) {
        let _ = self
            .lifecycle_service
            .lock()
            .await
            .disable(extension_id)
            .await;
    }

    async fn remove_lifecycle_package(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<(), ProductWorkflowError> {
        self.lifecycle_service
            .lock()
            .await
            .remove(extension_id)
            .await
            .map_err(map_extension_error)
    }

    async fn rollback_lifecycle_install(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<(), ProductWorkflowError> {
        let mut lifecycle = self.lifecycle_service.lock().await;
        lifecycle
            .remove(extension_id)
            .await
            .map_err(map_extension_error)
    }

    async fn restore_lifecycle_package(
        &self,
        package: &ExtensionPackage,
        previous_state: ExtensionActivationState,
    ) -> Result<(), ProductWorkflowError> {
        let mut lifecycle = self.lifecycle_service.lock().await;
        lifecycle
            .install(package.clone())
            .await
            .map_err(map_extension_error)?;
        match previous_state {
            ExtensionActivationState::Enabled => {
                lifecycle
                    .enable(&package.id)
                    .await
                    .map_err(map_extension_error)?;
            }
            ExtensionActivationState::Installed | ExtensionActivationState::Disabled => {
                lifecycle
                    .disable(&package.id)
                    .await
                    .map_err(map_extension_error)?;
            }
        }
        Ok(())
    }

    async fn restore_installation(
        &self,
        installation: &ExtensionInstallation,
    ) -> Result<(), ProductWorkflowError> {
        self.installation_store
            .upsert_installation(installation.clone())
            .await
            .map_err(map_extension_installation_error)
    }

    async fn restore_installation_records(
        &self,
        manifest: ExtensionManifestRecord,
        installation: ExtensionInstallation,
    ) -> Result<(), ProductWorkflowError> {
        self.installation_store
            .upsert_manifest(manifest)
            .await
            .map_err(map_extension_installation_error)?;
        self.installation_store
            .upsert_installation(installation)
            .await
            .map_err(map_extension_installation_error)
    }

    fn restore_active_publication(
        &self,
        package: &ExtensionPackage,
        previous_state: ExtensionActivationState,
    ) -> Result<(), ProductWorkflowError> {
        if previous_state == ExtensionActivationState::Enabled {
            self.active_extensions.publish(package)?;
        }
        Ok(())
    }

    async fn persist_install_plan(
        &self,
        plan: ExtensionInstallPlan,
    ) -> Result<(), ProductWorkflowError> {
        let extension_id = plan.installation.extension_id().clone();
        if let Err(error) = self
            .installation_store
            .upsert_manifest(plan.manifest_record)
            .await
        {
            return Err(map_extension_installation_error(error));
        }
        if let Err(error) = self
            .installation_store
            .upsert_installation(plan.installation)
            .await
        {
            let _ = self.installation_store.delete_manifest(&extension_id).await;
            return Err(map_extension_installation_error(error));
        }
        Ok(())
    }

    async fn delete_materialized_extension_files(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<(), ProductWorkflowError> {
        let Ok(extension_root) =
            VirtualPath::new(format!("/system/extensions/{}", extension_id.as_str()))
        else {
            return Ok(());
        };
        self.filesystem
            .delete(&extension_root)
            .await
            .map_err(|error| ProductWorkflowError::Transient {
                reason: format!("failed to remove extension files: {error}"),
            })
    }
}

struct HostedMcpDiscoveryRequest {
    base_package: ExtensionPackage,
    scope: ResourceScope,
    runtime_http_egress: Arc<dyn RuntimeHttpEgress>,
}

struct ExtensionInstallPlan {
    manifest_record: ExtensionManifestRecord,
    installation: ExtensionInstallation,
}

fn prepare_install(
    available: &AvailableExtensionPackage,
) -> Result<ExtensionInstallPlan, ProductWorkflowError> {
    let manifest_hash = available_manifest_hash(available)?;
    let host_ports = ironclaw_host_runtime::default_host_port_catalog().map_err(|error| {
        ProductWorkflowError::InvalidBindingRequest {
            reason: format!("host port catalog rejected extension install: {error}"),
        }
    })?;
    let contracts =
        ironclaw_host_runtime::default_host_api_contract_registry().map_err(|error| {
            ProductWorkflowError::InvalidBindingRequest {
                reason: format!("host API contract registry rejected extension install: {error}"),
            }
        })?;
    let manifest_record = ExtensionManifestRecord::from_toml_with_contracts(
        &available.manifest_toml,
        ManifestSource::HostBundled,
        &host_ports,
        Some(manifest_hash.clone()),
        &contracts,
    )
    .map_err(map_extension_installation_error)?;
    let installation_id = ExtensionInstallationId::new(available.package.id.as_str().to_string())
        .map_err(map_extension_installation_error)?;
    let installation = ExtensionInstallation::new(
        installation_id,
        available.package.id.clone(),
        ExtensionActivationState::Installed,
        ExtensionManifestRef::new(available.package.id.clone(), Some(manifest_hash)),
        Vec::new(),
        chrono::Utc::now(),
    )
    .map_err(map_extension_installation_error)?;
    Ok(ExtensionInstallPlan {
        manifest_record,
        installation,
    })
}

/// Build an [`ExtensionInstallPlan`] that carries the new manifest hash from `available`
/// while preserving the activation state and credential bindings from `existing`.
/// Used during restore to migrate a stored installation when the bundled manifest changes.
fn prepare_manifest_migration(
    available: &AvailableExtensionPackage,
    existing: &ExtensionInstallation,
) -> Result<ExtensionInstallPlan, ProductWorkflowError> {
    let manifest_hash = available_manifest_hash(available)?;
    let host_ports = ironclaw_host_runtime::default_host_port_catalog().map_err(|error| {
        ProductWorkflowError::InvalidBindingRequest {
            reason: format!("host port catalog rejected manifest migration: {error}"),
        }
    })?;
    let contracts =
        ironclaw_host_runtime::default_host_api_contract_registry().map_err(|error| {
            ProductWorkflowError::InvalidBindingRequest {
                reason: format!("host API contract registry rejected manifest migration: {error}"),
            }
        })?;
    let manifest_record = ExtensionManifestRecord::from_toml_with_contracts(
        &available.manifest_toml,
        ManifestSource::HostBundled,
        &host_ports,
        Some(manifest_hash.clone()),
        &contracts,
    )
    .map_err(map_extension_installation_error)?;
    let installation = ExtensionInstallation::new(
        existing.installation_id().clone(),
        existing.extension_id().clone(),
        existing.activation_state(),
        ExtensionManifestRef::new(existing.extension_id().clone(), Some(manifest_hash)),
        existing.credential_bindings().to_vec(),
        chrono::Utc::now(),
    )
    .map_err(map_extension_installation_error)?;
    Ok(ExtensionInstallPlan {
        manifest_record,
        installation,
    })
}

async fn migrate_host_bundled_manifest_hash(
    installation_store: &Arc<dyn ExtensionInstallationStore>,
    available: &AvailableExtensionPackage,
    installation: &ExtensionInstallation,
    hash_error: ProductWorkflowError,
) -> Result<(), ProductWorkflowError> {
    let stored_manifest = match installation_store
        .get_manifest(installation.extension_id())
        .await
        .map_err(map_extension_installation_error)?
    {
        Some(stored_manifest) => stored_manifest,
        None => return Err(hash_error),
    };
    if stored_manifest.manifest().source != ManifestSource::HostBundled {
        return Err(hash_error);
    }

    // For host-bundled (first-party) extensions, a manifest hash mismatch means
    // the binary was updated and the bundled manifest changed. Migrate the stored
    // records to the new hash while preserving activation state and bindings.
    tracing::warn!(
        extension_id = %installation.extension_id(),
        "bundled extension manifest hash changed; migrating stored installation to new manifest hash"
    );
    let migration_plan = prepare_manifest_migration(available, installation)?;
    installation_store
        .upsert_manifest_and_installation(
            migration_plan.manifest_record,
            migration_plan.installation,
        )
        .await
        .map_err(map_extension_installation_error)
}

fn validate_restored_manifest_hash(
    installation: &ExtensionInstallation,
    available: &AvailableExtensionPackage,
) -> Result<(), ProductWorkflowError> {
    let manifest_hash = available_manifest_hash(available)?;
    match installation.manifest_ref().manifest_hash() {
        Some(installed_hash) if installed_hash == &manifest_hash => Ok(()),
        _ => Err(map_extension_installation_error(
            ExtensionInstallationError::ManifestHashMismatch {
                extension_id: installation.extension_id().clone(),
            },
        )),
    }
}

fn available_manifest_hash(
    available: &AvailableExtensionPackage,
) -> Result<ManifestHash, ProductWorkflowError> {
    ManifestHash::new(sha256_digest_token(available.manifest_toml.as_bytes()))
        .map_err(map_extension_installation_error)
}

fn package_visible_capability_ids(package: &ExtensionPackage) -> Vec<String> {
    package
        .manifest
        .capabilities
        .iter()
        .filter(|capability| capability.visibility == CapabilityVisibility::Model)
        .map(|capability| capability.id.as_str().to_string())
        .collect()
}

fn activation_success_message(
    package_ref: &LifecyclePackageRef,
    package: &ExtensionPackage,
    visible_capability_ids: &[String],
) -> String {
    if package_declares_inbound_product_adapter(package) {
        if package_ref.id.as_str() == "slack" {
            return "Slack is installed as an inbound channel. If WebChat shows a Slack account connection panel, tell the user to DM the Slack app; the bot will reply with a pairing code. The user should paste that code into the Slack account connection panel in WebChat, not into normal chat. If the user's Slack account is already connected, continue the user's original request instead of asking them to pair again. Do not claim Slack message-reading tools are available unless a separate Slack read capability is installed.".to_string();
        }
        return format!(
            "{} is installed as an external channel. If WebChat shows a channel connection panel, tell the user to open the extension's app or bot, get the pairing code or connection challenge, and paste it into the WebChat connection panel rather than normal chat. If the user's channel account is already connected, continue the user's original request instead of asking them to pair again. Do not claim the channel can receive or send messages for the user until connection is confirmed.",
            package.manifest.name.as_str()
        );
    }
    if visible_capability_ids.is_empty() {
        return "Extension activation succeeded. No model-visible tools were published by this extension; follow any extension-specific setup or connection UI before claiming new capabilities are available.".to_string();
    }
    let mut message = String::from(
        "Extension activation succeeded and its tools are now available. No additional authorization or configuration is needed, including for write-capable tools, unless a later tool call reports auth_required. Do not ask the user for a token, OAuth, authorization, or configuration after activated=true.",
    );
    message.push_str(
        " These tools are now callable by exact name — invoke one directly with tool_call(name=\"<tool>\", arguments={ ... }), or tool_describe(name=\"<tool>\") first if you need its full schema. Do NOT call tool_search for these; you already have their names: ",
    );
    message.push_str(&visible_capability_ids.join(", "));
    message.push('.');
    message
}

// Build the structured connect requirement for an inbound channel. The Slack copy
// is kept identical to the connectable-channels descriptor
// (`slack_inbound_proof_code_connectable_channel`) so the in-chat panel and the
// Settings panel read identically — enforced by the cross-check test
// `slack_requirement_copy_matches_connectable_descriptor`, not just by convention.
// Any other inbound channel gets a generic proof-code prompt.
pub(crate) fn channel_connection_requirement(
    channel_id: &str,
    display_name: &str,
) -> ChannelConnectionRequirement {
    if channel_id == "slack" {
        ChannelConnectionRequirement {
            channel: "slack".to_string(),
            strategy: RebornChannelConnectStrategy::InboundProofCode,
            instructions: "Message the IronClaw Reborn app in Slack to get a pairing code, then paste it here. Codes expire in 10 minutes. If a code is invalid or expired, run /pair in Slack for a fresh one.".to_string(),
            input_placeholder: "Enter Slack pairing code...".to_string(),
            submit_label: "Connect".to_string(),
            error_message: "Invalid or expired Slack pairing code. Run /pair in Slack to get a new one.".to_string(),
        }
    } else {
        ChannelConnectionRequirement {
            channel: channel_id.to_string(),
            strategy: RebornChannelConnectStrategy::InboundProofCode,
            instructions: format!(
                "Open {}'s app or bot, get the pairing code, and paste it here.",
                display_name
            ),
            input_placeholder: "Enter pairing code".to_string(),
            submit_label: "Connect".to_string(),
            error_message: "Pairing failed. Check the code and try again.".to_string(),
        }
    }
}

fn package_declares_inbound_product_adapter(package: &ExtensionPackage) -> bool {
    package.manifest.host_apis.iter().any(|host_api| {
        host_api.id.as_str() == PRODUCT_ADAPTER_HOST_API_ID
            && host_api.section.as_str() == "product_adapter.inbound"
    })
}
fn extension_ids_from_package_ref(
    package_ref: &LifecyclePackageRef,
) -> Result<(ExtensionId, ExtensionInstallationId), ProductWorkflowError> {
    package_ref.require_kind(LifecyclePackageKind::Extension)?;
    let extension_id = ExtensionId::new(package_ref.id.as_str().to_string()).map_err(|error| {
        ProductWorkflowError::InvalidBindingRequest {
            reason: error.to_string(),
        }
    })?;
    let installation_id = ExtensionInstallationId::new(extension_id.as_str().to_string())
        .map_err(map_extension_installation_error)?;
    Ok((extension_id, installation_id))
}

fn phase_for_activation_state(state: ExtensionActivationState) -> LifecyclePhase {
    match state {
        ExtensionActivationState::Enabled => LifecyclePhase::Active,
        ExtensionActivationState::Disabled => LifecyclePhase::Disabled,
        ExtensionActivationState::Installed => LifecyclePhase::Installed,
    }
}

async fn search_installation_phase(
    extension: &AvailableExtensionPackage,
    installation: &ExtensionInstallation,
    credential_gate: Option<&RuntimeExtensionActivationCredentialGate>,
) -> Result<LifecyclePhase, ProductWorkflowError> {
    let phase = phase_for_activation_state(installation.activation_state());
    if phase != LifecyclePhase::Installed {
        return Ok(phase);
    }
    if search_credentials_configured(extension, credential_gate).await? {
        return Ok(LifecyclePhase::Configured);
    }
    Ok(phase)
}

async fn search_credentials_configured(
    extension: &AvailableExtensionPackage,
    credential_gate: Option<&RuntimeExtensionActivationCredentialGate>,
) -> Result<bool, ProductWorkflowError> {
    let requirements = package_runtime_credential_auth_requirements(&extension.package);
    if requirements.is_empty() {
        return Ok(false);
    }
    let Some(credential_gate) = credential_gate else {
        return Ok(false);
    };
    Ok(credential_gate
        .missing_requirements(requirements)
        .await
        .map_err(map_search_credential_stage_error)?
        .is_empty())
}

fn suppress_search_credential_onboarding(summary: &mut LifecycleExtensionSummary) {
    summary.credential_requirements.clear();
    summary.onboarding = None;
}

fn extension_search_has_ready_result(payload: Option<&LifecycleProductPayload>) -> bool {
    let Some(LifecycleProductPayload::ExtensionSearch { extensions, .. }) = payload else {
        return false;
    };
    extensions.iter().any(|extension| {
        matches!(
            extension.installation_phase,
            Some(LifecyclePhase::Configured | LifecyclePhase::Active)
        ) && !extension
            .summary
            .surface_kinds
            .contains(&LifecycleExtensionSurfaceKind::ExternalChannel)
            && extension.summary.credential_requirements.is_empty()
            && extension.summary.onboarding.is_none()
    })
}

fn extension_search_has_installed_external_channel_result(
    payload: Option<&LifecycleProductPayload>,
) -> bool {
    let Some(LifecycleProductPayload::ExtensionSearch { extensions, .. }) = payload else {
        return false;
    };
    extensions.iter().any(|extension| {
        matches!(
            extension.installation_phase,
            Some(LifecyclePhase::Installed | LifecyclePhase::Configured | LifecyclePhase::Active)
        ) && extension
            .summary
            .surface_kinds
            .contains(&LifecycleExtensionSurfaceKind::ExternalChannel)
    })
}

fn map_search_credential_stage_error(
    error: ironclaw_host_api::CredentialStageError,
) -> ProductWorkflowError {
    match error {
        ironclaw_host_api::CredentialStageError::AuthRequired => {
            ProductWorkflowError::InvalidBindingRequest {
                reason: "extension requires product auth credentials before search can project configured state".to_string(),
            }
        }
        ironclaw_host_api::CredentialStageError::Backend => {
            ProductWorkflowError::Transient {
                reason: "extension product auth credential state is temporarily unavailable"
                    .to_string(),
            }
        }
    }
}

fn map_extension_error(error: ExtensionError) -> ProductWorkflowError {
    match error {
        ExtensionError::Filesystem(_) | ExtensionError::LifecycleEventSink { .. } => {
            ProductWorkflowError::Transient {
                reason: error.to_string(),
            }
        }
        _ => ProductWorkflowError::InvalidBindingRequest {
            reason: error.to_string(),
        },
    }
}

fn map_extension_installation_error(error: ExtensionInstallationError) -> ProductWorkflowError {
    // TODO(#4091): split durable-store transient failures from malformed
    // lifecycle requests when ExtensionInstallationStore grows a DB backend.
    ProductWorkflowError::InvalidBindingRequest {
        reason: error.to_string(),
    }
}

fn hosted_mcp_discovery_error(error: HostedMcpDiscoveryError) -> ProductWorkflowError {
    match error {
        HostedMcpDiscoveryError::Transient(reason) => ProductWorkflowError::Transient {
            reason: format!("hosted MCP discovery failed: {reason}"),
        },
        HostedMcpDiscoveryError::Permanent(reason) => ProductWorkflowError::InvalidBindingRequest {
            reason: format!("hosted MCP discovery failed: {reason}"),
        },
    }
}

fn hosted_mcp_changed_during_discovery_error() -> ProductWorkflowError {
    ProductWorkflowError::Transient {
        reason: "extension changed while hosted MCP discovery was running; retry activation"
            .to_string(),
    }
}

fn compensation_failure(
    context: &str,
    original: impl std::fmt::Display,
    compensation: impl std::fmt::Display,
) -> ProductWorkflowError {
    ProductWorkflowError::Transient {
        reason: format!(
            "{context}; original error: {original}; compensation error: {compensation}"
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeSet,
        sync::atomic::{AtomicUsize, Ordering},
    };

    use super::hosted_mcp_test_support::HostedMcpDiscoveryEgress;
    use super::*;
    use crate::available_extensions::{
        AvailableExtensionAsset, AvailableExtensionAssetContent, AvailableExtensionPackage,
    };
    use async_trait::async_trait;
    use ironclaw_extensions::{
        ExtensionLifecycleEvent, ExtensionLifecycleEventSink, ExtensionLifecycleService,
        ExtensionManifest, ExtensionRegistry, InMemoryExtensionInstallationStore,
        SharedExtensionRegistry,
    };
    use ironclaw_filesystem::{
        DirEntry, FileStat, FilesystemError, FilesystemOperation, LocalFilesystem,
    };
    use ironclaw_host_api::{
        CapabilityId, ExtensionLifecycleOperation, HostPath, HostPortCatalog, InvocationId,
        MountAlias, MountGrant, MountPermissions, MountView, NetworkMethod, ResourceScope,
        RuntimeCredentialAccountSetup, RuntimeHttpEgress, RuntimeHttpEgressError,
        RuntimeHttpEgressRequest, RuntimeHttpEgressResponse, TenantId, TrustClass, UserId,
    };
    use ironclaw_host_runtime::{SPAWN_SUBAGENT_CAPABILITY_ID, builtin_first_party_package};
    use ironclaw_product_workflow::{
        LifecycleExtensionRuntimeKind, LifecycleExtensionSource, LifecycleProductAction,
        LifecycleProductContext, LifecycleProductFacade, LifecycleProductSurfaceContext,
        LifecycleReadinessBlocker,
    };
    use ironclaw_trust::{HostTrustPolicy, InvalidationBus, TrustPolicy};

    #[test]
    fn installed_external_channel_search_result_gets_activation_guidance() {
        let payload = LifecycleProductPayload::ExtensionSearch {
            extensions: vec![LifecycleSearchExtensionSummary {
                summary: LifecycleExtensionSummary {
                    package_ref: LifecyclePackageRef::new(LifecyclePackageKind::Extension, "slack")
                        .expect("valid package ref"),
                    name: "Slack".to_string(),
                    version: "1.0.0".to_string(),
                    description: "Slack channel".to_string(),
                    source: LifecycleExtensionSource::HostBundled,
                    runtime_kind: LifecycleExtensionRuntimeKind::WasmTool,
                    surface_kinds: vec![LifecycleExtensionSurfaceKind::ExternalChannel],
                    visible_capability_ids: Vec::new(),
                    visible_read_only_capability_ids: Vec::new(),
                    credential_requirements: Vec::new(),
                    onboarding: None,
                },
                installation_phase: Some(LifecyclePhase::Installed),
            }],
            count: 1,
        };

        assert!(extension_search_has_installed_external_channel_result(
            Some(&payload)
        ));
        assert!(!extension_search_has_ready_result(Some(&payload)));
    }

    #[test]
    fn activation_message_enumerates_published_tools_by_exact_name() {
        // Regression: the model only sees a *count* of deferred tools, so after
        // activating an extension it must be handed the exact tool names or it
        // assumes they are unavailable and gives up. The success message must name
        // every published capability and steer the model to direct invocation.
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "fixture")
            .expect("valid package ref");
        let package = fixture_extension_package().package;
        let visible_capability_ids = vec!["fixture.search".to_string()];
        let message = activation_success_message(&package_ref, &package, &visible_capability_ids);
        assert!(message.contains("fixture.search"));
        assert!(
            message.contains("callable by exact name"),
            "must steer the model to tool_call by name, got: {message}"
        );
        assert!(
            message.contains("Do NOT call tool_search for these"),
            "must stop the model from re-searching for already-named tools, got: {message}"
        );
    }

    #[test]
    fn activation_message_without_published_tools_keeps_the_base_message_only() {
        // Channel-only / tool-less extensions publish no model tools; the message
        // must not invent an empty tool list or the direct-invocation guidance.
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "fixture")
            .expect("valid package ref");
        let package = fixture_extension_package().package;
        let message = activation_success_message(&package_ref, &package, &[]);
        assert!(message.contains("Extension activation succeeded"));
        assert!(
            !message.contains("callable by exact name"),
            "no tools published ⇒ no direct-invocation guidance, got: {message}"
        );
    }

    #[tokio::test]
    async fn extension_lifecycle_installs_activates_and_removes_catalog_package() {
        let (_dir, storage_root, facade, active_registry, _installation_store) =
            extension_lifecycle_fixture();

        // safety: test-only lifecycle facade calls; no database transaction is involved.
        let search = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionSearch {
                    query: "fixture".to_string(),
                },
            )
            .await
            .expect("search extensions");
        assert_eq!(search.phase, LifecyclePhase::Discovered);
        let Some(LifecycleProductPayload::ExtensionSearch { extensions, .. }) =
            search.payload.as_ref()
        else {
            panic!("expected extension search payload");
        };
        assert_eq!(extensions.len(), 1);
        assert_eq!(
            extensions[0].summary.visible_read_only_capability_ids,
            vec!["fixture.search"]
        );

        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "fixture")
            .expect("valid ref");
        let install = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionInstall {
                    package_ref: package_ref.clone(),
                },
            )
            .await
            .expect("install extension");
        assert_eq!(install.phase, LifecyclePhase::Installed);
        assert!(
            storage_root
                .join("system/extensions/fixture/manifest.toml")
                .exists()
        );
        assert!(
            storage_root
                .join("system/extensions/fixture/wasm/fixture.wasm")
                .exists()
        );
        assert!(
            active_registry
                .snapshot()
                .get_extension(&ExtensionId::new("fixture").unwrap())
                .is_none()
        );

        let activate = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionActivate {
                    package_ref: package_ref.clone(),
                },
            )
            .await
            .expect("activate extension");
        assert_eq!(activate.phase, LifecyclePhase::Active);
        let active = active_registry.snapshot();
        assert!(
            active
                .get_extension(&ExtensionId::new("fixture").unwrap())
                .is_some()
        );
        assert!(
            active
                .get_capability(&ironclaw_host_api::CapabilityId::new("fixture.search").unwrap())
                .is_some()
        );
        assert!(
            active
                .get_capability(&ironclaw_host_api::CapabilityId::new("fixture.write").unwrap())
                .is_some()
        );

        let remove = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionRemove { package_ref },
            )
            .await
            .expect("remove extension");
        assert_eq!(remove.phase, LifecyclePhase::Removed);
        assert!(
            active_registry
                .snapshot()
                .get_extension(&ExtensionId::new("fixture").unwrap())
                .is_none()
        );
        assert!(
            !storage_root
                .join("system/extensions/fixture/manifest.toml")
                .exists()
        );
        assert!(
            !storage_root
                .join("system/extensions/fixture/wasm/fixture.wasm")
                .exists()
        );
    }

    #[tokio::test]
    async fn extension_activate_returns_slack_pairing_guidance_for_external_channel_package() {
        let (_dir, _storage_root, facade, _active_registry, _installation_store) =
            extension_lifecycle_fixture_with_catalog_and_service(
                AvailableExtensionCatalog::from_packages(vec![fixture_external_channel_package(
                    "slack", "Slack",
                )]),
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
            );
        let package_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "slack").expect("valid ref");
        facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionInstall {
                    package_ref: package_ref.clone(),
                },
            )
            .await
            .expect("install slack channel");

        let activate = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionActivate {
                    package_ref: package_ref.clone(),
                },
            )
            .await
            .expect("activate slack channel");

        assert_eq!(activate.phase, LifecyclePhase::Active);
        let message = activate.message.as_deref().expect("activation message");
        assert!(
            message.contains("DM the Slack app")
                && message.contains("pairing code")
                && message.contains("WebChat")
                && message.contains("not into normal chat")
                && message.contains("continue the user's original request")
                && message.contains("Do not claim Slack message-reading tools"),
            "Slack activation should guide the model into pairing UI, got: {message}"
        );
        assert!(
            !message.contains("still needs pairing"),
            "activation is package-level and must not claim the caller is unpaired once the user may already be connected: {message}"
        );
        let Some(LifecycleProductPayload::ExtensionActivate {
            visible_capability_ids,
            connection_required,
            ..
        }) = activate.payload.as_ref()
        else {
            panic!("expected extension activate payload");
        };
        assert!(
            visible_capability_ids.is_empty(),
            "Slack channel activation must not imply model-visible Slack read tools"
        );
        // The structured connect requirement is what drives the in-chat pairing
        // panel; the prose message above is model guidance only.
        let requirement = connection_required
            .as_ref()
            .expect("slack channel activation must carry a structured connection requirement");
        assert_eq!(requirement.channel, "slack");
        assert_eq!(
            requirement.strategy,
            RebornChannelConnectStrategy::InboundProofCode
        );
        assert_eq!(requirement.input_placeholder, "Enter Slack pairing code...");
        assert!(
            requirement.error_message.contains("/pair"),
            "invalid-code copy must point the user at /pair: {}",
            requirement.error_message
        );
    }

    #[tokio::test]
    async fn extension_activate_returns_generic_pairing_guidance_for_external_channel_package() {
        let (_dir, _storage_root, facade, _active_registry, _installation_store) =
            extension_lifecycle_fixture_with_catalog_and_service(
                AvailableExtensionCatalog::from_packages(vec![fixture_external_channel_package(
                    "telegram", "Telegram",
                )]),
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
            );
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "telegram")
            .expect("valid ref");
        facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionInstall {
                    package_ref: package_ref.clone(),
                },
            )
            .await
            .expect("install external channel");

        let activate = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionActivate { package_ref },
            )
            .await
            .expect("activate external channel");

        assert_eq!(activate.phase, LifecyclePhase::Active);
        let message = activate.message.as_deref().expect("activation message");
        assert!(
            message.contains("Telegram is installed as an external channel")
                && message.contains("app or bot")
                && message.contains("pairing code")
                && message.contains("WebChat connection panel")
                && message.contains("rather than normal chat")
                && message.contains("continue the user's original request")
                && message.contains("already connected")
                && message.contains("until connection is confirmed"),
            "external channel activation should guide the model into generic pairing UI, got: {message}"
        );
        let Some(LifecycleProductPayload::ExtensionActivate {
            connection_required,
            ..
        }) = activate.payload.as_ref()
        else {
            panic!("expected extension activate payload");
        };
        let requirement = connection_required
            .as_ref()
            .expect("external channel activation must carry a structured connection requirement");
        assert_eq!(requirement.channel, "telegram");
        assert_eq!(
            requirement.strategy,
            RebornChannelConnectStrategy::InboundProofCode
        );
        assert!(
            requirement.instructions.contains("Telegram"),
            "generic channel copy should name the channel: {}",
            requirement.instructions
        );
    }

    #[tokio::test]
    async fn extension_search_routes_active_external_channel_to_activation_pairing_flow() {
        let (_dir, _storage_root, facade, _active_registry, _installation_store) =
            extension_lifecycle_fixture_with_catalog_and_service(
                AvailableExtensionCatalog::from_packages(vec![fixture_external_channel_package(
                    "slack", "Slack",
                )]),
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
            );
        let package_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "slack").expect("valid ref");
        facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionInstall {
                    package_ref: package_ref.clone(),
                },
            )
            .await
            .expect("install slack channel");
        facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionActivate {
                    package_ref: package_ref.clone(),
                },
            )
            .await
            .expect("activate slack channel");

        let search = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionSearch {
                    query: "slack".to_string(),
                },
            )
            .await
            .expect("search active slack channel");

        let message = search.message.as_deref().expect("search guidance");
        assert!(
            message.contains("external channel")
                && message.contains("do not treat")
                && message.contains("builtin.extension_activate")
                && message.contains("WebChat connection panel")
                && message.contains("rather than normal chat"),
            "active external channel search should route models into activation/pairing, got: {message}"
        );
        assert!(
            !message.contains("Treat those results as ready"),
            "active external channels must not use ready-extension guidance: {message}"
        );
        let Some(LifecycleProductPayload::ExtensionSearch { extensions, .. }) =
            search.payload.as_ref()
        else {
            panic!("expected extension search payload");
        };
        let slack = extensions
            .iter()
            .find(|extension| extension.summary.package_ref.id.as_str() == "slack")
            .expect("slack search result");
        assert_eq!(slack.installation_phase, Some(LifecyclePhase::Active));
    }

    #[tokio::test]
    async fn active_model_visible_capabilities_only_include_enabled_lifecycle_extensions() {
        let (_dir, _storage_root, port, active_registry, _installation_store) =
            extension_management_port_fixture_with_catalog_and_service(
                AvailableExtensionCatalog::from_packages(vec![fixture_extension_package()]),
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
            );
        active_registry
            .upsert(builtin_first_party_package().expect("builtin package"))
            .expect("seed builtin package");
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "fixture")
            .expect("valid ref");
        port.install(package_ref.clone())
            .await
            .expect("install fixture extension");
        port.activate_with_prechecked_credentials_for_test(
            package_ref,
            ExtensionActivationMode::Static,
        )
        .await
        .expect("activate fixture extension");

        let capability_ids = port
            .active_model_visible_capabilities()
            .await
            .expect("active capabilities")
            .into_iter()
            .map(|capability| capability.id)
            .collect::<Vec<_>>();

        assert!(capability_ids.contains(&CapabilityId::new("fixture.search").unwrap()));
        assert!(!capability_ids.contains(&CapabilityId::new("fixture.write").unwrap()));
        assert!(
            !capability_ids.contains(&CapabilityId::new(SPAWN_SUBAGENT_CAPABILITY_ID).unwrap())
        );
    }

    #[test]
    fn activation_credential_requirements_coalesce_google_oauth_scope_sets() {
        let catalog =
            AvailableExtensionCatalog::from_first_party_assets().expect("first-party assets");
        for (extension_id, expected_scopes) in [
            (
                "google-calendar",
                vec![
                    "https://www.googleapis.com/auth/calendar.events",
                    "https://www.googleapis.com/auth/calendar.readonly",
                ],
            ),
            (
                "gmail",
                vec![
                    "https://www.googleapis.com/auth/gmail.modify",
                    "https://www.googleapis.com/auth/gmail.readonly",
                    "https://www.googleapis.com/auth/gmail.send",
                ],
            ),
        ] {
            let package_ref =
                LifecyclePackageRef::new(LifecyclePackageKind::Extension, extension_id)
                    .expect("valid package ref");
            let package = catalog
                .resolve(&package_ref)
                .expect("bundled Google package");

            let requirements = package_runtime_credential_auth_requirements(&package.package);

            assert_eq!(
                requirements.len(),
                1,
                "{extension_id} should activate with one Google OAuth requirement"
            );
            let requirement = &requirements[0];
            assert_eq!(requirement.provider.as_str(), "google");
            assert_eq!(requirement.requester_extension.as_str(), extension_id);
            let expected = expected_scopes
                .into_iter()
                .map(str::to_string)
                .collect::<BTreeSet<_>>();
            assert_eq!(
                requirement
                    .provider_scopes
                    .iter()
                    .cloned()
                    .collect::<BTreeSet<_>>(),
                expected
            );
            let RuntimeCredentialAccountSetup::OAuth { scopes } = &requirement.setup else {
                panic!("{extension_id} should use OAuth setup");
            };
            assert_eq!(scopes.iter().cloned().collect::<BTreeSet<_>>(), expected);
        }
    }

    #[tokio::test]
    async fn hosted_mcp_activation_publishes_discovered_tool_schemas() {
        let catalog =
            AvailableExtensionCatalog::from_first_party_assets().expect("first-party assets");
        let (_dir, _storage_root, port, active_registry, _installation_store) =
            extension_management_port_fixture_with_catalog_and_service(
                catalog,
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
            );
        let package_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "notion").expect("valid ref");
        let egress = Arc::new(HostedMcpDiscoveryEgress::default());

        port.install(package_ref.clone())
            .await
            .expect("install Notion MCP");
        port.activate_with_prechecked_credentials_for_test(
            package_ref,
            ExtensionActivationMode::HostedMcpDiscovery {
                scope: ResourceScope::local_default(
                    UserId::new("hosted-mcp-user").unwrap(),
                    InvocationId::new(),
                )
                .unwrap(),
                runtime_http_egress: egress.clone(),
            },
        )
        .await
        .expect("activate with discovery");

        let snapshot = active_registry.snapshot();
        assert!(
            snapshot
                .get_capability(&CapabilityId::new("notion.notion-fetch").unwrap())
                .is_none()
        );
        let search = snapshot
            .get_capability(&CapabilityId::new("notion.live-search").unwrap())
            .expect("discovered capability");
        assert_eq!(
            search.parameters_schema,
            serde_json::json!({
                "type": "object",
                "properties": {"query": {"type": "string"}},
                "required": ["query"]
            })
        );
        assert_eq!(
            egress.methods(),
            vec![
                "initialize".to_string(),
                "notifications/initialized".to_string(),
                "tools/list".to_string(),
            ]
        );
        assert_eq!(egress.credential_counts(), vec![1, 1, 1]);
    }

    #[tokio::test]
    async fn hosted_mcp_activation_falls_back_to_bundled_manifest_when_discovery_returns_no_tools()
    {
        let catalog =
            AvailableExtensionCatalog::from_first_party_assets().expect("first-party assets");
        let (_dir, _storage_root, port, active_registry, _installation_store) =
            extension_management_port_fixture_with_catalog_and_service(
                catalog,
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
            );
        let package_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "notion").expect("valid ref");

        port.install(package_ref.clone())
            .await
            .expect("install Notion MCP");
        let activate = port
            .activate_with_prechecked_credentials_for_test(
                package_ref,
                ExtensionActivationMode::HostedMcpDiscovery {
                    scope: hosted_mcp_scope("hosted-mcp-empty-tools"),
                    runtime_http_egress: Arc::new(EmptyToolsHostedMcpEgress),
                },
            )
            .await
            .expect("transient discovery failure should fall back to bundled manifest");

        assert_eq!(activate.phase, LifecyclePhase::Active);
        assert!(
            active_registry
                .snapshot()
                .get_capability(&CapabilityId::new("notion.notion-search").unwrap())
                .is_some(),
            "fallback activation must publish bundled Notion capabilities"
        );
    }

    #[tokio::test]
    async fn hosted_mcp_activation_rechecks_credentials_after_discovery_before_publish() {
        let catalog =
            AvailableExtensionCatalog::from_first_party_assets().expect("first-party assets");
        let (_dir, _storage_root, port, active_registry, _installation_store) =
            extension_management_port_fixture_with_catalog_and_service(
                catalog,
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
            );
        let package_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "notion").expect("valid ref");
        let credential_gate = FailsSecondCredentialGate {
            calls: Arc::new(AtomicUsize::new(0)),
        };
        let calls = Arc::clone(&credential_gate.calls);

        port.install(package_ref.clone())
            .await
            .expect("install Notion MCP");
        let error = port
            .activate_with_credential_gate(
                package_ref,
                ExtensionActivationMode::HostedMcpDiscovery {
                    scope: hosted_mcp_scope("hosted-mcp-credential-recheck"),
                    runtime_http_egress: Arc::new(HostedMcpDiscoveryEgress::default()),
                },
                credential_gate,
            )
            .await
            .expect_err("post-discovery credential recheck should fail activation");

        assert!(matches!(
            error,
            ProductWorkflowError::InvalidBindingRequest { .. }
        ));
        assert_eq!(
            calls.load(Ordering::SeqCst),
            2,
            "hosted MCP activation must check credentials before and after discovery"
        );
        assert!(
            active_registry
                .snapshot()
                .get_capability(&CapabilityId::new("notion.live-search").unwrap())
                .is_none(),
            "discovered tools must not publish after post-discovery credential failure"
        );
    }

    #[tokio::test]
    async fn hosted_mcp_activation_returns_transient_when_package_removed_during_discovery() {
        let catalog =
            AvailableExtensionCatalog::from_first_party_assets().expect("first-party assets");
        let (_dir, _storage_root, port, _active_registry, _installation_store) =
            extension_management_port_fixture_with_catalog_and_service(
                catalog,
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
            );
        let package_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "notion").expect("valid ref");
        let (egress, tools_list_started, release_tools_list) =
            BlockingToolsListHostedMcpEgress::new();

        port.install(package_ref.clone())
            .await
            .expect("install Notion MCP");
        let activation = tokio::spawn({
            let port = Arc::clone(&port);
            let package_ref = package_ref.clone();
            async move {
                port.activate_with_prechecked_credentials_for_test(
                    package_ref,
                    ExtensionActivationMode::HostedMcpDiscovery {
                        scope: hosted_mcp_scope("hosted-mcp-remove-race"),
                        runtime_http_egress: egress,
                    },
                )
                .await
            }
        });
        tools_list_started
            .await
            .expect("tools/list request should start");

        port.remove(package_ref)
            .await
            .expect("remove can proceed while discovery is in flight");
        release_tools_list
            .send(())
            .expect("release blocked tools/list response");
        let error = activation
            .await
            .expect("activation task joins")
            .expect_err("remove during discovery should be retryable");

        assert!(matches!(error, ProductWorkflowError::Transient { .. }));
    }

    #[tokio::test]
    async fn extension_activation_updates_local_dev_host_trust_policy() {
        let (_dir, _storage_root, port, _active_registry, _installation_store, trust_policy) =
            extension_management_port_fixture_with_catalog_service_and_trust(
                AvailableExtensionCatalog::from_packages(vec![fixture_extension_package()]),
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
            );
        let package = fixture_extension_package().package;
        let trust_input = extension_trust_policy_input(&package).expect("trust input");
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "fixture")
            .expect("valid ref");

        assert_eq!(
            trust_policy
                .evaluate(&trust_input)
                .expect("pre-activation trust")
                .effective_trust
                .class(),
            TrustClass::Sandbox
        );

        port.install(package_ref.clone())
            .await
            .expect("install fixture extension");
        port.activate_with_prechecked_credentials_for_test(
            package_ref.clone(),
            ExtensionActivationMode::Static,
        )
        .await
        .expect("activate fixture extension");
        let active_decision = trust_policy
            .evaluate(&trust_input)
            .expect("active extension trust");
        assert_eq!(
            active_decision.effective_trust.class(),
            TrustClass::UserTrusted
        );
        assert_eq!(
            active_decision.provenance,
            ironclaw_trust::TrustProvenance::AdminConfig
        );
        assert_eq!(
            active_decision.authority_ceiling.allowed_effects,
            vec![EffectKind::Network, EffectKind::ExternalWrite]
        );

        port.remove(package_ref)
            .await
            .expect("remove fixture extension");
        let removed_decision = trust_policy
            .evaluate(&trust_input)
            .expect("removed extension trust");
        assert_eq!(
            removed_decision.effective_trust.class(),
            TrustClass::Sandbox
        );
        assert!(
            removed_decision
                .authority_ceiling
                .allowed_effects
                .is_empty()
        );
    }

    #[tokio::test]
    async fn commit_activation_rolls_back_when_set_activation_state_fails() {
        let lifecycle_sink = Arc::new(RecordingLifecycleSink::default());
        let lifecycle_service = ExtensionLifecycleService::new(ExtensionRegistry::new())
            .with_event_sink(lifecycle_sink.clone());
        let (_dir, port, active_registry, failing_store, _trust_policy) =
            extension_port_with_set_activation_failing_store(lifecycle_service);
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "fixture")
            .expect("valid ref");

        port.install(package_ref.clone())
            .await
            .expect("install extension");
        let error = port
            .activate(package_ref, ExtensionActivationMode::Static)
            .await
            .expect_err("activation-state persistence failure is reported");

        assert!(matches!(
            error,
            ProductWorkflowError::InvalidBindingRequest { .. }
        ));
        assert!(
            active_registry
                .snapshot()
                .get_extension(&ExtensionId::new("fixture").unwrap())
                .is_none()
        );
        assert_eq!(
            fixture_installation_state(failing_store.as_ref()).await,
            ExtensionActivationState::Installed
        );
        assert!(
            lifecycle_sink
                .operations()
                .contains(&ExtensionLifecycleOperation::Disable)
        );
    }

    #[tokio::test]
    async fn commit_activation_rolls_back_when_publish_fails() {
        let (_dir, _storage_root, port, active_registry, installation_store) =
            extension_management_port_fixture_with_catalog_service_and_trust_policy(
                AvailableExtensionCatalog::from_packages(vec![fixture_extension_package()]),
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
                Arc::new(HostTrustPolicy::fail_closed()),
            );
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "fixture")
            .expect("valid ref");

        port.install(package_ref.clone())
            .await
            .expect("install extension");
        let error = port
            .activate(package_ref, ExtensionActivationMode::Static)
            .await
            .expect_err("publish failure is reported");

        assert!(matches!(
            error,
            ProductWorkflowError::InvalidBindingRequest { .. }
        ));
        assert!(
            active_registry
                .snapshot()
                .get_extension(&ExtensionId::new("fixture").unwrap())
                .is_none()
        );
        assert_eq!(
            fixture_installation_state(installation_store.as_ref()).await,
            ExtensionActivationState::Installed
        );
    }

    #[tokio::test]
    async fn commit_activation_publish_failure_preserves_previously_enabled_extension() {
        let lifecycle_sink = Arc::new(RecordingLifecycleSink::default());
        let lifecycle_service = ExtensionLifecycleService::new(ExtensionRegistry::new())
            .with_event_sink(lifecycle_sink.clone());
        let (_dir, _storage_root, port, _active_registry, installation_store) =
            extension_management_port_fixture_with_catalog_service_and_trust_policy(
                AvailableExtensionCatalog::from_packages(vec![fixture_extension_package()]),
                lifecycle_service,
                Arc::new(HostTrustPolicy::fail_closed()),
            );
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "fixture")
            .expect("valid ref");
        let extension_id = ExtensionId::new("fixture").expect("valid extension id");
        let installation_id = ExtensionInstallationId::new("fixture").expect("valid installation");

        port.install(package_ref.clone())
            .await
            .expect("install extension");
        installation_store
            .set_activation_state(&installation_id, ExtensionActivationState::Enabled)
            .await
            .expect("seed enabled installation");
        let error = port
            .commit_activation(
                package_ref,
                &extension_id,
                &installation_id,
                ExtensionActivationState::Enabled,
                fixture_extension_package().package,
            )
            .await
            .expect_err("publish failure is reported");

        assert!(matches!(
            error,
            ProductWorkflowError::InvalidBindingRequest { .. }
        ));
        assert_eq!(
            fixture_installation_state(installation_store.as_ref()).await,
            ExtensionActivationState::Enabled
        );
        let operations = lifecycle_sink.operations();
        assert!(operations.contains(&ExtensionLifecycleOperation::Enable));
        assert!(!operations.contains(&ExtensionLifecycleOperation::Disable));
    }

    #[tokio::test]
    async fn extension_lifecycle_search_propagates_installation_store_read_error() {
        let (_dir, port, _active_registry, _failing_store, _trust_policy) =
            extension_port_with_failing_store(
                ExtensionRegistry::new(),
                DeleteInstallationFailingStore::fail_get_installation(),
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
            );

        let error = port
            .search("fixture", None)
            .await
            .expect_err("search reports installation-store read failure");

        assert!(matches!(
            error,
            ProductWorkflowError::InvalidBindingRequest { .. }
        ));
    }

    #[tokio::test]
    async fn extension_lifecycle_search_rejects_mismatched_installation_row() {
        let (_dir, port, _active_registry, _failing_store, _trust_policy) =
            extension_port_with_failing_store(
                ExtensionRegistry::new(),
                DeleteInstallationFailingStore::mismatched_get_installation(),
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
            );

        let error = port
            .search("fixture", None)
            .await
            .expect_err("search reports mismatched installation row");

        assert!(matches!(
            error,
            ProductWorkflowError::InvalidBindingRequest { .. }
        ));
    }

    #[tokio::test]
    async fn active_extension_trust_policy_is_digest_pinned() {
        let (_dir, _storage_root, port, _active_registry, _installation_store, trust_policy) =
            extension_management_port_fixture_with_catalog_service_and_trust(
                AvailableExtensionCatalog::from_packages(vec![fixture_extension_package()]),
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
            );
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "fixture")
            .expect("valid ref");

        port.install(package_ref.clone())
            .await
            .expect("install fixture extension");
        port.activate_with_prechecked_credentials_for_test(
            package_ref,
            ExtensionActivationMode::Static,
        )
        .await
        .expect("activate fixture extension");

        let changed_package = fixture_extension_package_with_description(
            "Lifecycle fixture extension with changed manifest",
        )
        .package;
        let changed_trust_input =
            extension_trust_policy_input(&changed_package).expect("changed trust input");
        let changed_decision = trust_policy
            .evaluate(&changed_trust_input)
            .expect("changed active extension trust");
        assert_eq!(
            changed_decision.effective_trust.class(),
            TrustClass::Sandbox
        );
        assert_eq!(
            changed_decision.provenance,
            ironclaw_trust::TrustProvenance::Default
        );
    }

    #[tokio::test]
    async fn restore_enabled_extension_updates_local_dev_host_trust_policy() {
        let (_dir, _storage_root, port, _active_registry, installation_store, _trust_policy) =
            extension_management_port_fixture_with_catalog_service_and_trust(
                AvailableExtensionCatalog::from_packages(vec![fixture_extension_package()]),
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
            );
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "fixture")
            .expect("valid ref");
        port.install(package_ref.clone())
            .await
            .expect("install fixture extension");
        port.activate_with_prechecked_credentials_for_test(
            package_ref,
            ExtensionActivationMode::Static,
        )
        .await
        .expect("activate fixture extension");

        let restored_catalog =
            AvailableExtensionCatalog::from_packages(vec![fixture_extension_package()]);
        let restored_lifecycle = Arc::new(Mutex::new(ExtensionLifecycleService::new(
            ExtensionRegistry::new(),
        )));
        let restored_active_registry =
            Arc::new(SharedExtensionRegistry::new(ExtensionRegistry::new()));
        let restored_trust_policy = test_extension_trust_policy();
        let restored_active_extensions = test_active_extension_publisher(
            Arc::clone(&restored_active_registry),
            Arc::clone(&restored_trust_policy),
        );
        let installation_store: Arc<dyn ExtensionInstallationStore> = installation_store;

        restore_extension_lifecycle_state(
            &restored_catalog,
            &port.filesystem,
            &installation_store,
            &restored_lifecycle,
            &restored_active_extensions,
        )
        .await
        .expect("restore enabled extension lifecycle state");

        let package = fixture_extension_package().package;
        let trust_input = extension_trust_policy_input(&package).expect("trust input");
        assert_eq!(
            restored_trust_policy
                .evaluate(&trust_input)
                .expect("restored active extension trust")
                .effective_trust
                .class(),
            TrustClass::UserTrusted
        );
        assert!(
            restored_active_registry
                .snapshot()
                .get_extension(&ExtensionId::new("fixture").unwrap())
                .is_some()
        );
    }

    #[tokio::test]
    async fn restore_refreshes_materialized_extension_assets_from_catalog() {
        let (_dir, storage_root, port, _active_registry, installation_store, _trust_policy) =
            extension_management_port_fixture_with_catalog_service_and_trust(
                AvailableExtensionCatalog::from_packages(vec![fixture_extension_package()]),
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
            );
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "fixture")
            .expect("valid ref");
        port.install(package_ref.clone())
            .await
            .expect("install fixture extension");
        port.activate_with_prechecked_credentials_for_test(
            package_ref,
            ExtensionActivationMode::Static,
        )
        .await
        .expect("activate fixture extension");

        let wasm_path = storage_root.join("system/extensions/fixture/wasm/fixture.wasm");
        std::fs::write(&wasm_path, b"stale-installed-module").expect("corrupt installed module");

        let restored_lifecycle = Arc::new(Mutex::new(ExtensionLifecycleService::new(
            ExtensionRegistry::new(),
        )));
        let restored_active_registry =
            Arc::new(SharedExtensionRegistry::new(ExtensionRegistry::new()));
        let restored_trust_policy = test_extension_trust_policy();
        let restored_active_extensions = test_active_extension_publisher(
            Arc::clone(&restored_active_registry),
            Arc::clone(&restored_trust_policy),
        );
        let installation_store: Arc<dyn ExtensionInstallationStore> = installation_store;

        restore_extension_lifecycle_state(
            &AvailableExtensionCatalog::from_packages(vec![fixture_extension_package()]),
            &port.filesystem,
            &installation_store,
            &restored_lifecycle,
            &restored_active_extensions,
        )
        .await
        .expect("restore extension lifecycle state");

        assert_eq!(
            std::fs::read(wasm_path).expect("refreshed module"),
            b"\0asm\x01\0\0\0"
        );
    }

    #[tokio::test]
    async fn restore_enabled_host_bundled_extension_migrates_manifest_hash_and_trust_policy() {
        let (_dir, _storage_root, port, _active_registry, installation_store, _trust_policy) =
            extension_management_port_fixture_with_catalog_service_and_trust(
                AvailableExtensionCatalog::from_packages(vec![fixture_extension_package()]),
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
            );
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "fixture")
            .expect("valid ref");
        port.install(package_ref.clone())
            .await
            .expect("install fixture extension");
        port.activate_with_prechecked_credentials_for_test(
            package_ref,
            ExtensionActivationMode::Static,
        )
        .await
        .expect("activate fixture extension");

        let changed_available = fixture_extension_package_with_description(
            "Lifecycle fixture extension with changed manifest",
        );
        let changed_hash = available_manifest_hash(&changed_available).expect("changed hash");
        let changed_package = changed_available.package.clone();
        let changed_catalog = AvailableExtensionCatalog::from_packages(vec![changed_available]);
        let restored_lifecycle = Arc::new(Mutex::new(ExtensionLifecycleService::new(
            ExtensionRegistry::new(),
        )));
        let restored_active_registry =
            Arc::new(SharedExtensionRegistry::new(ExtensionRegistry::new()));
        let restored_trust_policy = test_extension_trust_policy();
        let restored_active_extensions = test_active_extension_publisher(
            Arc::clone(&restored_active_registry),
            Arc::clone(&restored_trust_policy),
        );
        let installation_store: Arc<dyn ExtensionInstallationStore> = installation_store;

        restore_extension_lifecycle_state(
            &changed_catalog,
            &port.filesystem,
            &installation_store,
            &restored_lifecycle,
            &restored_active_extensions,
        )
        .await
        .expect("host-bundled manifest hash mismatch migrates");

        let extension_id = ExtensionId::new("fixture").expect("valid extension id");
        let installation_id = ExtensionInstallationId::new("fixture").expect("valid installation");
        let stored_manifest = installation_store
            .get_manifest(&extension_id)
            .await
            .expect("read migrated manifest")
            .expect("migrated manifest");
        assert_eq!(stored_manifest.manifest_hash(), Some(&changed_hash));
        let stored_installation = installation_store
            .get_installation(&installation_id)
            .await
            .expect("read migrated installation")
            .expect("migrated installation");
        assert_eq!(
            stored_installation.manifest_ref().manifest_hash(),
            Some(&changed_hash)
        );
        let trust_input = extension_trust_policy_input(&changed_package).expect("trust input");
        assert_eq!(
            restored_trust_policy
                .evaluate(&trust_input)
                .expect("migrated extension trust")
                .effective_trust
                .class(),
            TrustClass::UserTrusted
        );
        assert!(
            restored_active_registry
                .snapshot()
                .get_extension(&extension_id)
                .is_some()
        );
    }

    #[tokio::test]
    async fn restore_enabled_local_extension_rejects_manifest_hash_mismatch() {
        let changed_available = fixture_extension_package_with_description(
            "Lifecycle fixture extension with changed manifest",
        );
        let package = changed_available.package.clone();
        let catalog = AvailableExtensionCatalog::from_packages(vec![changed_available]);
        let installation_store = Arc::new(InMemoryExtensionInstallationStore::default());
        let manifest_record = fixture_manifest_record_with_source(
            fixture_installed_local_manifest(),
            ManifestSource::InstalledLocal,
            Some("sha256:old".to_string()),
        );
        installation_store
            .upsert_manifest(manifest_record)
            .await
            .expect("upsert manifest");
        installation_store
            .upsert_installation(fixture_installation(
                Some("sha256:old".to_string()),
                ExtensionActivationState::Enabled,
            ))
            .await
            .expect("upsert installation");
        let restored_lifecycle = Arc::new(Mutex::new(ExtensionLifecycleService::new(
            ExtensionRegistry::new(),
        )));
        let restored_active_registry =
            Arc::new(SharedExtensionRegistry::new(ExtensionRegistry::new()));
        let restored_trust_policy = test_extension_trust_policy();
        let restored_active_extensions = test_active_extension_publisher(
            Arc::clone(&restored_active_registry),
            Arc::clone(&restored_trust_policy),
        );
        let installation_store: Arc<dyn ExtensionInstallationStore> = installation_store;
        let filesystem: Arc<dyn RootFilesystem> = Arc::new(LocalFilesystem::new());

        let error = restore_extension_lifecycle_state(
            &catalog,
            &filesystem,
            &installation_store,
            &restored_lifecycle,
            &restored_active_extensions,
        )
        .await
        .expect_err("non-host-bundled manifest hash mismatch fails closed");

        assert!(matches!(
            error,
            ProductWorkflowError::InvalidBindingRequest { .. }
        ));
        let trust_input = extension_trust_policy_input(&package).expect("trust input");
        assert_eq!(
            restored_trust_policy
                .evaluate(&trust_input)
                .expect("missing-hash extension trust")
                .effective_trust
                .class(),
            TrustClass::Sandbox
        );
        assert!(
            restored_active_registry
                .snapshot()
                .get_extension(&ExtensionId::new("fixture").unwrap())
                .is_none()
        );
    }

    #[tokio::test]
    async fn extension_lifecycle_installs_activates_and_removes_github() {
        let (_dir, storage_root, facade, active_registry, _installation_store) =
            github_extension_lifecycle_fixture();
        let facade =
            facade.with_runtime_credential_accounts(Arc::new(ConfiguredRuntimeCredentialAccounts));

        let search = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionSearch {
                    query: "github".to_string(),
                },
            )
            .await
            .expect("search extensions");
        assert_eq!(search.phase, LifecyclePhase::Discovered);
        let Some(LifecycleProductPayload::ExtensionSearch { extensions, .. }) =
            search.payload.as_ref()
        else {
            panic!("expected extension search payload");
        };
        assert_eq!(extensions.len(), 1);
        assert!(
            extensions[0]
                .summary
                .visible_read_only_capability_ids
                .iter()
                .any(|id| id == "github.search_issues")
        );
        assert!(
            extensions[0]
                .summary
                .visible_read_only_capability_ids
                .iter()
                .any(|id| id == "github.search_issues_pull_requests")
        );
        assert!(
            extensions[0]
                .summary
                .visible_read_only_capability_ids
                .iter()
                .any(|id| id == "github.get_issue")
        );

        let package_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "github").expect("valid ref");
        let install = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionInstall {
                    package_ref: package_ref.clone(),
                },
            )
            .await
            .expect("install extension");
        assert_eq!(install.phase, LifecyclePhase::Installed);
        assert!(
            storage_root
                .join("system/extensions/github/manifest.toml")
                .exists()
        );
        assert!(
            storage_root
                .join("system/extensions/github/wasm/github_tool.wasm")
                .exists()
        );
        assert!(
            active_registry
                .snapshot()
                .get_extension(&ExtensionId::new("github").unwrap())
                .is_none()
        );

        let installed_search = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionSearch {
                    query: "github".to_string(),
                },
            )
            .await
            .expect("search installed extension");
        let Some(LifecycleProductPayload::ExtensionSearch { extensions, .. }) =
            installed_search.payload.as_ref()
        else {
            panic!("expected extension search payload");
        };
        let github = extensions
            .iter()
            .find(|extension| extension.summary.package_ref.id.as_str() == "github")
            .expect("github search result");
        assert_eq!(github.installation_phase, Some(LifecyclePhase::Configured));
        assert!(
            github.summary.credential_requirements.is_empty(),
            "configured inactive GitHub search results must not expose satisfied PAT requirements"
        );
        assert!(
            github.summary.onboarding.is_none(),
            "configured inactive GitHub search results must not expose stale PAT setup onboarding"
        );

        let activate = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionActivate {
                    package_ref: package_ref.clone(),
                },
            )
            .await
            .expect("activate extension");
        assert_eq!(activate.phase, LifecyclePhase::Active);
        let active = active_registry.snapshot();
        assert!(
            active
                .get_extension(&ExtensionId::new("github").unwrap())
                .is_some()
        );
        assert!(
            active
                .get_capability(
                    &ironclaw_host_api::CapabilityId::new("github.search_issues").unwrap()
                )
                .is_some()
        );
        assert!(
            active
                .get_capability(
                    &ironclaw_host_api::CapabilityId::new("github.comment_issue").unwrap()
                )
                .is_some()
        );

        let active_search = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionSearch {
                    query: "github".to_string(),
                },
            )
            .await
            .expect("search active extension");
        let Some(LifecycleProductPayload::ExtensionSearch { extensions, .. }) =
            active_search.payload.as_ref()
        else {
            panic!("expected extension search payload");
        };
        let github = extensions
            .iter()
            .find(|extension| extension.summary.package_ref.id.as_str() == "github")
            .expect("github search result");
        assert_eq!(github.installation_phase, Some(LifecyclePhase::Active));
        assert!(
            github.summary.credential_requirements.is_empty(),
            "active GitHub search results must not expose satisfied PAT requirements"
        );
        assert!(
            github.summary.onboarding.is_none(),
            "active GitHub search results must not expose stale PAT setup onboarding"
        );

        let remove = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionRemove { package_ref },
            )
            .await
            .expect("remove extension");
        assert_eq!(remove.phase, LifecyclePhase::Removed);
        assert!(
            active_registry
                .snapshot()
                .get_extension(&ExtensionId::new("github").unwrap())
                .is_none()
        );
        assert!(
            !storage_root
                .join("system/extensions/github/manifest.toml")
                .exists()
        );
        assert!(
            !storage_root
                .join("system/extensions/github/wasm/github_tool.wasm")
                .exists()
        );
    }

    #[tokio::test]
    async fn extension_lifecycle_search_reports_credential_backend_failure_as_transient() {
        let (_dir, _storage_root, facade, _active_registry, _installation_store) =
            github_extension_lifecycle_fixture();
        let facade = facade.with_runtime_credential_accounts(Arc::new(
            BackendUnavailableRuntimeCredentialAccounts,
        ));
        let package_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "github").expect("valid ref");

        facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionInstall {
                    package_ref: package_ref.clone(),
                },
            )
            .await
            .expect("install extension");
        let error = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionSearch {
                    query: "github".to_string(),
                },
            )
            .await
            .expect_err("search reports credential backend failure");

        assert!(matches!(error, ProductWorkflowError::Transient { .. }));
    }

    #[tokio::test]
    async fn lifecycle_facade_blocks_credentialed_extension_activation_without_product_auth() {
        let (_dir, _storage_root, facade, active_registry, _installation_store) =
            github_extension_lifecycle_fixture();
        let facade =
            facade.with_runtime_credential_accounts(Arc::new(MissingRuntimeCredentialAccounts));
        let package_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "github").expect("valid ref");

        facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionInstall {
                    package_ref: package_ref.clone(),
                },
            )
            .await
            .expect("install extension");
        let error = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionActivate { package_ref },
            )
            .await
            .expect_err("missing product-auth account blocks activation");

        assert!(matches!(
            error,
            ProductWorkflowError::InvalidBindingRequest { .. }
        ));
        assert!(
            active_registry
                .snapshot()
                .get_extension(&ExtensionId::new("github").unwrap())
                .is_none()
        );
    }

    #[tokio::test]
    async fn lifecycle_facade_rejects_static_activation_for_hosted_mcp_packages() {
        let (_dir, _storage_root, facade, _active_registry, _installation_store) =
            extension_lifecycle_fixture_with_catalog_and_service(
                AvailableExtensionCatalog::from_first_party_assets().expect("first-party assets"),
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
            );
        let facade =
            facade.with_runtime_credential_accounts(Arc::new(ConfiguredRuntimeCredentialAccounts));
        let package_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "notion").expect("valid ref");

        facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionInstall {
                    package_ref: package_ref.clone(),
                },
            )
            .await
            .expect("install Notion MCP");
        let error = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionActivate { package_ref },
            )
            .await
            .expect_err("hosted MCP activation needs runtime egress services");

        assert!(matches!(
            error,
            ProductWorkflowError::InvalidBindingRequest { .. }
        ));
    }

    #[tokio::test]
    async fn lifecycle_facade_activates_hosted_mcp_with_runtime_egress() {
        let (_dir, _storage_root, facade, active_registry, _installation_store) =
            extension_lifecycle_fixture_with_catalog_and_service(
                AvailableExtensionCatalog::from_first_party_assets().expect("first-party assets"),
                ExtensionLifecycleService::new(ExtensionRegistry::new()),
            );
        let facade = facade
            .with_runtime_credential_accounts(Arc::new(ConfiguredRuntimeCredentialAccounts))
            .with_runtime_http_egress(Arc::new(HostedMcpDiscoveryEgress::default()));
        let package_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "notion").expect("valid ref");

        facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionInstall {
                    package_ref: package_ref.clone(),
                },
            )
            .await
            .expect("install Notion MCP");
        let activate = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionActivate { package_ref },
            )
            .await
            .expect("hosted MCP activation should use discovery egress");

        assert_eq!(activate.phase, LifecyclePhase::Active);
        assert!(
            active_registry
                .snapshot()
                .get_capability(&CapabilityId::new("notion.live-search").unwrap())
                .is_some()
        );
    }

    #[tokio::test]
    async fn extension_lifecycle_installs_activates_and_removes_gsuite() {
        let (_dir, storage_root, facade, active_registry, _installation_store) =
            github_extension_lifecycle_fixture();
        let facade =
            facade.with_runtime_credential_accounts(Arc::new(ConfiguredRuntimeCredentialAccounts));

        let search = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionSearch {
                    query: "google".to_string(),
                },
            )
            .await
            .expect("search extensions");
        assert_eq!(search.phase, LifecyclePhase::Discovered);
        let Some(LifecycleProductPayload::ExtensionSearch { extensions, .. }) =
            search.payload.as_ref()
        else {
            panic!("expected extension search payload");
        };
        let extension_ids = extensions
            .iter()
            .map(|extension| extension.summary.package_ref.id.as_str())
            .collect::<BTreeSet<_>>();
        assert_eq!(
            extension_ids,
            BTreeSet::from([
                "gmail",
                "google-calendar",
                "google-docs",
                "google-drive",
                "google-sheets",
                "google-slides",
            ])
        );
        let calendar = extensions
            .iter()
            .find(|extension| extension.summary.package_ref.id.as_str() == "google-calendar")
            .expect("google-calendar search result");
        assert_eq!(
            calendar.summary.visible_capability_ids,
            vec![
                "google-calendar.list_calendars",
                "google-calendar.list_events",
                "google-calendar.get_event",
                "google-calendar.find_free_slots",
                "google-calendar.create_event",
                "google-calendar.update_event",
                "google-calendar.delete_event",
                "google-calendar.add_attendees",
                "google-calendar.set_reminder",
            ]
        );
        assert_eq!(
            calendar.summary.visible_read_only_capability_ids,
            vec![
                "google-calendar.list_calendars",
                "google-calendar.list_events",
                "google-calendar.get_event",
                "google-calendar.find_free_slots",
            ]
        );
        let search = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionSearch {
                    query: "gmail".to_string(),
                },
            )
            .await
            .expect("search Gmail extension");
        assert_eq!(search.phase, LifecyclePhase::Discovered);
        let Some(LifecycleProductPayload::ExtensionSearch { extensions, .. }) =
            search.payload.as_ref()
        else {
            panic!("expected extension search payload");
        };
        assert_eq!(extensions.len(), 1);
        assert_eq!(extensions[0].summary.package_ref.id.as_str(), "gmail");
        assert_eq!(
            extensions[0].summary.visible_capability_ids,
            vec![
                "gmail.list_messages",
                "gmail.get_message",
                "gmail.send_message",
                "gmail.create_draft",
                "gmail.reply_to_message",
                "gmail.trash_message",
            ]
        );
        assert_eq!(
            extensions[0].summary.visible_read_only_capability_ids,
            vec!["gmail.list_messages", "gmail.get_message"]
        );

        let calendar_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "google-calendar")
                .expect("valid ref");
        let gmail_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "gmail").expect("valid ref");
        for package_ref in [calendar_ref.clone(), gmail_ref.clone()] {
            let install = facade
                .execute(
                    lifecycle_surface_context(),
                    LifecycleProductAction::ExtensionInstall {
                        package_ref: package_ref.clone(),
                    },
                )
                .await
                .expect("install extension");
            assert_eq!(install.phase, LifecyclePhase::Installed);
        }
        for path in [
            "system/extensions/google-calendar/manifest.toml",
            "system/extensions/google-calendar/schemas/google-calendar/list_events.input.v1.json",
            "system/extensions/google-calendar/prompts/google-calendar/create_event.md",
            "system/extensions/gmail/manifest.toml",
            "system/extensions/gmail/schemas/gmail/send_message.input.v1.json",
            "system/extensions/gmail/prompts/gmail/send_message.md",
        ] {
            assert!(storage_root.join(path).exists(), "missing {path}");
        }
        assert!(
            active_registry
                .snapshot()
                .get_extension(&ExtensionId::new("google-calendar").unwrap())
                .is_none()
        );

        for package_ref in [calendar_ref.clone(), gmail_ref.clone()] {
            let activate = facade
                .execute(
                    lifecycle_surface_context(),
                    LifecycleProductAction::ExtensionActivate { package_ref },
                )
                .await
                .expect("activate extension");
            assert_eq!(activate.phase, LifecyclePhase::Active);
        }
        let active = active_registry.snapshot();
        assert!(
            active
                .get_capability(
                    &ironclaw_host_api::CapabilityId::new("google-calendar.list_events").unwrap()
                )
                .is_some()
        );
        assert!(
            active
                .get_capability(
                    &ironclaw_host_api::CapabilityId::new("gmail.send_message").unwrap()
                )
                .is_some()
        );

        for package_ref in [calendar_ref, gmail_ref] {
            let remove = facade
                .execute(
                    lifecycle_surface_context(),
                    LifecycleProductAction::ExtensionRemove { package_ref },
                )
                .await
                .expect("remove extension");
            assert_eq!(remove.phase, LifecyclePhase::Removed);
        }
        assert!(
            active_registry
                .snapshot()
                .get_extension(&ExtensionId::new("gmail").unwrap())
                .is_none()
        );
        assert!(
            !storage_root
                .join("system/extensions/google-calendar/manifest.toml")
                .exists()
        );
        assert!(
            !storage_root
                .join("system/extensions/gmail/manifest.toml")
                .exists()
        );
    }

    #[tokio::test]
    async fn extension_install_rejects_skill_package_ref() {
        let (_dir, _storage_root, facade, _active_registry, _installation_store) =
            extension_lifecycle_fixture();

        let error = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionInstall {
                    package_ref: LifecyclePackageRef::new(LifecyclePackageKind::Skill, "fixture")
                        .expect("valid skill ref"),
                },
            )
            .await
            .expect_err("extension install rejects non-extension refs");

        assert!(matches!(
            error,
            ProductWorkflowError::InvalidBindingRequest { .. }
        ));
    }

    #[tokio::test]
    async fn extension_install_rejects_duplicate_without_overwriting_materialized_files() {
        let (_dir, storage_root, facade, _active_registry, _installation_store) =
            extension_lifecycle_fixture();
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "fixture")
            .expect("valid ref");

        facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionInstall {
                    package_ref: package_ref.clone(),
                },
            )
            .await
            .expect("initial install");
        let wasm_path = storage_root.join("system/extensions/fixture/wasm/fixture.wasm");
        std::fs::write(&wasm_path, b"existing-live-module").expect("rewrite installed module");

        let error = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionInstall { package_ref },
            )
            .await
            .expect_err("duplicate install is rejected before materialization");

        assert!(matches!(
            error,
            ProductWorkflowError::InvalidBindingRequest { .. }
        ));
        assert_eq!(
            std::fs::read(wasm_path).expect("installed module remains"),
            b"existing-live-module"
        );
    }

    #[tokio::test]
    async fn extension_activate_rejects_lifecycle_package_without_installation() {
        let dir = tempfile::tempdir().expect("tempdir");
        let storage_root = dir.path().join("local-dev");
        std::fs::create_dir_all(storage_root.join("system/extensions")).expect("storage root");
        let mut filesystem = LocalFilesystem::new();
        filesystem
            .mount_local(
                VirtualPath::new("/system/extensions").expect("valid virtual path"),
                HostPath::from_path_buf(storage_root.join("system/extensions")),
            )
            .expect("mount system extensions");
        let package = fixture_extension_package().package;
        let mut lifecycle_registry = ExtensionRegistry::new();
        lifecycle_registry
            .insert(package.clone())
            .expect("lifecycle package");
        let mut active_registry_initial = ExtensionRegistry::new();
        active_registry_initial
            .insert(package)
            .expect("active package");
        let active_registry = Arc::new(SharedExtensionRegistry::new(active_registry_initial));
        let port = RebornLocalExtensionManagementPort::new(
            Arc::new(filesystem),
            AvailableExtensionCatalog::from_packages(Vec::new()),
            Arc::new(InMemoryExtensionInstallationStore::default()),
            Arc::new(Mutex::new(ExtensionLifecycleService::new(
                lifecycle_registry,
            ))),
            test_active_extension_publisher(
                Arc::clone(&active_registry),
                test_extension_trust_policy(),
            ),
        );
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "fixture")
            .expect("valid ref");

        let error = port
            .activate(package_ref, ExtensionActivationMode::Static)
            .await
            .expect_err("activation requires an installation record");

        assert!(matches!(
            error,
            ProductWorkflowError::InvalidBindingRequest { .. }
        ));
        assert!(
            active_registry
                .snapshot()
                .get_extension(&ExtensionId::new("fixture").unwrap())
                .is_some()
        );
    }

    #[tokio::test]
    async fn extension_remove_rejects_uninstalled_ref_without_deleting_files() {
        let (_dir, storage_root, facade, _active_registry, _installation_store) =
            extension_lifecycle_fixture();
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "fixture")
            .expect("valid ref");
        let manifest_path = storage_root.join("system/extensions/fixture/manifest.toml");
        std::fs::create_dir_all(manifest_path.parent().expect("manifest parent"))
            .expect("extension directory");
        std::fs::write(&manifest_path, b"unmanaged manifest").expect("write unmanaged file");

        let error = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionRemove { package_ref },
            )
            .await
            .expect_err("remove requires an installation record");

        assert!(matches!(
            error,
            ProductWorkflowError::InvalidBindingRequest { .. }
        ));
        assert_eq!(
            std::fs::read(manifest_path).expect("unmanaged file remains"),
            b"unmanaged manifest"
        );
    }

    #[tokio::test]
    async fn extension_remove_lifecycle_failure_preserves_state() {
        let lifecycle_service = ExtensionLifecycleService::new(ExtensionRegistry::new())
            .with_event_sink(Arc::new(FailingRemoveLifecycleSink));
        let (_dir, storage_root, facade, active_registry, installation_store) =
            extension_lifecycle_fixture_with_service(lifecycle_service);
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "fixture")
            .expect("valid ref");

        facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionInstall {
                    package_ref: package_ref.clone(),
                },
            )
            .await
            .expect("install extension");
        facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionActivate {
                    package_ref: package_ref.clone(),
                },
            )
            .await
            .expect("activate extension");

        let error = facade
            .execute(
                lifecycle_surface_context(),
                LifecycleProductAction::ExtensionRemove { package_ref },
            )
            .await
            .expect_err("lifecycle remove failure is reported");

        assert!(matches!(error, ProductWorkflowError::Transient { .. }));
        let extension_id = ExtensionId::new("fixture").expect("valid extension id");
        let installation_id = ExtensionInstallationId::new("fixture").expect("valid installation");
        assert!(
            active_registry
                .snapshot()
                .get_extension(&extension_id)
                .is_some()
        );
        assert!(
            storage_root
                .join("system/extensions/fixture/manifest.toml")
                .exists()
        );
        assert!(
            storage_root
                .join("system/extensions/fixture/wasm/fixture.wasm")
                .exists()
        );
        let installation = installation_store
            .get_installation(&installation_id)
            .await
            .expect("read installation")
            .expect("installation remains");
        assert_eq!(
            installation.activation_state(),
            ExtensionActivationState::Enabled
        );
        assert!(
            installation_store
                .get_manifest(&extension_id)
                .await
                .expect("read manifest")
                .is_some()
        );
    }

    #[tokio::test]
    async fn extension_remove_installation_delete_failure_restores_active_trust_policy() {
        let (_dir, port, active_registry, failing_store, trust_policy) =
            extension_port_with_delete_installation_failing_store(ExtensionRegistry::new());
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "fixture")
            .expect("valid ref");

        port.install(package_ref.clone())
            .await
            .expect("install extension");
        port.activate_with_prechecked_credentials_for_test(
            package_ref.clone(),
            ExtensionActivationMode::Static,
        )
        .await
        .expect("activate extension");
        let package = fixture_extension_package().package;
        let trust_input = extension_trust_policy_input(&package).expect("trust input");
        assert_eq!(
            trust_policy
                .evaluate(&trust_input)
                .expect("active extension trust")
                .effective_trust
                .class(),
            TrustClass::UserTrusted
        );

        let error = port
            .remove(package_ref)
            .await
            .expect_err("delete installation failure is reported");

        assert!(matches!(
            error,
            ProductWorkflowError::InvalidBindingRequest { .. }
        ));
        let extension_id = ExtensionId::new("fixture").expect("valid extension id");
        let installation_id = ExtensionInstallationId::new("fixture").expect("valid installation");
        let installation = failing_store
            .get_installation(&installation_id)
            .await
            .expect("read installation")
            .expect("installation remains");
        assert_eq!(
            installation.activation_state(),
            ExtensionActivationState::Enabled
        );
        assert!(
            active_registry
                .snapshot()
                .get_extension(&extension_id)
                .is_some()
        );
        assert_eq!(
            trust_policy
                .evaluate(&trust_input)
                .expect("restored active extension trust")
                .effective_trust
                .class(),
            TrustClass::UserTrusted
        );
    }

    #[tokio::test]
    async fn extension_remove_manifest_delete_failure_restores_active_trust_policy() {
        let (_dir, port, active_registry, failing_store, trust_policy) =
            extension_port_with_delete_manifest_failing_store();
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "fixture")
            .expect("valid ref");

        port.install(package_ref.clone())
            .await
            .expect("install extension");
        port.activate_with_prechecked_credentials_for_test(
            package_ref.clone(),
            ExtensionActivationMode::Static,
        )
        .await
        .expect("activate extension");
        let package = fixture_extension_package().package;
        let trust_input = extension_trust_policy_input(&package).expect("trust input");

        let error = port
            .remove(package_ref)
            .await
            .expect_err("delete manifest failure is reported");

        assert!(matches!(
            error,
            ProductWorkflowError::InvalidBindingRequest { .. }
        ));
        assert_enabled_active_extension_state(&active_registry, failing_store.as_ref()).await;
        assert_eq!(
            trust_policy
                .evaluate(&trust_input)
                .expect("restored active extension trust")
                .effective_trust
                .class(),
            TrustClass::UserTrusted
        );
    }

    #[tokio::test]
    async fn extension_remove_file_delete_failure_restores_active_trust_policy() {
        let (_dir, port, active_registry, installation_store, trust_policy) =
            extension_port_with_file_delete_failing_filesystem();
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "fixture")
            .expect("valid ref");

        port.install(package_ref.clone())
            .await
            .expect("install extension");
        port.activate_with_prechecked_credentials_for_test(
            package_ref.clone(),
            ExtensionActivationMode::Static,
        )
        .await
        .expect("activate extension");
        let package = fixture_extension_package().package;
        let trust_input = extension_trust_policy_input(&package).expect("trust input");

        let error = port
            .remove(package_ref)
            .await
            .expect_err("delete files failure is reported");

        assert!(matches!(error, ProductWorkflowError::Transient { .. }));
        assert_enabled_active_extension_state(&active_registry, installation_store.as_ref()).await;
        assert_eq!(
            trust_policy
                .evaluate(&trust_input)
                .expect("restored active extension trust")
                .effective_trust
                .class(),
            TrustClass::UserTrusted
        );
    }

    #[tokio::test]
    async fn extension_auth_and_configure_return_unsupported() {
        let (_dir, _storage_root, facade, _active_registry, _installation_store) =
            extension_lifecycle_fixture();
        let package_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "fixture").unwrap();

        for action in [
            LifecycleProductAction::ExtensionAuth {
                package_ref: package_ref.clone(),
            },
            LifecycleProductAction::ExtensionConfigure {
                package_ref: package_ref.clone(),
                payload: None,
            },
        ] {
            let response = facade
                .execute(lifecycle_surface_context(), action)
                .await
                .expect("unsupported response");
            assert_unsupported_extension_response(
                response,
                "extension_auth_and_configure_not_yet_wired",
            );
        }
    }

    #[tokio::test]
    async fn project_package_returns_available_extension_projection() {
        let (_dir, _storage_root, facade, _active_registry, _installation_store) =
            extension_lifecycle_fixture();
        let response = facade
            .project_package(
                lifecycle_surface_context(),
                LifecyclePackageRef::new(LifecyclePackageKind::Extension, "fixture").unwrap(),
            )
            .await
            .expect("extension projection");

        assert_eq!(response.phase, LifecyclePhase::Discovered);
        let Some(LifecycleProductPayload::ExtensionList { extensions, count }) = response.payload
        else {
            panic!("expected extension list projection");
        };
        assert_eq!(count, 1);
        assert_eq!(extensions[0].summary.package_ref.id.as_str(), "fixture");
    }

    fn extension_lifecycle_fixture() -> (
        tempfile::TempDir,
        std::path::PathBuf,
        crate::lifecycle::RebornLocalLifecycleFacade,
        Arc<SharedExtensionRegistry>,
        Arc<InMemoryExtensionInstallationStore>,
    ) {
        extension_lifecycle_fixture_with_catalog_and_service(
            AvailableExtensionCatalog::from_packages(vec![fixture_extension_package()]),
            ExtensionLifecycleService::new(ExtensionRegistry::new()),
        )
    }

    fn extension_lifecycle_fixture_with_service(
        lifecycle_service: ExtensionLifecycleService,
    ) -> (
        tempfile::TempDir,
        std::path::PathBuf,
        crate::lifecycle::RebornLocalLifecycleFacade,
        Arc<SharedExtensionRegistry>,
        Arc<InMemoryExtensionInstallationStore>,
    ) {
        extension_lifecycle_fixture_with_catalog_and_service(
            AvailableExtensionCatalog::from_packages(vec![fixture_extension_package()]),
            lifecycle_service,
        )
    }

    fn github_extension_lifecycle_fixture() -> (
        tempfile::TempDir,
        std::path::PathBuf,
        crate::lifecycle::RebornLocalLifecycleFacade,
        Arc<SharedExtensionRegistry>,
        Arc<InMemoryExtensionInstallationStore>,
    ) {
        extension_lifecycle_fixture_with_catalog_and_service(
            AvailableExtensionCatalog::from_first_party_assets()
                .expect("first-party GitHub catalog"),
            ExtensionLifecycleService::new(ExtensionRegistry::new()),
        )
    }

    fn extension_management_port_fixture_with_catalog_and_service(
        catalog: AvailableExtensionCatalog,
        lifecycle_service: ExtensionLifecycleService,
    ) -> (
        tempfile::TempDir,
        std::path::PathBuf,
        Arc<RebornLocalExtensionManagementPort>,
        Arc<SharedExtensionRegistry>,
        Arc<InMemoryExtensionInstallationStore>,
    ) {
        let (dir, storage_root, extension_management, active_registry, installation_store, _) =
            extension_management_port_fixture_with_catalog_service_and_trust(
                catalog,
                lifecycle_service,
            );
        (
            dir,
            storage_root,
            extension_management,
            active_registry,
            installation_store,
        )
    }

    fn extension_management_port_fixture_with_catalog_service_and_trust(
        catalog: AvailableExtensionCatalog,
        lifecycle_service: ExtensionLifecycleService,
    ) -> (
        tempfile::TempDir,
        std::path::PathBuf,
        Arc<RebornLocalExtensionManagementPort>,
        Arc<SharedExtensionRegistry>,
        Arc<InMemoryExtensionInstallationStore>,
        Arc<HostTrustPolicy>,
    ) {
        let trust_policy = test_extension_trust_policy();
        let (dir, storage_root, extension_management, active_registry, installation_store) =
            extension_management_port_fixture_with_catalog_service_and_trust_policy(
                catalog,
                lifecycle_service,
                Arc::clone(&trust_policy),
            );
        (
            dir,
            storage_root,
            extension_management,
            active_registry,
            installation_store,
            trust_policy,
        )
    }

    fn extension_management_port_fixture_with_catalog_service_and_trust_policy(
        catalog: AvailableExtensionCatalog,
        lifecycle_service: ExtensionLifecycleService,
        trust_policy: Arc<HostTrustPolicy>,
    ) -> (
        tempfile::TempDir,
        std::path::PathBuf,
        Arc<RebornLocalExtensionManagementPort>,
        Arc<SharedExtensionRegistry>,
        Arc<InMemoryExtensionInstallationStore>,
    ) {
        let dir = tempfile::tempdir().expect("tempdir");
        let storage_root = dir.path().join("local-dev");
        std::fs::create_dir_all(storage_root.join("system/extensions")).expect("storage root");

        let mut filesystem = LocalFilesystem::new();
        filesystem
            .mount_local(
                VirtualPath::new("/projects").expect("valid virtual path"),
                HostPath::from_path_buf(storage_root.clone()),
            )
            .expect("mount storage root");
        filesystem
            .mount_local(
                VirtualPath::new("/system/extensions").expect("valid virtual path"),
                HostPath::from_path_buf(storage_root.join("system/extensions")),
            )
            .expect("mount system extensions");
        let filesystem = Arc::new(filesystem);
        let root_filesystem: Arc<dyn RootFilesystem> = filesystem.clone();
        let active_registry = Arc::new(SharedExtensionRegistry::new(ExtensionRegistry::new()));
        let installation_store = Arc::new(InMemoryExtensionInstallationStore::default());
        let extension_management = Arc::new(RebornLocalExtensionManagementPort::new(
            root_filesystem,
            catalog,
            installation_store.clone(),
            Arc::new(Mutex::new(lifecycle_service)),
            test_active_extension_publisher(
                Arc::clone(&active_registry),
                Arc::clone(&trust_policy),
            ),
        ));
        (
            dir,
            storage_root,
            extension_management,
            active_registry,
            installation_store,
        )
    }

    fn extension_lifecycle_fixture_with_catalog_and_service(
        catalog: AvailableExtensionCatalog,
        lifecycle_service: ExtensionLifecycleService,
    ) -> (
        tempfile::TempDir,
        std::path::PathBuf,
        crate::lifecycle::RebornLocalLifecycleFacade,
        Arc<SharedExtensionRegistry>,
        Arc<InMemoryExtensionInstallationStore>,
    ) {
        let dir = tempfile::tempdir().expect("tempdir");
        let storage_root = dir.path().join("local-dev");
        std::fs::create_dir_all(storage_root.join("system/extensions")).expect("storage root");

        let mut filesystem = LocalFilesystem::new();
        filesystem
            .mount_local(
                VirtualPath::new("/projects").expect("valid virtual path"),
                HostPath::from_path_buf(storage_root.clone()),
            )
            .expect("mount storage root");
        filesystem
            .mount_local(
                VirtualPath::new("/system/extensions").expect("valid virtual path"),
                HostPath::from_path_buf(storage_root.join("system/extensions")),
            )
            .expect("mount system extensions");
        let filesystem = Arc::new(filesystem);
        let root_filesystem: Arc<dyn RootFilesystem> = filesystem.clone();
        let skill_management = Arc::new(crate::lifecycle::RebornLocalSkillManagementPort::new(
            UserId::new("lifecycle-owner").expect("valid user"),
            root_filesystem.clone(),
            MountView::new(vec![MountGrant::new(
                MountAlias::new("/skills").expect("valid alias"),
                VirtualPath::new("/projects/skills").expect("valid path"),
                MountPermissions::read_write_list_delete(),
            )])
            .expect("valid mount view"),
        ));
        let active_registry = Arc::new(SharedExtensionRegistry::new(ExtensionRegistry::new()));
        let installation_store = Arc::new(InMemoryExtensionInstallationStore::default());
        let extension_management = Arc::new(RebornLocalExtensionManagementPort::new(
            root_filesystem,
            catalog,
            installation_store.clone(),
            Arc::new(Mutex::new(lifecycle_service)),
            test_active_extension_publisher(
                Arc::clone(&active_registry),
                test_extension_trust_policy(),
            ),
        ));
        let facade = crate::lifecycle::RebornLocalLifecycleFacade::new(skill_management)
            .with_extension_management(extension_management);
        (
            dir,
            storage_root,
            facade,
            active_registry,
            installation_store,
        )
    }

    fn extension_port_with_delete_installation_failing_store(
        initial_active_registry: ExtensionRegistry,
    ) -> (
        tempfile::TempDir,
        RebornLocalExtensionManagementPort,
        Arc<SharedExtensionRegistry>,
        Arc<DeleteInstallationFailingStore>,
        Arc<HostTrustPolicy>,
    ) {
        extension_port_with_delete_failing_store(
            initial_active_registry,
            DeleteInstallationFailingStore::default(),
        )
    }

    fn extension_port_with_delete_manifest_failing_store() -> (
        tempfile::TempDir,
        RebornLocalExtensionManagementPort,
        Arc<SharedExtensionRegistry>,
        Arc<DeleteInstallationFailingStore>,
        Arc<HostTrustPolicy>,
    ) {
        extension_port_with_delete_failing_store(
            ExtensionRegistry::new(),
            DeleteInstallationFailingStore::fail_manifest_delete(),
        )
    }

    fn extension_port_with_set_activation_failing_store(
        lifecycle_service: ExtensionLifecycleService,
    ) -> (
        tempfile::TempDir,
        RebornLocalExtensionManagementPort,
        Arc<SharedExtensionRegistry>,
        Arc<DeleteInstallationFailingStore>,
        Arc<HostTrustPolicy>,
    ) {
        extension_port_with_failing_store(
            ExtensionRegistry::new(),
            DeleteInstallationFailingStore::fail_set_activation_enabled(),
            lifecycle_service,
        )
    }

    fn extension_port_with_delete_failing_store(
        initial_active_registry: ExtensionRegistry,
        failing_store: DeleteInstallationFailingStore,
    ) -> (
        tempfile::TempDir,
        RebornLocalExtensionManagementPort,
        Arc<SharedExtensionRegistry>,
        Arc<DeleteInstallationFailingStore>,
        Arc<HostTrustPolicy>,
    ) {
        extension_port_with_failing_store(
            initial_active_registry,
            failing_store,
            ExtensionLifecycleService::new(ExtensionRegistry::new()),
        )
    }

    fn extension_port_with_failing_store(
        initial_active_registry: ExtensionRegistry,
        failing_store: DeleteInstallationFailingStore,
        lifecycle_service: ExtensionLifecycleService,
    ) -> (
        tempfile::TempDir,
        RebornLocalExtensionManagementPort,
        Arc<SharedExtensionRegistry>,
        Arc<DeleteInstallationFailingStore>,
        Arc<HostTrustPolicy>,
    ) {
        let dir = tempfile::tempdir().expect("tempdir");
        let storage_root = dir.path().join("local-dev");
        std::fs::create_dir_all(storage_root.join("system/extensions")).expect("storage root");
        let mut filesystem = LocalFilesystem::new();
        filesystem
            .mount_local(
                VirtualPath::new("/projects").expect("valid virtual path"),
                HostPath::from_path_buf(storage_root.clone()),
            )
            .expect("mount storage root");
        filesystem
            .mount_local(
                VirtualPath::new("/system/extensions").expect("valid virtual path"),
                HostPath::from_path_buf(storage_root.join("system/extensions")),
            )
            .expect("mount system extensions");
        let filesystem = Arc::new(filesystem);
        let root_filesystem: Arc<dyn RootFilesystem> = filesystem.clone();
        let active_registry = Arc::new(SharedExtensionRegistry::new(initial_active_registry));
        let trust_policy = test_extension_trust_policy();
        let failing_store = Arc::new(failing_store);
        let installation_store: Arc<dyn ExtensionInstallationStore> = failing_store.clone();
        let port = RebornLocalExtensionManagementPort::new(
            root_filesystem,
            AvailableExtensionCatalog::from_packages(vec![fixture_extension_package()]),
            installation_store,
            Arc::new(Mutex::new(lifecycle_service)),
            test_active_extension_publisher(
                Arc::clone(&active_registry),
                Arc::clone(&trust_policy),
            ),
        );
        (dir, port, active_registry, failing_store, trust_policy)
    }

    fn extension_port_with_file_delete_failing_filesystem() -> (
        tempfile::TempDir,
        RebornLocalExtensionManagementPort,
        Arc<SharedExtensionRegistry>,
        Arc<InMemoryExtensionInstallationStore>,
        Arc<HostTrustPolicy>,
    ) {
        let dir = tempfile::tempdir().expect("tempdir");
        let storage_root = dir.path().join("local-dev");
        std::fs::create_dir_all(storage_root.join("system/extensions")).expect("storage root");
        let mut filesystem = LocalFilesystem::new();
        filesystem
            .mount_local(
                VirtualPath::new("/projects").expect("valid virtual path"),
                HostPath::from_path_buf(storage_root.clone()),
            )
            .expect("mount storage root");
        filesystem
            .mount_local(
                VirtualPath::new("/system/extensions").expect("valid virtual path"),
                HostPath::from_path_buf(storage_root.join("system/extensions")),
            )
            .expect("mount system extensions");
        let filesystem: Arc<dyn RootFilesystem> = Arc::new(filesystem);
        let root_filesystem: Arc<dyn RootFilesystem> =
            Arc::new(DeleteFailingRootFilesystem { inner: filesystem });
        let active_registry = Arc::new(SharedExtensionRegistry::new(ExtensionRegistry::new()));
        let trust_policy = test_extension_trust_policy();
        let installation_store = Arc::new(InMemoryExtensionInstallationStore::default());
        let extension_installation_store: Arc<dyn ExtensionInstallationStore> =
            installation_store.clone();
        let port = RebornLocalExtensionManagementPort::new(
            root_filesystem,
            AvailableExtensionCatalog::from_packages(vec![fixture_extension_package()]),
            extension_installation_store,
            Arc::new(Mutex::new(ExtensionLifecycleService::new(
                ExtensionRegistry::new(),
            ))),
            test_active_extension_publisher(
                Arc::clone(&active_registry),
                Arc::clone(&trust_policy),
            ),
        );
        (dir, port, active_registry, installation_store, trust_policy)
    }

    struct FailingRemoveLifecycleSink;

    #[async_trait]
    impl ExtensionLifecycleEventSink for FailingRemoveLifecycleSink {
        async fn record_extension_lifecycle_event(
            &self,
            event: ExtensionLifecycleEvent,
        ) -> Result<(), ExtensionError> {
            if event.operation == ExtensionLifecycleOperation::Remove {
                return Err(ExtensionError::LifecycleEventSink {
                    extension_id: event.extension_id,
                    operation: event.operation,
                });
            }
            Ok(())
        }
    }

    #[derive(Default)]
    struct RecordingLifecycleSink {
        operations: std::sync::Mutex<Vec<ExtensionLifecycleOperation>>,
    }

    impl RecordingLifecycleSink {
        fn operations(&self) -> Vec<ExtensionLifecycleOperation> {
            self.operations
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone()
        }
    }

    #[async_trait]
    impl ExtensionLifecycleEventSink for RecordingLifecycleSink {
        async fn record_extension_lifecycle_event(
            &self,
            event: ExtensionLifecycleEvent,
        ) -> Result<(), ExtensionError> {
            self.operations
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(event.operation);
            Ok(())
        }
    }

    #[derive(Default)]
    struct DeleteInstallationFailingStore {
        inner: InMemoryExtensionInstallationStore,
        fail_manifest_delete: bool,
        fail_set_activation_enabled: bool,
        fail_get_installation: bool,
        mismatched_get_installation: bool,
    }

    impl DeleteInstallationFailingStore {
        fn fail_manifest_delete() -> Self {
            Self {
                inner: InMemoryExtensionInstallationStore::default(),
                fail_manifest_delete: true,
                fail_set_activation_enabled: false,
                fail_get_installation: false,
                mismatched_get_installation: false,
            }
        }

        fn fail_set_activation_enabled() -> Self {
            Self {
                inner: InMemoryExtensionInstallationStore::default(),
                fail_manifest_delete: false,
                fail_set_activation_enabled: true,
                fail_get_installation: false,
                mismatched_get_installation: false,
            }
        }

        fn fail_get_installation() -> Self {
            Self {
                inner: InMemoryExtensionInstallationStore::default(),
                fail_manifest_delete: false,
                fail_set_activation_enabled: false,
                fail_get_installation: true,
                mismatched_get_installation: false,
            }
        }

        fn mismatched_get_installation() -> Self {
            Self {
                inner: InMemoryExtensionInstallationStore::default(),
                fail_manifest_delete: false,
                fail_set_activation_enabled: false,
                fail_get_installation: false,
                mismatched_get_installation: true,
            }
        }
    }

    #[async_trait]
    impl ExtensionInstallationStore for DeleteInstallationFailingStore {
        async fn list_manifests(
            &self,
        ) -> Result<Vec<ExtensionManifestRecord>, ExtensionInstallationError> {
            self.inner.list_manifests().await
        }

        async fn get_manifest(
            &self,
            extension_id: &ExtensionId,
        ) -> Result<Option<ExtensionManifestRecord>, ExtensionInstallationError> {
            self.inner.get_manifest(extension_id).await
        }

        async fn upsert_manifest(
            &self,
            manifest: ExtensionManifestRecord,
        ) -> Result<(), ExtensionInstallationError> {
            self.inner.upsert_manifest(manifest).await
        }

        async fn upsert_manifest_and_installation(
            &self,
            manifest: ExtensionManifestRecord,
            installation: ExtensionInstallation,
        ) -> Result<(), ExtensionInstallationError> {
            self.inner
                .upsert_manifest_and_installation(manifest, installation)
                .await
        }

        async fn list_installations(
            &self,
        ) -> Result<Vec<ExtensionInstallation>, ExtensionInstallationError> {
            self.inner.list_installations().await
        }

        async fn list_enabled_installations(
            &self,
        ) -> Result<Vec<ExtensionInstallation>, ExtensionInstallationError> {
            self.inner.list_enabled_installations().await
        }

        async fn get_installation(
            &self,
            installation_id: &ExtensionInstallationId,
        ) -> Result<Option<ExtensionInstallation>, ExtensionInstallationError> {
            if self.fail_get_installation {
                return Err(ExtensionInstallationError::InvalidInstallation {
                    reason: "get installation failed".to_string(),
                });
            }
            if self.mismatched_get_installation {
                let extension_id = ExtensionId::new("other-fixture").expect("valid extension id");
                let installation = ExtensionInstallation::new(
                    installation_id.clone(),
                    extension_id.clone(),
                    ExtensionActivationState::Installed,
                    ExtensionManifestRef::new(extension_id, None),
                    Vec::new(),
                    chrono::Utc::now(),
                )
                .expect("mismatched installation fixture");
                return Ok(Some(installation));
            }
            self.inner.get_installation(installation_id).await
        }

        async fn upsert_installation(
            &self,
            installation: ExtensionInstallation,
        ) -> Result<(), ExtensionInstallationError> {
            self.inner.upsert_installation(installation).await
        }

        async fn set_activation_state(
            &self,
            installation_id: &ExtensionInstallationId,
            state: ExtensionActivationState,
        ) -> Result<(), ExtensionInstallationError> {
            if self.fail_set_activation_enabled && state == ExtensionActivationState::Enabled {
                return Err(ExtensionInstallationError::InvalidInstallation {
                    reason: "set activation state failed".to_string(),
                });
            }
            self.inner
                .set_activation_state(installation_id, state)
                .await
        }

        async fn delete_installation(
            &self,
            installation_id: &ExtensionInstallationId,
        ) -> Result<(), ExtensionInstallationError> {
            if self.fail_manifest_delete {
                self.inner.delete_installation(installation_id).await
            } else {
                Err(ExtensionInstallationError::InvalidInstallation {
                    reason: "delete installation failed".to_string(),
                })
            }
        }

        async fn delete_manifest(
            &self,
            extension_id: &ExtensionId,
        ) -> Result<(), ExtensionInstallationError> {
            if self.fail_manifest_delete {
                Err(ExtensionInstallationError::InvalidInstallation {
                    reason: "delete manifest failed".to_string(),
                })
            } else {
                self.inner.delete_manifest(extension_id).await
            }
        }

        async fn update_health(
            &self,
            installation_id: &ExtensionInstallationId,
            health: ironclaw_extensions::ExtensionHealthSnapshot,
        ) -> Result<(), ExtensionInstallationError> {
            self.inner.update_health(installation_id, health).await
        }
    }

    async fn fixture_installation_state<S>(store: &S) -> ExtensionActivationState
    where
        S: ExtensionInstallationStore + ?Sized,
    {
        let installation_id = ExtensionInstallationId::new("fixture").expect("valid installation");
        store
            .get_installation(&installation_id)
            .await
            .expect("read fixture installation")
            .expect("fixture installation remains")
            .activation_state()
    }

    struct DeleteFailingRootFilesystem {
        inner: Arc<dyn RootFilesystem>,
    }

    #[async_trait]
    impl RootFilesystem for DeleteFailingRootFilesystem {
        fn capabilities(&self) -> ironclaw_filesystem::BackendCapabilities {
            self.inner.capabilities()
        }

        async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
            self.inner.list_dir(path).await
        }

        async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
            self.inner.stat(path).await
        }

        async fn read_file(&self, path: &VirtualPath) -> Result<Vec<u8>, FilesystemError> {
            self.inner.read_file(path).await
        }

        async fn write_file(
            &self,
            path: &VirtualPath,
            bytes: &[u8],
        ) -> Result<(), FilesystemError> {
            self.inner.write_file(path, bytes).await
        }

        async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
            Err(FilesystemError::Backend {
                path: path.clone(),
                operation: FilesystemOperation::Delete,
                reason: "delete failed".to_string(),
            })
        }
    }

    async fn assert_enabled_active_extension_state<S>(
        active_registry: &SharedExtensionRegistry,
        installation_store: &S,
    ) where
        S: ExtensionInstallationStore + ?Sized,
    {
        let extension_id = ExtensionId::new("fixture").expect("valid extension id");
        let installation_id = ExtensionInstallationId::new("fixture").expect("valid installation");
        let installation = installation_store
            .get_installation(&installation_id)
            .await
            .expect("read installation")
            .expect("installation remains");
        assert_eq!(
            installation.activation_state(),
            ExtensionActivationState::Enabled
        );
        assert!(
            active_registry
                .snapshot()
                .get_extension(&extension_id)
                .is_some()
        );
    }

    fn hosted_mcp_scope(user_id: &str) -> ResourceScope {
        ResourceScope::local_default(
            UserId::new(user_id).expect("valid user"),
            InvocationId::new(),
        )
        .expect("valid local scope")
    }

    struct FailsSecondCredentialGate {
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl ExtensionActivationCredentialGate for FailsSecondCredentialGate {
        async fn ensure_credentials(
            &self,
            _package: &ExtensionPackage,
        ) -> Result<(), ProductWorkflowError> {
            if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
                return Ok(());
            }
            Err(ProductWorkflowError::InvalidBindingRequest {
                reason: "post-discovery credential recheck failed".to_string(),
            })
        }
    }

    struct EmptyToolsHostedMcpEgress;

    #[async_trait]
    impl RuntimeHttpEgress for EmptyToolsHostedMcpEgress {
        async fn execute(
            &self,
            request: RuntimeHttpEgressRequest,
        ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
            hosted_mcp_response_for_request(request, serde_json::json!({ "tools": [] })).await
        }
    }

    struct BlockingToolsListHostedMcpEgress {
        started: std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
        release: tokio::sync::Mutex<tokio::sync::oneshot::Receiver<()>>,
    }

    impl BlockingToolsListHostedMcpEgress {
        fn new() -> (
            Arc<Self>,
            tokio::sync::oneshot::Receiver<()>,
            tokio::sync::oneshot::Sender<()>,
        ) {
            let (started_tx, started_rx) = tokio::sync::oneshot::channel();
            let (release_tx, release_rx) = tokio::sync::oneshot::channel();
            (
                Arc::new(Self {
                    started: std::sync::Mutex::new(Some(started_tx)),
                    release: tokio::sync::Mutex::new(release_rx),
                }),
                started_rx,
                release_tx,
            )
        }
    }

    #[async_trait]
    impl RuntimeHttpEgress for BlockingToolsListHostedMcpEgress {
        async fn execute(
            &self,
            request: RuntimeHttpEgressRequest,
        ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
            let body = parse_test_json_rpc_body(&request)?;
            if body.get("method").and_then(serde_json::Value::as_str) == Some("tools/list") {
                if let Some(started) = self
                    .started
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take()
                {
                    let _ = started.send(());
                }
                let mut release = self.release.lock().await;
                let _ = (&mut *release).await;
            }
            hosted_mcp_response_for_body(
                body,
                request.body.len() as u64,
                discovered_tools_payload(),
            )
        }
    }

    async fn hosted_mcp_response_for_request(
        request: RuntimeHttpEgressRequest,
        tools_list_result: serde_json::Value,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        let request_bytes = request.body.len() as u64;
        let body = parse_test_json_rpc_body(&request)?;
        hosted_mcp_response_for_body(body, request_bytes, tools_list_result)
    }

    fn parse_test_json_rpc_body(
        request: &RuntimeHttpEgressRequest,
    ) -> Result<serde_json::Value, RuntimeHttpEgressError> {
        if request.method != NetworkMethod::Post {
            return Err(RuntimeHttpEgressError::Request {
                reason: "unexpected_method".to_string(),
                request_bytes: request.body.len() as u64,
                response_bytes: 0,
            });
        }
        serde_json::from_slice(&request.body).map_err(|_| RuntimeHttpEgressError::Request {
            reason: "invalid_json_rpc_body".to_string(),
            request_bytes: request.body.len() as u64,
            response_bytes: 0,
        })
    }

    fn hosted_mcp_response_for_body(
        body: serde_json::Value,
        request_bytes: u64,
        tools_list_result: serde_json::Value,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        let method = body
            .get("method")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| RuntimeHttpEgressError::Request {
                reason: "missing_json_rpc_method".to_string(),
                request_bytes,
                response_bytes: 0,
            })?;
        match method {
            "initialize" => test_runtime_json_response(
                body["id"].as_u64(),
                serde_json::json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {"tools": {}},
                    "serverInfo": {"name": "notion-test", "version": "1.0.0"}
                }),
                vec![("Mcp-Session-Id".to_string(), "session-1".to_string())],
            ),
            "notifications/initialized" => {
                test_runtime_json_response(None, serde_json::json!({}), Vec::new())
            }
            "tools/list" => {
                test_runtime_json_response(body["id"].as_u64(), tools_list_result, Vec::new())
            }
            _ => Err(RuntimeHttpEgressError::Request {
                reason: "unexpected_method".to_string(),
                request_bytes,
                response_bytes: 0,
            }),
        }
    }

    fn discovered_tools_payload() -> serde_json::Value {
        serde_json::json!({
            "tools": [
                {
                    "name": "live-search",
                    "description": "Search live Notion content",
                    "inputSchema": {
                        "type": "object",
                        "properties": {"query": {"type": "string"}},
                        "required": ["query"]
                    }
                }
            ]
        })
    }

    fn test_runtime_json_response(
        id: Option<u64>,
        result: serde_json::Value,
        extra_headers: Vec<(String, String)>,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        let mut headers = vec![("content-type".to_string(), "application/json".to_string())];
        headers.extend(extra_headers);
        let body = serde_json::to_vec(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": result,
        }))
        .expect("serialize test JSON-RPC response");
        Ok(RuntimeHttpEgressResponse {
            status: 200,
            headers,
            response_bytes: body.len() as u64,
            body,
            saved_body: None,
            request_bytes: 0,
            redaction_applied: false,
        })
    }

    struct MissingRuntimeCredentialAccounts;

    #[async_trait]
    impl crate::product_auth_runtime_credentials::RuntimeCredentialAccountSelectionService
        for MissingRuntimeCredentialAccounts
    {
        async fn select_configured_account_for_binding(
            &self,
            _lookup: ironclaw_auth::CredentialAccountSelectionRequest,
            _runtime_scope: ironclaw_auth::AuthProductScope,
        ) -> Result<ironclaw_auth::CredentialAccount, ironclaw_auth::AuthProductError> {
            Err(ironclaw_auth::AuthProductError::CredentialMissing)
        }

        async fn select_unique_configured_runtime_account(
            &self,
            _request: crate::product_auth_runtime_credentials::RuntimeCredentialAccountSelectionRequest,
        ) -> Result<ironclaw_auth::CredentialAccount, ironclaw_auth::AuthProductError> {
            Err(ironclaw_auth::AuthProductError::CredentialMissing)
        }
    }

    struct ConfiguredRuntimeCredentialAccounts;

    #[async_trait]
    impl crate::product_auth_runtime_credentials::RuntimeCredentialAccountSelectionService
        for ConfiguredRuntimeCredentialAccounts
    {
        async fn select_configured_account_for_binding(
            &self,
            _lookup: ironclaw_auth::CredentialAccountSelectionRequest,
            _runtime_scope: ironclaw_auth::AuthProductScope,
        ) -> Result<ironclaw_auth::CredentialAccount, ironclaw_auth::AuthProductError> {
            Err(ironclaw_auth::AuthProductError::CredentialMissing)
        }

        async fn select_unique_configured_runtime_account(
            &self,
            _request: crate::product_auth_runtime_credentials::RuntimeCredentialAccountSelectionRequest,
        ) -> Result<ironclaw_auth::CredentialAccount, ironclaw_auth::AuthProductError> {
            let now = chrono::Utc::now();
            Ok(ironclaw_auth::CredentialAccount {
                id: ironclaw_auth::CredentialAccountId::new(),
                scope: ironclaw_auth::AuthProductScope::new(
                    ResourceScope::local_default(
                        UserId::new("credential-user").expect("valid user"),
                        InvocationId::new(),
                    )
                    .expect("valid scope"),
                    ironclaw_auth::AuthSurface::Api,
                ),
                provider: ironclaw_auth::AuthProviderId::new("test-provider")
                    .expect("valid provider"),
                label: ironclaw_auth::CredentialAccountLabel::new("test-provider")
                    .expect("valid label"),
                status: ironclaw_auth::CredentialAccountStatus::Configured,
                ownership: ironclaw_auth::CredentialOwnership::UserReusable,
                owner_extension: None,
                granted_extensions: Vec::new(),
                access_secret: Some(
                    ironclaw_host_api::SecretHandle::new("test-secret")
                        .expect("valid secret handle"),
                ),
                refresh_secret: None,
                scopes: Vec::new(),
                created_at: now,
                updated_at: now,
            })
        }
    }

    struct BackendUnavailableRuntimeCredentialAccounts;

    #[async_trait]
    impl crate::product_auth_runtime_credentials::RuntimeCredentialAccountSelectionService
        for BackendUnavailableRuntimeCredentialAccounts
    {
        async fn select_configured_account_for_binding(
            &self,
            _lookup: ironclaw_auth::CredentialAccountSelectionRequest,
            _runtime_scope: ironclaw_auth::AuthProductScope,
        ) -> Result<ironclaw_auth::CredentialAccount, ironclaw_auth::AuthProductError> {
            Err(ironclaw_auth::AuthProductError::BackendUnavailable)
        }

        async fn select_unique_configured_runtime_account(
            &self,
            _request: crate::product_auth_runtime_credentials::RuntimeCredentialAccountSelectionRequest,
        ) -> Result<ironclaw_auth::CredentialAccount, ironclaw_auth::AuthProductError> {
            Err(ironclaw_auth::AuthProductError::BackendUnavailable)
        }
    }

    fn lifecycle_surface_context() -> LifecycleProductContext {
        LifecycleProductContext::Surface(LifecycleProductSurfaceContext {
            tenant_id: TenantId::new("lifecycle-tenant").expect("valid tenant"),
            user_id: UserId::new("lifecycle-owner").expect("valid user"),
            agent_id: None,
            project_id: None,
        })
    }

    fn test_extension_trust_policy() -> Arc<HostTrustPolicy> {
        Arc::new(
            HostTrustPolicy::new(vec![Box::new(ironclaw_trust::AdminConfig::new())])
                .expect("test trust policy"),
        )
    }

    fn test_active_extension_publisher(
        active_registry: Arc<SharedExtensionRegistry>,
        trust_policy: Arc<HostTrustPolicy>,
    ) -> ActiveExtensionPublisher {
        ActiveExtensionPublisher::new(
            active_registry,
            trust_policy,
            Arc::new(InvalidationBus::new()),
        )
    }

    fn fixture_extension_package() -> AvailableExtensionPackage {
        fixture_extension_package_from_manifest(fixture_extension_manifest())
    }

    fn fixture_extension_package_with_description(description: &str) -> AvailableExtensionPackage {
        let manifest = fixture_extension_manifest().replace(
            "description = \"Lifecycle fixture extension\"",
            &format!("description = \"{description}\""),
        );
        fixture_extension_package_from_manifest(&manifest)
    }

    fn fixture_external_channel_package(id: &str, name: &str) -> AvailableExtensionPackage {
        let manifest = format!(
            r#"
schema_version = "reborn.extension_manifest.v2"
id = "{id}"
name = "{name}"
version = "0.1.0"
description = "{name} channel fixture"
trust = "first_party_requested"

[runtime]
kind = "first_party"
service = "{id}_host"

[[host_api]]
id = "ironclaw.product_adapter/v1"
section = "product_adapter.inbound"

[product_adapter.inbound]
surface_kind = "external_channel"

[product_adapter.inbound.auth]
kind = "request_signature"
header_name = "X-Channel-Signature"
timestamp_header_name = "X-Channel-Timestamp"

[product_adapter.inbound.capabilities]
flags = ["inbound_messages"]

[[product_adapter.inbound.required_credentials]]
handle = "{id}_bot_token"

[[product_adapter.inbound.egress]]
host = "example.com"
credential_handle = "{id}_bot_token"
"#
        );
        let mut package =
            fixture_extension_package_from_manifest_with_product_adapter_contracts(&manifest, id);
        package.surface_kinds = vec![LifecycleExtensionSurfaceKind::ExternalChannel];
        package
    }

    fn fixture_extension_manifest() -> &'static str {
        r#"
schema_version = "reborn.extension_manifest.v2"
id = "fixture"
name = "Fixture Extension"
version = "0.1.0"
description = "Lifecycle fixture extension"
trust = "first_party_requested"

[runtime]
kind = "wasm"
module = "wasm/fixture.wasm"

[[capabilities]]
id = "fixture.search"
description = "Search fixture data"
effects = ["network"]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/search.input.json"
output_schema_ref = "schemas/search.output.json"

[[capabilities]]
id = "fixture.write"
description = "Write fixture data"
effects = ["network", "external_write"]
default_permission = "ask"
visibility = "host_internal"
input_schema_ref = "schemas/write.input.json"
output_schema_ref = "schemas/write.output.json"
"#
    }

    fn fixture_installed_local_manifest() -> &'static str {
        r#"
schema_version = "reborn.extension_manifest.v2"
id = "fixture"
name = "Fixture Extension"
version = "0.1.0"
description = "Installed local fixture extension"
trust = "third_party"

[runtime]
kind = "wasm"
module = "wasm/fixture.wasm"

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
id = "fixture.search"
description = "Search fixture data"
effects = ["network"]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/search.input.json"
output_schema_ref = "schemas/search.output.json"
"#
    }

    fn fixture_extension_package_from_manifest(manifest_toml: &str) -> AvailableExtensionPackage {
        fixture_extension_package_from_manifest_with_root(manifest_toml, "fixture")
    }

    fn fixture_extension_package_from_manifest_with_root(
        manifest_toml: &str,
        root_id: &str,
    ) -> AvailableExtensionPackage {
        let manifest = ExtensionManifest::parse(
            manifest_toml,
            ManifestSource::HostBundled,
            &HostPortCatalog::empty(),
        )
        .expect("fixture manifest");
        fixture_extension_package_from_parsed_manifest(manifest_toml, root_id, manifest)
    }

    fn fixture_extension_package_from_manifest_with_product_adapter_contracts(
        manifest_toml: &str,
        root_id: &str,
    ) -> AvailableExtensionPackage {
        let mut contracts = ironclaw_extensions::HostApiContractRegistry::new();
        contracts
            .register(Arc::new(
                ironclaw_product_adapter_registry::ProductAdapterHostApiContract::new()
                    .expect("product adapter host API contract"),
            ))
            .expect("register product adapter host API contract");
        let manifest = ExtensionManifest::parse_with_host_api_contracts(
            manifest_toml,
            ManifestSource::HostBundled,
            &HostPortCatalog::empty(),
            &contracts,
        )
        .expect("fixture manifest");
        fixture_extension_package_from_parsed_manifest(manifest_toml, root_id, manifest)
    }

    fn fixture_extension_package_from_parsed_manifest(
        manifest_toml: &str,
        root_id: &str,
        manifest: ExtensionManifest,
    ) -> AvailableExtensionPackage {
        let root =
            VirtualPath::new(format!("/system/extensions/{root_id}")).expect("extension root");
        let package = ExtensionPackage::from_manifest_toml(manifest, root, manifest_toml)
            .expect("fixture package");
        AvailableExtensionPackage {
            package_ref: LifecyclePackageRef::new(LifecyclePackageKind::Extension, root_id)
                .expect("fixture package ref"),
            manifest_toml: manifest_toml.to_string(),
            package,
            surface_kinds: Vec::new(),
            assets: vec![
                AvailableExtensionAsset {
                    path: "manifest.toml".to_string(),
                    content: AvailableExtensionAssetContent::Bytes(
                        manifest_toml.as_bytes().to_vec(),
                    ),
                },
                AvailableExtensionAsset {
                    path: "wasm/fixture.wasm".to_string(),
                    content: AvailableExtensionAssetContent::Bytes(b"\0asm\x01\0\0\0".to_vec()),
                },
            ],
        }
    }

    fn fixture_manifest_record_with_source(
        manifest_toml: &str,
        source: ManifestSource,
        manifest_hash: Option<String>,
    ) -> ExtensionManifestRecord {
        let host_ports =
            ironclaw_host_runtime::default_host_port_catalog().expect("host port catalog");
        let contracts = ironclaw_host_runtime::default_host_api_contract_registry()
            .expect("host API contracts");
        ExtensionManifestRecord::from_toml_with_contracts(
            manifest_toml,
            source,
            &host_ports,
            manifest_hash
                .map(ManifestHash::new)
                .transpose()
                .expect("valid manifest hash"),
            &contracts,
        )
        .expect("fixture manifest record")
    }

    fn fixture_installation(
        manifest_hash: Option<String>,
        activation_state: ExtensionActivationState,
    ) -> ExtensionInstallation {
        let extension_id = ExtensionId::new("fixture").expect("valid extension id");
        ExtensionInstallation::new(
            ExtensionInstallationId::new("fixture").expect("valid installation"),
            extension_id.clone(),
            activation_state,
            ExtensionManifestRef::new(
                extension_id,
                manifest_hash
                    .map(ManifestHash::new)
                    .transpose()
                    .expect("valid manifest hash"),
            ),
            Vec::new(),
            chrono::Utc::now(),
        )
        .expect("fixture installation")
    }

    fn assert_unsupported_extension_response(
        response: LifecycleProductResponse,
        expected_ref: &str,
    ) {
        assert_eq!(response.phase, LifecyclePhase::UnsupportedOrLegacy);
        assert!(response.blockers.iter().any(|blocker| matches!(
            blocker,
            LifecycleReadinessBlocker::Runtime { ref_id: Some(ref_id) }
                if ref_id.as_str() == expected_ref
        )));
    }
}
