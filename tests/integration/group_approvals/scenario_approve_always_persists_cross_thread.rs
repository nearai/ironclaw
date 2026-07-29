//! HEADLINE scenario: "approve always" in one thread → no gate in a later,
//! DIFFERENT thread of the same group.
//!
//! Thread A flips the per-`(tenant, user)` auto-approve toggle ON via the real
//! CAS-persisted `AutoApproveSettingStorePort`. Thread B, a distinct
//! conversation/thread, then runs the SAME gated capability and completes
//! WITHOUT blocking because it reads thread A's persisted setting.
//!
//! Non-vacuity: sibling `gate_then_approve`/`gate_then_deny` run FIRST
//! (auto-approve still OFF) and prove the gate genuinely fires, so thread B's
//! no-gate completion here is the setting flip, not a vacuous pass — if the
//! setting had not crossed the thread boundary, `submit_turn` would time out
//! waiting for `Completed` instead of returning.

use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::reply::RebornScriptedReply;
use serde_json::json;

pub async fn run(g: &RebornIntegrationGroup) -> HarnessResult<()> {
    // ── Thread A: flip auto-approve ON (persists to the shared store) ────────
    let enabler = g.thread("conv-approve-always-enabler").build().await?;
    enabler.enable_auto_approve().await?;

    // ── Thread B: DIFFERENT conversation, same group/shared store ────────────
    // The same gated capability now completes with NO approval gate.
    let user = g
        .thread("conv-approve-always-user")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.write_file",
                json!({"path": "/workspace/auto.txt", "content": "auto-approved"}),
            ),
            RebornScriptedReply::text("wrote without a gate"),
        ])
        .build()
        .await?;
    // `submit_turn` waits for `Completed` (see module doc's non-vacuity note).
    user.submit_turn("write the auto file").await?;
    // The write also PERSISTED to disk, not merely that the scripted reply
    // was emitted.
    user.assert_workspace_file_contains("auto.txt", "auto-approved")
        .await?;
    Ok(())
}
