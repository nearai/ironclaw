//! C-JOURNEY convergence scenario (the "mixed arm" companion to
//! `scenario_auth_then_approval_journey`): a single conversation whose FIRST
//! auth gate is DENIED, then a SECOND auth gate on the SAME thread is
//! RESOLVED — proving a denied auth gate does not poison a later successful
//! resolve on the same run/thread. Complementary to
//! `reborn_integration_auth_gate.rs` (which already covers single-turn
//! auth-deny in isolation): the value here is the CHAIN across two turns.
//! Each turn also chains an APPROVAL gate before the auth gate (see
//! `scenario_auth_then_approval_journey`'s module doc for why `github.get_repo`
//! raises both in sequence on this harness).
//!
//!   turn 1: `github.get_repo` -> `BlockedApproval` -> APPROVE -> `BlockedAuth`
//!           -> DENY -> resumes, the model sees a non-retryable authorization
//!           failure, no re-dispatch -> `Completed`;
//!   turn 2: `github.get_repo` again (still no credential seeded by turn 1's
//!           deny) -> a FRESH `BlockedApproval` -> APPROVE -> a FRESH
//!           `BlockedAuth` -> RESOLVE (seed a real credential + resume) ->
//!           the SAME parked capability re-dispatches for real -> `Completed`.

use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::reply::RebornScriptedReply;
use ironclaw_turns::TurnStatus;
use serde_json::json;

pub async fn run(g: &RebornIntegrationGroup) -> HarnessResult<()> {
    let h = g
        .thread("conv-journey-auth-deny-then-retry")
        .script([
            // turn 1 (2 entries: approval+auth-gated call + post-deny reply)
            RebornScriptedReply::tool_call(
                "github.get_repo",
                json!({"owner": "octocat", "repo": "hello-world"}),
            ),
            RebornScriptedReply::text("could not look up the repo without authorization"),
            // turn 2 (2 entries: approval+auth-gated call + post-resolve reply)
            RebornScriptedReply::tool_call(
                "github.get_repo",
                json!({"owner": "octocat", "repo": "hello-world"}),
            ),
            RebornScriptedReply::text(
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
    // Pin WHAT the deny path produced, not just that it reached a terminal
    // status: the finalized reply must be turn 1's own scripted post-deny
    // text (never turn 2's), and zero network egress must have escaped
    // despite the model still being told about the (declined) capability —
    // this also anchors the "turn 2's tool result can't be turn 1 residue"
    // reasoning in the `assert_tool_result_contains` pin below.
    h.assert_reply_contains("could not look up the repo without authorization")
        .await?;
    h.assert_egress_count(0).await?;

    // --- turn 2: SAME conversation, github call raises BOTH gates AGAIN
    //     (turn 1's deny did not seed a credential or leave a stale approval;
    //     a fresh dispatch still needs a fresh approval + still resolves
    //     AuthRequired) -> approve, then RESOLVE (seed credential + resume) ---
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
    // The SECOND github dispatch (post-resolve) actually EXECUTED through the
    // real capability path: the scripted network fixture body surfaced back as
    // a recorded Completed-path result — proving turn 1's deny did not poison
    // this run's later successful resolve. (`assert_tool_invoked` alone is not
    // discriminating — see `scenario_auth_then_approval_journey`; turn 1's
    // denied dispatch never produces a result, so the recorded result here can
    // only come from turn 2's credential-backed re-dispatch.)
    //
    // `assert_tool_result_contains` itself scans ALL captured results since
    // thread baseline, so on its own it can't rule out turn-1 residue. There
    // is no `*_since`-scoped variant for successful tool results (only
    // `assert_tool_error_since` exists, for the error family — see
    // `tests/support/reborn/assertions.rs`), so the discriminating power here
    // comes from PAIRING with the turn-1 negative arm above: turn 1's deny
    // path is now pinned to have produced zero egress
    // (`assert_egress_count(0)`) and its own distinct scripted reply, so no
    // successful "octocat/hello-world" result could have originated there —
    // this assertion can only be satisfied by turn 2's own dispatch.
    h.assert_tool_result_contains("octocat/hello-world").await?;
    Ok(())
}
