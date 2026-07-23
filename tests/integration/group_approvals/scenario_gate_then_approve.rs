//! Scenario: a gated `builtin.write_file` raises a real `BlockedApproval` gate;
//! approving it resumes the run to completion.
//!
//! Real path: scripted tool call → first-party runtime → `PermissionMode::Ask`
//! with auto-approve OFF → `TurnStatus::BlockedApproval` → `approve_gate`
//! (real `ApprovalResolver::approve_dispatch` issuing a lease) →
//! `coordinator.resume_turn` re-dispatches the originally-gated capability →
//! the model finalizes its reply → `Completed`.

use super::ironclaw_support::group::{HarnessResult, IronClawIntegrationGroup};
use super::ironclaw_support::reply::IronClawScriptedReply;
use ironclaw_turns::TurnStatus;
use serde_json::json;

pub async fn run(g: &IronClawIntegrationGroup) -> HarnessResult<()> {
    let h = g
        .thread("conv-approve")
        .script([
            IronClawScriptedReply::tool_call(
                "builtin.write_file",
                json!({"path": "/workspace/approved.txt", "content": "approved write"}),
            ),
            IronClawScriptedReply::text("file written after approval"),
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
    // The approved write actually re-ran AND PERSISTED to disk -- not merely
    // that the scripted reply was emitted.
    h.assert_workspace_file_contains("approved.txt", "approved write")
        .await?;

    // Double-resolve regression guard (C-DENYEDGE row 6): re-approving the
    // already-`Approved` gate must fail loudly (`NotPending`), not silently
    // no-op or hang.
    let err = h
        .approve_gate(run_id, &gate_ref)
        .await
        .err()
        .ok_or("expected err: re-approving an already-resolved gate must fail")?;
    let err_text = err.to_string();
    if !err_text.contains("approval request is not pending") {
        return Err(format!("expected the NotPending resolver error text, got: {err_text}").into());
    }
    if !err_text.contains("Approved") {
        return Err(format!(
            "expected the resolved ApprovalStatus::Approved token in the error, got: {err_text}"
        )
        .into());
    }
    Ok(())
}
