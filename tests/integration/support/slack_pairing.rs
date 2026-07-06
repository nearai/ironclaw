//! Shared fixtures for the W5-SLACK-PAIR sibling binaries
//! (`slack_pairing_redeem.rs`, `slack_pairing_actor_resolution.rs`) — lives
//! under `support/` so `scripts/ci/reborn-coverage-int-tier-tests.sh`'s flat
//! `tests/integration/*.rs` scan (every top-level file there is a Cargo
//! `[[test]]` binary) doesn't mistake it for one. Still a bare `#[path]`-
//! mounted sibling, NOT a `support/mod.rs` module: registering it there would
//! pull both self-contained binaries into the general
//! `RebornIntegrationHarness` support tree they deliberately skip.

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
