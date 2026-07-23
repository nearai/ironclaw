//! Scenario 2 (HEADLINE): install then remove an extension in thread A; thread B
//! (a DIFFERENT conversation) does NOT see it as installed over the shared store.
//!
//! Uses "notion" (not "github", which Scenario 1 installs into this same store
//! and never removes) so the install→remove cycle is self-contained.
//!
//! `installation_phase` is entirely absent from `extension_search` for a
//! not-installed extension (confirmed in `extension_lifecycle_capabilities.rs`'s
//! unit tests). Four threads, different conversation IDs, same
//! `Arc<HostRuntimeCapabilityHarness>`, prove cross-thread removal persistence.

use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::reply::RebornScriptedReply;
use serde_json::json;

pub async fn run(g: &RebornIntegrationGroup) -> HarnessResult<()> {
    // ── Phase 1: install "notion" ────────────────────────────────────────────
    let installer = g
        .thread("ext-remove-phase-install")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_install",
                json!({"extension_id": "notion"}),
            ),
            RebornScriptedReply::text("installed"),
        ])
        .build()
        .await?;
    installer
        .seed_capability_credential_account("notion", "itest notion ready path", &[])
        .await?;
    installer.submit_turn("install notion").await?;
    installer
        .assert_tool_invoked("builtin.extension_install")
        .await?;
    installer
        .assert_tool_result_contains("\"installed\":true")
        .await?;
    installer
        .assert_model_message_content_contains(r#"\"installed\":true"#)
        .await?;

    // ── Phase 2: remove "notion" (DIFFERENT conversation, SAME shared store) ─
    let remover = g
        .thread("ext-remove-phase-remove")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_remove",
                json!({"extension_id": "notion"}),
            ),
            RebornScriptedReply::text("removed"),
        ])
        .build()
        .await?;
    remover.submit_turn("remove notion").await?;
    remover
        .assert_tool_invoked("builtin.extension_remove")
        .await?;
    remover
        .assert_tool_result_contains("\"removed\":true")
        .await?;
    remover
        .assert_model_message_content_contains(r#"\"removed\":true"#)
        .await?;

    // ── Phase 3: retry removal after it is absent (another conversation) ────
    let retry_remover = g
        .thread("ext-remove-phase-retry")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_remove",
                json!({"extension_id": "notion"}),
            ),
            RebornScriptedReply::text("already removed"),
        ])
        .build()
        .await?;
    retry_remover.submit_turn("remove notion again").await?;
    retry_remover
        .assert_tool_invoked("builtin.extension_remove")
        .await?;
    retry_remover
        .assert_tool_result_contains("\"removed\":false")
        .await?;
    retry_remover
        .assert_model_message_content_contains(r#"\"removed\":false"#)
        .await?;

    // ── Phase 4: cross-thread search — "notion" must NOT be installed ───────
    let viewer = g
        .thread("ext-remove-phase-viewer")
        .script([
            RebornScriptedReply::tool_call("builtin.extension_search", json!({"query": "notion"})),
            RebornScriptedReply::text("searched"),
        ])
        .build()
        .await?;
    viewer.submit_turn("search notion after removal").await?;
    viewer
        .assert_tool_invoked("builtin.extension_search")
        .await?;

    // `assert_tool_result_contains` returns `Ok` when present, `Err` when
    // absent — invert to assert absence.
    if viewer
        .assert_tool_result_contains(r#""installation_phase":"setup_needed""#)
        .await
        .is_ok()
    {
        return Err(
            "removed extension still shows installation_phase:setup_needed in cross-thread search; \
             builtin.extension_remove did not propagate through the shared store"
                .into(),
        );
    }

    // Non-vacuity guard: catalog entry must still appear, so the absence
    // assertion above isn't vacuously true on an empty/errored result.
    if viewer
        .assert_tool_result_contains("\"notion\"")
        .await
        .is_err()
    {
        return Err(
            "non-vacuity guard failed: notion catalog entry must still appear in search results \
             after removal (bundled extension remains discoverable); absence assertion is vacuous"
                .into(),
        );
    }
    viewer
        .assert_model_message_content_contains(r#"\"id\":\"notion\""#)
        .await?;

    Ok(())
}
