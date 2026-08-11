//! Group integration test for multi-user flows over the group's ONE shared
//! runtime: a Shared-route channel conversation that refuses Direct-route
//! re-classification and runs as its pinger, plus per-actor isolation of
//! memory, approvals, turn state, and workspace (E-MULTIUSER / C-MULTIUSER;
//! issue #5479 is the original shared-runtime owner-scope fix these scenarios
//! all ride on). Ephemeral-per-ping threading itself is pinned at the
//! conversations tier and the full-path channel e2e, not here.

#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../../support/mod.rs"]
mod support;

mod scenario_auto_approve_isolation_across_actors;
mod scenario_memory_isolation_across_actors;
mod scenario_scoped_workspace_isolation;
mod scenario_shared_route_refuses_direct_reclassification;
mod scenario_turn_state_isolation_across_actors;

use reborn_support::group::{RebornIntegrationGroup, ScenarioReport};

#[tokio::test]
async fn multiuser_group_e2e() {
    let mut report = ScenarioReport::new();

    // Scenario 1 (#7377 run-acts-as-invoker): a Shared-route (bot mention)
    // channel conversation runs as its pinger and refuses a Direct-route probe
    // of the same conversation. Ephemeral-per-ping threading is pinned at the
    // conversations tier and the channel e2e, not here.
    let g = RebornIntegrationGroup::builtin_tools()
        .await
        .expect("builtin group builds");
    report.record(
        "shared_route_refuses_direct_reclassification",
        scenario_shared_route_refuses_direct_reclassification::run(&g).await,
    );

    // Scenario 2 (C-MULTIUSER): per-actor memory isolation — see
    // scenario_memory_isolation_across_actors for the seam.
    let memory_group = RebornIntegrationGroup::multiuser_memory_tools()
        .await
        .expect("multiuser memory group builds");
    report.record(
        "memory_isolation_across_actors",
        scenario_memory_isolation_across_actors::run(&memory_group).await,
    );

    // Scenario 3 (C-MULTIUSER): per-actor auto-approve isolation — see
    // scenario_auto_approve_isolation_across_actors for the seam.
    let approvals_group = RebornIntegrationGroup::multiuser_approvals()
        .await
        .expect("multiuser approvals group builds");
    report.record(
        "auto_approve_isolation_across_actors",
        scenario_auto_approve_isolation_across_actors::run(&approvals_group).await,
    );

    // Scenario 4 (C-MULTIUSER): per-actor turn/run-state isolation — see
    // scenario_turn_state_isolation_across_actors for the seam. Reuses the
    // plain `builtin_tools` group (no gate needed): the store's own
    // scope-equality gate is what's under test.
    report.record(
        "turn_state_isolation_across_actors",
        scenario_turn_state_isolation_across_actors::run(&g).await,
    );

    // Scenario 5 (C-MULTIUSER + PR #7062): per-caller-scoped workspace —
    // writes land in each actor's own subtree, a fresh actor reads an empty
    // workspace, and approval leases confine to the gate's own caller. See
    // scenario_scoped_workspace_isolation for the seam.
    let scoped_group = RebornIntegrationGroup::multiuser_scoped_workspace()
        .await
        .expect("multiuser scoped-workspace group builds");
    report.record(
        "scoped_workspace_isolation",
        scenario_scoped_workspace_isolation::run(&scoped_group).await,
    );

    report.assert_all_passed();
}
