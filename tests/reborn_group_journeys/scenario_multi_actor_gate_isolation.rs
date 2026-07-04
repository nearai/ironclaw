//! Permutation 3 — multi-actor journey: turn 1 is actor A, a subsequent turn is
//! actor B (via `with_actor_id`), each hitting its OWN approval gate over the
//! group's ONE shared coordinator. Pins that gate resolution + resume state stay
//! bound to the RAISING actor's turn: approving A's gate does NOT resolve B's,
//! and B's turn does not inherit A's approval — B still blocks on its own gate
//! and must be resolved independently under B's actor.
//!
//! Runs on `RebornIntegrationGroup::multiuser_approvals()`, whose per-actor
//! capability dispatch (the C-MULTIUSER `scope_capability_by_run_owner`
//! harness seam) scopes each actor's gated write to ITS OWN run owner, so
//! actor B's dispatch no longer dies with `driver_protocol_violation` under
//! actor A's user. Production already isolates capability dispatch by run
//! owner correctly; this seam makes that isolation observable at the harness
//! level.
//!
//! Complementary to (not a duplicate of): `reborn_group_approvals`'s
//! `concurrent_dual_gate_resume` (SAME actor, two threads parked simultaneously)
//! and `reborn_group_multiuser`'s `two_actors_own_threads` (distinct actors, NO
//! gate). This is distinct-actor × gate-resolution-binding — the axis neither
//! covers.

use super::reborn_support::builder::RebornIntegrationHarness;
use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::reply::RebornScriptedReply;
use super::reborn_support::session_thread::RebornThreadHarnessError;
use ironclaw_threads::SessionThreadError;
use ironclaw_turns::TurnStatus;
use serde_json::json;

/// Shared per-actor turn script: one gated write + one post-resume reply. Keeps
/// the two actor arms from fanning out into copy-pasted scripts.
fn gated_write_script(path: &str, reply: &str) -> [RebornScriptedReply; 2] {
    [
        RebornScriptedReply::tool_call(
            "builtin.write_file",
            json!({"path": path, "content": "ACTOR_PAYLOAD"}),
        ),
        RebornScriptedReply::text(reply),
    ]
}

pub async fn run(g: &RebornIntegrationGroup) -> HarnessResult<()> {
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

    // Actor B: its turn MUST still block on ITS OWN gate. If A's approval had
    // leaked to B (a resolution-scope bug), B's write would auto-complete and
    // `submit_turn_until_blocked` would fail fast on a terminal `Completed`
    // instead of returning a gate ref — this is the load-bearing isolation pin.
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
    // `/tenants/<tenant>/users/<user>/threads` subtree).
    //
    // Positive control FIRST: each actor must still be able to read its OWN
    // thread's history under its OWN scope, so the negative checks below
    // aren't vacuously true because `history()` never resolves anything on
    // this harness.
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
    reader: &RebornIntegrationHarness,
    reader_name: &str,
    other: &RebornIntegrationHarness,
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
        // Pin the SPECIFIC failure reason (cross-owner lookup resolves to
        // `SessionThreadError::UnknownThread` — scope-relative path
        // resolution, not a separate ACL check) rather than accepting any
        // `Err(_)` — an unrelated failure (e.g. a driver/backend error)
        // would otherwise vacuously satisfy a bare `.is_ok()` check without
        // proving isolation.
        Err(RebornThreadHarnessError::Thread(SessionThreadError::UnknownThread { .. })) => Ok(()),
        Err(other_err) => Err(format!(
            "isolation check for actor {other_name} under actor {reader_name}'s scope failed \
             for the WRONG reason (expected UnknownThread, got: {other_err})"
        )
        .into()),
    }
}
