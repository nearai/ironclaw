//! Reborn extension-host cluster.
//!
//! Groups the extension/skill host surface — first-party extension catalog
//! (`available_extensions`, `bundled_skills`, `gsuite`), credential requirement
//! and activation plumbing (`extension_activation_credentials`,
//! `extension_credential_requirements`, `webui_extension_credentials`), the
//! installation store and lifecycle command/capability stack
//! (`extension_installation_store`, `extension_lifecycle`,
//! `extension_lifecycle_capabilities`, `extension_lifecycle_command`,
//! `lifecycle`, `skill_learning`, `skill_listing`), and MCP discovery
//! (`mcp`, `mcp_discovery`) behind one internal module. The crate root re-exports
//! the same public items from here so the crate's public API is unchanged.

pub(crate) mod available_extension_import;
pub(crate) mod available_extensions;
pub(crate) mod bundled_skills;
pub(crate) mod channel_config;
pub(crate) mod channel_connection;
pub(crate) mod channel_delivery;
pub(crate) mod channel_dm_provisioning;
pub(crate) mod channel_dm_targets;
pub(crate) mod channel_egress;
pub(crate) mod channel_host;
pub(crate) mod channel_identity;
pub(crate) mod channel_identity_store;
pub(crate) mod channel_outbound_targets;
pub(crate) mod channel_pairing;
pub(crate) mod channel_pairing_serve;
pub(crate) mod channel_subject_routes;
pub(crate) mod channel_triggered_delivery;
pub(crate) mod extension_activation_credentials;
pub(crate) mod extension_bundle;
pub(crate) mod extension_credential_requirements;
pub(crate) mod extension_ingress;
pub(crate) mod extension_installation_store;
pub(crate) mod extension_lifecycle;
pub(crate) mod extension_lifecycle_capabilities;
#[cfg(test)]
pub(crate) mod extension_lifecycle_capabilities_auth_tests;
pub(crate) mod extension_lifecycle_command;
pub(crate) mod extension_removal_cleanup;
pub(crate) mod generic_host;
pub(crate) mod gsuite;
pub(crate) mod host_api_contracts;
#[cfg(test)]
mod host_remediation_contract_tests;
pub(crate) mod lifecycle;
pub(crate) mod mcp;
pub(crate) mod mcp_discovery;
pub(crate) mod provider_instance_readiness;
pub(crate) mod reply_contexts;
pub(crate) mod run_delivery_ports;
pub(crate) mod skill_learning;
pub(crate) mod skill_listing;
pub(crate) mod webui_extension_credentials;

// Keep the bundle policy owned by `extension_bundle`; lifecycle consumes only
// the decoder through this narrow module-level seam.
pub(crate) use extension_bundle::unzip_extension_bundle;
