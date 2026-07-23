//! Reborn integration test — cross-reopen capability durability (E-DURABLE seam).
//!
//! Installs an extension through a real turn, then reopens a FRESH, independent
//! `ExtensionInstallationStore` at the capability harness's on-disk storage root
//! and asserts the install survived — proving capability-produced state persists
//! to disk, not just to in-memory state. Parallels
//! `assert_reply_persists_after_reopen` for capability state.

#[allow(dead_code)]
#[path = "support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use reborn_support::group::RebornIntegrationGroup;
use reborn_support::reply::RebornScriptedReply;
use serde_json::json;

#[tokio::test]
async fn extension_install_survives_independent_reopen() {
    let group = RebornIntegrationGroup::extension_lifecycle()
        .await
        .expect("extension-lifecycle group builds");
    let harness = group
        .thread("conv-durable")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_install",
                json!({"extension_id": "github"}),
            ),
            RebornScriptedReply::text("installed"),
        ])
        .build()
        .await
        .expect("thread builds");
    harness
        .seed_capability_credential_account("github", "durable github ready path", &[])
        .await
        .expect("GitHub credential is ready for the durable install path");

    harness
        .submit_turn("install github")
        .await
        .expect("turn completes");
    harness
        .assert_tool_result_contains("\"installed\":true")
        .await
        .expect("install reported success");

    harness
        .assert_extension_install_persists_after_reopen("github")
        .await
        .expect("installed extension survives an independent reopen");
}
