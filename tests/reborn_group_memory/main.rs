//! Group integration tests for cross-thread memory persistence.
//!
//! A [`RebornIntegrationGroup`] owns one shared `HostRuntimeCapabilityHarness`
//! (one filesystem, one memory backend). State written by thread A is visible
//! to thread B because both share the same underlying store — the whole point.
//!
//! ## Why one sequential `#[tokio::test]`
//!
//! Scenario 1's writer must complete before the reader runs; a shared group
//! instance cannot be split across Cargo test cases without fragile global
//! state. One orchestrating function gives deterministic ordering for free.

#[allow(dead_code)]
#[path = "../support/reborn/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

mod scenario_write_then_read_cross_thread;

use reborn_support::group::{RebornIntegrationGroup, ScenarioReport};

#[tokio::test]
async fn memory_group_e2e() {
    let g = RebornIntegrationGroup::builtin_tools()
        .await
        .expect("group builds");
    let mut report = ScenarioReport::new();

    // Scenario 1 (HEADLINE): write in thread A → read in thread B over the
    // shared store. The write→read dependency is internal to this single
    // scenario; `report.record` is correct here.
    report.record(
        "write_then_read_cross_thread",
        scenario_write_then_read_cross_thread::run(&g).await,
    );

    report.assert_all_passed();
}
