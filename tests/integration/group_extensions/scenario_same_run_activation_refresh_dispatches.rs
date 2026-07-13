//! Scenario 6 (harness-port-seam P2, A1): install -> activate -> dispatch a
//! newly-activated capability, ALL WITHIN ONE RUN (one `submit_turn`) on one
//! thread. Distinct from Scenario 5 (cross-RUN: rejected pre-activation, then
//! dispatches on a SEPARATE later turn) — this closes the SAME-RUN gap.
//!
//! `builtin.extension_activate` clears `CapabilitySurfaceState`
//! (`loop_driver_host.rs`'s `SurfaceTrackingLoopCapabilityPort`), forcing the
//! next loop iteration's `visible_capabilities()` call through to production's
//! `RefreshingLocalDevCapabilityPort::build_inner`, which rebuilds the port
//! from the just-activated extension registry. Without that refresh, the
//! third tool call's capability id would not resolve against the surface
//! cached at turn start (pre-activation) and `assert_tool_invoked` below
//! would fail — that absence is the discriminating proof, mirroring
//! `scenario_uninstalled_tool_call_denied_until_activated`'s inverted check.
//!
//! Uses "google-calendar" (untouched by Scenarios 1-5) so activation starts
//! genuinely fresh in this run — reusing an already-active extension would
//! let the FIRST (turn-start) surface already contain the capability,
//! defeating the point.

use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::reply::RebornScriptedReply;
use ironclaw_auth::{GOOGLE_CALENDAR_EVENTS_SCOPE, GOOGLE_CALENDAR_READONLY_SCOPE};
use serde_json::json;

pub async fn run(g: &RebornIntegrationGroup) -> HarnessResult<()> {
    let h = g
        .thread("ext-same-run-activation-refresh")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_install",
                json!({"extension_id": "google-calendar"}),
            ),
            RebornScriptedReply::tool_call(
                "builtin.extension_activate",
                json!({"extension_id": "google-calendar"}),
            ),
            RebornScriptedReply::tool_call("google-calendar.list_calendars", json!({})),
            RebornScriptedReply::text("calendars listed"),
        ])
        .build()
        .await?;
    // Credential material must exist BEFORE submit_turn — activation's
    // credential gate resolves inline only if an account is already seeded,
    // else the run would park at BlockedAuth instead of completing this turn.
    h.seed_capability_credential_account(
        "google",
        "itest google calendar",
        &[GOOGLE_CALENDAR_READONLY_SCOPE, GOOGLE_CALENDAR_EVENTS_SCOPE],
    )
    .await?;

    h.submit_turn("install, activate, and list my calendars")
        .await?;
    h.assert_tool_result_contains("\"installed\":true").await?;
    h.assert_tool_result_contains("\"activated\":true").await?;
    // The discriminating proof: dispatch reached the capability port in the
    // SAME run that activated it, i.e. the surface refresh fired.
    h.assert_tool_invoked("google-calendar.list_calendars")
        .await?;
    h.assert_reply_contains("calendars listed").await?;
    Ok(())
}
