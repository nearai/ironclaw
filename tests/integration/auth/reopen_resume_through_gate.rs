//! S2 — reopen-resume-through-gate (restart recovery mid-gate).
//!
//! User flow: a turn parks on an approval gate, the service restarts, the user
//! approves, and the run resumes and completes. Existing durability coverage
//! (`group_approvals::scenario_approval_request_persists_after_reopen`) proves
//! the approval-REQUEST record survives an independent reopen, but that
//! scenario runs over the default `StorageMode::InMemory` group — the turn/gate
//! STATE (`FilesystemTurnStateStore`, holding `TurnStatus::BlockedApproval` +
//! `GateRef`) is never independently reopened there. This test drives a
//! `StorageMode::LibSql` group so BOTH stores are genuinely on-disk, reopens
//! BOTH independently of the live group, then resumes to completion — the
//! missing half of the "harness-rebuild-over-storage-root" seam.
//!
//! Scope note: this reopens the on-disk STORES (turn-state + approval-request)
//! independently of the live process, mirroring `assert_reply_persists_after_reopen`'s
//! established idiom — it does not additionally tear down and rebuild the
//! `TurnCoordinator`/`TurnRunScheduler` themselves (that would require
//! duplicating `RebornIntegrationGroupBuilder::into_group`'s full ~150-line
//! production-composition wiring, including the product-workflow binding
//! service, as a second construction path — a materially larger enabler than
//! this pair; see `tests/integration/support/group.rs:601` for the wiring this
//! would need to duplicate). The resume itself uses the harness's existing
//! store-level `approve_gate` path; real-dispatch gate resolution is owned by
//! a parallel lane and is not duplicated here.

#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../../support/mod.rs"]
mod support;

use ironclaw_run_state::ApprovalStatus;
use ironclaw_turns::TurnStatus;
use reborn_support::builder::StorageMode;
use reborn_support::group::RebornIntegrationGroup;
use reborn_support::reply::RebornScriptedReply;
use serde_json::json;

const CAPABILITY_ID: &str = "builtin.write_file";

#[tokio::test]
async fn gate_survives_storage_reopen_and_resumes_to_completion() {
    let g = RebornIntegrationGroup::builder()
        .storage(StorageMode::LibSql)
        .live_approvals()
        .await
        .expect("group builds");
    let h = g
        .thread("conv-reopen-resume")
        .script([
            RebornScriptedReply::tool_call(
                CAPABILITY_ID,
                json!({"path": "/workspace/reopen.txt", "content": "survives reopen"}),
            ),
            RebornScriptedReply::text("file written after reopen"),
        ])
        .build()
        .await
        .expect("harness builds");

    let (run_id, gate_ref) = h
        .submit_turn_until_blocked("write the reopen file")
        .await
        .expect("blocked on approval");

    // Turn-state layer: gate ref + BlockedApproval status survive an
    // independent fresh connection to the on-disk libsql file.
    h.assert_gate_survives_reopen(run_id, &gate_ref)
        .await
        .expect("gate survives turn-state reopen");

    // Approval-store layer: the pending request record survives an
    // independent reopen of the local-dev store at the same storage_root.
    let capability_harness = g
        .capability_harness()
        .expect("live_approvals always uses HostRuntime");
    let (request_id, scope) = capability_harness
        .approval_request_scope_for_test(&gate_ref)
        .expect("gate ref resolves to a request scope");
    let reopened_approvals =
        ironclaw_reborn_composition::test_support::open_local_dev_approval_request_store_for_test(
            &capability_harness.storage_root_for_test(),
        )
        .await
        .expect("fresh approval-request store opens at the same root");
    let record = reopened_approvals
        .get(&scope, request_id)
        .await
        .expect("approval store readable")
        .expect("approval request found after independent reopen");
    assert_eq!(
        record.status,
        ApprovalStatus::Pending,
        "gate must still be Pending after reopen, before resolution"
    );
    // Drop the independent connection before resuming through the live store,
    // mirroring the existing durability scenario's discipline.
    drop(reopened_approvals);

    // Resume through the gate and let the run complete.
    h.approve_gate(run_id, &gate_ref)
        .await
        .expect("approve resolves the gate");
    h.wait_for_status(run_id, TurnStatus::Completed)
        .await
        .expect("run resumes to completion");

    // The gated capability executed exactly once — not zero (lost gate) and
    // not twice (double-execution on resume).
    h.assert_capability_result_count(CAPABILITY_ID, 1)
        .await
        .expect("gated capability executed exactly once");
    h.assert_workspace_file_contains("reopen.txt", "survives reopen")
        .await
        .expect("approved write persisted to the workspace");
}
