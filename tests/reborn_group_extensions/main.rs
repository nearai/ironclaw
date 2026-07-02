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

// Scenario modules, declared alphabetically because rustfmt reorders `mod`
// declarations. The execution order — install → remove → activate — is set by
// the `report.record(...)` call sequence in `extensions_group_e2e` below, not
// by declaration order.
mod scenario_activate_then_active_cross_thread;
mod scenario_install_then_visible_cross_thread;
mod scenario_remove_then_absent_cross_thread;
mod scenario_search_ready_message_requires_credential;

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

    // Scenario 3: install in thread A → activate in thread B → search in thread C
    // confirms the extension reports `availability:available` over the shared
    // store (plus a direct durable-store read confirming activation specifically,
    // see the scenario's SHAPE NOTE). Closes the `extension_activate` int-tier
    // gap. Independent of Scenarios 1 & 2: it uses "web-access" (the only
    // credential-free bundled extension), untouched by "github"/"notion", so it
    // is self-contained.
    report.record(
        "activate_then_active_cross_thread",
        scenario_activate_then_active_cross_thread::run(&g).await,
    );

    // Scenario 4 (regression, bug #5416): extensions seeded `Enabled` with NO
    // credential account must NOT be reported by `builtin.extension_search` as
    // "already configured or active" / safe to skip asking for credentials —
    // across credential types: gmail (Google OAuth), github (GitHub OAuth),
    // notion (Notion OAuth / MCP). A credential-free extension (web-access)
    // must STILL be reported ready (control against over-suppression). Runs
    // after Scenarios 1-3 and reuses their shared-store state (github installed,
    // notion removed, web-access Enabled).
    report.record(
        "search_ready_message_requires_credential",
        scenario_search_ready_message_requires_credential::run(&g).await,
    );

    report.assert_all_passed();
}
