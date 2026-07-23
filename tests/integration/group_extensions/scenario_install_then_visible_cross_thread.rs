//! Scenario 1 (HEADLINE): install an extension in thread A; thread B (a
//! DIFFERENT conversation) sees it active over the shared store.
//!
//! A no-setup/credential-ready install auto-publishes the extension and
//! `extension_search` renders `installation_phase: "active"`. Different
//! conversation IDs but the same
//! `Arc<HostRuntimeCapabilityHarness>` prove cross-thread persistence.

use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::reply::RebornScriptedReply;
use serde_json::json;

pub async fn run(g: &RebornIntegrationGroup) -> HarnessResult<()> {
    // ── Thread A: installer ─────────────────────────────────────────────────
    let installer = g
        .thread("ext-installer")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_install",
                json!({"extension_id": "github"}),
            ),
            RebornScriptedReply::text("installed"),
        ])
        .build()
        .await?;
    installer
        .seed_capability_credential_account("github", "itest github ready path", &[])
        .await?;
    installer.submit_turn("install github").await?;
    installer
        .assert_tool_invoked("builtin.extension_install")
        .await?;
    installer
        .assert_tool_result_contains("\"installed\":true")
        .await?;
    installer
        .assert_tool_result_contains("\"phase\":\"active\"")
        .await?;
    installer
        .assert_model_message_content_contains(r#"\"installed\":true"#)
        .await?;

    // ── Thread B: viewer (DIFFERENT conversation, SAME shared store) ─────────
    let viewer = g
        .thread("ext-viewer")
        .script([
            RebornScriptedReply::tool_call("builtin.extension_search", json!({"query": "github"})),
            RebornScriptedReply::text("found"),
        ])
        .build()
        .await?;
    viewer.submit_turn("search github").await?;
    viewer
        .assert_tool_invoked("builtin.extension_search")
        .await?;
    // Assert the VALUE, not just the key — a `pending`/`failed` phase must not
    // satisfy this — so this proves thread B observes thread A's success.
    viewer
        .assert_tool_result_contains(r#""installation_phase":"active""#)
        .await?;
    viewer
        .assert_model_message_content_contains(r#"\"id\":\"github\""#)
        .await?;
    viewer
        .assert_model_message_content_contains(r#"\"installation_phase\":\"active\""#)
        .await?;

    // Non-vacuity guard: a never-installed marker must be absent, proving
    // `assert_tool_result_contains` discriminates rather than passing unconditionally.
    if viewer
        .assert_tool_result_contains("this-extension-does-not-exist-zzz")
        .await
        .is_ok()
    {
        return Err(
            "negative guard failed: search result must not contain a never-installed extension id"
                .into(),
        );
    }

    Ok(())
}
