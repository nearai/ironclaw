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
use reborn_support::harness::profiles::extension::PROMPT_DENIAL_DESCRIPTION;
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

/// Regression for the Attio incident: the post-signature `RegistryInstalled`
/// source makes catalog descriptions trusted prompt text, while a local
/// package's unsafe description degrades only that prompt entry instead of
/// denying the turn.
#[tokio::test]
async fn prompt_description_trust_is_enforced_at_the_real_turn_seam() {
    let group = RebornIntegrationGroup::extension_prompt_description_trust_probe()
        .await
        .expect("prompt-description trust probe group builds");
    let harness = group
        .thread("conv-prompt-description-trust")
        .script([RebornScriptedReply::text("prompt survived")])
        .build()
        .await
        .expect("thread builds");

    harness
        .submit_turn("continue after installing the extension")
        .await
        .expect("verified auth wording and one unsafe local description must not deny the turn");
    harness
        .assert_reply_contains("prompt survived")
        .await
        .expect("turn completes through persisted reply");
    harness
        .assert_model_tool_description_contains("verifiedprompt__invoke", PROMPT_DENIAL_DESCRIPTION)
        .await
        .expect("verified catalog description reaches the model intact, including Bearer");
    harness
        .assert_system_prompt_contains(PROMPT_DENIAL_DESCRIPTION)
        .await
        .expect("verified catalog description survives instruction-bundle validation");
    harness
        .assert_model_tools_contains("localprompt__healthy")
        .await
        .expect("safe sibling from the same local package remains advertised");
    harness
        .assert_system_prompt_contains("localprompt.healthy")
        .await
        .expect("safe local sibling remains in the validated prompt surface");
    harness
        .assert_system_prompt_excludes("localprompt.unsafe")
        .await
        .expect("only the unsafe untrusted prompt entry is omitted");
}
