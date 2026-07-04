//! C-JOURNEY (triggered-origin): a triggered fire whose run raises a real
//! `BlockedApproval` gate, gets resolved, and then CHAINS into a SECOND
//! `BlockedApproval` gate in the SAME run (the post-resume model call issues
//! another gated tool call instead of finalizing) — pinning that
//! `TurnOriginKind::ScheduledTrigger` origin survives BOTH resume hops, not
//! just the first.
//!
//! Distinct from `scenario_triggered_gate::run_approve`: that scenario is
//! ONE gate (tool_call, text) — a single resume hop. This scenario scripts
//! THREE model calls (tool_call, tool_call, text) so the run parks TWICE,
//! and reads `state.product_context.origin` at both parked states AND at
//! final `Completed`, closing the gap that a regression which only
//! preserved origin across the FIRST resume (e.g. a resume path that
//! rebuilds `product_context` from a fresh, non-trigger-aware default on the
//! second hop) would otherwise slip through undetected.

use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::reply::RebornScriptedReply;
use ironclaw_turns::{TurnOriginKind, TurnStatus};
use serde_json::json;

pub async fn run_chained_approve(g: &RebornIntegrationGroup) -> HarnessResult<()> {
    let h = g.thread("conv-triggered-chained-gate").build().await?;

    let submission = h
        .submit_triggered_turn_scripted(
            "write the scheduled report and then the follow-up note",
            [
                RebornScriptedReply::tool_call(
                    "builtin.write_file",
                    json!({"path": "/workspace/triggered-chained-a.txt", "content": "triggered chained write A"}),
                ),
                RebornScriptedReply::tool_call(
                    "builtin.write_file",
                    json!({"path": "/workspace/triggered-chained-b.txt", "content": "triggered chained write B"}),
                ),
                RebornScriptedReply::text("both scheduled writes complete after approval"),
            ],
        )
        .await?;

    // ---- First gate ----
    let state_gate1 = h
        .wait_for_status_in_scope(
            &submission.turn_scope,
            submission.run_id,
            TurnStatus::BlockedApproval,
        )
        .await?;
    let gate_ref_1 = state_gate1
        .gate_ref
        .clone()
        .ok_or("first blocked triggered run missing gate ref")?;
    if !gate_ref_1.as_str().starts_with("gate:approval-") {
        return Err(format!("expected a local-dev approval gate, got {gate_ref_1:?}").into());
    }
    assert_scheduled_trigger_origin(&state_gate1, "first BlockedApproval park")?;

    h.approve_gate_in_scope(&submission.turn_scope, submission.run_id, &gate_ref_1)
        .await?;

    // ---- Second gate: the post-resume model call chains into ANOTHER gated
    // tool call in the SAME run, not a finalizing text reply. ----
    let state_gate2 = h
        .wait_for_status_in_scope(
            &submission.turn_scope,
            submission.run_id,
            TurnStatus::BlockedApproval,
        )
        .await?;
    let gate_ref_2 = state_gate2
        .gate_ref
        .clone()
        .ok_or("second blocked triggered run missing gate ref")?;
    if !gate_ref_2.as_str().starts_with("gate:approval-") {
        return Err(format!("expected a local-dev approval gate, got {gate_ref_2:?}").into());
    }
    if gate_ref_2 == gate_ref_1 {
        return Err(format!(
            "second park reused the first gate_ref ({gate_ref_1:?}) — the chained \
             tool call did not raise a genuinely NEW gate"
        )
        .into());
    }
    assert_scheduled_trigger_origin(&state_gate2, "second BlockedApproval park (chained)")?;

    h.approve_gate_in_scope(&submission.turn_scope, submission.run_id, &gate_ref_2)
        .await?;

    // ---- Completion ----
    let state_completed = h
        .wait_for_status_in_scope(
            &submission.turn_scope,
            submission.run_id,
            TurnStatus::Completed,
        )
        .await?;
    assert_scheduled_trigger_origin(&state_completed, "final Completed state")?;

    // Both chained writes persisted — proves the SECOND gate's grant actually
    // re-dispatched its own capability call, not a replay of the first.
    h.assert_workspace_file_contains("triggered-chained-a.txt", "triggered chained write A")
        .await?;
    h.assert_workspace_file_contains("triggered-chained-b.txt", "triggered chained write B")
        .await?;

    // Reply persistence: the finalized reply (the turn's THIRD scripted model
    // call) lands in the trigger's own thread, readable through the same
    // thread-history boundary interactive replies use.
    h.thread_harness
        .assert_final_reply(
            submission.turn_scope.thread_id.clone(),
            "both scheduled writes complete after approval",
        )
        .await?;

    Ok(())
}

/// Read the run state fresh at the coordinator boundary (not the state handed
/// in from an earlier `wait_for_status_in_scope`/`approve_gate_in_scope`
/// call) and assert its `product_context.origin` is
/// `TurnOriginKind::ScheduledTrigger`. Re-reading independently at each of
/// the three checkpoints (not just trusting the ONE `TriggeredSubmission` for
/// the whole scenario) is what actually proves origin persists ACROSS both
/// resume hops, rather than merely being set once at submit time.
fn assert_scheduled_trigger_origin(
    state: &ironclaw_turns::TurnRunState,
    checkpoint: &str,
) -> HarnessResult<()> {
    let origin = state.product_context.as_ref().map(|context| context.origin);
    if origin != Some(TurnOriginKind::ScheduledTrigger) {
        return Err(format!(
            "expected TurnOriginKind::ScheduledTrigger to persist at {checkpoint}, got {origin:?}"
        )
        .into());
    }
    Ok(())
}
