//! Reborn extension-host/product assembly moved out of composition.
//!
//! This module owns the generic extension/channel/product adapters that used to
//! live under `ironclaw_reborn_composition::extension_host`. Composition should
//! only inject concrete host authority and wire the returned services.

pub mod admin_configuration;
pub mod admin_configuration_capability;
pub mod bundled_skills;
pub mod channel_connection;
pub mod channel_dm_provisioning;
pub mod channel_egress;
pub mod channel_host;
pub mod channel_identity;
pub mod channel_outbound_targets;
pub mod channel_pairing;
pub mod channel_pairing_serve;
pub mod channel_triggered_delivery;
pub mod extension_activation_credentials;
pub mod extension_ingress;
pub mod extension_lifecycle;
pub mod extension_lifecycle_capabilities;
pub mod extension_lifecycle_command;
pub mod lifecycle;
#[cfg(test)]
#[path = "lifecycle_test_support_tests.rs"]
pub mod lifecycle_test_support;
pub mod operator_config_capability;
pub mod provider_identity;
pub mod run_delivery_ports;
pub mod skill_auto_activate_capability;
pub mod skill_learning;
pub mod skill_listing;
pub mod webui_extension_credentials;

#[cfg(test)]
mod host_remediation_contract_tests;

#[derive(Debug, thiserror::Error)]
pub enum RebornExtensionHostBuildError {
    #[error("invalid reborn extension-host configuration: {reason}")]
    InvalidConfig { reason: String },
    #[error("reborn extension-host filesystem build failed")]
    Filesystem(#[from] ironclaw_filesystem::FilesystemError),
    #[error("reborn extension-host mount view construction failed")]
    Mount(#[from] ironclaw_host_api::HostApiError),
}

#[cfg(any(test, feature = "test-support"))]
pub async fn filesystem_installation_store_for_test()
-> ironclaw_extensions::ExtensionInstallationStore {
    use std::sync::Arc;

    use ironclaw_filesystem::InMemoryBackend;
    use ironclaw_host_api::{HostPortCatalog, VirtualPath};

    ironclaw_extensions::ExtensionInstallationStore::load_at(
        Arc::new(InMemoryBackend::new()),
        VirtualPath::new("/system/extensions/.installations/test").expect("valid test path"),
        HostPortCatalog::empty(),
        crate::product_extension_host_api_contract_registry()
            .expect("extension host API contracts"),
    )
    .await
    .expect("filesystem extension installation store")
}
