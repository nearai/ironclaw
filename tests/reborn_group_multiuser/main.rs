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
#[path = "../support/reborn/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

mod scenario_two_actors_own_threads;

use reborn_support::group::{RebornIntegrationGroup, ScenarioReport};

#[tokio::test]
async fn multiuser_group_e2e() {
    let g = RebornIntegrationGroup::builtin_tools()
        .await
        .expect("group builds");
    let mut report = ScenarioReport::new();

    // Single scenario: two distinct actors, each completing a turn on their
    // own thread over the SAME shared coordinator/scheduler/thread_service.
    report.record(
        "two_actors_own_threads",
        scenario_two_actors_own_threads::run(&g).await,
    );

    report.assert_all_passed();
}
