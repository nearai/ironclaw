//! Scenario 7: `builtin.extension_remove` on a channel extension must drive
//! `extension_lifecycle.rs::remove` -> `cleanup_channel_before_remove` ->
//! `disconnect_channel_for_cleanup` correctly for both channel-cleanup kinds
//! (#5851). Slack (`RemovableChannelCleanup::Required`) disconnects
//! unconditionally once a facade is installed; google-drive
//! (`IfConnectionFacadeSupportsChannel`) consults `caller_channel_connections`
//! first and, with no "google-drive" key in the facade's connection map,
//! disconnects zero times.
//!
//! Uses "slack" and "google-drive" (both untouched by Scenarios 1-6; slack's
//! catalog entry is `slack-v2-host-beta`-gated, unified on by the workspace
//! root's `[dev-dependencies]` for every `tests/integration/` binary).

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

    // ── Phase 1: install + remove "slack" (Required cleanup) ────────────────
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

    // ── Phase 2: install + remove "google-drive" (IfConnectionFacadeSupportsChannel
    //    cleanup; key absent from the facade's connection map above) ────────
    let installer2 = g
        .thread("ext-channel-no-disconnect-install")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_install",
                json!({"extension_id": "google-drive"}),
            ),
            RebornScriptedReply::text("installed"),
        ])
        .build()
        .await?;
    installer2.submit_turn("install google-drive").await?;
    installer2
        .assert_tool_result_contains("\"installed\":true")
        .await?;

    let remover2 = g
        .thread("ext-channel-no-disconnect-remove")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_remove",
                json!({"extension_id": "google-drive"}),
            ),
            RebornScriptedReply::text("removed"),
        ])
        .build()
        .await?;
    remover2.submit_turn("remove google-drive").await?;
    remover2
        .assert_tool_result_contains("\"removed\":true")
        .await?;

    // Exact equality: slack must disconnect exactly once, and the
    // google-drive removal above must not have added a second entry.
    let expected_user = capability_harness.capability_user_id().as_str().to_string();
    let disconnects = facade.disconnects();
    if disconnects != vec![(expected_user.clone(), "slack".to_string())] {
        return Err(format!(
            "builtin.extension_remove must disconnect the run owner's slack channel binding \
             exactly once (Required cleanup) and never google-drive \
             (IfConnectionFacadeSupportsChannel, key absent); expected \
             [({expected_user:?}, \"slack\")], got {disconnects:?}"
        )
        .into());
    }

    Ok(())
}
