//! Scenario 1 (HEADLINE): install an extension in thread A; thread B (a
//! DIFFERENT conversation) sees it installed over the shared store.
//!
//! `extension_search` renders `installation_phase: "installed"` only for an
//! already-installed extension. Different conversation IDs but the same
//! `Arc<HostRuntimeCapabilityHarness>` prove cross-thread persistence.

use super::ironclaw_support::group::{HarnessResult, IronClawIntegrationGroup};
use super::ironclaw_support::reply::IronClawScriptedReply;
use serde_json::json;

pub async fn run(g: &IronClawIntegrationGroup) -> HarnessResult<()> {
    // ── Thread A: installer ─────────────────────────────────────────────────
    let installer = g
        .thread("ext-installer")
        .script([
            IronClawScriptedReply::tool_call(
                "builtin.extension_install",
                json!({"extension_id": "github"}),
            ),
            IronClawScriptedReply::text("installed"),
        ])
        .build()
        .await?;
    installer.submit_turn("install github").await?;
    installer
        .assert_tool_invoked("builtin.extension_install")
        .await?;
    installer
        .assert_tool_result_contains("\"installed\":true")
        .await?;
    installer
        .assert_model_message_content_contains(r#"\"installed\":true"#)
        .await?;

    // ── Thread B: viewer (DIFFERENT conversation, SAME shared store) ─────────
    let viewer = g
        .thread("ext-viewer")
        .script([
            IronClawScriptedReply::tool_call(
                "builtin.extension_search",
                json!({"query": "github"}),
            ),
            IronClawScriptedReply::text("found"),
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
        .assert_tool_result_contains(r#""installation_phase":"installed""#)
        .await?;
    viewer
        .assert_model_message_content_contains(r#"\"id\":\"github\""#)
        .await?;
    viewer
        .assert_model_message_content_contains(r#"\"installation_phase\":\"installed\""#)
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
