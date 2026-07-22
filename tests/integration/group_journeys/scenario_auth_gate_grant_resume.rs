//! C-JOURNEY — isolated AUTH-14 / TOOL-5 pin: a capability blocked PURELY on a
//! missing credential (no approval gate in the path) raises a real `BlockedAuth`
//! gate; the user "submits credentials" (`resolve_auth_gate` seeds a real GitHub
//! credential account through the production manual-token flow); the parked
//! `github.get_repo` re-dispatches and the run completes with the real
//! credential-backed result surfaced back to the model.
//!
//! Distinct from `scenario_auth_then_approval_journey`, which reaches the auth
//! gate only AFTER an approval gate: here auto-approve is enabled so the ONLY
//! gate in the path is the auth gate. That isolates exactly the two owed
//! contracts — TOOL-5 (missing credential → generic auth gate → engine
//! completes → parked tool resumes) and AUTH-14 (grant stored → tool resumes)
//! — through the integration harness, without an approval gate confounding it.

use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::reply::RebornScriptedReply;
use ironclaw_turns::TurnStatus;
use serde_json::json;

pub async fn run(g: &RebornIntegrationGroup) -> HarnessResult<()> {
    let h = g
        .thread("conv-auth-gate-grant-resume")
        .script([
            // Gated tool-call turn = exactly TWO script entries: the call plus
            // the one post-resume model reply.
            RebornScriptedReply::tool_call(
                "github.get_repo",
                json!({"owner": "octocat", "repo": "hello-world"}),
            ),
            RebornScriptedReply::text("AUTHGRANT repo info retrieved after connecting github"),
        ])
        .build()
        .await?;

    // Auto-approve so `github.get_repo`'s Ask approval never fires — the auth
    // gate is the only block in the path.
    h.enable_auto_approve().await?;

    // Blocked purely on the missing credential: a real `gate:auth-` gate.
    let (run, auth_gate) = h
        .submit_turn_until_auth_blocked("AUTHGRANT look up the repo")
        .await?;

    // "User submits credentials": the grant is stored through the production
    // manual-token flow, then the parked capability re-dispatches.
    h.resolve_auth_gate(run, &auth_gate).await?;
    h.wait_for_status(run, TurnStatus::Completed).await?;

    // The credential-backed re-dispatch actually ran and returned the repo —
    // only the scripted body surfacing back proves the resume executed the tool
    // (dispatch-entry recording alone would not discriminate).
    h.assert_tool_result_contains("octocat/hello-world").await?;
    Ok(())
}
