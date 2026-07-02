//! Scenario 1 (HEADLINE): install an extension in thread A; thread B (a
//! DIFFERENT conversation) sees it installed over the shared store.
//!
//! Thread A calls `builtin.extension_install` for "github". Thread B calls
//! `builtin.extension_search` for "github" and asserts the result carries
//! `availability: "needs_auth"` — GitHub requires a credential and none is
//! configured here, so an installed-but-uncredentialed extension projects
//! `needs_auth` (not `not_installed`, which is what an unknown/never-installed
//! extension would show). Because the two threads use different conversation
//! IDs but the same `Arc<HostRuntimeCapabilityHarness>`, this proves
//! cross-thread extension persistence.

use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::reply::RebornScriptedReply;
use serde_json::json;

pub async fn run(g: &RebornIntegrationGroup) -> HarnessResult<()> {
    // ── Thread A: installer ─────────────────────────────────────────────────
    // Install the "github" extension. The installation is persisted to the
    // shared HostRuntimeCapabilityHarness filesystem so subsequent threads
    // see it immediately.
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
    installer.submit_turn("install github").await?;
    installer
        .assert_tool_invoked("builtin.extension_install")
        .await?;
    // Verify the install succeeded: output JSON contains `"installed":true`.
    installer
        .assert_tool_result_contains("\"installed\":true")
        .await?;

    // ── Thread B: viewer (DIFFERENT conversation, SAME shared store) ─────────
    // A distinct conversation_id produces a distinct binding and thread scope,
    // but the underlying `HostRuntimeCapabilityHarness` is Arc-cloned, so the
    // viewer reads from the exact same extension-install store the installer
    // just wrote to.
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
    // The search result carries `availability: "needs_auth"` for the github
    // package only once it is installed (before installation the field reads
    // `not_installed`). Assert the VALUE, not just the key, so the check proves
    // thread B observes thread A's *successful* installation over the shared
    // store.
    viewer
        .assert_tool_result_contains(r#""availability":"needs_auth""#)
        .await?;

    // Committed negative guard (non-vacuity): a marker for a never-installed,
    // non-existent extension must be ABSENT from the search result, so
    // `assert_tool_result_contains` is proven to discriminate rather than pass
    // unconditionally.
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
