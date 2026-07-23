//! Permutation 3 — multi-actor journey: turn 1 is actor A, a subsequent turn is
//! actor B (via `with_actor_id`), each hitting its OWN approval gate over the
//! group's ONE shared coordinator. Pins that gate resolution + resume state stay
//! bound to the RAISING actor's turn: approving A's gate does NOT resolve B's,
//! and B still blocks on its own gate, resolved independently under B's actor.
//!
//! Runs on `IronClawIntegrationGroup::multiuser_approvals()` (C-MULTIUSER
//! `scope_capability_by_run_owner` harness seam), which scopes each actor's
//! gated write to ITS OWN run owner so actor B's dispatch doesn't die with
//! `driver_protocol_violation` under actor A's user.
//!
//! Complementary to (not a duplicate of): `ironclaw_group_approvals`'s
//! `concurrent_dual_gate_resume` (SAME actor, two threads parked simultaneously)
//! and `ironclaw_group_multiuser`'s `two_actors_own_threads` (distinct actors, NO
//! gate). This is distinct-actor × gate-resolution-binding — the axis neither
//! covers.

use super::ironclaw_support::builder::IronClawIntegrationHarness;
use super::ironclaw_support::group::{HarnessResult, IronClawIntegrationGroup};
use super::ironclaw_support::reply::IronClawScriptedReply;
use super::ironclaw_support::session_thread::IronClawThreadHarnessError;
use ironclaw_threads::SessionThreadError;
use ironclaw_turns::TurnStatus;
use serde_json::json;

/// Shared per-actor turn script: one gated write + one post-resume reply. Keeps
/// the two actor arms from fanning out into copy-pasted scripts.
fn gated_write_script(path: &str, reply: &str) -> [IronClawScriptedReply; 2] {
    [
        IronClawScriptedReply::tool_call(
            "builtin.write_file",
            json!({"path": path, "content": "ACTOR_PAYLOAD"}),
        ),
        IronClawScriptedReply::text(reply),
    ]
}

pub async fn run(g: &IronClawIntegrationGroup) -> HarnessResult<()> {
    // Thread A: the group's default actor.
    let a = g
        .thread("conv-journey-actor-a")
        .script(gated_write_script(
            "/workspace/journey-actor-a.txt",
            "actor-a write approved",
        ))
        .build()
        .await?;

    // Thread B: a DISTINCT actor over the SAME shared coordinator.
    let b = g
        .thread("conv-journey-actor-b")
        .with_actor_id("reborn-journey-actor-b")
        .script(gated_write_script(
            "/workspace/journey-actor-b.txt",
            "actor-b write approved",
        ))
        .build()
        .await?;

    // Non-vacuity: the two threads must resolve genuinely DISTINCT owners, else
    // this degrades to the single-actor case already covered elsewhere.
    if a.binding.subject_user_id == b.binding.subject_user_id {
        return Err("with_actor_id seam no-op: both threads resolved the same owner".into());
    }

    // The `multiuser_approvals` group scopes capability dispatch by run owner
    // (C-MULTIUSER `scope_capability_by_run_owner` seam) and defaults auto-approve
    // ON per owner, so each actor's gate only fires once its OWN `(tenant, user)`
    // scope is set OFF. Disable both explicitly BEFORE any turn so BOTH actors
    // raise a real `BlockedApproval` gate under their own owner — the state whose
    // per-actor binding this scenario pins.
    let owner_a = a
        .binding
        .subject_user_id
        .as_ref()
        .ok_or("actor A binding has no subject user id")?;
    let owner_b = b
        .binding
        .subject_user_id
        .as_ref()
        .ok_or("actor B binding has no subject user id")?;
    g.disable_auto_approve_for_owner(owner_a).await?;
    g.disable_auto_approve_for_owner(owner_b).await?;

    // Actor A: raise + approve + complete.
    let (run_a, gate_a) = a
        .submit_turn_until_blocked("ACTOR_A write the file")
        .await
        .map_err(|e| format!("[A block] {e}"))?;
    a.approve_gate(run_a, &gate_a)
        .await
        .map_err(|e| format!("[A approve] {e}"))?;
    a.wait_for_status(run_a, TurnStatus::Completed)
        .await
        .map_err(|e| format!("[A complete] {e}"))?;
    a.assert_workspace_file_contains("journey-actor-a.txt", "ACTOR_PAYLOAD")
        .await?;

    // Load-bearing isolation pin: if A's approval leaked to B, B's write would
    // auto-complete and this call would fail fast on `Completed` instead of a gate ref.
    let (run_b, gate_b) = b
        .submit_turn_until_blocked("ACTOR_B write the file")
        .await
        .map_err(|e| format!("[B block — did B inherit A's approval?] {e}"))?;
    if run_b == run_a {
        return Err("actor B reused actor A's run id — resolution not actor-bound".into());
    }
    if gate_b.as_str() == gate_a.as_str() {
        return Err("actor B raised actor A's gate ref — gate not actor-bound".into());
    }
    b.approve_gate(run_b, &gate_b)
        .await
        .map_err(|e| format!("[B approve] {e}"))?;
    b.wait_for_status(run_b, TurnStatus::Completed)
        .await
        .map_err(|e| format!("[B complete] {e}"))?;
    b.assert_workspace_file_contains("journey-actor-b.txt", "ACTOR_PAYLOAD")
        .await?;

    // Owner isolation: neither actor's owner scope may read the other's thread
    // history (each owner's records live under a separate
    // `/tenants/<tenant>/users/<user>/threads` subtree). Positive control
    // FIRST: each actor must read its OWN history, so the negative checks
    // below aren't vacuously true.
    let a_own_history = a
        .thread_harness
        .history(a.binding.thread_id.clone())
        .await
        .map_err(|e| format!("[A own history] {e}"))?;
    if a_own_history.is_empty() {
        return Err("positive control failed: actor A's own history is empty".into());
    }
    let b_own_history = b
        .thread_harness
        .history(b.binding.thread_id.clone())
        .await
        .map_err(|e| format!("[B own history] {e}"))?;
    if b_own_history.is_empty() {
        return Err("positive control failed: actor B's own history is empty".into());
    }
    assert_history_isolated(&a, "A", &b, "B").await?;
    assert_history_isolated(&b, "B", &a, "A").await?;
    Ok(())
}

async fn assert_history_isolated(
    reader: &IronClawIntegrationHarness,
    reader_name: &str,
    other: &IronClawIntegrationHarness,
    other_name: &str,
) -> HarnessResult<()> {
    match reader
        .thread_harness
        .history(other.binding.thread_id.clone())
        .await
    {
        Ok(_) => Err(format!(
            "isolation failure: actor {other_name}'s thread is readable under actor \
             {reader_name}'s owner scope"
        )
        .into()),
        // Pin the SPECIFIC failure (cross-owner lookup -> UnknownThread via
        // scope-relative path resolution, not a separate ACL check) rather
        // than accepting any `Err(_)`, which an unrelated backend error
        // could satisfy without proving isolation.
        Err(IronClawThreadHarnessError::Thread(SessionThreadError::UnknownThread { .. })) => Ok(()),
        Err(other_err) => Err(format!(
            "isolation check for actor {other_name} under actor {reader_name}'s scope failed \
             for the WRONG reason (expected UnknownThread, got: {other_err})"
        )
        .into()),
    }
}
