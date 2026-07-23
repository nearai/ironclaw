//! Generic extension lifecycle host for IronClaw.
//!
//! This crate owns the extension model's generic core (overview.md §4–§6):
//! the [`entrypoint`] contract and binding rule, the two standard state
//! machines ([`state`]), the immutable [`active`] snapshot and its resolver
//! views, the loader ports ([`loaders`]), the installation-record
//! persistence port ([`store`]), and [`ExtensionHost`] — the only writer of
//! installation state and the active snapshot ([`lifecycle`]).
//!
//! It contains no concrete product name, protocol type, route, or behavior
//! branch: concrete extensions implement the [`ironclaw_host_api::ToolAdapter`]
//! and [`ironclaw_product_adapters::ChannelAdapter`] traits and are assembled
//! by the binary, never linked here.

pub mod active;
mod admin_configuration_service;
mod admin_configuration_store;
pub mod deployment_channels;
pub mod egress;
pub mod entrypoint;
pub mod ingress;
pub mod lifecycle;
pub mod loaders;
pub mod recipes;
pub mod resolver;
pub mod state;
pub mod store;

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

pub use active::{
    ActiveExtension, ActiveSnapshot, Generation, ResolvedToolBinding, SnapshotConflict,
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
pub use deployment_channels::{
    DeploymentChannelBinding, DeploymentChannelRegistry, DeploymentChannelRegistryError,
};
pub use entrypoint::{
    BindContext, BindError, ExtensionBindings, ExtensionEntrypoint, check_binding,
};
pub use lifecycle::{
    DrainController, EgressFactory, ExtensionHost, ExtensionHostDeps, HookError, LifecycleError,
    SnapshotWatch,
};
pub use loaders::{ExtensionLoader, LoadContext, LoadedExtension, NativeExtensionFactory};
pub use recipes::{SnapshotAuthRecipeResolver, VendorRecipeConflict, unified_vendor_recipes};
pub use resolver::SnapshotToolResolver;
pub use state::{AuthAccountState, InstallationState};
pub use store::{
    InstallationRecord, InstallationRecordStore, RehydratedInstallationRecordStore, StoreError,
};
