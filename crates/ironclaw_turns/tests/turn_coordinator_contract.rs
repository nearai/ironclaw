use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    time::Duration,
};

use chrono::{DateTime, TimeZone, Utc};
use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_turns::{
    AcceptedMessageRef, AdmissionRejection, AdmissionRejectionReason, BlockedReason,
    CancelRunRequest, DefaultTurnCoordinator, GateRef, GetRunStateRequest, IdempotencyKey,
    InMemoryTurnEventSink, InMemoryTurnStateStore, InMemoryTurnStateStoreLimits,
    ReplyTargetBindingRef, ResumeTurnRequest, RunProfileRequest, RunProfileVersion,
    SanitizedCancelReason, SanitizedFailure, SourceBindingRef, SubmitTurnRequest,
    SubmitTurnResponse, ThreadBusy, TurnActor, TurnAdmissionPolicy, TurnCheckpointId,
    TurnCoordinator, TurnError, TurnErrorCategory, TurnEventKind, TurnEventSink,
    TurnIdempotencyOperationKind, TurnIdempotencyOutcomeKind, TurnLeaseToken, TurnLifecycleEvent,
    TurnLockVersion, TurnRunId, TurnRunnerId, TurnScope, TurnStatus,
    events::EventCursor,
    runner::{
        BlockRunRequest, CancelRunCompletionRequest, ClaimRunRequest, CompleteRunRequest,
        FailRunRequest, HeartbeatRequest, TurnRunTransitionPort,
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
        resolved_run_profile_id,
        resolved_run_profile_version,
        event_cursor,
        accepted_message_ref,
        reply_target_binding_ref,
    } = response;
    assert_eq!(status, TurnStatus::Queued);
    assert_eq!(resolved_run_profile_id.as_str(), "default");
    assert_eq!(resolved_run_profile_version, RunProfileVersion::new(1));
    assert_eq!(accepted_message_ref, request.accepted_message_ref);
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
    assert_eq!(state.accepted_message_ref.as_str(), "message-thread-a");
    assert_eq!(state.source_binding_ref.as_str(), "source-web");
    assert_eq!(state.reply_target_binding_ref.as_str(), "reply-web");
    assert_eq!(state.resolved_run_profile_id.as_str(), "default");
    assert_eq!(
        state.resolved_run_profile_version,
        RunProfileVersion::new(1)
    );
    assert_eq!(state.received_at, received_at());
    assert_eq!(state.failure, None);
}

#[tokio::test]
async fn requested_run_profile_is_a_hint_not_resolved_authority() {
    let (coordinator, _store) = coordinator();
    let mut request = submit_request("thread-a", "idem-profile-hint");
    request.requested_run_profile = Some(RunProfileRequest::new("experimental-fast-lane").unwrap());

    let response = coordinator.submit_turn(request.clone()).await.unwrap();

    let SubmitTurnResponse::Accepted {
        run_id,
        resolved_run_profile_id,
        resolved_run_profile_version,
        ..
    } = response;
    assert_eq!(resolved_run_profile_id.as_str(), "default");
    assert_eq!(resolved_run_profile_version, RunProfileVersion::new(1));

    let state = coordinator
        .get_run_state(GetRunStateRequest {
            scope: request.scope,
            run_id,
        })
        .await
        .unwrap();
    assert_eq!(state.resolved_run_profile_id.as_str(), "default");
    assert_eq!(
        state.resolved_run_profile_version,
        RunProfileVersion::new(1)
    );
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
            event_cursor: EventCursor(1),
        }) if active_run_id == first_run_id
    ));

    let independent = coordinator
        .submit_turn(submit_request("thread-b", "idem-submit-c"))
        .await
        .unwrap();
    assert_ne!(accepted_run_id(&independent), first_run_id);
}

#[tokio::test]
async fn submit_turn_persistence_snapshot_has_atomic_success_artifacts() {
    let (coordinator, store) = coordinator();
    let request = submit_request("thread-a", "idem-submit-a");

    let response = coordinator.submit_turn(request.clone()).await.unwrap();
    let SubmitTurnResponse::Accepted {
        turn_id,
        run_id,
        status,
        event_cursor,
        ..
    } = response;

    let snapshot = store.persistence_snapshot();
    assert_eq!(snapshot.turns.len(), 1);
    assert_eq!(snapshot.runs.len(), 1);
    assert_eq!(snapshot.active_locks.len(), 1);
    assert_eq!(snapshot.checkpoints.len(), 0);
    assert_eq!(snapshot.idempotency_records.len(), 1);

    let turn = &snapshot.turns[0];
    assert_eq!(turn.turn_id, turn_id);
    assert_eq!(turn.scope, request.scope);
    assert_eq!(turn.actor, request.actor);
    assert_eq!(turn.accepted_message_ref, request.accepted_message_ref);
    assert_eq!(turn.source_binding_ref, request.source_binding_ref);
    assert_eq!(
        turn.reply_target_binding_ref,
        request.reply_target_binding_ref
    );
    assert_eq!(turn.created_at, request.received_at);

    let run = &snapshot.runs[0];
    assert_eq!(run.run_id, run_id);
    assert_eq!(run.turn_id, turn_id);
    assert_eq!(run.status, status);
    assert_eq!(run.event_cursor, event_cursor);
    assert_eq!(run.claim_count, 0);
    assert_eq!(run.runner_id, None);
    assert_eq!(run.lease_token, None);

    let lock = &snapshot.active_locks[0];
    assert_eq!(lock.key.scope, request.scope);
    assert_eq!(lock.run_id, run_id);
    assert_eq!(lock.status, TurnStatus::Queued);
    assert_eq!(lock.lock_version, TurnLockVersion::new(1));
    assert_eq!(lock.acquired_at, request.received_at);
    assert_eq!(lock.updated_at, request.received_at);

    let idempotency = &snapshot.idempotency_records[0];
    assert_eq!(idempotency.scope, request.scope);
    assert_eq!(idempotency.operation, TurnIdempotencyOperationKind::Submit);
    assert_eq!(idempotency.key, request.idempotency_key);
    assert_eq!(idempotency.turn_id, Some(turn_id));
    assert_eq!(idempotency.run_id, Some(run_id));
    assert_eq!(idempotency.outcome, TurnIdempotencyOutcomeKind::Accepted);
    assert_eq!(idempotency.created_at, request.received_at);
}

#[tokio::test]
async fn same_thread_lock_excludes_actor_identity() {
    let (coordinator, _store) = coordinator();
    let first = coordinator
        .submit_turn(submit_request("thread-a", "idem-submit-a"))
        .await
        .unwrap();
    let first_run_id = accepted_run_id(&first);
    let mut second_actor = submit_request("thread-a", "idem-submit-b");
    second_actor.actor = TurnActor::new(UserId::new("user2").unwrap());

    let busy = coordinator.submit_turn(second_actor).await.unwrap_err();

    assert!(matches!(
        busy,
        TurnError::ThreadBusy(ThreadBusy { active_run_id, .. }) if active_run_id == first_run_id
    ));
}

#[tokio::test]
async fn submit_turn_busy_path_records_idempotent_thread_busy_without_new_run() {
    let (coordinator, store) = coordinator();
    let first_run_id = accepted_run_id(
        &coordinator
            .submit_turn(submit_request("thread-a", "idem-submit-a"))
            .await
            .unwrap(),
    );
    let busy_request = submit_request("thread-a", "idem-submit-b");

    let busy = coordinator
        .submit_turn(busy_request.clone())
        .await
        .unwrap_err();

    let snapshot = store.persistence_snapshot();
    assert_eq!(snapshot.turns.len(), 1);
    assert_eq!(snapshot.runs.len(), 1);
    let busy_idempotency = snapshot
        .idempotency_records
        .iter()
        .find(|record| record.key == busy_request.idempotency_key)
        .expect("busy submit idempotency result must be recorded");
    assert_eq!(
        busy_idempotency.operation,
        TurnIdempotencyOperationKind::Submit
    );
    assert_eq!(busy_idempotency.turn_id, None);
    assert_eq!(busy_idempotency.run_id, Some(first_run_id));
    assert_eq!(
        busy_idempotency.outcome,
        TurnIdempotencyOutcomeKind::ThreadBusy
    );
    assert_eq!(busy_idempotency.created_at, busy_request.received_at);

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

    let duplicate = coordinator.submit_turn(busy_request).await.unwrap_err();
    assert_eq!(duplicate, busy);
    assert_eq!(store.persistence_snapshot().turns.len(), 1);
}

#[tokio::test]
async fn runner_claim_and_block_update_persistent_run_lock_and_checkpoint_records() {
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

    let claimed = store.persistence_snapshot();
    let run = claimed
        .runs
        .iter()
        .find(|record| record.run_id == run_id)
        .unwrap();
    assert_eq!(run.status, TurnStatus::Running);
    assert_eq!(run.runner_id, Some(runner_id));
    assert_eq!(run.lease_token, Some(lease_token));
    assert_eq!(run.claim_count, 1);
    let lock = claimed
        .active_locks
        .iter()
        .find(|lock| lock.run_id == run_id)
        .unwrap();
    assert_eq!(lock.status, TurnStatus::Running);
    assert_eq!(lock.lock_version, TurnLockVersion::new(2));

    let checkpoint_id = TurnCheckpointId::new();
    let gate_ref = GateRef::new("approval-gate").unwrap();
    store
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

    let blocked = store.persistence_snapshot();
    let run = blocked
        .runs
        .iter()
        .find(|record| record.run_id == run_id)
        .unwrap();
    assert_eq!(run.status, TurnStatus::BlockedApproval);
    assert_eq!(run.checkpoint_id, Some(checkpoint_id));
    assert_eq!(run.gate_ref, Some(gate_ref.clone()));
    assert_eq!(run.runner_id, None);
    assert_eq!(run.lease_token, None);
    let lock = blocked
        .active_locks
        .iter()
        .find(|lock| lock.run_id == run_id)
        .unwrap();
    assert_eq!(lock.status, TurnStatus::BlockedApproval);
    assert_eq!(lock.lock_version, TurnLockVersion::new(3));
    assert_eq!(blocked.checkpoints.len(), 1);
    let checkpoint = &blocked.checkpoints[0];
    assert_eq!(checkpoint.checkpoint_id, checkpoint_id);
    assert_eq!(checkpoint.run_id, run_id);
    assert_eq!(checkpoint.sequence, 1);
    assert_eq!(checkpoint.gate_ref, gate_ref);
}

#[tokio::test]
async fn resume_updates_persisted_run_binding_refs_and_replay_envelope() {
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
    let gate_ref = GateRef::new("approval-gate").unwrap();
    store
        .block_run(BlockRunRequest {
            run_id,
            runner_id,
            lease_token,
            checkpoint_id: TurnCheckpointId::new(),
            reason: BlockedReason::Approval {
                gate_ref: gate_ref.clone(),
            },
        })
        .await
        .unwrap();
    let resume_request = ResumeTurnRequest {
        scope: scope("thread-a"),
        actor: actor(),
        run_id,
        gate_resolution_ref: gate_ref,
        source_binding_ref: SourceBindingRef::new("source-web-resumed").unwrap(),
        reply_target_binding_ref: ReplyTargetBindingRef::new("reply-web-resumed").unwrap(),
        idempotency_key: IdempotencyKey::new("idem-resume-a").unwrap(),
    };

    let resumed = coordinator
        .resume_turn(resume_request.clone())
        .await
        .unwrap();

    let snapshot = store.persistence_snapshot();
    let run = snapshot
        .runs
        .iter()
        .find(|record| record.run_id == run_id)
        .unwrap();
    assert_eq!(run.source_binding_ref, resume_request.source_binding_ref);
    assert_eq!(
        run.reply_target_binding_ref,
        resume_request.reply_target_binding_ref
    );
    let replay_record = snapshot
        .idempotency_records
        .iter()
        .find(|record| record.key == resume_request.idempotency_key)
        .expect("resume idempotency record must be persisted");
    assert_eq!(replay_record.replay_resume().unwrap(), Ok(resumed));
}

#[tokio::test]
async fn persisted_submit_busy_and_cancel_replay_envelopes_are_reconstructable() {
    let (coordinator, store) = coordinator();
    let first_run_id = accepted_run_id(
        &coordinator
            .submit_turn(submit_request("thread-a", "idem-submit-a"))
            .await
            .unwrap(),
    );
    let busy_request = submit_request("thread-a", "idem-submit-b");
    let busy = coordinator
        .submit_turn(busy_request.clone())
        .await
        .unwrap_err();
    let cancel = coordinator
        .cancel_run(cancel_request(
            "thread-a",
            first_run_id,
            "idem-cancel-running-a",
        ))
        .await
        .unwrap();

    let snapshot = store.persistence_snapshot();
    let busy_record = snapshot
        .idempotency_records
        .iter()
        .find(|record| record.key == busy_request.idempotency_key)
        .expect("busy submit idempotency record must be persisted");
    assert_eq!(busy_record.replay_submit().unwrap(), Err(busy));
    let cancel_record = snapshot
        .idempotency_records
        .iter()
        .find(|record| record.key == IdempotencyKey::new("idem-cancel-running-a").unwrap())
        .expect("cancel idempotency record must be persisted");
    assert_eq!(cancel_record.replay_cancel().unwrap(), Ok(cancel));
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
async fn submit_turn_busy_idempotency_replays_after_thread_unlocks() {
    let (coordinator, store) = coordinator();
    let first_run_id = accepted_run_id(
        &coordinator
            .submit_turn(submit_request("thread-a", "idem-submit-a"))
            .await
            .unwrap(),
    );
    let busy_request = submit_request("thread-a", "idem-submit-b");
    let busy = coordinator
        .submit_turn(busy_request.clone())
        .await
        .unwrap_err();
    assert!(matches!(busy, TurnError::ThreadBusy(_)));

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

    let duplicate_after_unlock = coordinator.submit_turn(busy_request).await.unwrap_err();
    assert_eq!(duplicate_after_unlock, busy);
}

#[test]
fn concurrent_duplicate_submit_waits_for_in_flight_admission_result() {
    let store = Arc::new(InMemoryTurnStateStore::default());
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let policy = Arc::new(BlockingAdmissionPolicy {
        calls: AtomicUsize::new(0),
        entered: entered_tx,
        release: Mutex::new(release_rx),
    });

    let first_store = store.clone();
    let first_policy = policy.clone();
    let first = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let coordinator =
            DefaultTurnCoordinator::new(first_store).with_admission_policy(first_policy);
        runtime
            .block_on(coordinator.submit_turn(submit_request("thread-a", "idem-submit-concurrent")))
    });
    entered_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("first submit should enter admission policy");

    let second_store = store.clone();
    let second_policy = policy.clone();
    let second = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let coordinator =
            DefaultTurnCoordinator::new(second_store).with_admission_policy(second_policy);
        runtime
            .block_on(coordinator.submit_turn(submit_request("thread-a", "idem-submit-concurrent")))
    });

    std::thread::sleep(Duration::from_millis(50));
    release_tx
        .send(())
        .expect("first submit should still be waiting for admission release");

    let first = first.join().unwrap().unwrap();
    let second = second.join().unwrap().unwrap();
    assert_eq!(second, first);
    assert_eq!(policy.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn submit_turn_idempotency_replays_before_policy_is_rechecked() {
    let store = Arc::new(InMemoryTurnStateStore::default());
    let coordinator = DefaultTurnCoordinator::new(store)
        .with_admission_policy(Arc::new(AllowFirstThenDeny::default()));
    let request = submit_request("thread-a", "idem-submit-a");

    let first = coordinator.submit_turn(request.clone()).await.unwrap();
    let duplicate = coordinator.submit_turn(request).await.unwrap();

    assert_eq!(duplicate, first);
}

#[test]
fn submit_turn_admission_policy_can_reenter_store_without_deadlock() {
    let (sender, receiver) = mpsc::channel();

    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let store = Arc::new(InMemoryTurnStateStore::default());
        let coordinator = DefaultTurnCoordinator::new(store.clone())
            .with_admission_policy(Arc::new(ReentrantStorePolicy { store }));
        let result = runtime
            .block_on(coordinator.submit_turn(submit_request("thread-a", "idem-reentrant-policy")));
        let _ = sender.send(result);
    });

    let result = receiver
        .recv_timeout(Duration::from_secs(1))
        .expect("submit_turn should not deadlock when admission policy reads store state");
    assert!(matches!(result, Ok(SubmitTurnResponse::Accepted { .. })));
}

#[tokio::test]
async fn submit_turn_idempotency_replays_same_admission_rejection() {
    let store = Arc::new(InMemoryTurnStateStore::default());
    let coordinator = DefaultTurnCoordinator::new(store.clone())
        .with_admission_policy(Arc::new(DenyFirstThenAllow::default()));
    let request = submit_request("thread-a", "idem-submit-rejected");

    let first = coordinator.submit_turn(request.clone()).await.unwrap_err();
    let duplicate = coordinator.submit_turn(request).await.unwrap_err();

    assert_eq!(duplicate, first);
    assert_eq!(
        duplicate,
        TurnError::AdmissionRejected(AdmissionRejection::new(
            AdmissionRejectionReason::TenantLimit
        ))
    );
    assert!(store.events().is_empty());
}

#[tokio::test]
async fn idempotency_persistence_snapshot_retains_each_operation_kind_capacity() {
    let store = Arc::new(InMemoryTurnStateStore::with_limits(
        InMemoryTurnStateStoreLimits {
            max_idempotency_records: 1,
            ..InMemoryTurnStateStoreLimits::default()
        },
    ));
    let coordinator = DefaultTurnCoordinator::new(store.clone());
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
    let gate_ref = GateRef::new("approval-gate").unwrap();
    store
        .block_run(BlockRunRequest {
            run_id,
            runner_id,
            lease_token,
            checkpoint_id: TurnCheckpointId::new(),
            reason: BlockedReason::Approval {
                gate_ref: gate_ref.clone(),
            },
        })
        .await
        .unwrap();
    coordinator
        .resume_turn(ResumeTurnRequest {
            scope: scope("thread-a"),
            actor: actor(),
            run_id,
            gate_resolution_ref: gate_ref,
            source_binding_ref: SourceBindingRef::new("source-web-resumed").unwrap(),
            reply_target_binding_ref: ReplyTargetBindingRef::new("reply-web-resumed").unwrap(),
            idempotency_key: IdempotencyKey::new("idem-resume-a").unwrap(),
        })
        .await
        .unwrap();
    coordinator
        .cancel_run(cancel_request("thread-a", run_id, "idem-cancel-a"))
        .await
        .unwrap();

    let snapshot = store.persistence_snapshot();
    assert_eq!(snapshot.idempotency_records.len(), 3);
    assert!(snapshot.idempotency_records.iter().any(|record| {
        record.operation == TurnIdempotencyOperationKind::Submit
            && record.key == IdempotencyKey::new("idem-submit-a").unwrap()
    }));
    assert!(snapshot.idempotency_records.iter().any(|record| {
        record.operation == TurnIdempotencyOperationKind::Resume
            && record.key == IdempotencyKey::new("idem-resume-a").unwrap()
    }));
    assert!(snapshot.idempotency_records.iter().any(|record| {
        record.operation == TurnIdempotencyOperationKind::Cancel
            && record.key == IdempotencyKey::new("idem-cancel-a").unwrap()
    }));
}

#[tokio::test]
async fn idempotency_persistence_snapshot_drops_records_when_replay_cache_prunes_them() {
    let store = Arc::new(InMemoryTurnStateStore::with_limits(
        InMemoryTurnStateStoreLimits {
            max_idempotency_records: 1,
            ..InMemoryTurnStateStoreLimits::default()
        },
    ));
    let coordinator = DefaultTurnCoordinator::new(store.clone());

    coordinator
        .submit_turn(submit_request("thread-a", "idem-submit-a"))
        .await
        .unwrap();
    coordinator
        .submit_turn(submit_request("thread-b", "idem-submit-b"))
        .await
        .unwrap();

    let snapshot = store.persistence_snapshot();
    assert!(!snapshot.idempotency_records.iter().any(|record| {
        record.operation == TurnIdempotencyOperationKind::Submit
            && record.key == IdempotencyKey::new("idem-submit-a").unwrap()
    }));
    assert!(snapshot.idempotency_records.iter().any(|record| {
        record.operation == TurnIdempotencyOperationKind::Submit
            && record.key == IdempotencyKey::new("idem-submit-b").unwrap()
    }));
}

#[tokio::test]
async fn idempotency_replay_helpers_require_matching_operation_kind() {
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
    let gate_ref = GateRef::new("approval-gate").unwrap();
    store
        .block_run(BlockRunRequest {
            run_id,
            runner_id,
            lease_token,
            checkpoint_id: TurnCheckpointId::new(),
            reason: BlockedReason::Approval {
                gate_ref: gate_ref.clone(),
            },
        })
        .await
        .unwrap();
    coordinator
        .resume_turn(ResumeTurnRequest {
            scope: scope("thread-a"),
            actor: actor(),
            run_id,
            gate_resolution_ref: gate_ref,
            source_binding_ref: SourceBindingRef::new("source-web-resumed").unwrap(),
            reply_target_binding_ref: ReplyTargetBindingRef::new("reply-web-resumed").unwrap(),
            idempotency_key: IdempotencyKey::new("idem-resume-a").unwrap(),
        })
        .await
        .unwrap();
    coordinator
        .cancel_run(cancel_request("thread-a", run_id, "idem-cancel-a"))
        .await
        .unwrap();

    let snapshot = store.persistence_snapshot();
    let submit = snapshot
        .idempotency_records
        .iter()
        .find(|record| record.operation == TurnIdempotencyOperationKind::Submit)
        .unwrap();
    assert!(submit.replay_submit().is_some());
    let mut mislabeled_submit = submit.clone();
    mislabeled_submit.operation = TurnIdempotencyOperationKind::Cancel;
    assert!(mislabeled_submit.replay_submit().is_none());

    let resume = snapshot
        .idempotency_records
        .iter()
        .find(|record| record.operation == TurnIdempotencyOperationKind::Resume)
        .unwrap();
    assert!(resume.replay_resume().is_some());
    let mut mislabeled_resume = resume.clone();
    mislabeled_resume.operation = TurnIdempotencyOperationKind::Submit;
    assert!(mislabeled_resume.replay_resume().is_none());

    let cancel = snapshot
        .idempotency_records
        .iter()
        .find(|record| record.operation == TurnIdempotencyOperationKind::Cancel)
        .unwrap();
    assert!(cancel.replay_cancel().is_some());
    let mut mislabeled_cancel = cancel.clone();
    mislabeled_cancel.operation = TurnIdempotencyOperationKind::Resume;
    assert!(mislabeled_cancel.replay_cancel().is_none());
}

#[tokio::test]
async fn idempotency_retention_keeps_the_newest_result_when_pruned() {
    let store = Arc::new(InMemoryTurnStateStore::with_limits(
        InMemoryTurnStateStoreLimits {
            max_idempotency_records: 2,
            ..InMemoryTurnStateStoreLimits::default()
        },
    ));
    let coordinator = DefaultTurnCoordinator::new(store);

    coordinator
        .submit_turn(submit_request("thread-a", "idem-submit-a"))
        .await
        .unwrap();
    coordinator
        .submit_turn(submit_request("thread-b", "idem-submit-b"))
        .await
        .unwrap();
    let newest = coordinator
        .submit_turn(submit_request("thread-c", "idem-submit-c"))
        .await
        .unwrap();

    let duplicate_newest = coordinator
        .submit_turn(submit_request("thread-c", "idem-submit-c"))
        .await
        .unwrap();

    assert_eq!(duplicate_newest, newest);
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

    assert_eq!(err, TurnError::ScopeNotFound);
    assert_eq!(err.category(), TurnErrorCategory::ScopeNotFound);
    assert_eq!(err.adapter_status_code(), 404);
}

#[test]
fn admission_rejection_reason_status_mapping_is_user_actionable() {
    assert_eq!(
        TurnError::AdmissionRejected(AdmissionRejection::new(
            AdmissionRejectionReason::TenantLimit
        ))
        .adapter_status_code(),
        429
    );
    assert_eq!(
        TurnError::AdmissionRejected(AdmissionRejection::new(
            AdmissionRejectionReason::ProfileRejected
        ))
        .category(),
        TurnErrorCategory::InvalidRequest
    );
    assert_eq!(
        TurnError::AdmissionRejected(AdmissionRejection::new(
            AdmissionRejectionReason::ProfileRejected
        ))
        .adapter_status_code(),
        400
    );
    assert_eq!(
        TurnError::AdmissionRejected(AdmissionRejection::new(AdmissionRejectionReason::Policy))
            .adapter_status_code(),
        403
    );
    assert_eq!(
        TurnError::AdmissionRejected(AdmissionRejection::new(
            AdmissionRejectionReason::Unauthorized
        ))
        .adapter_status_code(),
        403
    );
    assert_eq!(
        TurnError::AdmissionRejected(AdmissionRejection::new(
            AdmissionRejectionReason::Unavailable
        ))
        .adapter_status_code(),
        503
    );
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
        TurnError::AdmissionRejected(AdmissionRejection::new(
            AdmissionRejectionReason::TenantLimit
        ))
    );
    assert!(err.is_expected_admission_outcome());
    assert_eq!(err.category(), TurnErrorCategory::AdmissionRejected);
    assert_eq!(err.adapter_status_code(), 429);
    assert!(store.events().is_empty());
    assert_eq!(
        TurnError::AdmissionRejected(AdmissionRejection::new(
            AdmissionRejectionReason::Unauthorized
        ))
        .adapter_status_code(),
        403
    );
    assert_eq!(
        TurnError::AdmissionRejected(AdmissionRejection::new(AdmissionRejectionReason::Policy))
            .adapter_status_code(),
        403
    );
    assert_eq!(
        TurnError::AdmissionRejected(AdmissionRejection::new(
            AdmissionRejectionReason::ProfileRejected
        ))
        .adapter_status_code(),
        400
    );
    assert_eq!(
        TurnError::AdmissionRejected(AdmissionRejection::new(
            AdmissionRejectionReason::Unavailable
        ))
        .adapter_status_code(),
        503
    );
    assert_eq!(
        coordinator
            .get_run_state(GetRunStateRequest {
                scope: request.scope,
                run_id: TurnRunId::new(),
            })
            .await
            .unwrap_err(),
        TurnError::ScopeNotFound
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

    let resume_request = ResumeTurnRequest {
        scope: scope("thread-a"),
        actor: actor(),
        run_id,
        gate_resolution_ref: gate_ref,
        source_binding_ref: SourceBindingRef::new("source-web").unwrap(),
        reply_target_binding_ref: ReplyTargetBindingRef::new("reply-web").unwrap(),
        idempotency_key: IdempotencyKey::new("idem-resume-a").unwrap(),
    };
    let resumed = coordinator
        .resume_turn(resume_request.clone())
        .await
        .unwrap();
    let event_count_after_resume = store.events().len();
    let duplicate = coordinator.resume_turn(resume_request).await.unwrap();
    assert_eq!(duplicate, resumed);
    assert_eq!(store.events().len(), event_count_after_resume);
    assert_eq!(resumed.status, TurnStatus::Queued);
}

#[tokio::test]
async fn resume_turn_with_wrong_gate_resolution_ref_is_invalid_request() {
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
    store
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
        .unwrap();

    let err = coordinator
        .resume_turn(ResumeTurnRequest {
            scope: scope("thread-a"),
            actor: actor(),
            run_id,
            gate_resolution_ref: GateRef::new("wrong-gate").unwrap(),
            source_binding_ref: SourceBindingRef::new("source-web").unwrap(),
            reply_target_binding_ref: ReplyTargetBindingRef::new("reply-web").unwrap(),
            idempotency_key: IdempotencyKey::new("idem-resume-wrong-gate").unwrap(),
        })
        .await
        .unwrap_err();

    assert_eq!(
        err,
        TurnError::InvalidRequest {
            reason: "gate resolution reference mismatch".to_string(),
        }
    );
    assert_eq!(err.adapter_status_code(), 400);
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
async fn runner_can_terminally_cancel_running_run_and_release_lock() {
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

    let cancelled = store
        .cancel_run(CancelRunCompletionRequest {
            run_id,
            runner_id,
            lease_token,
        })
        .await
        .unwrap();

    assert_eq!(cancelled.status, TurnStatus::Cancelled);
    assert!(cancelled.failure.is_none());
    let next = coordinator
        .submit_turn(submit_request("thread-a", "idem-submit-b"))
        .await
        .unwrap();
    assert_ne!(accepted_run_id(&next), run_id);
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

    let completed_after_cancel = store
        .complete_run(CompleteRunRequest {
            run_id,
            runner_id,
            lease_token,
        })
        .await
        .unwrap_err();
    assert_eq!(
        completed_after_cancel,
        TurnError::InvalidTransition {
            from: TurnStatus::CancelRequested,
            to: TurnStatus::Completed,
        }
    );

    let failed_after_cancel = store
        .fail_run(FailRunRequest {
            run_id,
            runner_id,
            lease_token,
            failure: SanitizedFailure::new("late_failure").unwrap(),
        })
        .await
        .unwrap_err();
    assert_eq!(
        failed_after_cancel,
        TurnError::InvalidTransition {
            from: TurnStatus::CancelRequested,
            to: TurnStatus::Failed,
        }
    );

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
async fn sanitized_failure_rejects_empty_controlled_oversized_or_unsanitized_categories() {
    assert!(SanitizedFailure::new("policy").is_ok());
    assert!(SanitizedFailure::new("policy_timeout").is_ok());
    assert!(SanitizedFailure::new("").is_err());
    assert!(SanitizedFailure::new("backend\nsecret=leaked").is_err());
    assert!(SanitizedFailure::new("x".repeat(257)).is_err());
    assert!(SanitizedFailure::new("/Users/alice/.ssh/config").is_err());
    assert!(SanitizedFailure::new("https://internal.example/error").is_err());
    assert!(SanitizedFailure::new("openai api key sk-test failed").is_err());
    assert!(SanitizedFailure::new("policy-timeout").is_err());
}

#[test]
fn bounded_refs_validate_during_deserialization() {
    assert!(serde_json::from_str::<AcceptedMessageRef>("\"message-ok\"").is_ok());
    assert!(serde_json::from_str::<AcceptedMessageRef>("\"\"").is_err());
    assert!(serde_json::from_str::<SourceBindingRef>("\"source\\nsecret\"").is_err());
    assert!(serde_json::from_str::<RunProfileRequest>("\"default\"").is_ok());
    assert!(serde_json::from_str::<RunProfileRequest>("\"profile\\nsecret\"").is_err());
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
        requested_run_profile: Some(RunProfileRequest::new("default").unwrap()),
        idempotency_key: IdempotencyKey::new(idempotency_key).unwrap(),
        received_at: received_at(),
    }
}

fn received_at() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 5, 5, 12, 0, 0).unwrap()
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

struct BlockingAdmissionPolicy {
    calls: AtomicUsize,
    entered: mpsc::Sender<()>,
    release: Mutex<mpsc::Receiver<()>>,
}

impl TurnAdmissionPolicy for BlockingAdmissionPolicy {
    fn check_submit(&self, _request: &SubmitTurnRequest) -> Result<(), AdmissionRejection> {
        if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            let _ = self.entered.send(());
            self.release
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(1))
                .expect("test should release first admission check");
        }
        Ok(())
    }
}

struct ReentrantStorePolicy {
    store: Arc<InMemoryTurnStateStore>,
}

impl TurnAdmissionPolicy for ReentrantStorePolicy {
    fn check_submit(&self, _request: &SubmitTurnRequest) -> Result<(), AdmissionRejection> {
        let _ = self.store.events();
        Ok(())
    }
}

#[derive(Default)]
struct AllowFirstThenDeny {
    calls: AtomicUsize,
}

impl TurnAdmissionPolicy for AllowFirstThenDeny {
    fn check_submit(&self, _request: &SubmitTurnRequest) -> Result<(), AdmissionRejection> {
        if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            Ok(())
        } else {
            Err(AdmissionRejection::new(
                AdmissionRejectionReason::TenantLimit,
            ))
        }
    }
}

#[derive(Default)]
struct DenyFirstThenAllow {
    calls: AtomicUsize,
}

impl TurnAdmissionPolicy for DenyFirstThenAllow {
    fn check_submit(&self, _request: &SubmitTurnRequest) -> Result<(), AdmissionRejection> {
        if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            Err(AdmissionRejection::new(
                AdmissionRejectionReason::TenantLimit,
            ))
        } else {
            Ok(())
        }
    }
}

struct DenyAll;

impl TurnAdmissionPolicy for DenyAll {
    fn check_submit(&self, _request: &SubmitTurnRequest) -> Result<(), AdmissionRejection> {
        Err(AdmissionRejection::new(
            AdmissionRejectionReason::TenantLimit,
        ))
    }
}
