//! Scenario 2 (HEADLINE): install then remove an extension in thread A; thread B
//! (a DIFFERENT conversation) does NOT see it as installed over the shared store.
//!
//! Uses "notion" (NOT "github") so this scenario is self-contained: Scenario 1
//! installs "github" into the same shared store and never removes it, so a fresh
//! install→remove cycle here must use a different bundled extension to observe a
//! real install result rather than an already-installed no-op.
//!
//! Thread A-install calls `builtin.extension_install` for "notion". Thread
//! A-remove (a distinct conversation) calls `builtin.extension_remove` for the
//! same package. Thread B calls `builtin.extension_search` for "notion" and
//! asserts the result does NOT carry `installation_phase: "installed"` — the
//! field is absent in search results for extensions that are not installed.
//! Because all three conversations use different conversation IDs but the same
//! `Arc<HostRuntimeCapabilityHarness>`, this proves cross-thread extension
//! removal persistence: a remove in thread A is durably visible to thread B.
//!
//! The `builtin.extension_remove` arg shape is `{"extension_id": "<id>"}`,
//! identical to `builtin.extension_install` and `builtin.extension_activate`
//! (confirmed from `schemas/builtin/extension_remove.input.v1.json`).
//!
//! The remove output carries `"removed":true` in its payload on success
//! (confirmed from the unit test in `extension_lifecycle_capabilities.rs` that
//! calls `assert_eq!(remove["payload"]["removed"], true)`).
//!
//! The search output contains `installation_phase: "installed"` only when the
//! extension is in the installed lifecycle state — the field is entirely absent
//! when the extension is not installed (confirmed from the unit test that calls
//! `assert_eq!(available_github.get("installation_phase"), None)` before install
//! and `assert_eq!(installed_github["installation_phase"], "installed")` after).
//! After removal the extension reverts to the pre-install state, so
//! `installation_phase` disappears again.

use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::reply::RebornScriptedReply;
use serde_json::json;

pub async fn run(g: &RebornIntegrationGroup) -> HarnessResult<()> {
    // ── Phase 1: install "notion" ────────────────────────────────────────────
    // Install a fresh extension so there is something to remove. "notion" is
    // chosen because Scenario 1 installs "github" into this same shared store
    // and never removes it; re-installing an already-installed extension would
    // be a no-op and produce no `"installed":true` result. "notion" is untouched
    // by Scenario 1, so this is a genuine fresh install.
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
    installer.submit_turn("install notion").await?;
    installer
        .assert_tool_invoked("builtin.extension_install")
        .await?;
    // Confirm the install succeeded: output carries `"installed":true`.
    installer
        .assert_tool_result_contains("\"installed\":true")
        .await?;

    // ── Phase 2: remove "notion" (DIFFERENT conversation, SAME shared store) ─
    // A distinct conversation_id → distinct binding/thread scope, but the
    // same `HostRuntimeCapabilityHarness`, so the remover can see and delete
    // the installation that Phase 1 just wrote.
    //
    // Capability: `builtin.extension_remove`
    // Arg shape:  `{"extension_id": "<id>"}` — same as install/activate
    // Success output: payload carries `"removed":true`
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
    // Confirm the real remove succeeded: capability output carries `"removed":true`.
    remover
        .assert_tool_result_contains("\"removed\":true")
        .await?;

    // ── Phase 3: cross-thread search — "notion" must NOT be installed ───────
    // A DIFFERENT conversation_id produces a distinct binding and thread scope
    // but Arc-clones the same `HostRuntimeCapabilityHarness`, so the viewer
    // reads from the exact same extension-install store the remover just wrote to.
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

    // Absence assertion: after removal `installation_phase` reverts to absent.
    // `assert_tool_result_contains` returns `Ok` when the needle IS present and
    // `Err` when it is absent — invert to assert absence.
    if viewer
        .assert_tool_result_contains(r#""installation_phase":"installed""#)
        .await
        .is_ok()
    {
        return Err(
            "removed extension still shows installation_phase:installed in cross-thread search; \
             builtin.extension_remove did not propagate through the shared store"
                .into(),
        );
    }

    // Non-vacuity guard: "notion" must still appear in the catalog search result
    // (as an available-but-not-installed bundled extension), proving the search
    // actually ran and returned catalog entries. The absence of
    // `installation_phase` is therefore meaningful — not a symptom of an empty
    // or errored result.
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

    Ok(())
}
