//! Scenario: a gated `builtin.write_file` raises a real `BlockedApproval` gate;
//! approving it resumes the run to completion.
//!
//! Real path: scripted tool call → first-party runtime → `PermissionMode::Ask`
//! with auto-approve OFF → `TurnStatus::BlockedApproval` → `approve_gate`
//! (real `ApprovalResolver::approve_dispatch` issuing a lease) →
//! `coordinator.resume_turn` re-dispatches the originally-gated capability →
//! the model finalizes its reply → `Completed`.

use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::reply::RebornScriptedReply;
use ironclaw_turns::TurnStatus;
use serde_json::json;

pub async fn run(g: &RebornIntegrationGroup) -> HarnessResult<()> {
    let h = g
        .thread("conv-approve")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.write_file",
                json!({"path": "/workspace/approved.txt", "content": "approved write"}),
            ),
            RebornScriptedReply::text("file written after approval"),
        ])
        .build()
        .await?;

    // Real gate: the write blocks on a persisted approval request.
    let (run_id, gate_ref) = h
        .submit_turn_until_blocked("write the approval file")
        .await?;

    // Approve through the real resolver + resume; the gated capability re-runs.
    h.approve_gate(run_id, &gate_ref).await?;
    h.wait_for_status(run_id, TurnStatus::Completed).await?;
    // The approved write actually re-ran AND PERSISTED: the real file on disk
    // holds the written content. This proves approve→resume re-dispatched the
    // gated capability and the write took effect — not merely that the scripted
    // reply was emitted (`builtin.write_file`'s result does not echo content).
    h.assert_workspace_file_contains("approved.txt", "approved write")
        .await?;
    Ok(())
}
