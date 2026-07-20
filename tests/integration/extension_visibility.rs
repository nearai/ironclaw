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
