//! W4-ASK-EACH-ONCE (#5306 class): a capability under an explicit
//! `ToolPermissionOverride::AskEachTime` override raises a real
//! `BlockedApproval` gate, and approving it resumes the run to `Completed` in
//! ONE round trip -- it must NOT re-gate the just-approved resume.
//!
//! Regression: pre-#5306, `require_approval_for_profile_policy` checked the
//! `ask_each_time` override BEFORE consulting the resume's one-shot approval
//! lease, so a resumed dispatch re-hit the `ask_each_time` branch and gated
//! AGAIN -- an unresumable `BlockedApproval` loop. Fixed by checking the
//! lease first.
//!
//! `live_approvals()`'s plain Ask-mode gate (`scenario_gate_then_approve.rs`)
//! never reaches the `ask_each_time` branch, so this scenario installs the
//! override explicitly via `set_ask_each_time_override_for_test`.

use super::ironclaw_support::group::{HarnessResult, IronClawIntegrationGroup};
use super::ironclaw_support::reply::IronClawScriptedReply;
use ironclaw_host_api::CapabilityId;
use ironclaw_turns::TurnStatus;
use serde_json::json;

pub async fn run(g: &IronClawIntegrationGroup) -> HarnessResult<()> {
    let h = g
        .thread("conv-ask-each-time")
        .script([
            IronClawScriptedReply::tool_call(
                "builtin.write_file",
                json!({"path": "/workspace/ask-each-time.txt", "content": "ask each time payload"}),
            ),
            IronClawScriptedReply::text("file written after ask-each-time approval"),
        ])
        .build()
        .await?;

    // Install the explicit AskEachTime override under the same (tenant, user)
    // scope key `require_approval_for_profile_policy` reads `tool_override` under.
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

    // Discriminating assertion: pre-#5306 the resumed dispatch re-blocks, so
    // `wait_for_status(Completed)` times out instead of returning `Ok`.
    h.approve_gate(run_id, &gate_ref).await?;
    h.wait_for_status(run_id, TurnStatus::Completed).await?;

    // The approved write actually re-ran AND PERSISTED through the ONE resume
    // -- not merely that some terminal status was reached.
    h.assert_workspace_file_contains("ask-each-time.txt", "ask each time payload")
        .await?;

    // "Resumes exactly once" companion proof: re-approving the same
    // already-resolved gate_ref must fail NotPending, not succeed against a
    // silently re-raised gate.
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
