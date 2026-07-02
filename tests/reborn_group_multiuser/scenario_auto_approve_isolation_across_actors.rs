//! C-MULTIUSER scenario: per-actor AUTO-APPROVE (always-allow) isolation.
//!
//! Covers both the "approval-settings non-leak" and "auto-approve non-leak"
//! requirements — they are the SAME mechanic: the always-allow toggle is keyed
//! on the run owner's `(tenant, user)` scope (`AutoApproveSettingKey`, keyed on
//! `{tenant_id, user_id}`), read at capability dispatch. A grant made for actor
//! A's owner must NOT let actor B's identical, otherwise-gated call through.
//!
//! Actor A grants always-allow for its own owner, then A's gated `write_file`
//! completes with NO approval gate. Actor B — a DISTINCT actor over the group's
//! ONE shared auto-approve store — issues the IDENTICAL `write_file` call and
//! still raises a real `BlockedApproval` gate, because A's grant is scoped to
//! A's owner alone.
//!
//! Seam: `RebornIntegrationGroup::multiuser_approvals` builds the file-approval
//! backend with `with_run_owner_scoped_capability_dispatch`, so each actor's
//! dispatch (and thus the auto-approve lookup, the persisted approval request,
//! and the gate-evidence lookup) is keyed on that actor's OWN owner — matching
//! production, where the run owner IS the capability user. Without per-actor
//! scoping, all actors collapse onto one fixed capability user and this
//! isolation is unobservable.

use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::reply::RebornScriptedReply;
use serde_json::json;

pub async fn run(g: &RebornIntegrationGroup) -> HarnessResult<()> {
    // ── Actor A (default actor): grant always-allow for A's owner, then write ─
    let a = g
        .thread("conv-approve-iso-a")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.write_file",
                json!({"path": "/workspace/actor_a.txt", "content": "actor-a-write"}),
            ),
            RebornScriptedReply::text("wrote without a gate"),
        ])
        .build()
        .await?;
    // Grant always-allow for A's owner ONLY.
    let a_owner = a
        .binding
        .subject_user_id
        .clone()
        .ok_or("actor A binding missing subject user id")?;
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
            RebornScriptedReply::tool_call(
                "builtin.write_file",
                json!({"path": "/workspace/actor_b.txt", "content": "actor-b-write"}),
            ),
            // Trailing reply so a LEAK (B wrongly inheriting A's grant) surfaces
            // as a clean `Completed` status mismatch in `submit_turn_until_blocked`
            // rather than a script-exhausted model error.
            RebornScriptedReply::text("this should never be reached — B must gate"),
        ])
        .build()
        .await?;
    // Non-vacuity: distinct owners, else "B gates" would be the same scope as A.
    if a.binding.subject_user_id == b.binding.subject_user_id {
        return Err("with_actor_id seam no-op: both actors resolved the same owner".into());
    }
    // Give B its OWN explicit always-allow=OFF (auto-approve defaults ON per
    // `AUTO_APPROVE_DEFAULT_ENABLED`), so B is a genuine gating actor. The
    // isolation claim is then unambiguous: A's ON and B's OFF coexist per user.
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

    Ok(())
}
