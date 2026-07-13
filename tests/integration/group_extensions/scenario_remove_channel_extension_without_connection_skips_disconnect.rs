//! Scenario 8 (negative companion to Scenario 7): `builtin.extension_remove`
//! on a credentialed, non-channel extension (google-drive) whose caller has
//! NO recorded channel connection must still remove the extension, but must
//! NOT call `disconnect_channel_for_caller` — proving
//! `disconnect_channel_for_cleanup`'s facade-connection gate (not a bare
//! "channel qualifies") decides whether disconnect fires. Google-drive hits
//! `RemovableChannelCleanup::IfConnectionFacadeSupportsChannel` (a
//! credentialed extension whose id isn't slack/`ExternalChannel`), which DOES
//! consult `caller_channel_connections` before disconnecting — unlike
//! Scenario 7's slack (`Required`, unconditional once a facade exists).
//!
//! Builds its own group/runtime: the channel-connection facade slot is a
//! once-only `OnceLock` per runtime, and Scenario 7 already fills the shared
//! group's slot with a "connected" facade.

use std::sync::Arc;

use super::reborn_support::doubles::RecordingChannelConnectionFacade;
use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::reply::RebornScriptedReply;
use serde_json::json;

pub async fn run(_g: &RebornIntegrationGroup) -> HarnessResult<()> {
    let g = RebornIntegrationGroup::extension_lifecycle().await?;
    let capability_harness = g
        .capability_harness()
        .ok_or("extension_lifecycle group always uses a HostRuntime capability")?;
    let services = capability_harness
        .reborn_services_for_test()
        .ok_or("extension_lifecycle harness retains RebornServices")?;
    let facade = Arc::new(RecordingChannelConnectionFacade::default());
    if !services.set_channel_connection_facade_for_test(facade.clone()) {
        return Err("channel-connection facade slot already filled or no local runtime".into());
    }

    // ── Phase 1: install "google-drive" (untouched by Scenarios 1-7) ────────
    let installer = g
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
    installer.submit_turn("install google-drive").await?;
    installer
        .assert_tool_result_contains("\"installed\":true")
        .await?;

    // ── Phase 2: remove "google-drive"; removal succeeds, no disconnect ─────
    let remover = g
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
    remover.submit_turn("remove google-drive").await?;
    remover
        .assert_tool_result_contains("\"removed\":true")
        .await?;

    let disconnects = facade.disconnects();
    if !disconnects.is_empty() {
        return Err(format!(
            "removal must not disconnect a channel the facade reports the caller as \
             not connected to; expected no disconnect calls, got {disconnects:?}"
        )
        .into());
    }

    Ok(())
}
