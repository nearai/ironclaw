//! Scenario: two distinct actors, each on their own thread, both complete a
//! turn over the group's ONE shared runtime (Option P, #5465).
//!
//! FIX-5479 / E-MULTIUSER: the shared runtime resolves each turn's thread
//! through a per-turn owner-scope rewrite (`ThreadScopeResolver::
//! resolve_for_turn`, the same mechanism production already uses for
//! multi-user WebChat) rather than a construction-time-fixed owner. Before
//! the fix, any actor other than the group's canonical default actor failed
//! deterministically with `driver_unavailable` / "unknown thread".

use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::reply::RebornScriptedReply;

pub async fn run(g: &RebornIntegrationGroup) -> HarnessResult<()> {
    // Thread A: the group's default actor (`HARNESS_ACTOR_ID`).
    let a = g
        .thread("conv-multiuser-a")
        .script([RebornScriptedReply::text("reply-for-actor-a")])
        .build()
        .await?;
    a.submit_turn("hello from actor a")
        .await
        .map_err(|e| format!("[step A submit] {e}"))?;
    a.assert_reply_contains("reply-for-actor-a")
        .await
        .map_err(|e| format!("[step A assert] {e}"))?;

    // Thread B: a DISTINCT actor over the SAME shared coordinator/scheduler/
    // thread_service — this is exactly the path that previously raised
    // `driver_unavailable` for any non-canonical owner.
    let b = g
        .thread("conv-multiuser-b")
        .with_actor_id("reborn-actor-b")
        .script([RebornScriptedReply::text("reply-for-actor-b")])
        .build()
        .await?;
    // Non-vacuity pin: the seam must resolve a genuinely DISTINCT owner for
    // thread B. If `with_actor_id` ever regressed to a no-op, both bindings
    // would share the default actor's owner and this scenario would silently
    // degrade to the already-covered one-actor/two-conversations case.
    if a.binding.subject_user_id == b.binding.subject_user_id {
        return Err("with_actor_id seam no-op: both threads resolved the same owner".into());
    }

    b.submit_turn("hello from actor b")
        .await
        .map_err(|e| format!("[step B submit] {e}"))?;
    b.assert_reply_contains("reply-for-actor-b")
        .await
        .map_err(|e| format!("[step B assert] {e}"))?;

    // Isolation negative guards (prove genuine owner separation, not just
    // "didn't crash" — the banana-99 pattern):
    // 1. Actor A's own thread must not surface actor B's reply.
    if a.assert_reply_contains("reply-for-actor-b").await.is_ok() {
        return Err("isolation failure: actor A's thread surfaced actor B's reply".into());
    }
    // 2. Actor B's thread must not be readable through actor A's owner scope
    //    at all — B's records live under a separate `owners/<user>` subtree.
    if a
        .thread_harness
        .history(b.binding.thread_id.clone())
        .await
        .is_ok()
    {
        return Err(
            "isolation failure: actor B's thread is readable under actor A's owner scope".into(),
        );
    }

    Ok(())
}
