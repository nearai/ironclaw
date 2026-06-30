//! E-MULTIUSER seam smoke test: `RebornThreadBuilder::with_actor_id` threads a
//! per-thread actor id through binding resolution (the probe) AND turn
//! submission, so distinct actors resolve to distinct bindings.
//!
//! Three assertions, each catches one wiring point:
//! - distinct `actor_user_id` per actor (`assert_submitted_actor_user_id` inside
//!   `submit_turn`) catches the SUBMIT wiring: if `submit_turn_async` reverted to
//!   `HARNESS_ACTOR_ID` the turn scope mismatches the binding scope and the
//!   assertion fails before the status wait;
//! - the actor-B reply landing in actor-B's own thread catches routing after submit;
//! - distinct `subject_user_id` + `actor_user_id` per actor catch the PROBE wiring
//!   (if the probe reverted to the constant, both actors would hash to the same ids).
//!
//! This is binding-scope isolation only; capability-scope isolation
//! (per-actor memory/approvals/projects) is a C-MULTIUSER concern.

#[allow(dead_code)]
#[path = "support/reborn/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
mod support;

use reborn_support::group::RebornIntegrationGroup;
use reborn_support::reply::RebornScriptedReply;

#[tokio::test]
async fn distinct_actor_ids_resolve_to_distinct_bindings() {
    let group = RebornIntegrationGroup::builtin_tools()
        .await
        .expect("group builds");

    // Thread A: the default actor.
    let harness_a = group
        .thread("conv-multiuser-a")
        .script([RebornScriptedReply::text("reply-a")])
        .build()
        .await
        .expect("thread a builds");
    harness_a.submit_turn("hi from a").await.expect("a completes");
    harness_a
        .assert_reply_contains("reply-a")
        .await
        .expect("a's reply lands in a's thread");
    let subject_a = harness_a.binding.subject_user_id.clone();

    // Thread B: a distinct actor id.
    let harness_b = group
        .thread("conv-multiuser-b")
        .with_actor_id("reborn-actor-b")
        .script([RebornScriptedReply::text("reply-b")])
        .build()
        .await
        .expect("thread b builds");
    // `submit_turn` internally calls `assert_submitted_actor_user_id` before the
    // status wait — this line fails if `submit_turn_async` regressed to
    // `HARNESS_ACTOR_ID`, because the submitted scope would mismatch harness-b's
    // binding scope (wrong actor_user_id → wrong thread_owner → ScopeNotFound).
    harness_b.submit_turn("hi from b").await.expect("b completes");
    harness_b
        .assert_reply_contains("reply-b")
        .await
        .expect("b's reply lands in b's thread");
    let subject_b = harness_b.binding.subject_user_id.clone();

    // Catches the probe-wiring mutation: distinct actors → distinct subject users.
    assert_ne!(
        subject_a, subject_b,
        "distinct actor ids must resolve to distinct subject user ids"
    );
    // Catches the probe-wiring mutation at the actor_user_id level: distinct
    // actors must hash to distinct actor_user_ids via `user_id_for_binding`.
    assert_ne!(
        harness_a.binding.actor_user_id,
        harness_b.binding.actor_user_id,
        "distinct actor ids must resolve to distinct actor_user_ids"
    );
}
