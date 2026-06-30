//! Group integration tests for cross-thread extension-lifecycle persistence.
//!
//! A [`RebornIntegrationGroup`] owns one shared `HostRuntimeCapabilityHarness`
//! (one extension-install store). An extension installed by thread A is visible
//! to thread B because both share the same underlying store — the whole point.
//!
//! ## Why one sequential `#[tokio::test]`
//!
//! The installer thread must complete before the viewer thread runs; a shared
//! group instance cannot be split across Cargo test cases without fragile global
//! state. One orchestrating function gives deterministic ordering for free.

#[allow(dead_code)]
#[path = "../support/reborn/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

mod scenario_install_then_visible_cross_thread;
mod scenario_remove_then_absent_cross_thread;

use reborn_support::group::{RebornIntegrationGroup, ScenarioReport};

#[tokio::test]
async fn extensions_group_e2e() {
    let g = RebornIntegrationGroup::extension_lifecycle()
        .await
        .expect("group builds");
    let mut report = ScenarioReport::new();

    // Scenario 1 (HEADLINE): install in thread A → search in thread B over the
    // shared store. Installer must succeed before the viewer runs, so we use
    // `report.record` which records the result without early-aborting.
    report.record(
        "install_then_visible_cross_thread",
        scenario_install_then_visible_cross_thread::run(&g).await,
    );

    // Scenario 2: install + remove in thread A → search in thread B confirms
    // the extension is no longer installed over the shared store. Independent
    // of Scenario 1: Scenario 1 installs "github" and never removes it;
    // Scenario 2 installs + removes "notion" so it is self-contained and does
    // not depend on Scenario 1's shared-store state.
    report.record(
        "remove_then_absent_cross_thread",
        scenario_remove_then_absent_cross_thread::run(&g).await,
    );

    report.assert_all_passed();
}
