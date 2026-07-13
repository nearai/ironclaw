//! Scenario 7: `builtin.extension_remove` on a channel extension (slack) must
//! disconnect the caller's per-user channel binding, matching the WebUI remove
//! path. Regression coverage for the channel-cleanup call site
//! `extension_lifecycle.rs::remove` -> `cleanup_channel_before_remove` ->
//! `disconnect_channel_for_cleanup` (#5851). For a `RemovableChannelCleanup::Required`
//! extension (slack, matched by id in `removable_channel_cleanup_for_summary`),
//! disconnect fires unconditionally once a facade is installed — the
//! connection map is not consulted (see the negative companion scenario for
//! the `IfConnectionFacadeSupportsChannel` extension that DOES consult it).
//!
//! Uses "slack" (untouched by Scenarios 1-6; catalog entry is
//! `slack-v2-host-beta`-gated, unified on by the workspace root's
//! `[dev-dependencies]` for every `tests/integration/` binary).

use std::sync::Arc;

use super::reborn_support::doubles::RecordingChannelConnectionFacade;
use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::reply::RebornScriptedReply;
use serde_json::json;

pub async fn run(g: &RebornIntegrationGroup) -> HarnessResult<()> {
    let capability_harness = g
        .capability_harness()
        .ok_or("extension_lifecycle group always uses a HostRuntime capability")?;
    let services = capability_harness
        .reborn_services_for_test()
        .ok_or("extension_lifecycle harness retains RebornServices")?;
    let facade = Arc::new(RecordingChannelConnectionFacade::with_connections(&[(
        "slack", true,
    )]));
    if !services.set_channel_connection_facade_for_test(facade.clone()) {
        return Err("channel-connection facade slot already filled or no local runtime".into());
    }

    // ── Phase 1: install "slack" ─────────────────────────────────────────
    let installer = g
        .thread("ext-channel-remove-install")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_install",
                json!({"extension_id": "slack"}),
            ),
            RebornScriptedReply::text("installed"),
        ])
        .build()
        .await?;
    installer.submit_turn("install slack").await?;
    installer
        .assert_tool_result_contains("\"installed\":true")
        .await?;

    // ── Phase 2: remove "slack"; the disconnect must be recorded ────────────
    let remover = g
        .thread("ext-channel-remove-remove")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_remove",
                json!({"extension_id": "slack"}),
            ),
            RebornScriptedReply::text("removed"),
        ])
        .build()
        .await?;
    remover.submit_turn("remove slack").await?;
    remover
        .assert_tool_result_contains("\"removed\":true")
        .await?;

    let disconnects = facade.disconnects();
    let expected_user = capability_harness.capability_user_id().as_str().to_string();
    if disconnects != vec![(expected_user.clone(), "slack".to_string())] {
        return Err(format!(
            "model-invoked builtin.extension_remove must disconnect the run owner's slack \
             channel binding like the WebUI remove path; expected [({expected_user:?}, \
             \"slack\")], got {disconnects:?}"
        )
        .into());
    }

    Ok(())
}
