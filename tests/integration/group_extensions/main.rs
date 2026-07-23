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
mod scenario_credential_extension_lifecycle_state_machine;
mod scenario_extension_install_github_normal_gate;
mod scenario_extension_install_instance_not_configured;
mod scenario_extension_install_reauth_gate;
mod scenario_google_family_install_gate_and_shared_account;
mod scenario_install_then_active_cross_thread;
mod scenario_install_then_visible_cross_thread;
mod scenario_install_unknown_extension_id_fails_safely;
mod scenario_malformed_lifecycle_arguments_are_structured;
mod scenario_remove_then_absent_cross_thread;
mod scenario_slack_channel_lifecycle_state_machine;
mod scenario_slack_state_survives_reopen;
mod scenario_uninstalled_tool_call_denied_until_active;

use reborn_support::group::{RebornIntegrationGroup, ScenarioReport};

#[test]
fn malformed_lifecycle_arguments_are_structured() {
    run_async_test_with_stack(
        "malformed_lifecycle_arguments_are_structured",
        malformed_lifecycle_arguments_are_structured_inner,
    );
}

async fn malformed_lifecycle_arguments_are_structured_inner() {
    let g = RebornIntegrationGroup::extension_lifecycle()
        .await
        .expect("group builds");
    scenario_malformed_lifecycle_arguments_are_structured::run(&g)
        .await
        .expect("malformed lifecycle arguments retain structured repair detail");
}

#[test]
fn extensions_group_e2e() {
    run_async_test_with_stack("extensions_group_e2e", extensions_group_e2e_inner);
}

async fn extensions_group_e2e_inner() {
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

    // Scenario 3: install → active → search across three threads; closes the
    // extension-install reconciliation int-tier gap. Uses "web-access" (credential-free,
    // untouched by Scenarios 1-2) to stay self-contained.
    report.record(
        "install_then_active_cross_thread",
        scenario_install_then_active_cross_thread::run(&g).await,
    );

    // Scenario 4 (W4-EXT-MANIFEST-ERR): an unknown extension_id fails
    // `builtin.extension_install` safely with `Failed{InputEncode}`. Uses a
    // nonexistent id, touching no shared-store state other scenarios read.
    report.record(
        "install_unknown_extension_id_fails_safely",
        scenario_install_unknown_extension_id_fails_safely::run(&g).await,
    );

    // Scenario 4.5 (provider-instance readiness map, two-phase — see the
    // module doc): Phase 1 — installing a google-family
    // extension with NO Google OAuth backend configured (this harness's
    // default state — never wired otherwise) must fail early with a generic,
    // caller-safe unavailable result and keep running without exposing the
    // administrator schema or parking an unresolvable BlockedAuth gate. Phase 2 — a
    // SEPARATE, freshly built group with a Google OAuth backend configured
    // must fall through to the ordinary per-account BlockedAuth gate instead
    // (already green today). The readiness-map chokepoint gates the "google"
    // PROVIDER build-time-wide (not just the specific package), so Scenario 5
    // now runs on its OWN isolated, Google-OAuth-configured group instead of
    // `g` — see that scenario's module doc.
    report.record(
        "extension_install_instance_not_configured",
        scenario_extension_install_instance_not_configured::run(&g).await,
    );

    // Scenario 4.6: negative case pinning current behavior — a provider with
    // no instance-config requirement (github: manual-token, user-credential
    // gate) must keep raising the normal per-account BlockedAuth gate,
    // proving the readiness check doesn't false-positive on it. Retries
    // github's EXISTING setup-needed install from Scenario 1; runs
    // before Scenario 7 (which later finishes setup with a seeded
    // credential — a denied gate here leaves no persistent credential state,
    // matching the notion reauth-gate scenario's proven deny-then-reconfigure
    // shape).
    report.record(
        "extension_install_github_normal_gate",
        scenario_extension_install_github_normal_gate::run(&g).await,
    );

    // Scenario 5: a model call to a not-installed extension capability is
    // rejected fail-closed at the model gateway until install publishes it
    // publishes it. Uses "gmail" on its own isolated, Google-OAuth-configured
    // group (see that scenario's module doc) — `g` itself is passed but
    // unused, kept for call-site symmetry with every other scenario.
    report.record(
        "uninstalled_tool_call_denied_until_active",
        scenario_uninstalled_tool_call_denied_until_active::run(&g).await,
    );

    // Scenario 6 (issue #6105): the Slack channel lifecycle state machine —
    // install → connect → use → remove (real personal-connection
    // cleanup) → reconnect → reinstall → use again, asserting connection
    // state, durable bindings, lifecycle phase, and tool dispatchability stay
    // consistent at every transition. Uses the delivery-profile group because
    // Slack's channel binding is assembled there.
    // dependent: must pass before scenario 8 consumes its reconnected end
    // state — `.expect()` (not `report.record`) so a lifecycle regression is
    // reported HERE, not misattributed to the restart-survival probe.
    let slack_g = RebornIntegrationGroup::extension_delivery()
        .await
        .expect("slack delivery group builds");
    scenario_slack_channel_lifecycle_state_machine::run(&slack_g)
        .await
        .expect("slack_channel_lifecycle_state_machine");

    // Scenario 7 (issue #6105, T3): exit edges for a credential-injection
    // extension — finish setup → use → remove (#6029's wedged edge) → surfaces
    // flip → reconfigure + reinstall → use again. Reuses "github" AFTER
    // every earlier scenario that reads it has run (scenario 1 leaves it
    // installed but setup-needed), so phase 1 retries the idempotent install
    // after seeding credentials. The reinstall arm scenario 7 drives is
    // post-remove, phase 5.
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
        scenario_slack_state_survives_reopen::run(&slack_g).await,
    );

    // Scenario 9 (issue #6105 bucket-3 arms): install-time re-auth gate —
    // install over a REVOKED credential parks BlockedAuth with a renderable
    // provider requirement (#6043 shape), persists no misleading Failed error
    // (#5878's reported readiness-reconciliation surface), and a reconfigure
    // unwedges it. Uses "notion" (removed by scenario 2; credentials
    // untouched by every other scenario).
    report.record(
        "extension_install_reauth_gate",
        scenario_extension_install_reauth_gate::run(&g).await,
    );

    // Scenario 10: the Google-family install-and-connect journeys — a
    // wrongly-scoped shared google account must not satisfy a calendar
    // install (parks a renderable google gate), bulk installs park
    // INDEPENDENT gates, denial leaves a clean retry, and one
    // correctly-scoped google account then unlocks calendar AND drive. Like
    // Scenario 5, it builds an isolated Google-OAuth-configured group so the
    // provider-instance readiness check can fall through to account gating.
    report.record(
        "google_family_install_gate_and_shared_account",
        scenario_google_family_install_gate_and_shared_account::run(&g).await,
    );

    report.assert_all_passed();
}

fn run_async_test_with_stack<F, Fut>(name: &'static str, test: F)
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + 'static,
{
    let handle = std::thread::Builder::new()
        .name(name.to_string())
        .stack_size(16 * 1024 * 1024)
        .spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("tokio test runtime")
                .block_on(test());
        })
        .expect("spawn stack-sized test thread");
    if let Err(panic) = handle.join() {
        std::panic::resume_unwind(panic);
    }
}
