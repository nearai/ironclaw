//! C-DURABLE: a pending approval request survives an independent reopen of
//! the approval-request store at the SAME on-disk local-dev `storage_root` —
//! proving approval state persists to disk, not just in memory. Parallels
//! `assert_reply_persists_after_reopen` (thread history) and
//! `reborn_integration_durable.rs` (extension installs).
//!
//! Raises a real `BlockedApproval` gate, reopens a FRESH `ApprovalRequestStorePort`
//! at the same root, and asserts the `Pending` record is there independent of
//! the live `Arc` the running group holds, then resolves the gate normally.

use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::reply::RebornScriptedReply;
use ironclaw_run_state::ApprovalStatus;
use ironclaw_turns::TurnStatus;
use serde_json::json;

pub async fn run(g: &RebornIntegrationGroup) -> HarnessResult<()> {
    let h = g
        .thread("conv-approval-durable")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.write_file",
                json!({"path": "/workspace/durable.txt", "content": "durable write"}),
            ),
            RebornScriptedReply::text("file written after approval"),
        ])
        .build()
        .await?;
    let (run_id, gate_ref) = h
        .submit_turn_until_blocked("write the durability file")
        .await?;

    let capability_harness = g
        .capability_harness()
        .ok_or("live_approvals always uses HostRuntime")?;
    let (request_id, scope) = capability_harness.approval_request_scope_for_test(&gate_ref)?;

    // Reopen a FRESH store at the same on-disk root, independent of the live
    // `Arc`, and confirm the pending request is there.
    let reopened =
        ironclaw_reborn_composition::test_support::open_local_dev_approval_request_store_for_test(
            &capability_harness.storage_root_for_test(),
        )
        .await?;
    let record = reopened
        .get(&scope, request_id)
        .await?
        .ok_or("approval request not found after independent reopen")?;
    if record.status != ApprovalStatus::Pending {
        return Err(format!(
            "expected Pending status after reopen, got {:?}",
            record.status
        )
        .into());
    }
    // Drop the independent connection before resuming through the live store to
    // avoid two open libsql connections spanning a subsequent write.
    drop(reopened);

    // Resolve normally so this scenario leaves no run permanently blocked.
    h.approve_gate(run_id, &gate_ref).await?;
    h.wait_for_status(run_id, TurnStatus::Completed).await?;
    Ok(())
}
