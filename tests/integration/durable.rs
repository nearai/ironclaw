//! IronClaw integration test — cross-reopen capability durability (E-DURABLE seam).
//!
//! Installs an extension through a real turn, then reopens a FRESH, independent
//! `ExtensionInstallationStore` at the capability harness's on-disk storage root
//! and asserts the install survived — proving capability-produced state persists
//! to disk, not just to in-memory state. Parallels
//! `assert_reply_persists_after_reopen` for capability state.

#[allow(dead_code)]
#[path = "support/mod.rs"]
mod ironclaw_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use ironclaw_support::group::IronClawIntegrationGroup;
use ironclaw_support::reply::IronClawScriptedReply;
use serde_json::json;

#[tokio::test]
async fn extension_install_survives_independent_reopen() {
    let group = IronClawIntegrationGroup::extension_lifecycle()
        .await
        .expect("extension-lifecycle group builds");
    let harness = group
        .thread("conv-durable")
        .script([
            IronClawScriptedReply::tool_call(
                "builtin.extension_install",
                json!({"extension_id": "github"}),
            ),
            IronClawScriptedReply::text("installed"),
        ])
        .build()
        .await
        .expect("thread builds");

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
