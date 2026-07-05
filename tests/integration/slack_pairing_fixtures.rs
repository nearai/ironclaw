//! Shared fixtures for the W5-SLACK-PAIR sibling binaries
//! (`slack_pairing_redeem.rs`, `slack_pairing_actor_resolution.rs`).
//!
//! Both binaries are deliberately self-contained (no `tests/integration/support/`
//! harness — see each file's own header comment for why), so this is a narrow
//! `#[path]`-mounted sibling file rather than a new module registered in the
//! large shared `support/mod.rs` tree: it exists only to avoid the two
//! binaries drifting on the one tenant/installation/mount-view/binding-service
//! fixture shape they both need, not to pull either one into the general
//! harness.

use std::sync::Arc;

use ironclaw_host_api::{
    AgentId, MountAlias, MountGrant, MountPermissions, MountView, ProjectId, TenantId, UserId,
    VirtualPath,
};
use ironclaw_product_adapters::AdapterInstallationId;
use ironclaw_reborn_composition::{
    RebornUserIdentityBindingStore, SlackPersonalBindingInstallation,
    SlackPersonalUserBindingService,
    slack_serve::{SlackApiAppId, SlackInstallationSelector, SlackTeamId},
};

pub fn tenant_id() -> TenantId {
    TenantId::new("tenant-alpha").expect("valid tenant id")
}

pub fn installation_id() -> AdapterInstallationId {
    AdapterInstallationId::new("install-alpha").expect("valid installation id")
}

pub fn tenant_shared_mount_view() -> MountView {
    MountView::new(vec![MountGrant::new(
        MountAlias::new("/tenant-shared").expect("valid mount alias"),
        VirtualPath::new("/tenants/tenant-alpha/shared").expect("valid virtual path"),
        MountPermissions::read_write_list_delete(),
    )])
    .expect("valid mount view")
}

pub fn binding_service(
    binding_store: Arc<dyn RebornUserIdentityBindingStore>,
) -> SlackPersonalUserBindingService {
    SlackPersonalUserBindingService::new(
        [SlackPersonalBindingInstallation {
            tenant_id: tenant_id(),
            installation_id: installation_id(),
            selector: SlackInstallationSelector::AppTeam {
                api_app_id: SlackApiAppId::new("A123"),
                team_id: SlackTeamId::new("T123"),
            },
        }],
        binding_store,
    )
}

pub fn host_ids() -> (UserId, AgentId, Option<ProjectId>) {
    (
        UserId::new("user:host").expect("valid user id"),
        AgentId::new("agent:host").expect("valid agent id"),
        Some(ProjectId::new("project:host").expect("valid project id")),
    )
}
