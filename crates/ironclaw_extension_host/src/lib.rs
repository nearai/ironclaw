//! Generic extension lifecycle host for IronClaw Reborn.
//!
//! This crate owns the extension model's generic core (overview.md §4–§6):
//! the [`entrypoint`] contract and binding rule, the two standard state
//! machines ([`state`]), the immutable [`active`] snapshot and its resolver
//! views, the loader ports ([`loaders`]), the installation-record
//! persistence port ([`store`]), and [`ExtensionHost`] — the only writer of
//! installation state and the active snapshot ([`lifecycle`]).
//!
//! It contains no concrete product name, protocol route, or behavior branch:
//! concrete extensions implement the [`ironclaw_host_api::tool_adapter::ToolAdapter`] and
//! [`ironclaw_product::ChannelAdapter`] traits and are supplied by the binary.
//! The generic assembly layer binds those adapters and resolved manifests to
//! the host-runtime lane binder without linking concrete extension crates.

extern crate self as ironclaw_extension_host;

mod activation_credentials;
pub mod activation_transaction;
pub mod active;
mod active_publication;
pub mod admin_configuration;
pub mod admin_configuration_capability;
mod admin_configuration_service;
mod admin_configuration_store;
pub mod available_extension_import;
pub mod available_extensions;
pub mod bundled_skills;
pub mod capability_surface;
pub mod channel_command_roles;
pub mod channel_config;
pub mod channel_connection;
pub mod channel_delivery;
pub mod channel_dm_provisioning;
pub mod channel_dm_targets;
pub mod channel_egress;
pub mod channel_host;
pub mod channel_identity;
pub mod channel_identity_binding;
pub mod channel_identity_store;
pub mod channel_lifecycle;
pub mod channel_outbound_targets;
pub mod channel_pairing;
pub mod channel_pairing_serve;
pub mod channel_subject_routes;
pub mod channel_triggered_delivery;
pub mod deployment_channels;
pub mod egress;
pub mod entrypoint;
pub mod extension_activation_credentials;
pub mod extension_bundle;
pub mod extension_credential_requirements;
pub mod extension_ingress;
pub mod extension_lifecycle;
pub mod extension_lifecycle_capabilities;
pub mod extension_lifecycle_command;
pub mod first_party_package;
pub mod generic_host;
pub mod host_api_contracts;
mod hosted_mcp_discovery_authority;
pub mod inbound_batches;
pub mod ingress;
pub mod install_policy;
pub mod ironhub;
pub mod lifecycle;
pub mod lifecycle_product_service;
pub mod lifecycle_restore;
pub mod lifecycle_vocabulary;
pub mod loaders;
pub mod mcp;
pub mod mcp_discovery;
pub mod nearai_mcp;
pub mod operator_config_capability;
pub mod product_lifecycle;
pub mod provider_identity;
pub mod provider_instance_readiness;
pub mod recipes;
pub mod removal_cleanup;
pub mod reply_contexts;
pub mod resolver;
pub mod run_delivery_ports;
pub mod skill_auto_activate_capability;
pub mod skill_learning;
pub mod skill_listing;
pub mod state;
pub mod store;
pub mod webui_extension_credentials;

mod build_error;
#[cfg(test)]
mod host_remediation_contract_tests;
#[cfg(test)]
#[path = "test_support/lifecycle.rs"]
pub mod lifecycle_test_support;

#[cfg(any(test, feature = "test-support"))]
pub async fn filesystem_installation_store_for_test()
-> ironclaw_extensions::ExtensionInstallationStore {
    use std::sync::Arc;

    use ironclaw_filesystem::InMemoryBackend;
    use ironclaw_host_api::{host_port::HostPortCatalog, path::VirtualPath};

    ironclaw_extensions::ExtensionInstallationStore::load_at(
        Arc::new(InMemoryBackend::new()),
        VirtualPath::new("/system/extensions/.installations/test").expect("valid test path"),
        HostPortCatalog::empty(),
        product_extension_host_api_contract_registry().expect("extension host API contracts"),
    )
    .await
    .expect("filesystem extension installation store")
}

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

pub use activation_credentials::{
    ExtensionActivationCredentialGate, ExtensionActivationCredentialReadiness,
    PrecheckedExtensionActivationCredentialGate, UnavailableExtensionActivationCredentialGate,
    missing_activation_credentials_error,
};
pub use active::{
    ActiveExtension, ActiveSnapshot, BoundExtension, Generation, ResolvedToolBinding,
    SnapshotConflict,
};
pub use active_publication::{ActiveExtensionPublisher, extension_trust_policy_input};
pub use admin_configuration_service::{
    AdminConfigurationFieldState, AdminConfigurationGroupState, AdminConfigurationService,
    AdminConfigurationServiceError, AdminConfigurationSubmittedValue,
};
pub use admin_configuration_store::{
    AdminConfigurationCommit, AdminConfigurationIdempotencyKey, AdminConfigurationRecord,
    AdminConfigurationRequestDigest, AdminConfigurationReservation,
    AdminConfigurationReserveOutcome, AdminConfigurationStoreError, AdminConfigurationValueRef,
    FilesystemAdminConfigurationStore,
};
pub use available_extension_import::{
    extension_asset_path, imported_extension_package, inline_extension_dir_assets,
    materialize_available_extension, registry_extension_package,
};
pub use available_extensions::{
    AdminConfigurationCatalogUse, AvailableExtensionAsset, AvailableExtensionAssetContent,
    AvailableExtensionCatalog, AvailableExtensionPackage, bytes_asset,
    nearai_mcp_manifest_toml_for_config, surface_kinds_from_manifest_record,
    visible_capability_ids, visible_read_only_capability_ids,
};
pub use build_error::RebornExtensionHostBuildError as RebornBuildError;
pub use build_error::RebornExtensionHostBuildError;
pub use channel_config::{
    ChannelConfigError, ChannelConfigFieldStatus, ChannelConfigReactivation,
    ChannelConfigReactivationError, ChannelConfigService, RebornChannelConfigProductService,
};
pub use channel_delivery::{IngressReplyContextSource, SnapshotChannelDeliveryResolver};
pub use channel_dm_targets::{
    ChannelDmTargetError, ChannelDmTargetRecord, DM_TARGET_CONVERSATION_ID_KEY,
    DM_TARGET_SPACE_ID_KEY, FilesystemChannelDmTargetStore, dm_target_payload,
};
pub use channel_identity::{
    DiscoveredChannelExtension, channel_config_connection_scope_source,
    discover_channel_extensions, handle_declares_claim,
};
pub use channel_identity_store::{
    FilesystemChannelIdentityStore, channel_identity_mount_view,
    path_segment as channel_identity_path_segment,
};
pub use channel_lifecycle::{
    channel_connect_strategy, channel_connection_requirement,
    package_declares_inbound_product_adapter,
};
pub use channel_subject_routes::{
    ChannelConfigSubjectRouteResolver, SharedChannelAdmissionHandles, handle_declares_field,
    managed_channel_subject_user_id, shared_channel_admission_handles,
};
pub use deployment_channels::{
    DeploymentChannelBinding, DeploymentChannelRegistry, DeploymentChannelRegistryError,
};
pub use entrypoint::{
    BindContext, BindError, ExtensionBindings, ExtensionEntrypoint, check_binding,
};
pub use extension_bundle::{
    ExtensionBundleError, MAX_EXTENSION_BUNDLE_FILES, MAX_EXTENSION_BUNDLE_UNCOMPRESSED_BYTES,
    unzip_extension_bundle,
};
pub use extension_credential_requirements::{
    can_merge_lifecycle_credential_setup, lifecycle_credential_setup,
    manifest_runtime_credential_auth_requirements, merge_lifecycle_credential_setup,
    package_runtime_credential_auth_requirements, product_auth_credential_source,
};
pub use first_party_package::{
    FirstPartyHandlerRegistrar, FirstPartyPackageAsset, FirstPartyPackageBundle,
    FirstPartyPackageOAuthSetup, FirstPartyPackageOnboarding, FirstPartyRegistrarContext,
    first_party_reserved_extension_ids,
};
pub use generic_host::{
    BootInstallationRecordsError, GenericExtensionHost, GenericExtensionHostParams,
    boot_installation_records, build_generic_extension_host, effective_resolved_for_package,
};
pub use host_api_contracts::product_extension_host_api_contract_registry;
pub use inbound_batches::FilesystemInboundBatchStore;
pub use install_policy::{
    RemoveDecision, decide_install_on_existing, decide_remove, derive_owner,
    ensure_caller_may_operate, install_scope_for_owner,
};
pub use ironclaw_host_runtime::{
    FirstPartyCapabilityError, FirstPartyCapabilityHandler, FirstPartyCapabilityRegistry,
    FirstPartyCapabilityRequest, FirstPartyCapabilityResult, ProductAuthProviderRuntimePorts,
};
pub use lifecycle::{
    DrainController, EgressFactory, ExtensionHost, ExtensionHostDeps, HookError, LifecycleError,
    SnapshotWatch,
};
pub use lifecycle_product_service::ExtensionHostLifecycleProductService;
pub use lifecycle_restore::{
    ExtensionInstallPlan, available_manifest_hash, package_visible_capability_ids, prepare_install,
    restore_extension_lifecycle_state,
};
pub use lifecycle_vocabulary::{ActiveExtensionCapability, ExtensionActivationMode};
pub use loaders::{ExtensionLoader, LoadContext, LoadedExtension, NativeExtensionFactory};
pub use mcp::{RegistryMcpEgressPlanner, hosted_http_mcp_runtime};
pub use mcp_discovery::{
    HostedMcpDiscoveryError, discover_hosted_mcp_package, is_hosted_http_mcp_package,
};
pub use nearai_mcp::{
    DEFAULT_NEARAI_MCP_BASE_URL, NearAiMcpBootstrapConfig, NearAiMcpBootstrapConfigError,
    NearAiMcpEndpoint, durable_product_auth_storage_enabled, nearai_mcp_endpoint_from_base,
    nearai_mcp_endpoint_from_env,
};
pub use product_lifecycle::{ExtensionCredentialCleanup, ExtensionLifecycleManager};
pub use provider_instance_readiness::{
    ProviderInstanceReadinessInput, provider_instance_readiness_map,
};
pub use recipes::{SnapshotAuthRecipeResolver, VendorRecipeConflict, unified_vendor_recipes};
pub use removal_cleanup::{
    ExtensionRemovalChannelId, ExtensionRemovalCleanupAdapter, ExtensionRemovalCleanupAdapterId,
    ExtensionRemovalCleanupBinding, ExtensionRemovalCleanupContext,
    ExtensionRemovalCleanupRegistry, ExtensionRemovalCleanupRequirement,
};
pub use reply_contexts::FilesystemReplyContextStore;
pub use resolver::SnapshotToolResolver;
pub use state::{AuthAccountState, InstallationState};
pub use store::{
    InstallationRecord, InstallationRecordStore, RehydratedInstallationRecordStore, StoreError,
};
