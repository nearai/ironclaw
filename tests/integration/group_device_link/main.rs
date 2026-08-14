//! Reborn integration group — the **linked-account (device-link)** journeys of
//! the bundled Telegram package.
//!
//! Closes the `tests/CLAUDE.md` §7 gap "Telegram has no group-tier lifecycle
//! scenario": the profile mounts the real shipped `manifest.toml`, satisfies
//! its `[admin_configuration]` (including the MTProto `telegram_api_id` /
//! `telegram_api_hash` the linked-device surface needs) through the production
//! administrator-configuration capability, and binds all three surfaces the
//! manifest declares through the same `NativeExtensionFactory` seam the binary
//! uses. The single fake is the vendor half — a scripted `DeviceLinkAdapter`
//! and a scripted linked-account `ToolAdapter`, because the real ones speak
//! MTProto over a raw socket with no injectable seam.
//!
//! What each scenario asserts at a seam, never `wait_for_status(Completed)`
//! alone: the published active snapshot's tool surface, the recorded
//! `ToolAdapter::invoke` calls (capability id + dispatching actor), the durable
//! credential-account store, and the parked run's auth-gate state.

#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../../support/mod.rs"]
mod support;

mod admin_config;
mod scenario_actor_isolation;
mod scenario_handshake_mints_and_serves;
mod scenario_link_call_unlink;
mod scenario_revoked_session_reauth;

use reborn_support::group::{RebornIntegrationGroup, ScenarioReport};

#[tokio::test]
async fn device_link_linked_account_group() {
    let (group, handles) = RebornIntegrationGroup::device_link_linked_account()
        .await
        .expect("linked-account device-link group builds");

    // Dependent: every later scenario needs the package installed, configured,
    // and publishing its linked tool surface, so a failure here stops the run.
    scenario_link_call_unlink::run(&group, &handles)
        .await
        .expect("link -> linked tool call -> unlink");

    let mut report = ScenarioReport::new();
    report.record(
        "actor_isolation",
        scenario_actor_isolation::run(&group, &handles).await,
    );
    report.record(
        "revoked_session_reauth",
        scenario_revoked_session_reauth::run(&group, &handles).await,
    );
    // Runs last because it proves the real extension-removal path, which
    // intentionally removes Telegram from the shared group runtime.
    report.record(
        "handshake_mints_serves_and_removes",
        scenario_handshake_mints_and_serves::run(&group, &handles).await,
    );
    report.assert_all_passed();
}
