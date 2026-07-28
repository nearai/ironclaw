//! Scenario pair: a TRIGGERED run (real `TrustedTriggerFireSubmitter` origin,
//! `TurnOriginKind::ScheduledTrigger`) raises a real `BlockedApproval` gate
//! mid-fire; the creator approves (or denies) and the run resumes to terminal.
//!
//! Mirrors `reborn_group_approvals/scenario_gate_then_{approve,deny}.rs`, but
//! the gated turn arrives through the trusted-trigger submit wire
//! (`submit_triggered_turn_scripted`) instead of the interactive
//! `submit_turn_until_blocked` — pinning that trigger-origin runs raise, park
//! on, and resume from approval gates exactly like interactive runs (nothing
//! in the scheduled_trigger surface/deny-map (#5505) suppresses the
//! `builtin.write_file` Ask gate; only trigger mutator verbs are stripped).
//!
//! Each arm builds its OWN `live_approvals` group: the two fires would
//! otherwise interleave one shared script and one shared approval store,
//! making gate/coverage attribution ambiguous.

use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::reply::RebornScriptedReply;
use ironclaw_host_api::TenantId;
use ironclaw_turns::{ResumeTurnPrecondition, TurnStatus};
use serde_json::json;

/// Approve arm: triggered fire → gate raised → approve → gated write re-runs
/// and PERSISTS → `Completed`.
pub async fn run_approve(g: &RebornIntegrationGroup) -> HarnessResult<()> {
    let h = g.thread("conv-triggered-gate-approve").build().await?;

    let submission = h
        .submit_triggered_turn_scripted(
            "write the scheduled report",
            [
                RebornScriptedReply::tool_call(
                    "builtin.write_file",
                    json!({"path": "/workspace/triggered-approved.txt", "content": "triggered approved write"}),
                ),
                RebornScriptedReply::text("report written after approval"),
            ],
        )
        .await?;

    // The triggered run parks on a REAL approval gate (same store, same
    // PermissionMode::Ask path as interactive runs — auto-approve is disabled
    // at group construction).
    let state = h
        .wait_for_status_in_scope(
            &submission.turn_scope,
            submission.run_id,
            TurnStatus::BlockedApproval,
        )
        .await?;
    let gate_ref = state
        .gate_ref
        .ok_or("blocked triggered run missing gate ref")?;
    if !gate_ref.as_str().starts_with("gate:approval-") {
        return Err(format!("expected a local-dev approval gate, got {gate_ref:?}").into());
    }

    h.approve_gate_in_scope(&submission.turn_scope, submission.run_id, &gate_ref)
        .await?;
    h.wait_for_status_in_scope(
        &submission.turn_scope,
        submission.run_id,
        TurnStatus::Completed,
    )
    .await?;

    // Side-effect proof (mirrors the interactive approve arm).
    h.assert_workspace_file_contains("triggered-approved.txt", "triggered approved write")
        .await?;
    Ok(())
}

/// Deny arm: triggered fire → gate raised → deny → the executor surfaces a
/// non-retryable authorization failure to the model, which finalizes →
/// `Completed`, and the denied side effect never executed.
pub async fn run_deny(g: &RebornIntegrationGroup) -> HarnessResult<()> {
    let h = g.thread("conv-triggered-gate-deny").build().await?;

    let submission = h
        .submit_triggered_turn_scripted(
            "write the scheduled report",
            [
                RebornScriptedReply::tool_call(
                    "builtin.write_file",
                    json!({"path": "/workspace/triggered-denied.txt", "content": "should not persist"}),
                ),
                RebornScriptedReply::text("understood, the scheduled write was not authorized"),
            ],
        )
        .await?;

    let state = h
        .wait_for_status_in_scope(
            &submission.turn_scope,
            submission.run_id,
            TurnStatus::BlockedApproval,
        )
        .await?;
    let gate_ref = state
        .gate_ref
        .ok_or("blocked triggered run missing gate ref")?;
    if !gate_ref.as_str().starts_with("gate:approval-") {
        return Err(format!("expected a local-dev approval gate, got {gate_ref:?}").into());
    }

    h.deny_gate_in_scope(&submission.turn_scope, submission.run_id, &gate_ref)
        .await?;
    h.wait_for_status_in_scope(
        &submission.turn_scope,
        submission.run_id,
        TurnStatus::Completed,
    )
    .await?;

    // Side-effect proof (mirrors the interactive deny arm).
    h.assert_workspace_file_absent("triggered-denied.txt")
        .await?;
    Ok(())
}

/// C-DENYEDGE (row 1): a resume for the RIGHT `run_id` but a MUTATED
/// `TurnScope` (different tenant) must be rejected, not silently resolved
/// against the wrong tenant's run.
///
/// Drives a triggered fire to a real `BlockedApproval` gate like
/// [`run_approve`], then resumes the SAME `run_id` under a mismatched tenant
/// scope. The turn-state store's `resume_turn_once` checks `record.scope !=
/// request.scope` BEFORE consulting gate/precondition/actor, so this
/// deterministically hits `TurnError::ScopeNotFound` and reinserts the
/// taken-out record unconditionally — the gate's `BlockedApproval` state is
/// untouched by the rejected attempt.
///
/// Drives `resume_run_in_scope` directly (not `approve_gate_in_scope`/
/// `deny_gate_in_scope`): those first flip the approval record to
/// Approved/Denied in the scope-independent local-dev store even when the
/// resume itself is rejected, which would corrupt the still-`Pending` state
/// this scenario's non-vacuity check depends on.
pub async fn run_wrong_scope_resume_rejected(g: &RebornIntegrationGroup) -> HarnessResult<()> {
    let h = g.thread("conv-triggered-gate-wrong-scope").build().await?;

    let submission = h
        .submit_triggered_turn_scripted(
            "write the scheduled report",
            [
                RebornScriptedReply::tool_call(
                    "builtin.write_file",
                    json!({"path": "/workspace/triggered-wrong-scope.txt", "content": "triggered wrong-scope write"}),
                ),
                RebornScriptedReply::text("report written after approval"),
            ],
        )
        .await?;

    let state = h
        .wait_for_status_in_scope(
            &submission.turn_scope,
            submission.run_id,
            TurnStatus::BlockedApproval,
        )
        .await?;
    let gate_ref = state
        .gate_ref
        .ok_or("blocked triggered run missing gate ref")?;
    if !gate_ref.as_str().starts_with("gate:approval-") {
        return Err(format!("expected a local-dev approval gate, got {gate_ref:?}").into());
    }

    // Same run_id, DIFFERENT tenant_id — the attacker-controlled scope.
    let mut mutated_scope = submission.turn_scope.clone();
    mutated_scope.tenant_id = TenantId::new("wrong-tenant-scope-attack")?;

    // The idempotency key inside `resume_run_in_scope` is scope-qualified by
    // the turn store (`RunIdempotencyKey { scope, run_id, key }`), so this
    // rejected attempt under the mutated scope cannot replay-collide with the
    // real resume below.
    let err = h
        .resume_run_in_scope(
            &mutated_scope,
            submission.run_id,
            gate_ref.clone(),
            None,
            ResumeTurnPrecondition::BlockedApprovalGate,
        )
        .await
        .err()
        .ok_or("expected mis-scoped resume to be rejected with ScopeNotFound, but it queued")?;
    // Pin the EXACT `TurnError::ScopeNotFound` Display string
    // (`crates/ironclaw_turns/src/status.rs:403-404`), not just `.is_err()` —
    // a discriminating negative assertion.
    if err.to_string() != "turn run not found" {
        return Err(format!(
            "expected TurnError::ScopeNotFound (\"turn run not found\"), got: {err}"
        )
        .into());
    }

    // Non-vacuity: the mutated-scope call didn't corrupt run state, and the
    // gate is still genuinely live under the REAL scope/gate_ref.
    h.approve_gate_in_scope(&submission.turn_scope, submission.run_id, &gate_ref)
        .await?;
    h.wait_for_status_in_scope(
        &submission.turn_scope,
        submission.run_id,
        TurnStatus::Completed,
    )
    .await?;
    h.assert_workspace_file_contains("triggered-wrong-scope.txt", "triggered wrong-scope write")
        .await?;
    Ok(())
}
