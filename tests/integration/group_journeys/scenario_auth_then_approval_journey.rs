//! C-JOURNEY convergence scenario: one conversation chains an APPROVAL gate
//! then an AUTH gate on the SAME capability call (turn 1, `github.get_repo`),
//! then a plain APPROVAL gate on a different capability (turn 2, `write_file`),
//! then a follow-up (turn 3) — all on ONE `HostRuntimeCapabilityHarness`
//! runtime. Distinct from `scenario_interactive_approval_journey`
//! (approval → approval only): here gate classes chain WITHIN one capability
//! call AND across turns, with history carried across the gate-class boundary.
//!
//! Requires `IronClawIntegrationGroup::live_auth_and_approval()` — the converged
//! group whose capability harness surfaces both an unseeded `github.get_repo`
//! capability and real file-tool approval stores on the SAME runtime.

use super::ironclaw_support::group::{HarnessResult, IronClawIntegrationGroup};
use super::ironclaw_support::reply::IronClawScriptedReply;
use ironclaw_turns::TurnStatus;
use serde_json::json;

pub async fn run(g: &IronClawIntegrationGroup) -> HarnessResult<()> {
    let h = g
        .thread("conv-journey-auth-then-approval")
        .script([
            // turn 1 (2 entries: approval+auth-gated call + post-resolve reply)
            IronClawScriptedReply::tool_call(
                "github.get_repo",
                json!({"owner": "octocat", "repo": "hello-world"}),
            ),
            IronClawScriptedReply::text(
                "AUTHJOURNEY_TURN1 repo info retrieved after connecting github",
            ),
            // turn 2 (2 entries: approval-gated call + post-resume reply)
            IronClawScriptedReply::tool_call(
                "builtin.write_file",
                json!({"path": "/workspace/auth-journey-approved.txt", "content": "AUTH_JOURNEY_PAYLOAD"}),
            ),
            IronClawScriptedReply::text("file written after approval"),
            // turn 3 (1 entry: plain follow-up reply)
            IronClawScriptedReply::text("AUTH_JOURNEY_FINAL_REPLY summarizing the session"),
        ])
        .build()
        .await?;

    // --- turn 1: github.get_repo -- APPROVE the action, then RESOLVE auth ---
    let (run1, approval_gate1) = h
        .submit_turn_until_blocked("AUTHJOURNEY_TURN1 look up the repo")
        .await?;
    h.approve_gate(run1, &approval_gate1).await?;
    // Approving the action re-dispatches the still-uncredentialed capability,
    // which blocks AGAIN -- this time on the auth gate.
    let auth_state1 = h.wait_for_status(run1, TurnStatus::BlockedAuth).await?;
    let auth_gate1 = auth_state1
        .gate_ref
        .ok_or("blocked auth run missing gate ref")?;
    if !auth_gate1.as_str().starts_with("gate:auth-") {
        return Err(format!("expected an auth gate ref, got {auth_gate1:?}").into());
    }
    h.resolve_auth_gate(run1, &auth_gate1).await?;
    h.wait_for_status(run1, TurnStatus::Completed).await?;
    // Mutation-verified: `assert_tool_invoked` alone doesn't discriminate here
    // (recorded at dispatch entry); only the scripted body surfacing back
    // proves the credential-backed re-dispatch actually ran.
    h.assert_tool_result_contains("octocat/hello-world").await?;

    // --- turn 2: write_file approval gate (same conversation, next turn) ---
    let (run2, gate2) = h
        .submit_turn_until_blocked("AUTHJOURNEY_TURN2 write the approved file")
        .await?;
    if run2 == run1 {
        return Err("turn 2 reused turn 1's run id -- turns did not chain".into());
    }
    h.approve_gate(run2, &gate2).await?;
    h.wait_for_status(run2, TurnStatus::Completed).await?;
    // The approved write actually re-ran AND persisted (real on-disk state).
    h.assert_workspace_file_contains("auth-journey-approved.txt", "AUTH_JOURNEY_PAYLOAD")
        .await?;

    // --- turn 3: plain follow-up sees prior context (across BOTH gate classes) ---
    let run3 = h
        .submit_turn("AUTHJOURNEY_TURN3 what happened so far")
        .await?;
    if run3 == run2 || run3 == run1 {
        return Err("turn 3 reused a prior run id -- turns did not chain".into());
    }
    h.assert_reply_contains("AUTH_JOURNEY_FINAL_REPLY").await?;
    // Turn 3's request must contain BOTH turn 1's and turn 3's text — real
    // context-carryover across the approval->auth->approval chain, not a
    // tautology (only turn 3's request can, since turn 1 predates it).
    h.assert_model_request_contains_all(&["AUTHJOURNEY_TURN1", "AUTHJOURNEY_TURN3"])
        .await?;
    Ok(())
}
