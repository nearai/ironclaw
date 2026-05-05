use std::sync::Arc;

use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_turns::{
    AcceptedMessageRef, AdmissionRejection, BlockedReason, CancelRunRequest,
    DefaultTurnCoordinator, GateRef, GetRunStateRequest, IdempotencyKey, InMemoryTurnEventSink,
    InMemoryTurnStateStore, ReplyTargetBindingRef, ResumeTurnRequest, SanitizedCancelReason,
    SanitizedFailure, SourceBindingRef, SubmitTurnRequest, SubmitTurnResponse, ThreadBusy,
    TurnActor, TurnAdmissionPolicy, TurnCheckpointId, TurnCoordinator, TurnError, TurnEventKind,
    TurnEventSink, TurnLeaseToken, TurnLifecycleEvent, TurnRunId, TurnRunProfile, TurnRunnerId,
    TurnScope, TurnStatus,
    events::EventCursor,
    runner::{
        BlockRunRequest, ClaimRunRequest, CompleteRunRequest, FailRunRequest, HeartbeatRequest,
        TurnRunTransitionPort,
    },
};

#[tokio::test]
async fn submit_turn_accepts_only_canonical_refs_and_returns_redacted_metadata() {
    let (coordinator, _store) = coordinator();
    let request = submit_request("thread-a", "idem-submit-a");

    let response = coordinator.submit_turn(request.clone()).await.unwrap();

    let SubmitTurnResponse::Accepted {
        turn_id: _,
        run_id,
        status,
        profile,
        event_cursor,
        reply_target_binding_ref,
    } = response;
    assert_eq!(status, TurnStatus::Queued);
    assert_eq!(profile.profile_name, "default");
    assert_eq!(reply_target_binding_ref, request.reply_target_binding_ref);
    assert_eq!(event_cursor.0, 1);

    let state = coordinator
        .get_run_state(GetRunStateRequest {
            scope: request.scope,
            run_id,
        })
        .await
        .unwrap();
    assert_eq!(state.status, TurnStatus::Queued);
    assert_eq!(state.source_binding_ref.as_str(), "source-web");
    assert_eq!(state.reply_target_binding_ref.as_str(), "reply-web");
    assert_eq!(state.failure, None);
}

#[tokio::test]
async fn same_thread_active_run_returns_busy_but_different_threads_run_independently() {
    let (coordinator, _store) = coordinator();
    let first = coordinator
        .submit_turn(submit_request("thread-a", "idem-submit-a"))
        .await
        .unwrap();
    let first_run_id = accepted_run_id(&first);

    let busy = coordinator
        .submit_turn(submit_request("thread-a", "idem-submit-b"))
        .await
        .unwrap_err();
    assert!(matches!(
        busy,
        TurnError::ThreadBusy(ThreadBusy {
            active_run_id,
            status: TurnStatus::Queued,
            ..
        }) if active_run_id == first_run_id
    ));

    let independent = coordinator
        .submit_turn(submit_request("thread-b", "idem-submit-c"))
        .await
        .unwrap();
    assert_ne!(accepted_run_id(&independent), first_run_id);
}

#[tokio::test]
async fn submit_turn_idempotency_replays_same_success_result() {
    let (coordinator, _store) = coordinator();
    let first = coordinator
        .submit_turn(submit_request("thread-a", "idem-submit-a"))
        .await
        .unwrap();
    let duplicate = coordinator
        .submit_turn(submit_request("thread-a", "idem-submit-a"))
        .await
        .unwrap();
    assert_eq!(duplicate, first);
}

#[tokio::test]
async fn transient_busy_submit_is_not_cached_after_thread_unlocks() {
    let (coordinator, store) = coordinator();
    let first_run_id = accepted_run_id(
        &coordinator
            .submit_turn(submit_request("thread-a", "idem-submit-a"))
            .await
            .unwrap(),
    );
    let busy_request = submit_request("thread-a", "idem-submit-b");
    assert!(matches!(
        coordinator
            .submit_turn(busy_request.clone())
            .await
            .unwrap_err(),
        TurnError::ThreadBusy(_)
    ));

    let runner_id = TurnRunnerId::new();
    let lease_token = TurnLeaseToken::new();
    store
        .claim_next_run(ClaimRunRequest {
            runner_id,
            lease_token,
            scope_filter: None,
        })
        .await
        .unwrap()
        .unwrap();
    store
        .complete_run(CompleteRunRequest {
            run_id: first_run_id,
            runner_id,
            lease_token,
        })
        .await
        .unwrap();

    let accepted_after_unlock = coordinator.submit_turn(busy_request).await.unwrap();
    assert_ne!(accepted_run_id(&accepted_after_unlock), first_run_id);
}

#[tokio::test]
async fn submit_turn_idempotency_is_scoped_to_canonical_thread() {
    let (coordinator, _store) = coordinator();
    let first = coordinator
        .submit_turn(submit_request("thread-a", "shared-idempotency-key"))
        .await
        .unwrap();

    let second = coordinator
        .submit_turn(submit_request("thread-b", "shared-idempotency-key"))
        .await
        .unwrap();

    assert_ne!(accepted_run_id(&second), accepted_run_id(&first));
}

#[tokio::test]
async fn get_run_state_wrong_scope_returns_not_found_without_leaking_existence() {
    let (coordinator, _store) = coordinator();
    let response = coordinator
        .submit_turn(submit_request("thread-a", "idem-submit-a"))
        .await
        .unwrap();
    let run_id = accepted_run_id(&response);

    let err = coordinator
        .get_run_state(GetRunStateRequest {
            scope: scope("thread-other"),
            run_id,
        })
        .await
        .unwrap_err();

    assert_eq!(err, TurnError::NotFound);
}

#[tokio::test]
async fn admission_policy_rejection_is_typed_and_does_not_create_run() {
    let store = Arc::new(InMemoryTurnStateStore::default());
    let coordinator =
        DefaultTurnCoordinator::new(store.clone()).with_admission_policy(Arc::new(DenyAll));
    let request = submit_request("thread-a", "idem-submit-a");

    let err = coordinator.submit_turn(request.clone()).await.unwrap_err();

    assert_eq!(
        err,
        TurnError::AdmissionRejected(AdmissionRejection::new("tenant_limit"))
    );
    assert!(store.events().is_empty());
    assert_eq!(
        coordinator
            .get_run_state(GetRunStateRequest {
                scope: request.scope,
                run_id: TurnRunId::new(),
            })
            .await
            .unwrap_err(),
        TurnError::NotFound
    );
}

#[tokio::test]
async fn runner_claims_queued_run_with_lease_and_heartbeat_requires_matching_lease() {
    let (coordinator, store) = coordinator();
    let run_id = accepted_run_id(
        &coordinator
            .submit_turn(submit_request("thread-a", "idem-submit-a"))
            .await
            .unwrap(),
    );
    let runner_id = TurnRunnerId::new();
    let lease_token = TurnLeaseToken::new();

    let claimed = store
        .claim_next_run(ClaimRunRequest {
            runner_id,
            lease_token,
            scope_filter: Some(scope("thread-a")),
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(claimed.state.run_id, run_id);
    assert_eq!(claimed.state.status, TurnStatus::Running);

    let stale = store
        .heartbeat(HeartbeatRequest {
            run_id,
            runner_id,
            lease_token: TurnLeaseToken::new(),
        })
        .await
        .unwrap_err();
    assert_eq!(stale, TurnError::LeaseMismatch);

    let cursor = store
        .heartbeat(HeartbeatRequest {
            run_id,
            runner_id,
            lease_token,
        })
        .await
        .unwrap();
    assert!(cursor.0 >= 3);
}

#[tokio::test]
async fn blocked_run_persists_checkpoint_and_keeps_same_thread_lock_until_resume() {
    let (coordinator, store) = coordinator();
    let run_id = accepted_run_id(
        &coordinator
            .submit_turn(submit_request("thread-a", "idem-submit-a"))
            .await
            .unwrap(),
    );
    let runner_id = TurnRunnerId::new();
    let lease_token = TurnLeaseToken::new();
    store
        .claim_next_run(ClaimRunRequest {
            runner_id,
            lease_token,
            scope_filter: None,
        })
        .await
        .unwrap()
        .unwrap();
    let checkpoint_id = TurnCheckpointId::new();
    let gate_ref = GateRef::new("approval-gate").unwrap();

    let blocked = store
        .block_run(BlockRunRequest {
            run_id,
            runner_id,
            lease_token,
            checkpoint_id,
            reason: BlockedReason::Approval {
                gate_ref: gate_ref.clone(),
            },
        })
        .await
        .unwrap();
    assert_eq!(blocked.status, TurnStatus::BlockedApproval);
    assert_eq!(blocked.checkpoint_id, Some(checkpoint_id));
    assert_eq!(blocked.gate_ref, Some(gate_ref.clone()));

    let busy = coordinator
        .submit_turn(submit_request("thread-a", "idem-submit-b"))
        .await
        .unwrap_err();
    assert!(matches!(busy, TurnError::ThreadBusy(_)));

    let resumed = coordinator
        .resume_turn(ResumeTurnRequest {
            scope: scope("thread-a"),
            actor: actor(),
            run_id,
            gate_resolution_ref: gate_ref,
            idempotency_key: IdempotencyKey::new("idem-resume-a").unwrap(),
        })
        .await
        .unwrap();
    assert_eq!(resumed.status, TurnStatus::Queued);
}

#[tokio::test]
async fn cancel_run_is_idempotent_and_marks_running_run_cancel_requested_without_releasing_lock() {
    let (coordinator, store) = coordinator();
    let run_id = accepted_run_id(
        &coordinator
            .submit_turn(submit_request("thread-a", "idem-submit-a"))
            .await
            .unwrap(),
    );
    store
        .claim_next_run(ClaimRunRequest {
            runner_id: TurnRunnerId::new(),
            lease_token: TurnLeaseToken::new(),
            scope_filter: None,
        })
        .await
        .unwrap()
        .unwrap();
    let cancel = cancel_request("thread-a", run_id, "idem-cancel-a");

    let first = coordinator.cancel_run(cancel.clone()).await.unwrap();
    let duplicate = coordinator.cancel_run(cancel).await.unwrap();
    assert_eq!(duplicate, first);
    assert_eq!(first.status, TurnStatus::CancelRequested);

    let busy = coordinator
        .submit_turn(submit_request("thread-a", "idem-submit-b"))
        .await
        .unwrap_err();
    assert!(matches!(busy, TurnError::ThreadBusy(_)));
}

#[tokio::test]
async fn cancel_run_for_queued_run_terminally_cancels_and_releases_lock() {
    let (coordinator, store) = coordinator();
    let run_id = accepted_run_id(
        &coordinator
            .submit_turn(submit_request("thread-a", "idem-submit-a"))
            .await
            .unwrap(),
    );

    let cancelled = coordinator
        .cancel_run(cancel_request("thread-a", run_id, "idem-cancel-a"))
        .await
        .unwrap();
    assert_eq!(cancelled.status, TurnStatus::Cancelled);
    assert_eq!(
        store.events().last().unwrap().sanitized_reason.as_deref(),
        Some("user_requested")
    );

    let next = coordinator
        .submit_turn(submit_request("thread-a", "idem-submit-b"))
        .await
        .unwrap();
    assert_ne!(accepted_run_id(&next), run_id);
}

#[tokio::test]
async fn cancelled_running_run_cannot_be_reopened_as_blocked() {
    let (coordinator, store) = coordinator();
    let run_id = accepted_run_id(
        &coordinator
            .submit_turn(submit_request("thread-a", "idem-submit-a"))
            .await
            .unwrap(),
    );
    let runner_id = TurnRunnerId::new();
    let lease_token = TurnLeaseToken::new();
    store
        .claim_next_run(ClaimRunRequest {
            runner_id,
            lease_token,
            scope_filter: None,
        })
        .await
        .unwrap()
        .unwrap();
    coordinator
        .cancel_run(cancel_request("thread-a", run_id, "idem-cancel-a"))
        .await
        .unwrap();

    let blocked_after_cancel = store
        .block_run(BlockRunRequest {
            run_id,
            runner_id,
            lease_token,
            checkpoint_id: TurnCheckpointId::new(),
            reason: BlockedReason::Approval {
                gate_ref: GateRef::new("approval-gate").unwrap(),
            },
        })
        .await
        .unwrap_err();
    assert_eq!(
        blocked_after_cancel,
        TurnError::InvalidTransition {
            from: TurnStatus::CancelRequested,
            to: TurnStatus::BlockedApproval,
        }
    );

    let state = coordinator
        .get_run_state(GetRunStateRequest {
            scope: scope("thread-a"),
            run_id,
        })
        .await
        .unwrap();
    assert_eq!(state.status, TurnStatus::CancelRequested);
}

#[tokio::test]
async fn sanitized_failure_rejects_empty_controlled_or_oversized_categories() {
    assert!(SanitizedFailure::new("policy").is_ok());
    assert!(SanitizedFailure::new("").is_err());
    assert!(SanitizedFailure::new("backend\nsecret=leaked").is_err());
    assert!(SanitizedFailure::new("x".repeat(257)).is_err());
}

#[test]
fn bounded_refs_validate_during_deserialization() {
    assert!(serde_json::from_str::<AcceptedMessageRef>("\"message-ok\"").is_ok());
    assert!(serde_json::from_str::<AcceptedMessageRef>("\"\"").is_err());
    assert!(serde_json::from_str::<SourceBindingRef>("\"source\\nsecret\"").is_err());
    let oversized = format!("\"{}\"", "x".repeat(257));
    assert!(serde_json::from_str::<GateRef>(&oversized).is_err());
}

#[test]
fn sanitized_failure_validates_during_deserialization() {
    let failure = serde_json::from_str::<SanitizedFailure>("{\"category\":\"policy\"}").unwrap();
    assert_eq!(failure.category(), "policy");
    assert!(serde_json::from_str::<SanitizedFailure>("{\"category\":\"\"}").is_err());
    assert!(
        serde_json::from_str::<SanitizedFailure>("{\"category\":\"backend\\nsecret\"}").is_err()
    );
}

#[tokio::test]
async fn in_memory_event_sink_retains_a_bounded_tail() {
    let sink = InMemoryTurnEventSink::default();
    for cursor in 1..=10_001 {
        sink.publish(TurnLifecycleEvent {
            cursor: EventCursor(cursor),
            scope: scope("thread-a"),
            run_id: TurnRunId::new(),
            status: TurnStatus::Queued,
            kind: TurnEventKind::Submitted,
            sanitized_reason: None,
        })
        .await
        .unwrap();
    }

    let events = sink.events();
    assert_eq!(events.len(), 10_000);
    assert_eq!(events.first().unwrap().cursor, EventCursor(2));
    assert_eq!(events.last().unwrap().cursor, EventCursor(10_001));
}

#[tokio::test]
async fn terminal_runner_outcome_releases_lock_exactly_once() {
    let (coordinator, store) = coordinator();
    let run_id = accepted_run_id(
        &coordinator
            .submit_turn(submit_request("thread-a", "idem-submit-a"))
            .await
            .unwrap(),
    );
    let runner_id = TurnRunnerId::new();
    let lease_token = TurnLeaseToken::new();
    store
        .claim_next_run(ClaimRunRequest {
            runner_id,
            lease_token,
            scope_filter: None,
        })
        .await
        .unwrap()
        .unwrap();

    let completed = store
        .complete_run(CompleteRunRequest {
            run_id,
            runner_id,
            lease_token,
        })
        .await
        .unwrap();
    assert_eq!(completed.status, TurnStatus::Completed);

    let next = coordinator
        .submit_turn(submit_request("thread-a", "idem-submit-b"))
        .await
        .unwrap();
    assert_ne!(accepted_run_id(&next), run_id);

    let stale = store
        .fail_run(FailRunRequest {
            run_id,
            runner_id,
            lease_token,
            failure: SanitizedFailure::new("late_failure").unwrap(),
        })
        .await
        .unwrap_err();
    assert_eq!(stale, TurnError::LeaseMismatch);
}

fn coordinator() -> (
    DefaultTurnCoordinator<InMemoryTurnStateStore>,
    Arc<InMemoryTurnStateStore>,
) {
    let store = Arc::new(InMemoryTurnStateStore::default());
    (DefaultTurnCoordinator::new(store.clone()), store)
}

fn submit_request(thread: &str, idempotency_key: &str) -> SubmitTurnRequest {
    SubmitTurnRequest {
        scope: scope(thread),
        actor: actor(),
        accepted_message_ref: AcceptedMessageRef::new(format!("message-{thread}")).unwrap(),
        source_binding_ref: SourceBindingRef::new("source-web").unwrap(),
        reply_target_binding_ref: ReplyTargetBindingRef::new("reply-web").unwrap(),
        profile: TurnRunProfile::new("default"),
        idempotency_key: IdempotencyKey::new(idempotency_key).unwrap(),
    }
}

fn cancel_request(thread: &str, run_id: TurnRunId, idempotency_key: &str) -> CancelRunRequest {
    CancelRunRequest {
        scope: scope(thread),
        actor: actor(),
        run_id,
        reason: SanitizedCancelReason::UserRequested,
        idempotency_key: IdempotencyKey::new(idempotency_key).unwrap(),
    }
}

fn accepted_run_id(response: &SubmitTurnResponse) -> TurnRunId {
    let SubmitTurnResponse::Accepted { run_id, .. } = response;
    *run_id
}

fn scope(thread: &str) -> TurnScope {
    TurnScope::new(
        TenantId::new("tenant1").unwrap(),
        AgentId::new("agent1").unwrap(),
        Some(ProjectId::new("project1").unwrap()),
        ThreadId::new(thread).unwrap(),
    )
}

fn actor() -> TurnActor {
    TurnActor::new(UserId::new("user1").unwrap())
}

struct DenyAll;

impl TurnAdmissionPolicy for DenyAll {
    fn check_submit(&self, _request: &SubmitTurnRequest) -> Result<(), AdmissionRejection> {
        Err(AdmissionRejection::new("tenant_limit"))
    }
}
