//! Scenario: a gated `builtin.write_file` raises a real `BlockedApproval` gate;
//! denying it resumes the run so the model sees a non-retryable authorization
//! failure (not a hang) and finalizes a reply.
//!
//! Real path: gate fires → `deny_gate` (real `ApprovalResolver::deny`, no lease)
//! → `coordinator.resume_turn` with `GateResumeDisposition::Denied` → the
//! executor surfaces an authorization failure to the model → the model finalizes
//! its reply → terminal.

use super::ironclaw_support::group::{HarnessResult, IronClawIntegrationGroup};
use super::ironclaw_support::reply::IronClawScriptedReply;
use ironclaw_turns::TurnStatus;
use serde_json::json;

pub async fn run(g: &IronClawIntegrationGroup) -> HarnessResult<()> {
    let h = g
        .thread("conv-deny")
        .script([
            IronClawScriptedReply::tool_call(
                "builtin.write_file",
                json!({"path": "/workspace/denied.txt", "content": "should not persist"}),
            ),
            IronClawScriptedReply::text("understood, the write was not authorized"),
        ])
        .build()
        .await?;

    let (run_id, gate_ref) = h.submit_turn_until_blocked("write the denied file").await?;

    // Non-vacuous: the deny→resume pipeline must drive the run to terminal
    // `Completed`, not hang/Fail. (Scripted final reply text is not asserted
    // -- the model emits it unconditionally, so it wouldn't discriminate.)
    h.deny_gate(run_id, &gate_ref).await?;
    h.wait_for_status(run_id, TurnStatus::Completed).await?;

    // The denied write must NOT have executed: the gated capability is never
    // re-dispatched, so the file is absent on disk.
    h.assert_workspace_file_absent("denied.txt").await?;
    Ok(())
}
