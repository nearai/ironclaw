//! Scenario: a gated `builtin.write_file` call raises a real `BlockedApproval`
//! gate; resolving it via a REAL `submit_inbound(ApprovalResolution)` (the
//! literal dispatch arm a product adapter's "approve"/"deny" reply hits)
//! resumes the run — not `approve_gate`/`deny_gate`'s direct
//! `TurnCoordinator::resume_turn` shortcut. Requires the group to be built
//! with `.with_real_gate_dispatch_services()`, which wires the REAL
//! `ApprovalInteractionService` over the group's own shared turn-state store.
//!
//! Real path: scripted tool call → first-party runtime → `PermissionMode::Ask`
//! with auto-approve OFF → `TurnStatus::BlockedApproval` → real
//! `ApprovalInteractionService::resolve` (via `DefaultProductSurface::submit_inbound`)
//! → `coordinator.resume_turn` → the gated capability re-dispatches (approve)
//! or the run finalizes an authorization failure (deny).

use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::reply::RebornScriptedReply;
use ironclaw_product_adapters::{ApprovalDecision, ProductInboundAck};
use ironclaw_turns::TurnStatus;
use serde_json::json;

pub async fn approve(g: &RebornIntegrationGroup) -> HarnessResult<()> {
    let h = g
        .thread("conv-submit-inbound-approve")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.write_file",
                json!({"path": "/workspace/submit_inbound_approved.txt", "content": "approved via submit_inbound"}),
            ),
            RebornScriptedReply::text("file written after submit_inbound approval"),
        ])
        .build()
        .await?;

    let (run_id, gate_ref) = h
        .submit_turn_until_blocked("write the submit_inbound approval file")
        .await?;

    let ack = h
        .submit_approval_resolution(&gate_ref, ApprovalDecision::ApproveOnce)
        .await?;
    if !matches!(ack, ProductInboundAck::Accepted { .. }) {
        return Err(
            format!("expected an Accepted ack for the real resolution, got {ack:?}").into(),
        );
    }
    h.wait_for_status(run_id, TurnStatus::Completed).await?;

    // Seam assertion (not `wait_for_status` alone): the approved write must
    // have actually re-dispatched and persisted through the real capability
    // path.
    h.assert_workspace_file_contains("submit_inbound_approved.txt", "approved via submit_inbound")
        .await?;
    Ok(())
}

pub async fn deny(g: &RebornIntegrationGroup) -> HarnessResult<()> {
    let h = g
        .thread("conv-submit-inbound-deny")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.write_file",
                json!({"path": "/workspace/submit_inbound_denied.txt", "content": "should not persist"}),
            ),
            RebornScriptedReply::text("understood, the write was not authorized"),
        ])
        .build()
        .await?;

    let (run_id, gate_ref) = h
        .submit_turn_until_blocked("write the submit_inbound denied file")
        .await?;

    let ack = h
        .submit_approval_resolution(&gate_ref, ApprovalDecision::Deny)
        .await?;
    if !matches!(ack, ProductInboundAck::Accepted { .. }) {
        return Err(
            format!("expected an Accepted ack for the real resolution, got {ack:?}").into(),
        );
    }
    h.wait_for_status(run_id, TurnStatus::Completed).await?;

    // Seam assertion: the denied capability must never have re-dispatched, so
    // the file is absent on disk — not merely that the run reached Completed.
    h.assert_workspace_file_absent("submit_inbound_denied.txt")
        .await?;
    Ok(())
}
