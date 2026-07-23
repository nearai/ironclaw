//! Canonical multi-turn JOURNEY over ONE conversation/harness: chains, on a
//! single thread, turn 1 (gated `write_file` → APPROVE → persists), turn 2
//! (gated `write_file` → DENY → suppressed, non-retryable auth failure), turn 3
//! (plain follow-up carrying prior turns' history).
//!
//! Journey value (not re-tested here) = the CHAINING across turns on one
//! conversation over the group's ONE shared coordinator/turn-store. Single-gate
//! mechanics (approve/deny/resume) are already pinned by `ironclaw_group_approvals`;
//! this asserts they COMPOSE across a live session with no per-turn state bleed
//! (turn 2's deny doesn't un-persist turn 1's write; turn 3 still sees both).

use super::ironclaw_support::group::{HarnessResult, IronClawIntegrationGroup};
use super::ironclaw_support::reply::IronClawScriptedReply;
use ironclaw_turns::TurnStatus;
use serde_json::json;

pub async fn run(g: &IronClawIntegrationGroup) -> HarnessResult<()> {
    let h = g
        .thread("conv-journey-approval")
        .script([
            // turn 1 (2 entries: gated call + post-resume reply)
            IronClawScriptedReply::tool_call(
                "builtin.write_file",
                json!({"path": "/workspace/journey-approved.txt", "content": "APPROVED_PAYLOAD"}),
            ),
            IronClawScriptedReply::text("first file written after approval"),
            // turn 2 (2 entries: gated call + post-resume reply)
            IronClawScriptedReply::tool_call(
                "builtin.write_file",
                json!({"path": "/workspace/journey-denied.txt", "content": "should not persist"}),
            ),
            IronClawScriptedReply::text("understood, the second write was not authorized"),
            // turn 3 (1 entry: plain follow-up reply)
            IronClawScriptedReply::text("JOURNEY_FINAL_REPLY summarizing the session"),
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
    // Turn 3's request must carry BOTH turn 1's and turn 3's text — real
    // context-carryover, not a tautology (only turn 3's request can, since
    // turn 1 predates it).
    h.assert_model_request_contains_all(&["JOURNEY_TURN1", "JOURNEY_TURN3"])
        .await?;
    Ok(())
}
