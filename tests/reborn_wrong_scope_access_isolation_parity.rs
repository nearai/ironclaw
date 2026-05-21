#[allow(dead_code)]
#[path = "support/reborn/mod.rs"]
mod reborn_support;
mod support;

use ironclaw_host_api::{TenantId, UserId};
use ironclaw_loop_support::HostManagedModelResponse;
use ironclaw_threads::ThreadScope;
use ironclaw_turns::{TurnActor, TurnScope, TurnStatus};
use reborn_support::{
    harness::{RebornBinaryE2EHarness, RecordingTestCapabilityPort, trace_tool_call_response},
    model_replay::RebornTraceReplayModelGateway,
};

#[tokio::test]
async fn reborn_wrong_scope_access_isolation_parity() {
    let model_gateway = RebornTraceReplayModelGateway::with_responses([
        trace_tool_call_response(),
        HostManagedModelResponse::assistant_reply("wrong scope must not resume this reply"),
    ]);
    let mut harness = RebornBinaryE2EHarness::with_harness_blocked_evidence(
        "room-wrong-scope-access",
        model_gateway,
        RecordingTestCapabilityPort::approval_then_echo(),
    )
    .await
    .expect("harness");
    harness.start();

    let submitted = harness
        .submit_text("event-wrong-scope-access", "needs approval")
        .await
        .expect("submit approval turn");
    let blocked = harness
        .wait_for_submitted_status(&submitted, TurnStatus::BlockedApproval)
        .await
        .expect("blocked approval");
    let gate_ref = blocked.gate_ref.expect("blocked run exposes gate ref");

    let wrong_turn_scope = wrong_tenant_turn_scope(&submitted.scope);
    assert!(
        harness
            .run_state_in_scope(wrong_turn_scope.clone(), submitted.run_id)
            .await
            .is_err(),
        "wrong tenant scope must not read run state or leak run existence"
    );

    let wrong_thread_scope = wrong_tenant_thread_scope(&submitted.thread_scope);
    assert!(
        harness
            .history_for_thread_in_scope(wrong_thread_scope, submitted.thread_id.clone())
            .await
            .is_err(),
        "wrong tenant scope must not access thread history"
    );

    assert!(
        harness
            .cancel_run_as(
                wrong_turn_scope.clone(),
                submitted.actor.clone(),
                submitted.run_id,
                format!("wrong-tenant-cancel-{}", submitted.run_id),
            )
            .await
            .is_err(),
        "wrong tenant scope must not cancel another tenant run"
    );
    assert_eq!(
        harness
            .run_state(submitted.run_id)
            .await
            .expect("state after wrong tenant cancel")
            .status,
        TurnStatus::BlockedApproval,
        "wrong tenant cancel must not mutate original run"
    );

    let wrong_actor = TurnActor::new(UserId::new("wrong-actor-e2e").expect("valid user id"));
    assert!(
        harness
            .resume_with_gate_as(
                submitted.scope.clone(),
                wrong_actor.clone(),
                submitted.run_id,
                gate_ref.clone(),
                format!("wrong-actor-resume-{}", submitted.run_id),
            )
            .await
            .is_err(),
        "wrong actor must not resume another user's blocked run"
    );
    assert_eq!(
        harness
            .run_state(submitted.run_id)
            .await
            .expect("state after wrong actor resume")
            .status,
        TurnStatus::BlockedApproval,
        "wrong actor resume must not mutate original run"
    );

    harness
        .cancel_blocked_turn(submitted.run_id)
        .await
        .expect("owner can still cancel after wrong-scope attempts");
    harness
        .wait_for_status(submitted.run_id, TurnStatus::Cancelled)
        .await
        .expect("owner cancel completes");

    harness.shutdown().await;
}

fn wrong_tenant_turn_scope(scope: &TurnScope) -> TurnScope {
    let mut wrong = scope.clone();
    wrong.tenant_id = TenantId::new("tenant-wrong-scope-e2e").expect("valid tenant id");
    wrong
}

fn wrong_tenant_thread_scope(scope: &ThreadScope) -> ThreadScope {
    let mut wrong = scope.clone();
    wrong.tenant_id = TenantId::new("tenant-wrong-scope-e2e").expect("valid tenant id");
    wrong
}
