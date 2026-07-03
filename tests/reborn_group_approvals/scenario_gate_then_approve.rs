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

    // Double-resolve regression guard (C-DENYEDGE row 6): approving the SAME
    // already-`Completed` gate a second time must fail loudly, not silently
    // no-op or hang. The approval record is already `Approved` from the first
    // `approve_gate` call above, so this second call's `approve_local_dev_gate`
    // hits `ApprovalResolver::approve_capability_action`'s
    // `record.status != Pending` check and returns
    // `ApprovalResolutionError::NotPending { status: Approved }` before the
    // resume/coordinator layer is even reached.
    let err = h
        .approve_gate(run_id, &gate_ref)
        .await
        .expect_err("re-approving an already-resolved gate must fail");
    let err_text = err.to_string();
    assert!(
        err_text.contains("approval request is not pending"),
        "expected the NotPending resolver error text, got: {err_text}"
    );
    assert!(
        err_text.contains("Approved"),
        "expected the resolved ApprovalStatus::Approved token in the error, got: {err_text}"
    );
    Ok(())
}
