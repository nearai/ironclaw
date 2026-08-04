//! IronHub catalog client for IronClaw Reborn.
//!
//! The module verifies the signed catalog, installs digest-pinned tools and
//! skills, persists redacted immutable receipts, reports installed status, and
//! performs explicit updates with rollback. There is no background update
//! path; executable or instruction changes remain approval-gated.

mod agent_link;
mod artifact_hosts;
mod capabilities;
mod catalog;
mod link_service;
mod model;
mod package;
mod render;
mod service;

#[cfg(test)]
mod tests;

pub use agent_link::{IronhubSharedKey, IronhubSharedKeyError};
pub use artifact_hosts::artifact_network_policy;
pub use capabilities::{
    IRONHUB_INFO_CAPABILITY_ID, IRONHUB_INSTALL_CAPABILITY_ID, IRONHUB_SEARCH_CAPABILITY_ID,
    IRONHUB_STATUS_CAPABILITY_ID, IRONHUB_UPDATE_CAPABILITY_ID, extend_builtin_first_party_package,
    insert_handlers,
};
pub use link_service::{
    IronhubLinkBuildError, IronhubLinkStateError, IronhubLinkStateStore, RebornIronhubLinkService,
};
pub use model::{
    IronHubCommand, IronHubCommandError, IronHubEntryKind, IronHubEntrySummary,
    IronHubInstallOptions, IronHubInstallationSummary, IronHubPhase, IronHubProvenance,
    IronHubResponse, IronHubUpdateOptions,
};

pub use render::render_reborn_ironhub_response;
pub use service::{
    IronhubManifestUrl, RebornIronHubRuntime, execute_reborn_ironhub_command,
    execute_reborn_ironhub_service_command, validated_manifest_url,
};

#[cfg(test)]
mod public_surface_tests {
    use super::*;

    #[test]
    fn capability_ids_are_stable_at_the_module_root() {
        assert_eq!(IRONHUB_SEARCH_CAPABILITY_ID, "builtin.ironhub_search");
        assert_eq!(IRONHUB_INFO_CAPABILITY_ID, "builtin.ironhub_info");
        assert_eq!(IRONHUB_STATUS_CAPABILITY_ID, "builtin.ironhub_status");
        assert_eq!(IRONHUB_INSTALL_CAPABILITY_ID, "builtin.ironhub_install");
        assert_eq!(IRONHUB_UPDATE_CAPABILITY_ID, "builtin.ironhub_update");
    }
}
