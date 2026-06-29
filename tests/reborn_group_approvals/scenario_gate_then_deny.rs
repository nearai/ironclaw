//! Scenario: a gated `builtin.write_file` raises a real `BlockedApproval` gate;
//! denying it resumes the run so the model sees a non-retryable authorization
//! failure (not a hang) and finalizes a reply.
//!
//! Real path: gate fires → `deny_gate` (real `ApprovalResolver::deny`, no lease)
//! → `coordinator.resume_turn` with `GateResumeDisposition::Denied` → the
//! executor surfaces an authorization failure to the model → the model finalizes
//! its reply → terminal.

use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::reply::RebornScriptedReply;
use ironclaw_turns::TurnStatus;
use serde_json::json;

pub async fn run(g: &RebornIntegrationGroup) -> HarnessResult<()> {
    let h = g
        .thread("conv-deny")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.write_file",
                json!({"path": "/workspace/denied.txt", "content": "should not persist"}),
            ),
            RebornScriptedReply::text("understood, the write was not authorized"),
        ])
        .build()
        .await?;

    let (run_id, gate_ref) = h.submit_turn_until_blocked("write the denied file").await?;

    // Deny + resume. Real guard (non-vacuous): the deny→resume pipeline drives the
    // run to terminal `Completed` (the model sees a non-retryable authorization
    // failure and finalizes a reply) — it would hang/Fail if deny or resume were
    // broken. (The scripted final reply text is NOT asserted: the scripted model
    // emits it unconditionally, so it would not discriminate.)
    h.deny_gate(run_id, &gate_ref).await?;
    h.wait_for_status(run_id, TurnStatus::Completed).await?;

    // The denied write must NOT have executed: unlike the approve path, the gated
    // capability is never re-dispatched, so the target file is never created. We
    // assert the real persisted state — the file is absent on disk — which proves
    // deny blocked the side effect, not merely that the run terminated.
    h.assert_workspace_file_absent("denied.txt").await?;
    Ok(())
}
