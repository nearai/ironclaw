//! C-JOURNEY convergence scenario: a single conversation that raises an
//! APPROVAL gate then an AUTH gate on the SAME capability call (turn 1,
//! `github.get_repo` — a real WASM capability chains BOTH gate classes:
//! the user must first consent to the action, then the missing GitHub
//! credential blocks it), followed by a plain APPROVAL gate on a different
//! capability (turn 2, `write_file`), chained by a plain follow-up (turn 3).
//! All on the SAME `HostRuntimeCapabilityHarness` runtime. Distinct from
//! `scenario_interactive_approval_journey` (approval → approval only): the
//! value here is that gate classes chain WITHIN one capability call AND
//! ACROSS turns, all resolving happily on ONE `build_reborn_services` runtime,
//! with history carried across the gate-class boundary.
//!
//!   turn 1: `github.get_repo` -> `BlockedApproval` -> APPROVE -> resumes,
//!           the still-uncredentialed capability re-dispatches and blocks
//!           again at `BlockedAuth` -> RESOLVE (seed a real GitHub credential
//!           account through product-auth, then resume) -> the SAME parked
//!           capability re-dispatches for real -> `Completed`;
//!   turn 2: gated `write_file` -> `BlockedApproval` -> APPROVE -> resumes,
//!           the write re-dispatches and persists -> `Completed`;
//!   turn 3: a plain follow-up user message -> the model's request carries
//!           BOTH turn 1's and turn 3's history -> replies -> `Completed`.
//!
//! Requires `RebornIntegrationGroup::live_auth_and_approval()` — the
//! converged group whose capability harness
//! (`HostRuntimeCapabilityHarness::file_and_github_auth_tools`) surfaces
//! both an unseeded `github.get_repo` capability and real file-tool approval
//! stores on the SAME runtime.

use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::reply::RebornScriptedReply;
use ironclaw_turns::TurnStatus;
use serde_json::json;

pub async fn run(g: &RebornIntegrationGroup) -> HarnessResult<()> {
    let h = g
        .thread("conv-journey-auth-then-approval")
        .script([
            // turn 1 (2 entries: approval+auth-gated call + post-resolve reply)
            RebornScriptedReply::tool_call(
                "github.get_repo",
                json!({"owner": "octocat", "repo": "hello-world"}),
            ),
            RebornScriptedReply::text(
                "AUTHJOURNEY_TURN1 repo info retrieved after connecting github",
            ),
            // turn 2 (2 entries: approval-gated call + post-resume reply)
            RebornScriptedReply::tool_call(
                "builtin.write_file",
                json!({"path": "/workspace/auth-journey-approved.txt", "content": "AUTH_JOURNEY_PAYLOAD"}),
            ),
            RebornScriptedReply::text("file written after approval"),
            // turn 3 (1 entry: plain follow-up reply)
            RebornScriptedReply::text("AUTH_JOURNEY_FINAL_REPLY summarizing the session"),
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
    // The parked github capability actually EXECUTED post-resolve: the scripted
    // network fixture body (`octocat/hello-world`) surfaced back as a recorded
    // Completed-path capability result. `assert_tool_invoked` alone is NOT
    // discriminating here (mutation-verified): invocations are recorded at
    // dispatch ENTRY, and a resume without credentials completes the run anyway
    // by surfacing the second `AuthRequired` as a model-visible failure — only
    // the result CONTENT proves the credential-backed re-dispatch really ran.
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
    // The model request for turn 3 carries the earlier turns' history: ONE
    // captured request contains BOTH turn 1's and turn 3's user text. Only
    // turn 3's request can (turn 1's request predates turn 3), so this is a
    // real context-carryover proof spanning the approval->auth->approval chain.
    h.assert_model_request_contains_all(&["AUTHJOURNEY_TURN1", "AUTHJOURNEY_TURN3"])
        .await?;
    Ok(())
}
