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
//! concrete extensions implement the [`ironclaw_host_api::ToolAdapter`] and
//! [`ironclaw_product::ChannelAdapter`] traits and are supplied by the binary.
//! The generic assembly layer binds those adapters and resolved manifests to
//! the host-runtime lane binder without linking concrete extension crates.

pub mod active;
mod admin_configuration_service;
mod admin_configuration_store;
pub mod channel_config;
pub mod channel_delivery;
pub mod channel_dm_targets;
pub mod deployment_channels;
pub mod egress;
pub mod entrypoint;
pub mod extension_bundle;
pub mod extension_credential_requirements;
pub mod generic_host;
pub mod ingress;
pub mod lifecycle;
pub mod loaders;
pub mod mcp;
pub mod mcp_discovery;
pub mod provider_instance_readiness;
pub mod recipes;
pub mod reply_contexts;
pub mod resolver;
pub mod state;
pub mod store;

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

pub use active::{
    ActiveExtension, ActiveSnapshot, BoundExtension, Generation, ResolvedToolBinding,
    SnapshotConflict,
};
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
pub use channel_config::{
    ChannelConfigError, ChannelConfigFieldStatus, ChannelConfigReactivation,
    ChannelConfigReactivationError, ChannelConfigService, RebornChannelConfigFacade,
};
pub use channel_delivery::{IngressReplyContextSource, SnapshotChannelDeliveryResolver};
pub use channel_dm_targets::{
    ChannelDmTargetError, ChannelDmTargetRecord, DM_TARGET_CONVERSATION_ID_KEY,
    DM_TARGET_SPACE_ID_KEY, FilesystemChannelDmTargetStore, dm_target_payload,
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
pub use generic_host::{
    GenericExtensionHost, GenericExtensionHostParams, build_generic_extension_host,
    effective_resolved_for_package,
};
pub use lifecycle::{
    DrainController, EgressFactory, ExtensionHost, ExtensionHostDeps, HookError, LifecycleError,
    SnapshotWatch,
};
pub use loaders::{ExtensionLoader, LoadContext, LoadedExtension, NativeExtensionFactory};
pub use mcp::{RegistryMcpEgressPlanner, hosted_http_mcp_runtime};
pub use mcp_discovery::{
    HostedMcpDiscoveryError, discover_hosted_mcp_package, is_hosted_http_mcp_package,
};
pub use provider_instance_readiness::{
    ProviderInstanceReadinessInput, provider_instance_readiness_map,
};
pub use recipes::{SnapshotAuthRecipeResolver, VendorRecipeConflict, unified_vendor_recipes};
pub use reply_contexts::FilesystemReplyContextStore;
pub use resolver::SnapshotToolResolver;
pub use state::{AuthAccountState, InstallationState};
pub use store::{
    InstallationRecord, InstallationRecordStore, RehydratedInstallationRecordStore, StoreError,
};
