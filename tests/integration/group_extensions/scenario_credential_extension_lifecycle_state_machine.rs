//! Scenario 7 (T3 of issue #6105; the #6029/#6049 failure shapes): EXIT EDGES
//! for a credential-injection (OAuth-backed) extension — GitHub — through the
//! real lifecycle capabilities: install-and-reconcile → use → **remove** (the edge #6029
//! reports as missing/wedged) → surfaces flip together → reconfigure (fresh
//! credential — the model-tool analog of the Configure card) → reinstall →
//! use again.
//!
//! Differs from the Slack scenario (scenario 6) on the axis that matters:
//! GitHub has NO channel connection — its "connected" surface is the
//! product-auth credential account injected at dispatch. The surfaces that
//! must flip together on remove are therefore:
//! - **lifecycle phase** seen by `builtin.extension_search`;
//! - **tool dispatchability** of `github.*` through the real capability port.
//!
//! Runs after every earlier scenario that reads "github": scenario 1 leaves
//! it installed (a same-member reinstall is rejected "already installed" by
//! design — `install_policy.rs`), so phase 1 reconciles the existing member —
//! the state a real user's Extensions page is in when #6029 bites. The fresh
//! single-install arm is pinned in phase 5, after the full remove.
//! (Scenarios 8 and 9 run later but touch only slack/notion state.)

use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::reply::RebornScriptedReply;
use serde_json::json;

pub async fn run(_g: &RebornIntegrationGroup) -> HarnessResult<()> {
    let isolated = RebornIntegrationGroup::extension_lifecycle().await?;
    let g = &isolated;
    // ── Phase 1: install/reconcile the member; tools get published ──────────
    let lifecycle = g
        .thread("gh-lifecycle-install")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_install",
                json!({"extension_id": "github"}),
            ),
            RebornScriptedReply::text("github ready"),
        ])
        .build()
        .await?;
    // Real secret material through the production manual-token flow —
    // install reconciliation and dispatch-time staging both select this
    // account (same seed shape as scenario 6's slack_personal account).
    lifecycle
        .seed_capability_credential_account("github", "itest github", &[])
        .await?;
    lifecycle.submit_turn("install github").await?;
    lifecycle
        .assert_tool_result_contains("\"phase\":\"active\"")
        .await?;
    lifecycle
        .assert_model_message_content_contains(r#"\"phase\":\"active\""#)
        .await?;
    lifecycle
        .assert_tool_result_contains(r#""github.get_repo""#)
        .await?;

    // ── Phase 2: use — a github.* call dispatches through the real port ─────
    // The caller uses TWO capabilities so per-thread baseline slicing keeps
    // the post-remove negative check clean: get_repo only before the remove,
    // create_issue only after it (mirrors scenario 6's search/send split).
    let caller = g
        .thread("gh-lifecycle-caller")
        .script([
            RebornScriptedReply::tool_call(
                "github.get_repo",
                json!({"owner": "octocat", "repo": "hello-world"}),
            ),
            RebornScriptedReply::text("looked up repo"),
            RebornScriptedReply::tool_call(
                "github.create_issue",
                json!({"owner": "octocat", "repo": "hello-world",
                       "title": "after remove?", "body": "should not dispatch"}),
            ),
            RebornScriptedReply::text("github unavailable"),
            RebornScriptedReply::tool_call(
                "github.create_issue",
                json!({"owner": "octocat", "repo": "hello-world",
                       "title": "after restore", "body": "should dispatch"}),
            ),
            RebornScriptedReply::text("filed the issue"),
        ])
        .build()
        .await?;
    caller.submit_turn("look up the repo").await?;
    caller.assert_tool_invoked("github.get_repo").await?;
    caller.assert_reply_contains("looked up repo").await?;

    // ── Phase 3: the exit edge — extension_remove must succeed (#6029) ──────
    let remover = g
        .thread("gh-lifecycle-remove")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_remove",
                json!({"extension_id": "github"}),
            ),
            RebornScriptedReply::text("github removed"),
        ])
        .build()
        .await?;
    remover.submit_turn("remove github").await?;
    remover
        .assert_tool_result_contains("\"removed\":true")
        .await?;
    remover
        .assert_model_message_content_contains(r#"\"removed\":true"#)
        .await?;

    // Lifecycle phase flips (fresh viewer thread so phase-1 results can't
    // satisfy the negative check)…
    let viewer = g
        .thread("gh-lifecycle-viewer-after-remove")
        .script([
            RebornScriptedReply::tool_call("builtin.extension_search", json!({"query": "github"})),
            RebornScriptedReply::text("searched catalog"),
        ])
        .build()
        .await?;
    viewer.submit_turn("search github after removal").await?;
    if viewer
        .assert_tool_result_contains(r#""installation_phase":"active""#)
        .await
        .is_ok()
    {
        return Err(
            "github still shows installation_phase:active after extension_remove; \
             the #6029 wedge — lifecycle phase did not flip with the remove"
                .into(),
        );
    }
    // Non-vacuity: the catalog entry itself must still be discoverable.
    viewer.assert_tool_result_contains("\"github\"").await?;
    viewer
        .assert_model_message_content_contains(r#"\"id\":\"github\""#)
        .await?;

    // ── Phase 4: use after remove — dispatch is rejected fail-closed ────────
    caller.submit_turn("file an issue").await?;
    caller
        .assert_tool_not_invoked("github.create_issue")
        .await
        .map_err(|error| {
            format!(
                "github.create_issue dispatched after extension_remove; removal must \
                 unpublish the capability surface: {error}"
            )
        })?;
    caller.assert_reply_contains("github unavailable").await?;

    // ── Phase 5: reconfigure (fresh credential) + reinstall restores use ───
    // The model-tool analog of the Extensions page's Configure card: a new
    // credential account for the same provider, then reactivation.
    let restorer = g
        .thread("gh-lifecycle-restore")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_install",
                json!({"extension_id": "github"}),
            ),
            RebornScriptedReply::text("github restored"),
        ])
        .build()
        .await?;
    restorer
        .seed_capability_credential_account("github", "itest github reconfigure", &[])
        .await?;
    restorer.submit_turn("reinstall github").await?;
    restorer
        .assert_tool_result_contains("\"installed\":true")
        .await?;
    restorer
        .assert_tool_result_contains("\"phase\":\"active\"")
        .await?;
    restorer
        .assert_model_message_content_contains(r#"\"installed\":true"#)
        .await?;
    restorer
        .assert_model_message_content_contains(r#"\"phase\":\"active\""#)
        .await?;

    // ── Phase 6: the exact call that was rejected now dispatches ────────────
    caller.submit_turn("file the issue again").await?;
    caller.assert_tool_invoked("github.create_issue").await?;
    caller.assert_reply_contains("filed the issue").await?;

    Ok(())
}
