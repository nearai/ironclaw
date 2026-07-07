//! Group integration test for FIX-5479 / E-MULTIUSER: a second, distinct
//! actor submitting to its own thread over the group's ONE shared runtime.
//!
//! Before the fix, `RebornIntegrationGroupBuilder::into_group`'s shared
//! runtime resolved every thread through a construction-time-fixed owner
//! scope (the group's canonical/default actor), so any OTHER actor's turn
//! failed deterministically with `driver_unavailable` / "unknown thread"
//! (issue #5479). `with_actor_id` on `RebornThreadBuilder` is the seam that
//! exercises a second owner over the shared runtime.

#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../../support/mod.rs"]
mod support;

mod scenario_auto_approve_isolation_across_actors;
mod scenario_memory_isolation_across_actors;
mod scenario_turn_state_isolation_across_actors;
mod scenario_two_actors_own_threads;

use reborn_support::group::{RebornIntegrationGroup, ScenarioReport};

#[tokio::test]
async fn multiuser_group_e2e() {
    let mut report = ScenarioReport::new();

    // Scenario 1 (E-MULTIUSER): two distinct actors complete turns on their
    // own threads over the shared coordinator (see module doc for the
    // `with_actor_id` seam).
    let g = RebornIntegrationGroup::builtin_tools()
        .await
        .expect("builtin group builds");
    report.record(
        "two_actors_own_threads",
        scenario_two_actors_own_threads::run(&g).await,
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

    report.assert_all_passed();
}
