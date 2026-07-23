//! C-JOURNEY convergence scenario (companion to
//! `scenario_auth_then_approval_journey`): a single conversation whose FIRST
//! auth gate is DENIED, then a SECOND auth gate on the SAME thread is
//! RESOLVED — proves a denied auth gate does not poison a later successful
//! resolve on the same run/thread. Each turn also chains an approval gate
//! before the auth gate (see `scenario_auth_then_approval_journey`'s module
//! doc for why `github.get_repo` raises both in sequence on this harness).
//!
//!   turn 1: approve -> BlockedAuth -> DENY -> Completed (no re-dispatch)
//!   turn 2: fresh approve -> fresh BlockedAuth -> RESOLVE -> re-dispatches
//!           for real -> Completed

use super::ironclaw_support::group::{HarnessResult, IronClawIntegrationGroup};
use super::ironclaw_support::reply::IronClawScriptedReply;
use ironclaw_turns::TurnStatus;
use serde_json::json;

pub async fn run(g: &IronClawIntegrationGroup) -> HarnessResult<()> {
    let h = g
        .thread("conv-journey-auth-deny-then-retry")
        .script([
            // turn 1 (2 entries: approval+auth-gated call + post-deny reply)
            IronClawScriptedReply::tool_call(
                "github.get_repo",
                json!({"owner": "octocat", "repo": "hello-world"}),
            ),
            IronClawScriptedReply::text("could not look up the repo without authorization"),
            // turn 2 (2 entries: approval+auth-gated call + post-resolve reply)
            IronClawScriptedReply::tool_call(
                "github.get_repo",
                json!({"owner": "octocat", "repo": "hello-world"}),
            ),
            IronClawScriptedReply::text(
                "AUTHDENYRETRY_TURN2 repo info retrieved after connecting github",
            ),
        ])
        .build()
        .await?;

    // --- turn 1: approve the action, then DENY the auth gate ---
    let (run1, approval_gate1) = h
        .submit_turn_until_blocked("AUTHDENYRETRY_TURN1 look up the repo the first time")
        .await?;
    h.approve_gate(run1, &approval_gate1).await?;
    let auth_state1 = h.wait_for_status(run1, TurnStatus::BlockedAuth).await?;
    let auth_gate1 = auth_state1
        .gate_ref
        .ok_or("blocked auth run missing gate ref")?;
    h.deny_auth_gate(run1, &auth_gate1).await?;
    h.wait_for_status(run1, TurnStatus::Completed).await?;
    // Pin WHAT the deny path produced: turn 1's own scripted reply, and zero
    // network egress despite the model being told about the declined
    // capability — anchors the "turn 2 result can't be turn 1 residue"
    // reasoning below.
    h.assert_reply_contains("could not look up the repo without authorization")
        .await?;
    h.assert_egress_count(0).await?;

    // --- turn 2: SAME conversation; turn 1's deny left no credential or
    //     stale approval, so the dispatch raises both gates fresh again ---
    let (run2, approval_gate2) = h
        .submit_turn_until_blocked("AUTHDENYRETRY_TURN2 look up the repo the second time")
        .await?;
    if run2 == run1 {
        return Err("turn 2 reused turn 1's run id -- turns did not chain".into());
    }
    h.approve_gate(run2, &approval_gate2).await?;
    let auth_state2 = h.wait_for_status(run2, TurnStatus::BlockedAuth).await?;
    let auth_gate2 = auth_state2
        .gate_ref
        .ok_or("blocked auth run missing gate ref")?;
    h.resolve_auth_gate(run2, &auth_gate2).await?;
    h.wait_for_status(run2, TurnStatus::Completed).await?;
    h.assert_reply_contains("AUTHDENYRETRY_TURN2").await?;
    // Proves the post-resolve dispatch actually executed: turn 1's denied
    // dispatch never produces a result, and `assert_tool_result_contains` has
    // no `*_since`-scoped variant (unlike `assert_tool_error_since`), so its
    // discriminating power here comes from pairing with the turn-1 negative
    // arm above (zero egress + turn 1's own distinct reply) — this can only
    // be satisfied by turn 2's own credential-backed dispatch.
    h.assert_tool_result_contains("octocat/hello-world").await?;
    Ok(())
}
