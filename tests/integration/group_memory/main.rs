//! Group integration tests for cross-thread memory persistence.
//!
//! A [`IronClawIntegrationGroup`] owns one shared `HostRuntimeCapabilityHarness`
//! (one filesystem, one memory backend). State written by thread A is visible
//! to thread B because both share the same underlying store — the whole point.
//!
//! One sequential `#[tokio::test]`: a shared group instance can't split across
//! Cargo test cases without fragile global state, and each scenario's
//! writer must complete before its reader/searcher/lister runs. Scenarios seed
//! their own data, so ordering between them doesn't matter.

#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod ironclaw_support;
#[allow(dead_code)]
#[path = "../../support/mod.rs"]
mod support;

mod scenario_memory_search_finds_seeded;
mod scenario_memory_tree_reflects_structure;
mod scenario_write_then_read_cross_thread;

use ironclaw_support::group::{IronClawIntegrationGroup, ScenarioReport};

#[tokio::test]
async fn memory_group_e2e() {
    let g = IronClawIntegrationGroup::builtin_tools()
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

    // Scenario 2: seed a document, then locate it via `memory_search` from a
    // different conversation over the shared FTS-backed store.
    report.record(
        "memory_search_finds_seeded",
        scenario_memory_search_finds_seeded::run(&g).await,
    );

    // Scenario 3: seed a nested document, then assert `memory_tree` reflects the
    // directory structure when listed from a different conversation.
    report.record(
        "memory_tree_reflects_structure",
        scenario_memory_tree_reflects_structure::run(&g).await,
    );

    report.assert_all_passed();
}
