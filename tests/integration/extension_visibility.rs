//! HostInternal surface-hiding through a live turn.
//!
//! A registered, granted extension capability whose manifest declares
//! `visibility = "host_internal"` must never be advertised to the model
//! (absent from the CompletionRequest tool definitions) and a model call to
//! it must be rejected without reaching the capability port, while its
//! `model`-visible sibling from the SAME package is advertised. The fixture
//! is parsed by the production manifest parser and published through the same
//! registry step activation uses, and BOTH capabilities are granted — so the
//! registry-level visibility filter is the only thing under test.

#[allow(dead_code)]
#[path = "support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use ironclaw_auth::{GOOGLE_CALENDAR_EVENTS_SCOPE, GOOGLE_CALENDAR_READONLY_SCOPE};
use reborn_support::group::RebornIntegrationGroup;
use reborn_support::reply::RebornScriptedReply;
use serde_json::json;

/// One turn covers the whole matrix: the first model request captures the
/// advertised tool list (sibling present, host_internal absent), the scripted
/// call to the hidden capability is rejected fail-closed at the model gateway
/// (never advertised nor resolvable), and the run recovers via a model retry.
#[tokio::test]
async fn host_internal_capability_is_hidden_from_the_model_and_uncallable() {
    let group = RebornIntegrationGroup::extension_visibility_probe()
        .await
        .expect("visibility-probe group builds");
    let harness = group
        .thread("conv-visprobe")
        .script([
            RebornScriptedReply::tool_call("visprobe.audit", json!({})),
            RebornScriptedReply::text("audit denied"),
        ])
        .build()
        .await
        .expect("thread builds");

    harness
        .submit_turn("audit something")
        .await
        .expect("turn completes: the rejected hidden-capability call recovers via a model retry");

    // Disclosure seam: the model-visible sibling IS advertised (non-vacuity —
    // the package is published and granted), the host_internal one is NOT.
    harness
        .assert_model_tools_contains("visprobe__search")
        .await
        .expect("model-visible sibling advertised to the model");
    harness
        .assert_model_tools_excludes("visprobe__audit")
        .await
        .expect("host_internal capability never advertised to the model");

    // Dispatch seam: the hidden capability never reached the capability port.
    harness
        .assert_tool_not_invoked("visprobe.audit")
        .await
        .expect("host_internal capability call must never reach the capability port");
    harness
        .assert_reply_contains("audit denied")
        .await
        .expect("run recovered after the rejected call");
}

/// Install -> activate -> dispatch a newly-activated capability within ONE run: activation clears
/// `CapabilitySurfaceState` (`capability_may_change_visible_surface`), so the next iteration's
/// surface rebuild picks up the just-activated extension. Discriminator: without that refresh,
/// `assert_tool_invoked("google-calendar.list_calendars")` below fails because the capability id
/// wouldn't resolve against the turn-start surface. Uses "google-calendar" — not active at the
/// start of this run and therefore absent from the initial surface.
#[tokio::test]
async fn same_run_activation_refresh_dispatches_newly_activated_capability() {
    let group = RebornIntegrationGroup::extension_lifecycle()
        .await
        .expect("extension-lifecycle group builds");
    let h = group
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
        .await
        .expect("thread builds");
    // Credential material must exist BEFORE submit_turn — activation's
    // credential gate resolves inline only if an account is already seeded,
    // else the run would park at BlockedAuth instead of completing this turn.
    h.seed_capability_credential_account(
        "google",
        "itest google calendar",
        &[GOOGLE_CALENDAR_READONLY_SCOPE, GOOGLE_CALENDAR_EVENTS_SCOPE],
    )
    .await
    .expect("credential account seeds");

    h.submit_turn("install, activate, and list my calendars")
        .await
        .expect("turn completes");
    h.assert_tool_result_contains("\"installed\":true")
        .await
        .expect("install reported success");
    h.assert_tool_result_contains("\"activated\":true")
        .await
        .expect("activate reported success");
    // The discriminating proof: dispatch reached the capability port in the
    // SAME run that activated it, i.e. the surface refresh fired.
    h.assert_tool_invoked("google-calendar.list_calendars")
        .await
        .expect("newly-activated capability dispatched within the same run");
    h.assert_reply_contains("calendars listed")
        .await
        .expect("run completed with the expected reply");
}
