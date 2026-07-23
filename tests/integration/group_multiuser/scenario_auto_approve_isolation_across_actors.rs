//! C-MULTIUSER scenario: per-actor AUTO-APPROVE (always-allow) isolation. The
//! always-allow toggle is keyed on the run owner's `(tenant, user)` scope
//! (`AutoApproveSettingKey`), read at capability dispatch — a grant made for
//! actor A's owner must NOT let actor B's identical, otherwise-gated call
//! through. Covers both "approval-settings non-leak" and "auto-approve
//! non-leak", which are the SAME mechanic.
//!
//! Actor A grants always-allow for its own owner, then A's gated `write_file`
//! completes with NO gate. Actor B — a DISTINCT actor over the SAME shared
//! auto-approve store — issues the IDENTICAL call and still raises a real
//! `BlockedApproval` gate, because A's grant is scoped to A's owner alone.
//!
//! Seam: `IronClawIntegrationGroup::multiuser_approvals` builds the file-approval
//! backend with `with_run_owner_scoped_capability_dispatch`, keying dispatch
//! (auto-approve lookup, approval request, gate evidence) on each actor's OWN
//! owner — matching production, where the run owner IS the capability user.

use super::ironclaw_support::group::{HarnessResult, IronClawIntegrationGroup};
use super::ironclaw_support::reply::IronClawScriptedReply;
use serde_json::json;

pub async fn run(g: &IronClawIntegrationGroup) -> HarnessResult<()> {
    // ── Actor A (default actor): grant always-allow for A's owner, then write ─
    let a = g
        .thread("conv-approve-iso-a")
        .script([
            IronClawScriptedReply::tool_call(
                "builtin.write_file",
                json!({"path": "/workspace/actor_a.txt", "content": "actor-a-write"}),
            ),
            IronClawScriptedReply::text("wrote without a gate"),
        ])
        .build()
        .await?;
    // Grant always-allow for A's owner ONLY.
    let a_owner = a
        .binding
        .subject_user_id
        .clone()
        .ok_or("actor A binding missing subject user id")?;
    // Force A OFF first: `AUTO_APPROVE_DEFAULT_ENABLED` is `true`, so a no-op
    // `enable_auto_approve_for_owner` could otherwise mask a broken grant.
    g.disable_auto_approve_for_owner(&a_owner)
        .await
        .map_err(|e| format!("[A pre-disable] {e}"))?;
    g.enable_auto_approve_for_owner(&a_owner)
        .await
        .map_err(|e| format!("[A grant] {e}"))?;
    // `submit_turn` waits for `Completed`; it would fail if A blocked on a gate
    // (i.e. if the grant did not reach A's dispatch scope).
    a.submit_turn("write the actor-a file")
        .await
        .map_err(|e| format!("[A write submit — grant must skip the gate] {e}"))?;
    a.assert_tool_invoked("builtin.write_file")
        .await
        .map_err(|e| format!("[A write invoked] {e}"))?;
    a.assert_workspace_file_contains("actor_a.txt", "actor-a-write")
        .await
        .map_err(|e| format!("[A write persisted] {e}"))?;

    // ── Actor B (DISTINCT actor, SAME shared auto-approve store): still gates ─
    let b = g
        .thread("conv-approve-iso-b")
        .with_actor_id("reborn-actor-b")
        .script([
            IronClawScriptedReply::tool_call(
                "builtin.write_file",
                json!({"path": "/workspace/actor_b.txt", "content": "actor-b-write"}),
            ),
            // Trailing reply so a LEAK (B wrongly inheriting A's grant) surfaces
            // as a clean `Completed` status mismatch in `submit_turn_until_blocked`
            // rather than a script-exhausted model error.
            IronClawScriptedReply::text("this should never be reached — B must gate"),
        ])
        .build()
        .await?;
    // Non-vacuity: distinct owners, else "B gates" would be the same scope as A.
    if a.binding.subject_user_id == b.binding.subject_user_id {
        return Err("with_actor_id seam no-op: both actors resolved the same owner".into());
    }
    // Give B its OWN explicit always-allow=OFF so B is a genuine gating actor;
    // A's ON and B's OFF then coexist unambiguously per owner.
    let b_owner = b
        .binding
        .subject_user_id
        .clone()
        .ok_or("actor B binding missing subject user id")?;
    g.disable_auto_approve_for_owner(&b_owner)
        .await
        .map_err(|e| format!("[B disable] {e}"))?;
    // ISOLATION: B's identical call raises a real approval gate — A's grant does
    // not cross the actor boundary. `submit_turn_until_blocked` asserts the run
    // reaches `BlockedApproval` with a `gate:approval-` ref.
    let (_run_id, _gate_ref) = b
        .submit_turn_until_blocked("write the actor-b file")
        .await
        .map_err(|e| format!("[B must still gate — A's grant must not apply] {e}"))?;
    // The gate blocked BEFORE the capability performed its write — no side effect.
    b.assert_workspace_file_absent("actor_b.txt")
        .await
        .map_err(|e| format!("[B write must not have happened while blocked] {e}"))?;

    Ok(())
}
