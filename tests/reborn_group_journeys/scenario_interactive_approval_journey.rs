//! Canonical multi-turn JOURNEY over ONE conversation/harness — the deterministic
//! twin of the live approval-gate canary flow. Chains, on a single thread:
//!
//!   turn 1: gated `write_file` → `BlockedApproval` → APPROVE → resumes, the
//!           write re-dispatches and persists → `Completed`;
//!   turn 2: gated `write_file` → `BlockedApproval` → DENY → resumes, the model
//!           sees a non-retryable authorization failure, side effect suppressed
//!           → `Completed`;
//!   turn 3: a plain follow-up user message → the model's request carries the
//!           PRIOR turns' history → replies → `Completed`.
//!
//! Journey value (not re-tested here) = the CHAINING across turns on one
//! conversation: gate-resolve → next turn → gate-resolve → follow-up, all over
//! the group's ONE shared coordinator/turn-store with the SAME binding. The
//! single-gate mechanics (approve/deny/resume correctness) are already pinned by
//! `reborn_group_approvals`; this asserts they COMPOSE across a live session and
//! that per-turn state does not bleed (turn 2's deny does not un-persist turn 1's
//! approved write; turn 3 still sees both).

use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::reply::RebornScriptedReply;
use ironclaw_turns::TurnStatus;
use serde_json::json;

pub async fn run(g: &RebornIntegrationGroup) -> HarnessResult<()> {
    let h = g
        .thread("conv-journey-approval")
        .script([
            // turn 1 (2 entries: gated call + post-resume reply)
            RebornScriptedReply::tool_call(
                "builtin.write_file",
                json!({"path": "/workspace/journey-approved.txt", "content": "APPROVED_PAYLOAD"}),
            ),
            RebornScriptedReply::text("first file written after approval"),
            // turn 2 (2 entries: gated call + post-resume reply)
            RebornScriptedReply::tool_call(
                "builtin.write_file",
                json!({"path": "/workspace/journey-denied.txt", "content": "should not persist"}),
            ),
            RebornScriptedReply::text("understood, the second write was not authorized"),
            // turn 3 (1 entry: plain follow-up reply)
            RebornScriptedReply::text("JOURNEY_FINAL_REPLY summarizing the session"),
        ])
        .build()
        .await?;

    // --- turn 1: approve --------------------------------------------------
    let (run1, gate1) = h
        .submit_turn_until_blocked("JOURNEY_TURN1 write the approved file")
        .await?;
    h.approve_gate(run1, &gate1).await?;
    h.wait_for_status(run1, TurnStatus::Completed).await?;
    // The approved write actually re-ran AND persisted (real on-disk state).
    h.assert_workspace_file_contains("journey-approved.txt", "APPROVED_PAYLOAD")
        .await?;

    // --- turn 2: deny (same conversation, next turn) ----------------------
    let (run2, gate2) = h
        .submit_turn_until_blocked("JOURNEY_TURN2 write the denied file")
        .await?;
    // Distinct run id: the second turn is a genuinely new run on the same thread,
    // not a re-emission of turn 1.
    if run2 == run1 {
        return Err("turn 2 reused turn 1's run id — turns did not chain".into());
    }
    h.deny_gate(run2, &gate2).await?;
    h.wait_for_status(run2, TurnStatus::Completed).await?;
    // Deny suppressed the second side effect...
    h.assert_workspace_file_absent("journey-denied.txt").await?;
    // ...and did NOT disturb turn 1's persisted result (no cross-turn bleed).
    h.assert_workspace_file_contains("journey-approved.txt", "APPROVED_PAYLOAD")
        .await?;

    // --- turn 3: plain follow-up sees prior context -----------------------
    let run3 = h.submit_turn("JOURNEY_TURN3 what happened so far").await?;
    if run3 == run2 || run3 == run1 {
        return Err("turn 3 reused a prior run id — turns did not chain".into());
    }
    h.assert_reply_contains("JOURNEY_FINAL_REPLY").await?;
    // The model request for turn 3 carries the earlier turns' history: ONE
    // captured request contains BOTH turn 1's and turn 3's user text. Only turn
    // 3's request can (turn 1's request predates turn 3), so this is a real
    // context-carryover proof, not a tautology.
    h.assert_model_request_contains_all(&["JOURNEY_TURN1", "JOURNEY_TURN3"])
        .await?;
    Ok(())
}
