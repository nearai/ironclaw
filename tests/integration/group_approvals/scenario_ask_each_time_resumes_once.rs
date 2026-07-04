//! W4-ASK-EACH-ONCE (#5306 class): a capability under an explicit
//! `ToolPermissionOverride::AskEachTime` override raises a real
//! `BlockedApproval` gate, and approving it resumes the run to `Completed` in
//! ONE round trip -- it must NOT re-gate the just-approved resume.
//!
//! Pre-#5306, `require_approval_for_profile_policy` checked the explicit
//! `ask_each_time` override (and the "hard floor" force-approval class)
//! BEFORE consulting the matching one-shot approval lease a resume carries.
//! So a resumed dispatch for an `AskEachTime`-overridden capability hit the
//! `ask_each_time` branch first and gated AGAIN, ignoring the lease that was
//! just issued for exactly this invocation -- the run could never reach
//! `Completed` (an unresumable BlockedApproval loop). The fix reordered the
//! one-shot-lease check to run FIRST.
//!
//! `live_approvals()`'s plain `write_file`/`read_file` @ `PermissionMode::Ask`
//! gate (exercised by `scenario_gate_then_approve.rs`) does NOT reach the
//! `ask_each_time` branch at all (no override is installed there), so it
//! cannot exercise this ordering bug -- this scenario installs the override
//! explicitly via `set_ask_each_time_override_for_test` before submitting.

use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::reply::RebornScriptedReply;
use ironclaw_host_api::CapabilityId;
use ironclaw_turns::TurnStatus;
use serde_json::json;

pub async fn run(g: &RebornIntegrationGroup) -> HarnessResult<()> {
    let h = g
        .thread("conv-ask-each-time")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.write_file",
                json!({"path": "/workspace/ask-each-time.txt", "content": "ask each time payload"}),
            ),
            RebornScriptedReply::text("file written after ask-each-time approval"),
        ])
        .build()
        .await?;

    // Install the explicit AskEachTime override for this run's real dispatch
    // (tenant, user) -- the same scope key `require_approval_for_profile_policy`
    // reads `tool_override` under.
    g.capability_harness()
        .ok_or("live_approvals always uses HostRuntime")?
        .set_ask_each_time_override_for_test(
            &CapabilityId::new("builtin.write_file")?,
            h.binding.tenant_id.clone(),
            h.binding.actor_user_id.clone(),
        )
        .await?;

    // Real gate: the ask-each-time override forces approval, same as the plain
    // Ask-mode gate.
    let (run_id, gate_ref) = h
        .submit_turn_until_blocked("write the ask-each-time file")
        .await?;

    // Approve through the real resolver + resume. This is the discriminating
    // assertion: pre-#5306, the resumed dispatch re-hits the `ask_each_time`
    // branch and re-blocks (the run never reaches `Completed`, so
    // `wait_for_status(Completed)` times out against the still-`BlockedApproval`
    // status instead of returning `Ok`).
    h.approve_gate(run_id, &gate_ref).await?;
    h.wait_for_status(run_id, TurnStatus::Completed).await?;

    // The approved write actually re-ran AND PERSISTED through the ONE resume
    // -- not merely that some terminal status was reached.
    h.assert_workspace_file_contains("ask-each-time.txt", "ask each time payload")
        .await?;

    // "Resumes exactly once" companion proof: the SAME gate_ref is already
    // resolved (Approved) from the single resume above, so a second approve
    // on it must fail NotPending, not succeed against a fresh re-raised gate
    // (which would indicate a second, distinct BlockedApproval was silently
    // created and resolved somewhere in between).
    let err =
        h.approve_gate(run_id, &gate_ref).await.err().ok_or(
            "expected err: re-approving the already-resolved ask-each-time gate must fail",
        )?;
    let err_text = err.to_string();
    if !err_text.contains("approval request is not pending") {
        return Err(format!("expected the NotPending resolver error text, got: {err_text}").into());
    }
    Ok(())
}
