//! Scenario: two distinct actors, each on their own thread, both complete a
//! turn over the group's ONE shared runtime (Option P, #5465).
//!
//! FIX-5479 / E-MULTIUSER: the shared runtime resolves each turn's thread
//! through a per-turn owner-scope rewrite (`ThreadScopeResolver::
//! resolve_for_turn`, the same mechanism production already uses for
//! multi-user WebChat) rather than a construction-time-fixed owner. Before
//! the fix, any actor other than the group's canonical default actor failed
//! deterministically with `driver_unavailable` / "unknown thread".

use super::ironclaw_support::builder::IronClawIntegrationHarness;
use super::ironclaw_support::group::{HarnessResult, IronClawIntegrationGroup};
use super::ironclaw_support::reply::IronClawScriptedReply;

pub async fn run(g: &IronClawIntegrationGroup) -> HarnessResult<()> {
    // Thread A: the group's default actor (`HARNESS_ACTOR_ID`).
    let a = g
        .thread("conv-multiuser-a")
        .script([IronClawScriptedReply::text("reply-for-actor-a")])
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
        .script([IronClawScriptedReply::text("reply-for-actor-b")])
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

    // Isolation negative guards, SYMMETRIC both ways: neither actor's owner
    // scope may surface the other's reply or read the other's thread history.
    assert_cannot_read_other_actor(&a, "A", &b, "B", "reply-for-actor-b").await?;
    assert_cannot_read_other_actor(&b, "B", &a, "A", "reply-for-actor-a").await?;

    Ok(())
}

/// One direction of the owner-isolation negative guard: `reader`'s thread must
/// not surface `other`'s reply, and `other`'s thread must not be readable
/// through `reader`'s owner scope at all (each owner's records live under a
/// separate `/tenants/<tenant>/users/<user>/threads` subtree). Called once per
/// direction so the symmetric check can't silently de-sync.
async fn assert_cannot_read_other_actor(
    reader: &IronClawIntegrationHarness,
    reader_name: &str,
    other: &IronClawIntegrationHarness,
    other_name: &str,
    other_reply: &str,
) -> HarnessResult<()> {
    if reader.assert_reply_contains(other_reply).await.is_ok() {
        return Err(format!(
            "isolation failure: actor {reader_name}'s thread surfaced actor {other_name}'s reply"
        )
        .into());
    }
    if reader
        .thread_harness
        .history(other.binding.thread_id.clone())
        .await
        .is_ok()
    {
        return Err(format!(
            "isolation failure: actor {other_name}'s thread is readable under actor \
             {reader_name}'s owner scope"
        )
        .into());
    }
    Ok(())
}
