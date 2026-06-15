use std::sync::Arc;

use chrono::{Duration as ChronoDuration, TimeZone, Utc};
use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_loop_support::EventPublishingTurnRunTransitionPort;
use ironclaw_turns::{
    AcceptedMessageRef, BlockedReason, DefaultTurnCoordinator, GateRef, IdempotencyKey,
    InMemoryTurnEventSink, InMemoryTurnStateStore, LoopCheckpointStateRef, ReplyTargetBindingRef,
    RunProfileRequest, SourceBindingRef, SubmitTurnRequest, SubmitTurnResponse, TurnActor,
    TurnCheckpointId, TurnCoordinator, TurnEventKind, TurnEventSink, TurnLeaseToken, TurnRunId,
    TurnRunnerId, TurnScope, TurnStatus,
    runner::{
        BlockRunRequest, ClaimRunRequest, CompleteRunRequest, RecoverExpiredLeasesRequest,
        TurnRunTransitionPort,
    },
};

fn scope(thread: &str) -> TurnScope {
    TurnScope::new(
        TenantId::new("tenant1").unwrap(),
        Some(AgentId::new("agent1").unwrap()),
        Some(ProjectId::new("project1").unwrap()),
        ThreadId::new(thread).unwrap(),
    )
}

fn actor() -> TurnActor {
    TurnActor::new(UserId::new("user1").unwrap())
}

fn submit_request(thread: &str, idempotency_key: &str) -> SubmitTurnRequest {
    SubmitTurnRequest {
        scope: scope(thread),
        actor: actor(),
        accepted_message_ref: AcceptedMessageRef::new(format!("message-{idempotency_key}"))
            .unwrap(),
        source_binding_ref: SourceBindingRef::new("source-web").unwrap(),
        reply_target_binding_ref: ReplyTargetBindingRef::new("reply-web").unwrap(),
        requested_run_profile: Some(RunProfileRequest::new("default").unwrap()),
        idempotency_key: IdempotencyKey::new(idempotency_key).unwrap(),
        received_at: Utc.with_ymd_and_hms(2026, 5, 5, 12, 0, 0).unwrap(),
        requested_run_id: None,
        parent_run_id: None,
        subagent_depth: 0,
        spawn_tree_root_run_id: None,
        product_context: None,
    }
}

fn accepted_run_id(response: &SubmitTurnResponse) -> TurnRunId {
    let SubmitTurnResponse::Accepted { run_id, .. } = response;
    *run_id
}

#[tokio::test]
async fn event_publishing_transition_port_publishes_blocked_and_terminal_events() {
    let store = Arc::new(InMemoryTurnStateStore::default());
    let coordinator = DefaultTurnCoordinator::new(store.clone());
    let sink = Arc::new(InMemoryTurnEventSink::default());
    let transition_port = EventPublishingTurnRunTransitionPort::new(
        Arc::clone(&store) as Arc<dyn TurnRunTransitionPort>,
        Arc::clone(&sink) as Arc<dyn TurnEventSink>,
    );

    let blocked_run_id = accepted_run_id(
        &coordinator
            .submit_turn(submit_request(
                "thread-transition-blocked",
                "idem-transition-blocked",
            ))
            .await
            .unwrap(),
    );
    let blocked_runner_id = TurnRunnerId::new();
    let blocked_lease = TurnLeaseToken::new();
    transition_port
        .claim_next_run(ClaimRunRequest {
            runner_id: blocked_runner_id,
            lease_token: blocked_lease,
            scope_filter: Some(scope("thread-transition-blocked")),
        })
        .await
        .unwrap()
        .unwrap();
    transition_port
        .block_run(BlockRunRequest {
            run_id: blocked_run_id,
            runner_id: blocked_runner_id,
            lease_token: blocked_lease,
            checkpoint_id: TurnCheckpointId::new(),
            state_ref: LoopCheckpointStateRef::new("checkpoint:block-state").unwrap(),
            reason: BlockedReason::AwaitDependentRun {
                gate_ref: GateRef::new("gate-dependent-run").unwrap(),
            },
        })
        .await
        .unwrap();

    let completed_run_id = accepted_run_id(
        &coordinator
            .submit_turn(submit_request(
                "thread-transition-completed",
                "idem-transition-completed",
            ))
            .await
            .unwrap(),
    );
    let completed_runner_id = TurnRunnerId::new();
    let completed_lease = TurnLeaseToken::new();
    transition_port
        .claim_next_run(ClaimRunRequest {
            runner_id: completed_runner_id,
            lease_token: completed_lease,
            scope_filter: Some(scope("thread-transition-completed")),
        })
        .await
        .unwrap()
        .unwrap();
    transition_port
        .complete_run(CompleteRunRequest {
            run_id: completed_run_id,
            runner_id: completed_runner_id,
            lease_token: completed_lease,
        })
        .await
        .unwrap();

    let events = sink.events();
    assert!(events.iter().any(|event| {
        event.run_id == blocked_run_id
            && event.kind == TurnEventKind::Blocked
            && event.status == TurnStatus::BlockedDependentRun
    }));
    assert!(events.iter().any(|event| {
        event.run_id == completed_run_id
            && event.kind == TurnEventKind::Completed
            && event.status == TurnStatus::Completed
    }));
}

#[tokio::test]
async fn event_publishing_transition_port_publishes_expired_lease_terminal_events() {
    let store = Arc::new(InMemoryTurnStateStore::default());
    let coordinator = DefaultTurnCoordinator::new(store.clone());
    let sink = Arc::new(InMemoryTurnEventSink::default());
    let transition_port = EventPublishingTurnRunTransitionPort::new(
        Arc::clone(&store) as Arc<dyn TurnRunTransitionPort>,
        Arc::clone(&sink) as Arc<dyn TurnEventSink>,
    );

    let empty = transition_port
        .recover_expired_leases(RecoverExpiredLeasesRequest {
            now: Utc::now() + ChronoDuration::seconds(120),
            scope_filter: None,
        })
        .await
        .unwrap();
    assert!(empty.recovered.is_empty());
    assert!(sink.events().is_empty());

    let first = accepted_run_id(
        &coordinator
            .submit_turn(submit_request(
                "thread-recover-event-a",
                "idem-recover-event-a",
            ))
            .await
            .unwrap(),
    );
    let second = accepted_run_id(
        &coordinator
            .submit_turn(submit_request(
                "thread-recover-event-b",
                "idem-recover-event-b",
            ))
            .await
            .unwrap(),
    );
    for scope_filter in [
        Some(scope("thread-recover-event-a")),
        Some(scope("thread-recover-event-b")),
    ] {
        transition_port
            .claim_next_run(ClaimRunRequest {
                runner_id: TurnRunnerId::new(),
                lease_token: TurnLeaseToken::new(),
                scope_filter,
            })
            .await
            .unwrap()
            .unwrap();
    }

    let recovered = transition_port
        .recover_expired_leases(RecoverExpiredLeasesRequest {
            now: Utc::now() + ChronoDuration::seconds(120),
            scope_filter: None,
        })
        .await
        .unwrap();
    assert_eq!(recovered.recovered.len(), 2);
    let events = sink.events();
    let recovered_events = events
        .iter()
        .filter(|event| {
            event.kind == TurnEventKind::Failed
                && event.sanitized_reason.as_deref() == Some("lease_expired")
        })
        .collect::<Vec<_>>();
    assert_eq!(recovered_events.len(), 2);
    assert!(recovered_events.iter().any(|event| event.run_id == first));
    assert!(recovered_events.iter().any(|event| event.run_id == second));
}
