//! Scenario 3 (HEADLINE): install in thread A, ACTIVATE in thread B, confirm
//! thread C observes ACTIVE (not merely installed) over the shared store —
//! closes the `extension_activate` int-tier gap (install/search/remove already
//! had cross-thread coverage).
//!
//! Uses "web-access": the only bundled extension that activates without
//! credentials (others raise an auth gate — see
//! `local_dev_extension_activate_returns_auth_gate_for_missing_extension_credentials`
//! in `extension_lifecycle_capabilities.rs`), and it's untouched by Scenarios
//! 1-2 ("github"/"notion"), so this is a genuine fresh transition.
//!
//! Per `extension_lifecycle.rs::commit_activation` / `search_installation_phase`:
//! a successful activate yields `"activated":true` + `visible_capability_ids`;
//! `extension_search` renders an active extension as
//! `"installation_phase":"active"` vs `"installed"` for installed-but-inactive.

use super::ironclaw_support::group::{HarnessResult, IronClawIntegrationGroup};
use super::ironclaw_support::reply::IronClawScriptedReply;
use serde_json::json;

pub async fn run(g: &IronClawIntegrationGroup) -> HarnessResult<()> {
    // ── Thread A: installer ─────────────────────────────────────────────────
    let installer = g
        .thread("ext-activate-phase-install")
        .script([
            IronClawScriptedReply::tool_call(
                "builtin.extension_install",
                json!({"extension_id": "web-access"}),
            ),
            IronClawScriptedReply::text("installed"),
        ])
        .build()
        .await?;
    installer.submit_turn("install web-access").await?;
    installer
        .assert_tool_invoked("builtin.extension_install")
        .await?;
    installer
        .assert_tool_result_contains("\"installed\":true")
        .await?;
    installer
        .assert_model_message_content_contains(r#"\"installed\":true"#)
        .await?;

    // ── Thread B: activator (DIFFERENT conversation, SAME shared store) ──────
    let activator = g
        .thread("ext-activate-phase-activate")
        .script([
            IronClawScriptedReply::tool_call(
                "builtin.extension_activate",
                json!({"extension_id": "web-access"}),
            ),
            IronClawScriptedReply::text("activated"),
        ])
        .build()
        .await?;
    activator.submit_turn("activate web-access").await?;
    activator
        .assert_tool_invoked("builtin.extension_activate")
        .await?;
    // Assert the VALUE, not just the key, so an `activated:false` / auth-gate
    // outcome cannot satisfy this.
    activator
        .assert_tool_result_contains("\"activated\":true")
        .await?;
    activator
        .assert_model_message_content_contains(r#"\"activated\":true"#)
        .await?;
    // `web-access.search` coming online is the observable proof that activation
    // published the tool surface (mere install does NOT publish capabilities).
    activator
        .assert_tool_result_contains(r#""web-access.search""#)
        .await?;

    // ── Thread C: viewer (DIFFERENT conversation, SAME shared store) ─────────
    let viewer = g
        .thread("ext-activate-phase-viewer")
        .script([
            IronClawScriptedReply::tool_call(
                "builtin.extension_search",
                json!({"query": "web-access"}),
            ),
            IronClawScriptedReply::text("searched"),
        ])
        .build()
        .await?;
    viewer
        .submit_turn("search web-access after activation")
        .await?;
    viewer
        .assert_tool_invoked("builtin.extension_search")
        .await?;
    // Assert the VALUE so a still-`installed` phase cannot satisfy this.
    viewer
        .assert_tool_result_contains(r#""installation_phase":"active""#)
        .await?;
    viewer
        .assert_model_message_content_contains(r#"\"id\":\"web-access\""#)
        .await?;
    viewer
        .assert_model_message_content_contains(r#"\"installation_phase\":\"active\""#)
        .await?;

    // Discriminating guard: a no-op activate would still surface "installed"
    // here; its absence proves the phase genuinely advanced.
    if viewer
        .assert_tool_result_contains(r#""installation_phase":"installed""#)
        .await
        .is_ok()
    {
        return Err(
            "web-access still shows installation_phase:installed after a cross-thread activate; \
             builtin.extension_activate did not advance the lifecycle through the shared store"
                .into(),
        );
    }

    // Non-vacuity guard: catalog entry must still appear, so the active-phase
    // assertion above isn't vacuously true on an empty/errored result.
    if viewer
        .assert_tool_result_contains("\"web-access\"")
        .await
        .is_err()
    {
        return Err(
            "non-vacuity guard failed: web-access catalog entry must appear in search results \
             after activation; the active-phase assertion would otherwise be vacuous"
                .into(),
        );
    }

    Ok(())
}
