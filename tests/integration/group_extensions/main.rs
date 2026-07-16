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
#[path = "../support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../../support/mod.rs"]
mod support;

// Modules are alphabetical (rustfmt reorders `mod` decls); execution order is
// set by the `report.record(...)` sequence below, not declaration order.
mod scenario_activate_then_active_cross_thread;
mod scenario_credential_extension_lifecycle_state_machine;
mod scenario_extension_activation_reauth_gate;
mod scenario_install_then_visible_cross_thread;
mod scenario_install_unknown_extension_id_fails_safely;
mod scenario_remove_then_absent_cross_thread;
mod scenario_slack_channel_lifecycle_state_machine;
mod scenario_slack_state_survives_reopen;
mod scenario_uninstalled_tool_call_denied_until_activated;

use reborn_support::group::{RebornIntegrationGroup, ScenarioReport};

#[tokio::test]
async fn extensions_group_e2e() {
    let g = RebornIntegrationGroup::extension_lifecycle()
        .await
        .expect("group builds");
    let mut report = ScenarioReport::new();

    // Scenario 1 (HEADLINE): install in thread A, search in thread B. Installer
    // must succeed before the viewer runs; `report.record` avoids early-abort.
    report.record(
        "install_then_visible_cross_thread",
        scenario_install_then_visible_cross_thread::run(&g).await,
    );

    // Scenario 2: install+remove in thread A, search in thread B. Uses "notion"
    // (not "github", which Scenario 1 never removes) to stay self-contained.
    report.record(
        "remove_then_absent_cross_thread",
        scenario_remove_then_absent_cross_thread::run(&g).await,
    );

    // Scenario 3: install → activate → search across three threads; closes the
    // extension_activate int-tier gap. Uses "web-access" (credential-free,
    // untouched by Scenarios 1-2) to stay self-contained.
    report.record(
        "activate_then_active_cross_thread",
        scenario_activate_then_active_cross_thread::run(&g).await,
    );

    // Scenario 4 (W4-EXT-MANIFEST-ERR): an unknown extension_id fails
    // `builtin.extension_install` safely with `Failed{InputEncode}`. Uses a
    // nonexistent id, touching no shared-store state other scenarios read.
    report.record(
        "install_unknown_extension_id_fails_safely",
        scenario_install_unknown_extension_id_fails_safely::run(&g).await,
    );

    // Scenario 5: a model call to a not-installed extension capability is
    // rejected fail-closed at the model gateway until real install+activate
    // publishes it. Uses "gmail" (untouched by Scenarios 1-4).
    report.record(
        "uninstalled_tool_call_denied_until_activated",
        scenario_uninstalled_tool_call_denied_until_activated::run(&g).await,
    );

    // Scenario 6 (issue #6105): the Slack channel lifecycle state machine —
    // install → activate → connect → use → remove (real personal-connection
    // cleanup) → reconnect → reinstall → use again, asserting connection
    // state, durable bindings, lifecycle phase, and tool dispatchability stay
    // consistent at every transition. Uses "slack" (untouched by 1-5).
    // dependent: must pass before scenario 8 consumes its reconnected end
    // state — `.expect()` (not `report.record`) so a lifecycle regression is
    // reported HERE, not misattributed to the restart-survival probe.
    scenario_slack_channel_lifecycle_state_machine::run(&g)
        .await
        .expect("slack_channel_lifecycle_state_machine");

    // Scenario 7 (issue #6105, T3): exit edges for a credential-injection
    // extension — activate → use → remove (#6029's wedged edge) → surfaces
    // flip → reconfigure + reactivate → use again. Reuses "github" AFTER
    // every earlier scenario that reads it has run (scenario 1 leaves it
    // installed and active-phase-free), so phase 1 activates the
    // already-installed package — the exact Extensions-page state #6029
    // bites in (same-member re-INSTALL is rejected "already installed" by
    // design; the reinstall arm scenario 7 drives is post-remove, phase 5).
    report.record(
        "credential_extension_lifecycle_state_machine",
        scenario_credential_extension_lifecycle_state_machine::run(&g).await,
    );

    // Scenario 8 (issue #6105, T5): restart survival — the connected/installed
    // Slack state scenario 6 left behind reads back through FRESH store
    // handles reopened at the same on-disk storage_root (what a process
    // restart reconstructs). Scenario 6's `.expect()` above guarantees the
    // end state this consumes.
    report.record(
        "slack_state_survives_reopen",
        scenario_slack_state_survives_reopen::run(&g).await,
    );

    // Scenario 9 (issue #6105 bucket-3 arms): activation-time re-auth gate —
    // activate over a REVOKED credential parks BlockedAuth with a renderable
    // provider requirement (#6043 shape), persists no misleading Failed error
    // (#5878's reported extension_activate surface), and a reconfigure
    // unwedges it. Uses "notion" (removed by scenario 2; credentials
    // untouched by every other scenario).
    report.record(
        "extension_activation_reauth_gate",
        scenario_extension_activation_reauth_gate::run(&g).await,
    );

    report.assert_all_passed();
}
