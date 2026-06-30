//! HEADLINE scenario: "approve always" in one thread → no gate in a later,
//! DIFFERENT thread of the same group.
//!
//! Thread A flips the per-`(tenant, user)` auto-approve toggle ON via the real
//! CAS-persisted `AutoApproveSettingStore` (shared across the group). Thread B —
//! a distinct conversation/thread — then runs the SAME gated capability and
//! completes WITHOUT blocking, because it reads thread A's persisted setting.
//! This is the exact user requirement: set approve-always once, and subsequent
//! threads invoking the same tool are not prompted.
//!
//! Non-vacuity: the sibling `gate_then_approve`/`gate_then_deny` scenarios run
//! FIRST (auto-approve still OFF) and prove the gate genuinely fires — so thread
//! B's no-gate completion here is the setting flip, not a vacuous pass. The
//! `submit_turn` call waits for `Completed`; if the setting had NOT crossed the
//! thread boundary, the write would block and `submit_turn` would time out and
//! error.

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
    // `submit_turn` waits for `Completed`; it would time out if thread B blocked
    // on a gate (i.e. if thread A's setting had not crossed the thread boundary).
    user.submit_turn("write the auto file").await?;
    // The auto-approved write actually ran (no gate) AND PERSISTED: the real
    // file on disk holds the written content — proving thread A's auto-approve
    // setting crossed the thread boundary AND the write took effect, not just
    // that the scripted reply was emitted.
    user.assert_workspace_file_contains("auto.txt", "auto-approved")
        .await?;
    Ok(())
}
