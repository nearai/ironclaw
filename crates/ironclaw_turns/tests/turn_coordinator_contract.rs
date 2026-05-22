use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    task::{Context, Poll},
    time::Duration,
};

use chrono::{DateTime, Duration as ChronoDuration, TimeZone, Utc};
use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_turns::{
    AcceptedMessageRef, AdmissionRejection, AdmissionRejectionReason, AllowAllTurnAdmissionPolicy,
    BlockedReason, CancelRunRequest, DefaultTurnCoordinator, GateRef, GetRunStateRequest,
    IdempotencyKey, InMemoryRunProfileResolver, InMemoryTurnEventSink, InMemoryTurnStateStore,
    InMemoryTurnStateStoreLimits, LoopCheckpointStateRef, LoopExitMapping, LoopGateRef,
    ReplyTargetBindingRef, ResolvedRunProfile, ResumeTurnRequest, RunProfileId, RunProfileRequest,
    RunProfileResolutionError, RunProfileResolutionRequest, RunProfileResolver, RunProfileVersion,
    SanitizedCancelReason, SanitizedFailure, SourceBindingRef, StaticTurnAdmissionLimitProvider,
    SubmitTurnRequest, SubmitTurnResponse, ThreadBusy, TurnActor, TurnAdmissionAxisKind,
    TurnAdmissionBucketKind, TurnAdmissionBucketScope, TurnAdmissionCapacityDenial,
    TurnAdmissionClass, TurnAdmissionPolicy, TurnCheckpointId, TurnCoordinator, TurnError,
    TurnErrorCategory, TurnEventKind, TurnEventProjectionCursor, TurnEventProjectionError,
    TurnEventProjectionRequest, TurnEventProjectionService, TurnEventSink,
    TurnIdempotencyOperationKind, TurnIdempotencyOutcomeKind, TurnIdempotencyRecord,
    TurnIdempotencyReplay, TurnLeaseToken, TurnLifecycleEvent, TurnLockVersion, TurnRunId,
    TurnRunProfile, TurnRunState, TurnRunWake, TurnRunWakeNotifier, TurnRunWakeNotifyError,
    TurnRunnerId, TurnScope, TurnStateStore, TurnStatus,
    events::EventCursor,
    run_profile::LoopModelRouteSnapshot,
    runner::{
        ApplyValidatedLoopExitRequest, BlockRunRequest, CancelRunCompletionRequest,
        ClaimRunRequest, ClaimedTurnRun, CompleteRunRequest, FailRunRequest, HeartbeatRequest,
        RecordModelRouteSnapshotRequest, RecoverExpiredLeasesRequest, RecoverExpiredLeasesResponse,
        TurnRunTransitionPort, TurnRunnerOutcome,
    },
};

async fn apply_test_loop_exit<P>(
    port: &P,
    run_id: TurnRunId,
    runner_id: TurnRunnerId,
    lease_token: TurnLeaseToken,
    mapping: LoopExitMapping,
) -> Result<TurnRunState, TurnError>
where
    P: TurnRunTransitionPort + ?Sized,
{
    port.apply_validated_loop_exit(ApplyValidatedLoopExitRequest {
        run_id,
        runner_id,
        lease_token,
        mapping,
    })
    .await
}

fn completed_mapping() -> LoopExitMapping {
    LoopExitMapping::RunnerOutcome(TurnRunnerOutcome::Completed)
}

fn protocol_recovery_mapping() -> LoopExitMapping {
    LoopExitMapping::RecoveryRequired {
        failure: SanitizedFailure::new("driver_protocol_violation").unwrap(),
    }
}

fn cancelled_mapping() -> LoopExitMapping {
    LoopExitMapping::RunnerOutcome(TurnRunnerOutcome::Cancelled)
}

fn failed_mapping(category: &'static str) -> LoopExitMapping {
    LoopExitMapping::RunnerOutcome(TurnRunnerOutcome::Failed {
        failure: SanitizedFailure::new(category).unwrap(),
    })
}

fn approval_blocked_mapping(
    checkpoint_id: TurnCheckpointId,
    state_ref: LoopCheckpointStateRef,
    gate_ref: &LoopGateRef,
) -> LoopExitMapping {
    LoopExitMapping::RunnerOutcome(TurnRunnerOutcome::Blocked {
        checkpoint_id,
        state_ref,
        reason: BlockedReason::Approval {
            gate_ref: GateRef::new(gate_ref.as_str()).unwrap(),
        },
    })
}

struct BlockingRunProfileResolver {
    started: mpsc::Sender<()>,
}

impl BlockingRunProfileResolver {
    fn new(started: mpsc::Sender<()>) -> Self {
        Self { started }
    }
}

#[async_trait::async_trait]
impl RunProfileResolver for BlockingRunProfileResolver {
    async fn resolve_run_profile(
        &self,
        _request: RunProfileResolutionRequest,
    ) -> Result<ResolvedRunProfile, RunProfileResolutionError> {
        let _ = self.started.send(());
        std::future::pending::<Result<ResolvedRunProfile, RunProfileResolutionError>>().await
    }
}

#[test]
fn turn_scope_agent_id_is_optional() {
    let scope = TurnScope::new(
        TenantId::new("tenant1").unwrap(),
        None,
        Some(ProjectId::new("project1").unwrap()),
        ThreadId::new("thread-a").unwrap(),
    );

    assert_eq!(scope.agent_id, None);
}

#[tokio::test]
async fn turn_lifecycle_projection_replays_submit_block_resume_complete_without_raw_refs() {
    let (coordinator, store) = coordinator();
    let mut request = submit_request("thread-turn-events", "idem-turn-events-submit");
    request.accepted_message_ref =
        AcceptedMessageRef::new("message-TURN_RAW_INPUT_SENTINEL_3022 /tmp/turn-private-path")
            .unwrap();
    request.source_binding_ref = SourceBindingRef::new("source-TURN_SOURCE_SENTINEL_3022").unwrap();
    request.reply_target_binding_ref =
        ReplyTargetBindingRef::new("reply-TURN_REPLY_SENTINEL_3022").unwrap();

    let run_id = accepted_run_id(&coordinator.submit_turn(request.clone()).await.unwrap());
    let runner_id = TurnRunnerId::new();
    let lease_token = TurnLeaseToken::new();
    store
        .claim_next_run(ClaimRunRequest {
            runner_id,
            lease_token,
            scope_filter: Some(request.scope.clone()),
        })
        .await
        .unwrap()
        .unwrap();
    let gate_ref = GateRef::new("gate-TURN_GATE_SENTINEL_3022").unwrap();
    store
        .block_run(BlockRunRequest {
            run_id,
            runner_id,
            lease_token,
            checkpoint_id: TurnCheckpointId::new(),
            state_ref: block_state_ref(),
            reason: BlockedReason::Approval {
                gate_ref: gate_ref.clone(),
            },
        })
        .await
        .unwrap();
    coordinator
        .resume_turn(ResumeTurnRequest {
            scope: request.scope.clone(),
            actor: actor(),
            run_id,
            gate_resolution_ref: gate_ref,
            precondition: ironclaw_turns::ResumeTurnPrecondition::AnyBlockedGate,
            source_binding_ref: SourceBindingRef::new("source-TURN_RESUME_SOURCE_SENTINEL_3022")
                .unwrap(),
            reply_target_binding_ref: ReplyTargetBindingRef::new(
                "reply-TURN_RESUME_REPLY_SENTINEL_3022",
            )
            .unwrap(),
            idempotency_key: IdempotencyKey::new("idem-turn-events-resume").unwrap(),
        })
        .await
        .unwrap();
    let next_lease_token = TurnLeaseToken::new();
    store
        .claim_next_run(ClaimRunRequest {
            runner_id,
            lease_token: next_lease_token,
            scope_filter: Some(request.scope.clone()),
        })
        .await
        .unwrap()
        .unwrap();
    store
        .complete_run(CompleteRunRequest {
            run_id,
            runner_id,
            lease_token: next_lease_token,
        })
        .await
        .unwrap();

    let projection = TurnEventProjectionService::new(store.clone());
    let snapshot = projection
        .snapshot(TurnEventProjectionRequest {
            scope: request.scope.clone(),
            after: None,
            limit: 10,
        })
        .await
        .unwrap();

    assert_eq!(
        snapshot
            .entries
            .iter()
            .map(|entry| entry.kind.clone())
            .collect::<Vec<_>>(),
        vec![
            TurnEventKind::Submitted,
            TurnEventKind::RunnerClaimed,
            TurnEventKind::Blocked,
            TurnEventKind::Resumed,
            TurnEventKind::RunnerClaimed,
            TurnEventKind::Completed,
        ]
    );
    assert!(
        snapshot
            .entries
            .iter()
            .all(|entry| entry.scope == request.scope)
    );
    assert!(snapshot.entries.iter().all(|entry| entry.run_id == run_id));
    assert_eq!(
        snapshot.entries.last().unwrap().status,
        TurnStatus::Completed
    );

    let foreign = projection
        .updates(TurnEventProjectionRequest {
            scope: scope("thread-foreign-turn-events"),
            after: Some(snapshot.next_cursor.clone()),
            limit: 10,
        })
        .await
        .expect_err("foreign turn projection cursor must force rebase");
    assert!(matches!(
        foreign,
        TurnEventProjectionError::RebaseRequired { .. }
    ));

    let serialized = serde_json::to_string(&snapshot).unwrap();
    assert_no_forbidden_turn_event_content(
        "turn lifecycle projection",
        &serialized,
        &[
            "TURN_RAW_INPUT_SENTINEL_3022",
            "/tmp/turn-private-path",
            "TURN_SOURCE_SENTINEL_3022",
            "TURN_REPLY_SENTINEL_3022",
            "TURN_GATE_SENTINEL_3022",
            "TURN_RESUME_SOURCE_SENTINEL_3022",
            "TURN_RESUME_REPLY_SENTINEL_3022",
        ],
    );
}

#[tokio::test]
async fn turn_lifecycle_projection_replays_failed_terminal_with_sanitized_reason_without_raw_refs()
{
    let (coordinator, store) = coordinator();
    let mut request = submit_request("thread-turn-failed-events", "idem-turn-failed-submit");
    request.accepted_message_ref = AcceptedMessageRef::new(
        "message-TURN_FAILED_ACCEPTED_SENTINEL_3022 /tmp/turn-failed-private",
    )
    .unwrap();
    request.source_binding_ref =
        SourceBindingRef::new("source-TURN_FAILED_SOURCE_SENTINEL_3022").unwrap();
    request.reply_target_binding_ref =
        ReplyTargetBindingRef::new("reply-TURN_FAILED_REPLY_SENTINEL_3022").unwrap();

    let run_id = accepted_run_id(&coordinator.submit_turn(request.clone()).await.unwrap());
    let runner_id = TurnRunnerId::new();
    let lease_token = TurnLeaseToken::new();
    store
        .claim_next_run(ClaimRunRequest {
            runner_id,
            lease_token,
            scope_filter: Some(request.scope.clone()),
        })
        .await
        .unwrap()
        .unwrap();

    let failed = apply_test_loop_exit(
        store.as_ref(),
        run_id,
        runner_id,
        lease_token,
        failed_mapping("driver_bug"),
    )
    .await
    .unwrap();
    assert_eq!(failed.status, TurnStatus::Failed);
    assert_eq!(
        failed.failure.as_ref().map(SanitizedFailure::category),
        Some("driver_bug")
    );

    let projection = TurnEventProjectionService::new(store.clone());
    let snapshot = projection
        .snapshot(TurnEventProjectionRequest {
            scope: request.scope.clone(),
            after: None,
            limit: 10,
        })
        .await
        .unwrap();

    assert_eq!(
        snapshot
            .entries
            .iter()
            .map(|entry| entry.kind.clone())
            .collect::<Vec<_>>(),
        vec![
            TurnEventKind::Submitted,
            TurnEventKind::RunnerClaimed,
            TurnEventKind::Failed,
        ]
    );
    assert!(
        snapshot
            .entries
            .iter()
            .all(|entry| entry.scope == request.scope)
    );
    assert!(snapshot.entries.iter().all(|entry| entry.run_id == run_id));
    let failed_entry = snapshot.entries.last().unwrap();
    assert_eq!(failed_entry.status, TurnStatus::Failed);
    assert_eq!(failed_entry.sanitized_reason.as_deref(), Some("driver_bug"));

    let projection_json = serde_json::to_string(&snapshot).unwrap();
    assert!(projection_json.contains("driver_bug"));
    let retained_events_json = serde_json::to_string(&store.persistence_snapshot().events).unwrap();
    let forbidden = [
        "TURN_FAILED_ACCEPTED_SENTINEL_3022",
        "TURN_FAILED_SOURCE_SENTINEL_3022",
        "TURN_FAILED_REPLY_SENTINEL_3022",
        "TURN_FAILED_FAILURE_REASON_SENTINEL_3022",
        "TURN_FAILED_USAGE_SENTINEL_3022",
        "TURN_FAILED_EXIT_SENTINEL_3022",
        "/tmp/turn-failed-private",
    ];
    assert_no_forbidden_turn_event_content(
        "failed turn lifecycle projection",
        &projection_json,
        &forbidden,
    );
    assert_no_forbidden_turn_event_content(
        "failed retained turn lifecycle events",
        &retained_events_json,
        &forbidden,
    );
}

#[tokio::test]
async fn turn_lifecycle_projection_replays_cancelled_terminal_without_raw_refs() {
    let (coordinator, store) = coordinator();
    let mut request = submit_request("thread-turn-cancelled-events", "idem-turn-cancel-submit");
    request.accepted_message_ref = AcceptedMessageRef::new(
        "message-TURN_CANCELLED_ACCEPTED_SENTINEL_3022 /tmp/turn-cancelled-private",
    )
    .unwrap();
    request.source_binding_ref =
        SourceBindingRef::new("source-TURN_CANCELLED_SOURCE_SENTINEL_3022").unwrap();
    request.reply_target_binding_ref =
        ReplyTargetBindingRef::new("reply-TURN_CANCELLED_REPLY_SENTINEL_3022").unwrap();

    let run_id = accepted_run_id(&coordinator.submit_turn(request.clone()).await.unwrap());
    let runner_id = TurnRunnerId::new();
    let lease_token = TurnLeaseToken::new();
    store
        .claim_next_run(ClaimRunRequest {
            runner_id,
            lease_token,
            scope_filter: Some(request.scope.clone()),
        })
        .await
        .unwrap()
        .unwrap();
    let cancel_requested = coordinator
        .cancel_run(CancelRunRequest {
            scope: request.scope.clone(),
            actor: actor(),
            run_id,
            reason: SanitizedCancelReason::OperatorRequested,
            idempotency_key: IdempotencyKey::new("idem-turn-cancel-running").unwrap(),
        })
        .await
        .unwrap();
    assert_eq!(cancel_requested.status, TurnStatus::CancelRequested);

    let cancelled = apply_test_loop_exit(
        store.as_ref(),
        run_id,
        runner_id,
        lease_token,
        cancelled_mapping(),
    )
    .await
    .unwrap();
    assert_eq!(cancelled.status, TurnStatus::Cancelled);

    let projection = TurnEventProjectionService::new(store.clone());
    let snapshot = projection
        .snapshot(TurnEventProjectionRequest {
            scope: request.scope.clone(),
            after: None,
            limit: 10,
        })
        .await
        .unwrap();

    assert_eq!(
        snapshot
            .entries
            .iter()
            .map(|entry| entry.kind.clone())
            .collect::<Vec<_>>(),
        vec![
            TurnEventKind::Submitted,
            TurnEventKind::RunnerClaimed,
            TurnEventKind::CancelRequested,
            TurnEventKind::Cancelled,
        ]
    );
    assert!(
        snapshot
            .entries
            .iter()
            .all(|entry| entry.scope == request.scope)
    );
    assert!(snapshot.entries.iter().all(|entry| entry.run_id == run_id));
    let cancel_requested_entry = snapshot
        .entries
        .iter()
        .find(|entry| entry.kind == TurnEventKind::CancelRequested)
        .unwrap();
    assert_eq!(
        cancel_requested_entry.sanitized_reason.as_deref(),
        Some("operator_requested")
    );
    let cancelled_entry = snapshot.entries.last().unwrap();
    assert_eq!(cancelled_entry.status, TurnStatus::Cancelled);
    assert_eq!(cancelled_entry.sanitized_reason.as_deref(), None);

    let projection_json = serde_json::to_string(&snapshot).unwrap();
    assert!(projection_json.contains("operator_requested"));
    let retained_events_json = serde_json::to_string(&store.persistence_snapshot().events).unwrap();
    let forbidden = [
        "TURN_CANCELLED_ACCEPTED_SENTINEL_3022",
        "TURN_CANCELLED_SOURCE_SENTINEL_3022",
        "TURN_CANCELLED_REPLY_SENTINEL_3022",
        "TURN_CANCELLED_REASON_SENTINEL_3022",
        "TURN_CANCELLED_EXIT_SENTINEL_3022",
        "/tmp/turn-cancelled-private",
    ];
    assert_no_forbidden_turn_event_content(
        "cancelled turn lifecycle projection",
        &projection_json,
        &forbidden,
    );
    assert_no_forbidden_turn_event_content(
        "cancelled retained turn lifecycle events",
        &retained_events_json,
        &forbidden,
    );
}

#[tokio::test]
async fn turn_lifecycle_projection_requires_rebase_for_pruned_or_fabricated_cursors() {
    let store = Arc::new(InMemoryTurnStateStore::with_limits(
        InMemoryTurnStateStoreLimits {
            max_events: 2,
            ..InMemoryTurnStateStoreLimits::default()
        },
    ));
    let coordinator = DefaultTurnCoordinator::new(store.clone());
    let mut request = submit_request("thread-turn-gap", "idem-turn-gap-submit");
    request.accepted_message_ref =
        AcceptedMessageRef::new("message-TURN_GAP_RAW_SENTINEL_3022 /tmp/turn-gap-private")
            .unwrap();

    let run_id = accepted_run_id(&coordinator.submit_turn(request.clone()).await.unwrap());
    let runner_id = TurnRunnerId::new();
    let lease_token = TurnLeaseToken::new();
    store
        .claim_next_run(ClaimRunRequest {
            runner_id,
            lease_token,
            scope_filter: Some(request.scope.clone()),
        })
        .await
        .unwrap()
        .unwrap();
    store
        .heartbeat(HeartbeatRequest {
            run_id,
            runner_id,
            lease_token,
        })
        .await
        .unwrap();

    let projection = TurnEventProjectionService::new(store.clone());
    let origin = TurnEventProjectionCursor::origin_for_scope(request.scope.clone());
    let pruned_origin = projection
        .snapshot(TurnEventProjectionRequest {
            scope: request.scope.clone(),
            after: Some(origin),
            limit: 10,
        })
        .await
        .expect_err("pruned turn lifecycle origin cursor must require rebase");
    assert!(matches!(
        pruned_origin,
        TurnEventProjectionError::RebaseRequired { .. }
    ));

    let fabricated = projection
        .updates(TurnEventProjectionRequest {
            scope: request.scope.clone(),
            after: Some(TurnEventProjectionCursor::for_scope(
                request.scope.clone(),
                EventCursor(999),
            )),
            limit: 10,
        })
        .await
        .expect_err("fabricated beyond-head turn lifecycle cursor must require rebase");
    assert!(matches!(
        fabricated,
        TurnEventProjectionError::RebaseRequired { .. }
    ));

    let serialized_events = serde_json::to_string(&store.events()).unwrap();
    let debug_errors = format!("{pruned_origin:?} {fabricated:?}");
    let forbidden = ["TURN_GAP_RAW_SENTINEL_3022", "/tmp/turn-gap-private"];
    assert_no_forbidden_turn_event_content("retained turn events", &serialized_events, &forbidden);
    assert_no_forbidden_turn_event_content(
        "turn projection rebase error",
        &debug_errors,
        &forbidden,
    );
}

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
            scope: request.scope.clone(),
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

    let snapshot = _store.persistence_snapshot();
    let run = snapshot
        .runs
        .iter()
        .find(|record| record.run_id == run_id)
        .unwrap();
    assert_eq!(run.profile.id.as_str(), "default");
    assert_eq!(
        run.profile.resolved.profile_id.as_str(),
        "interactive_default"
    );
    assert_eq!(
        run.profile.resolved.loop_driver.id.as_str(),
        "lightweight_loop"
    );

    let claimed = _store
        .claim_next_run(ClaimRunRequest {
            runner_id: TurnRunnerId::new(),
            lease_token: TurnLeaseToken::new(),
            scope_filter: Some(request.scope),
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        claimed.resolved_run_profile.profile_id.as_str(),
        "interactive_default"
    );
    assert_eq!(
        claimed.resolved_run_profile.loop_driver.id.as_str(),
        "lightweight_loop"
    );
}

#[tokio::test]
async fn unauthorized_requested_run_profile_rejects_before_persisting_run() {
    let (coordinator, store) = coordinator();
    let mut request = submit_request("thread-a", "idem-profile-hint");
    request.requested_run_profile = Some(RunProfileRequest::new("long_running_mission").unwrap());

    let err = coordinator.submit_turn(request.clone()).await.unwrap_err();

    assert_eq!(
        err,
        TurnError::AdmissionRejected(AdmissionRejection::new(
            AdmissionRejectionReason::Unauthorized
        ))
    );
    assert!(store.events().is_empty());
    assert!(store.persistence_snapshot().runs.is_empty());

    let duplicate = coordinator.submit_turn(request).await.unwrap_err();
    assert_eq!(duplicate, err);
}

#[test]
fn legacy_turn_run_profile_payload_deserializes_with_synthetic_resolved_snapshot() {
    let legacy = r#"{
        "id":"default",
        "version":1,
        "allow_steering":false,
        "auto_queue_followups":false
    }"#;

    let profile = serde_json::from_str::<TurnRunProfile>(legacy).unwrap();

    assert_eq!(profile.id.as_str(), "default");
    assert_eq!(profile.version, RunProfileVersion::new(1));
    assert!(!profile.allow_steering);
    assert_eq!(profile.resolved.profile_id.as_str(), "default");
    assert_eq!(profile.resolved.profile_version, RunProfileVersion::new(1));
    assert_eq!(profile.resolved.loop_driver.id.as_str(), "lightweight_loop");
    assert!(!profile.resolved.steering_policy.allow_steering);
    assert!(
        profile
            .resolved
            .provenance
            .sources
            .iter()
            .any(|source| source.summary
                == "legacy persisted turn run profile reconstructed without raw authority handles")
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
async fn submit_turn_wakes_runner_only_after_accepting_queued_run() {
    let store = Arc::new(InMemoryTurnStateStore::default());
    let notifier = Arc::new(RecordingWakeNotifier::default());
    let coordinator = DefaultTurnCoordinator::new(store)
        .with_wake_notifier(notifier.clone())
        .with_admission_policy(Arc::new(DenyFirstThenAllow::default()));
    let rejected_request = submit_request("thread-a", "idem-rejected");

    let rejected = coordinator.submit_turn(rejected_request).await.unwrap_err();
    assert!(matches!(rejected, TurnError::AdmissionRejected(_)));
    assert!(notifier.wakes().is_empty());

    let accepted_request = submit_request("thread-a", "idem-accepted");
    let accepted = coordinator
        .submit_turn(accepted_request.clone())
        .await
        .unwrap();
    let run_id = accepted_run_id(&accepted);
    assert_eq!(
        notifier.wakes(),
        vec![TurnRunWake {
            scope: accepted_request.scope,
            run_id,
            status: TurnStatus::Queued,
            event_cursor: EventCursor(1),
        }]
    );

    let busy = coordinator
        .submit_turn(submit_request("thread-a", "idem-busy"))
        .await
        .unwrap_err();
    assert!(matches!(busy, TurnError::ThreadBusy(_)));
    assert_eq!(notifier.wakes().len(), 1);
}

#[tokio::test]
async fn resume_turn_wakes_runner_for_same_run_after_requeue() {
    let store = Arc::new(InMemoryTurnStateStore::default());
    let notifier = Arc::new(RecordingWakeNotifier::default());
    let coordinator =
        DefaultTurnCoordinator::new(store.clone()).with_wake_notifier(notifier.clone());
    let run_id = accepted_run_id(
        &coordinator
            .submit_turn(submit_request("thread-a", "idem-submit-a"))
            .await
            .unwrap(),
    );
    notifier.clear();
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
            state_ref: block_state_ref(),
            reason: BlockedReason::Approval {
                gate_ref: gate_ref.clone(),
            },
        })
        .await
        .unwrap();

    let resumed = coordinator
        .resume_turn(ResumeTurnRequest {
            scope: scope("thread-a"),
            actor: actor(),
            run_id,
            gate_resolution_ref: gate_ref,
            precondition: ironclaw_turns::ResumeTurnPrecondition::AnyBlockedGate,
            source_binding_ref: SourceBindingRef::new("source-web-resumed").unwrap(),
            reply_target_binding_ref: ReplyTargetBindingRef::new("reply-web-resumed").unwrap(),
            idempotency_key: IdempotencyKey::new("idem-resume-a").unwrap(),
        })
        .await
        .unwrap();

    assert_eq!(
        notifier.wakes(),
        vec![TurnRunWake {
            scope: scope("thread-a"),
            run_id,
            status: TurnStatus::Queued,
            event_cursor: resumed.event_cursor,
        }]
    );
}

#[tokio::test]
async fn cancel_run_wakes_runner_for_active_cancel_requested_run() {
    let store = Arc::new(InMemoryTurnStateStore::default());
    let notifier = Arc::new(RecordingWakeNotifier::default());
    let coordinator =
        DefaultTurnCoordinator::new(store.clone()).with_wake_notifier(notifier.clone());
    let run_id = accepted_run_id(
        &coordinator
            .submit_turn(submit_request("thread-a", "idem-submit-a"))
            .await
            .unwrap(),
    );
    notifier.clear();
    store
        .claim_next_run(ClaimRunRequest {
            runner_id: TurnRunnerId::new(),
            lease_token: TurnLeaseToken::new(),
            scope_filter: None,
        })
        .await
        .unwrap()
        .unwrap();

    let cancelled = coordinator
        .cancel_run(cancel_request("thread-a", run_id, "idem-cancel-a"))
        .await
        .unwrap();

    assert_eq!(
        notifier.wakes(),
        vec![TurnRunWake {
            scope: scope("thread-a"),
            run_id,
            status: TurnStatus::CancelRequested,
            event_cursor: cancelled.event_cursor,
        }]
    );
}

#[tokio::test]
async fn submit_turn_ignores_wake_notification_failure_after_persisting_run() {
    let store = Arc::new(InMemoryTurnStateStore::default());
    let coordinator = DefaultTurnCoordinator::new(store.clone())
        .with_wake_notifier(Arc::new(FailingWakeNotifier));

    let response = coordinator
        .submit_turn(submit_request("thread-a", "idem-submit-a"))
        .await
        .unwrap();
    let run_id = accepted_run_id(&response);

    let state = coordinator
        .get_run_state(GetRunStateRequest {
            scope: scope("thread-a"),
            run_id,
        })
        .await
        .unwrap();
    assert_eq!(state.status, TurnStatus::Queued);
}

#[tokio::test]
async fn resume_turn_ignores_wake_notification_panic_after_requeue() {
    let store = Arc::new(InMemoryTurnStateStore::default());
    let coordinator = DefaultTurnCoordinator::new(store.clone())
        .with_wake_notifier(Arc::new(PanickingWakeNotifier));
    let run_id = store
        .submit_turn(
            submit_request("thread-a", "idem-submit-a"),
            &AllowAllTurnAdmissionPolicy,
            &InMemoryRunProfileResolver::default(),
        )
        .await
        .map(|response| accepted_run_id(&response))
        .unwrap();
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
            state_ref: block_state_ref(),
            reason: BlockedReason::Approval {
                gate_ref: gate_ref.clone(),
            },
        })
        .await
        .unwrap();

    let resumed = coordinator
        .resume_turn(ResumeTurnRequest {
            scope: scope("thread-a"),
            actor: actor(),
            run_id,
            gate_resolution_ref: gate_ref,
            precondition: ironclaw_turns::ResumeTurnPrecondition::AnyBlockedGate,
            source_binding_ref: SourceBindingRef::new("source-web-resumed").unwrap(),
            reply_target_binding_ref: ReplyTargetBindingRef::new("reply-web-resumed").unwrap(),
            idempotency_key: IdempotencyKey::new("idem-resume-a").unwrap(),
        })
        .await
        .unwrap();

    assert_eq!(resumed.run_id, run_id);
    assert_eq!(resumed.status, TurnStatus::Queued);
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
async fn same_thread_busy_is_transient_and_checked_before_admission_capacity() {
    let limits = StaticTurnAdmissionLimitProvider::default()
        .with_total_limit(TurnAdmissionAxisKind::Tenant, 1);
    let store = Arc::new(InMemoryTurnStateStore::with_admission_limit_provider(
        Arc::new(limits),
    ));
    let coordinator = DefaultTurnCoordinator::new(store.clone());
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
    assert_eq!(store.active_admission_reservations().len(), 1);
    assert_eq!(store.persistence_snapshot().idempotency_records.len(), 1);

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

    let duplicate_after_unlock = coordinator.submit_turn(busy_request).await.unwrap();
    assert_ne!(accepted_run_id(&duplicate_after_unlock), first_run_id);
    assert_eq!(store.persistence_snapshot().turns.len(), 2);
}

#[tokio::test]
async fn tenant_capacity_denial_is_structured_idempotent_and_all_or_nothing() {
    let limits = StaticTurnAdmissionLimitProvider::default()
        .with_total_limit(TurnAdmissionAxisKind::Tenant, 1);
    let store = Arc::new(InMemoryTurnStateStore::with_admission_limit_provider(
        Arc::new(limits),
    ));
    let coordinator = DefaultTurnCoordinator::new(store.clone());
    coordinator
        .submit_turn(submit_request("thread-a", "idem-submit-a"))
        .await
        .unwrap();
    let denied_request = submit_request("thread-b", "idem-submit-b");

    let denied = coordinator
        .submit_turn(denied_request.clone())
        .await
        .unwrap_err();
    assert!(matches!(
        denied,
        TurnError::AdmissionRejected(AdmissionRejection {
            reason: AdmissionRejectionReason::TenantLimit,
            capacity_denial: Some(TurnAdmissionCapacityDenial {
                axis_kind: TurnAdmissionAxisKind::Tenant,
                bucket_kind: TurnAdmissionBucketKind::Total,
                limit: 1,
                active_count: 1,
                ..
            }),
            ..
        })
    ));
    assert_eq!(store.persistence_snapshot().turns.len(), 1);
    assert_eq!(store.active_admission_reservations().len(), 1);

    let duplicate_denied = coordinator.submit_turn(denied_request).await.unwrap_err();
    assert_eq!(duplicate_denied, denied);
    assert_eq!(store.persistence_snapshot().turns.len(), 1);
    assert_eq!(store.active_admission_reservations().len(), 1);
}

#[tokio::test]
async fn concurrent_submits_cannot_oversubscribe_admission_limit() {
    let limits = StaticTurnAdmissionLimitProvider::default()
        .with_total_limit(TurnAdmissionAxisKind::Tenant, 1);
    let store = Arc::new(InMemoryTurnStateStore::with_admission_limit_provider(
        Arc::new(limits),
    ));
    let coordinator = DefaultTurnCoordinator::new(store.clone());

    let (first, second) = tokio::join!(
        coordinator.submit_turn(submit_request("thread-a", "idem-submit-a")),
        coordinator.submit_turn(submit_request("thread-b", "idem-submit-b")),
    );

    let accepted = [first.as_ref(), second.as_ref()]
        .into_iter()
        .filter(|result| matches!(result, Ok(SubmitTurnResponse::Accepted { .. })))
        .count();
    let rejected = [first.as_ref(), second.as_ref()]
        .into_iter()
        .filter(|result| matches!(result, Err(TurnError::AdmissionRejected(_))))
        .count();
    assert_eq!(accepted, 1);
    assert_eq!(rejected, 1);
    assert_eq!(store.persistence_snapshot().turns.len(), 1);
    assert_eq!(store.active_admission_reservations().len(), 1);
}

#[tokio::test]
async fn capacity_denial_does_not_advance_event_cursor() {
    let limits = StaticTurnAdmissionLimitProvider::default()
        .with_total_limit(TurnAdmissionAxisKind::Tenant, 1);
    let store = Arc::new(InMemoryTurnStateStore::with_admission_limit_provider(
        Arc::new(limits),
    ));
    let coordinator = DefaultTurnCoordinator::new(store.clone());
    let first_run_id = accepted_run_id(
        &coordinator
            .submit_turn(submit_request("thread-a", "idem-submit-a"))
            .await
            .unwrap(),
    );

    let denied = coordinator
        .submit_turn(submit_request("thread-b", "idem-submit-b"))
        .await
        .unwrap_err();
    assert!(matches!(denied, TurnError::AdmissionRejected(_)));

    let runner_id = TurnRunnerId::new();
    let lease_token = TurnLeaseToken::new();
    store
        .claim_next_run(ClaimRunRequest {
            runner_id,
            lease_token,
            scope_filter: Some(scope("thread-a")),
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

    let SubmitTurnResponse::Accepted { event_cursor, .. } = coordinator
        .submit_turn(submit_request("thread-c", "idem-submit-c"))
        .await
        .unwrap();
    assert_eq!(event_cursor, EventCursor(4));
}

#[tokio::test]
async fn snapshot_load_ignores_legacy_submit_thread_busy_replays() {
    let source_store = Arc::new(InMemoryTurnStateStore::default());
    let source_coordinator = DefaultTurnCoordinator::new(source_store.clone());
    let first_run_id = accepted_run_id(
        &source_coordinator
            .submit_turn(submit_request("thread-a", "idem-submit-a"))
            .await
            .unwrap(),
    );
    let mut snapshot = source_store.persistence_snapshot();
    let legacy_busy_key = IdempotencyKey::new("idem-legacy-busy").unwrap();
    snapshot.idempotency_records.push(TurnIdempotencyRecord {
        scope: scope("thread-a"),
        operation: TurnIdempotencyOperationKind::Submit,
        key: legacy_busy_key.clone(),
        turn_id: None,
        run_id: Some(first_run_id),
        outcome: TurnIdempotencyOutcomeKind::ThreadBusy,
        replay: TurnIdempotencyReplay::SubmitThreadBusy(ThreadBusy {
            active_run_id: first_run_id,
            status: TurnStatus::Queued,
            event_cursor: EventCursor(1),
        }),
        created_at: received_at(),
        expires_at: None,
    });
    let restored = Arc::new(
        InMemoryTurnStateStore::from_persistence_snapshot(
            snapshot,
            InMemoryTurnStateStoreLimits::default(),
        )
        .unwrap(),
    );
    let runner_id = TurnRunnerId::new();
    let lease_token = TurnLeaseToken::new();
    restored
        .claim_next_run(ClaimRunRequest {
            runner_id,
            lease_token,
            scope_filter: Some(scope("thread-a")),
        })
        .await
        .unwrap()
        .unwrap();
    restored
        .complete_run(CompleteRunRequest {
            run_id: first_run_id,
            runner_id,
            lease_token,
        })
        .await
        .unwrap();

    let restored_coordinator = DefaultTurnCoordinator::new(restored.clone());
    let accepted_after_unlock = restored_coordinator
        .submit_turn(submit_request("thread-a", legacy_busy_key.as_str()))
        .await
        .unwrap();
    assert_ne!(accepted_run_id(&accepted_after_unlock), first_run_id);
}

#[tokio::test]
async fn snapshot_load_synthesizes_reservations_for_legacy_active_runs() {
    let source_store = Arc::new(InMemoryTurnStateStore::default());
    let source_coordinator = DefaultTurnCoordinator::new(source_store.clone());
    source_coordinator
        .submit_turn(submit_request("thread-a", "idem-submit-a"))
        .await
        .unwrap();
    let mut snapshot = source_store.persistence_snapshot();
    assert_eq!(snapshot.admission_reservations.len(), 1);
    snapshot.admission_reservations.clear();

    let limits = StaticTurnAdmissionLimitProvider::default()
        .with_total_limit(TurnAdmissionAxisKind::Tenant, 1);
    let restored = Arc::new(
        InMemoryTurnStateStore::from_persistence_snapshot_with_admission_limit_provider(
            snapshot,
            InMemoryTurnStateStoreLimits::default(),
            Arc::new(limits),
        )
        .unwrap(),
    );
    assert_eq!(restored.active_admission_reservations().len(), 1);

    let restored_coordinator = DefaultTurnCoordinator::new(restored.clone());
    let denied = restored_coordinator
        .submit_turn(submit_request("thread-b", "idem-submit-b"))
        .await
        .unwrap_err();
    assert!(matches!(
        denied,
        TurnError::AdmissionRejected(AdmissionRejection {
            capacity_denial: Some(TurnAdmissionCapacityDenial {
                axis_kind: TurnAdmissionAxisKind::Tenant,
                limit: 1,
                active_count: 1,
                ..
            }),
            ..
        })
    ));
    assert_eq!(restored.persistence_snapshot().turns.len(), 1);
}

#[tokio::test]
async fn snapshot_load_recovers_released_or_mismatched_reservations_for_active_runs() {
    let source_store = Arc::new(InMemoryTurnStateStore::default());
    let source_coordinator = DefaultTurnCoordinator::new(source_store.clone());
    source_coordinator
        .submit_turn(submit_request("thread-a", "idem-submit-a"))
        .await
        .unwrap();
    let source_snapshot = source_store.persistence_snapshot();
    assert_eq!(source_snapshot.admission_reservations.len(), 1);

    for corrupt_reservation in ["released", "empty_buckets"] {
        let mut snapshot = source_snapshot.clone();
        match corrupt_reservation {
            "released" => snapshot.admission_reservations[0].released = true,
            "empty_buckets" => snapshot.admission_reservations[0].buckets.clear(),
            _ => unreachable!(),
        }
        let limits = StaticTurnAdmissionLimitProvider::default()
            .with_total_limit(TurnAdmissionAxisKind::Tenant, 1);
        let restored = Arc::new(
            InMemoryTurnStateStore::from_persistence_snapshot_with_admission_limit_provider(
                snapshot,
                InMemoryTurnStateStoreLimits::default(),
                Arc::new(limits),
            )
            .unwrap(),
        );
        let reservations = restored.active_admission_reservations();
        assert_eq!(reservations.len(), 1, "case {corrupt_reservation}");
        assert!(!reservations[0].released, "case {corrupt_reservation}");
        assert_eq!(
            reservations[0].buckets.len(),
            8,
            "case {corrupt_reservation}"
        );

        let restored_coordinator = DefaultTurnCoordinator::new(restored.clone());
        let denied = restored_coordinator
            .submit_turn(submit_request(
                &format!("thread-b-{corrupt_reservation}"),
                &format!("idem-submit-b-{corrupt_reservation}"),
            ))
            .await
            .unwrap_err();
        assert!(
            matches!(
                denied,
                TurnError::AdmissionRejected(AdmissionRejection {
                    capacity_denial: Some(TurnAdmissionCapacityDenial {
                        axis_kind: TurnAdmissionAxisKind::Tenant,
                        limit: 1,
                        active_count: 1,
                        ..
                    }),
                    ..
                })
            ),
            "case {corrupt_reservation}: {denied:?}"
        );
    }
}

#[tokio::test]
async fn terminal_run_releases_admission_reservation_for_new_thread() {
    let limits = StaticTurnAdmissionLimitProvider::default()
        .with_total_limit(TurnAdmissionAxisKind::Tenant, 1);
    let store = Arc::new(InMemoryTurnStateStore::with_admission_limit_provider(
        Arc::new(limits),
    ));
    let coordinator = DefaultTurnCoordinator::new(store.clone());
    let first_run_id = accepted_run_id(
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
        .complete_run(CompleteRunRequest {
            run_id: first_run_id,
            runner_id,
            lease_token,
        })
        .await
        .unwrap();

    assert!(store.active_admission_reservations().is_empty());
    let second = coordinator
        .submit_turn(submit_request("thread-b", "idem-submit-b"))
        .await
        .unwrap();
    assert_ne!(accepted_run_id(&second), first_run_id);
    assert_eq!(store.active_admission_reservations().len(), 1);
}

#[tokio::test]
async fn snapshot_load_drops_released_reservations_without_retained_run_records() {
    let source_store = Arc::new(InMemoryTurnStateStore::default());
    let source_coordinator = DefaultTurnCoordinator::new(source_store.clone());
    let run_id = accepted_run_id(
        &source_coordinator
            .submit_turn(submit_request("thread-a", "idem-submit-a"))
            .await
            .unwrap(),
    );
    let runner_id = TurnRunnerId::new();
    let lease_token = TurnLeaseToken::new();
    source_store
        .claim_next_run(ClaimRunRequest {
            runner_id,
            lease_token,
            scope_filter: Some(scope("thread-a")),
        })
        .await
        .unwrap()
        .unwrap();
    source_store
        .complete_run(CompleteRunRequest {
            run_id,
            runner_id,
            lease_token,
        })
        .await
        .unwrap();
    let mut snapshot = source_store.persistence_snapshot();
    assert_eq!(snapshot.admission_reservations.len(), 1);
    assert!(snapshot.admission_reservations[0].released);
    snapshot.runs.clear();

    let restored = InMemoryTurnStateStore::from_persistence_snapshot(
        snapshot,
        InMemoryTurnStateStoreLimits::default(),
    )
    .unwrap();

    assert!(
        restored
            .persistence_snapshot()
            .admission_reservations
            .is_empty()
    );
}

#[tokio::test]
async fn model_route_snapshot_persists_across_snapshot_restore_and_recovery() {
    let source_store = Arc::new(InMemoryTurnStateStore::default());
    let source_coordinator = DefaultTurnCoordinator::new(source_store.clone());
    let run_id = accepted_run_id(
        &source_coordinator
            .submit_turn(submit_request("thread-route", "idem-route"))
            .await
            .unwrap(),
    );
    let first_runner = TurnRunnerId::new();
    let first_lease = TurnLeaseToken::new();
    source_store
        .claim_next_run(ClaimRunRequest {
            runner_id: first_runner,
            lease_token: first_lease,
            scope_filter: Some(scope("thread-route")),
        })
        .await
        .unwrap()
        .unwrap();
    let snapshot = LoopModelRouteSnapshot::new(
        "openrouter",
        "anthropic/claude-sonnet-4",
        "config:v1",
        "auth:v1",
    );
    source_store
        .record_model_route_snapshot(RecordModelRouteSnapshotRequest {
            run_id,
            runner_id: first_runner,
            lease_token: first_lease,
            snapshot: snapshot.clone(),
        })
        .await
        .unwrap();

    let restored = Arc::new(
        InMemoryTurnStateStore::from_persistence_snapshot(
            source_store.persistence_snapshot(),
            InMemoryTurnStateStoreLimits::default(),
        )
        .unwrap(),
    );
    assert_eq!(
        restored
            .get_run_state(GetRunStateRequest {
                scope: scope("thread-route"),
                run_id,
            })
            .await
            .unwrap()
            .resolved_model_route,
        Some(snapshot.clone())
    );

    let recovered = restored
        .recover_expired_leases(RecoverExpiredLeasesRequest {
            now: Utc::now() + ChronoDuration::hours(1),
            scope_filter: Some(scope("thread-route")),
        })
        .await
        .unwrap();

    assert_eq!(recovered.recovered[0].resolved_model_route, Some(snapshot));
}

#[tokio::test]
async fn record_model_route_snapshot_rejects_secret_like_fields() {
    let store = Arc::new(InMemoryTurnStateStore::default());
    let coordinator = DefaultTurnCoordinator::new(store.clone());
    let run_id = accepted_run_id(
        &coordinator
            .submit_turn(submit_request("thread-route-secret", "idem-route-secret"))
            .await
            .unwrap(),
    );
    let runner_id = TurnRunnerId::new();
    let lease_token = TurnLeaseToken::new();
    store
        .claim_next_run(ClaimRunRequest {
            runner_id,
            lease_token,
            scope_filter: Some(scope("thread-route-secret")),
        })
        .await
        .unwrap()
        .unwrap();

    for snapshot in [
        LoopModelRouteSnapshot::new("sk-secret-provider", "gpt-4", "config:v1", "auth:v1"),
        LoopModelRouteSnapshot::new(
            "openrouter",
            "anthropic/secret-model",
            "config:v1",
            "auth:v1",
        ),
        LoopModelRouteSnapshot::new("openrouter", "gpt-4", "config:api_key", "auth:v1"),
        LoopModelRouteSnapshot::new("openrouter", "gpt-4", "config:v1", "auth:bearer"),
    ] {
        let error = store
            .record_model_route_snapshot(RecordModelRouteSnapshotRequest {
                run_id,
                runner_id,
                lease_token,
                snapshot,
            })
            .await
            .unwrap_err();
        assert!(matches!(error, TurnError::InvalidRequest { .. }));
    }

    assert_eq!(
        store
            .get_run_state(GetRunStateRequest {
                scope: scope("thread-route-secret"),
                run_id,
            })
            .await
            .unwrap()
            .resolved_model_route,
        None
    );
}

#[tokio::test]
async fn record_model_route_snapshot_is_idempotent_and_rejects_route_changes() {
    let store = Arc::new(InMemoryTurnStateStore::default());
    let coordinator = DefaultTurnCoordinator::new(store.clone());
    let run_id = accepted_run_id(
        &coordinator
            .submit_turn(submit_request(
                "thread-route-idempotent",
                "idem-route-idempotent",
            ))
            .await
            .unwrap(),
    );
    let runner_id = TurnRunnerId::new();
    let lease_token = TurnLeaseToken::new();
    store
        .claim_next_run(ClaimRunRequest {
            runner_id,
            lease_token,
            scope_filter: Some(scope("thread-route-idempotent")),
        })
        .await
        .unwrap()
        .unwrap();

    let initial = LoopModelRouteSnapshot::new(
        "openrouter",
        "anthropic/claude-sonnet-4",
        "config:v1",
        "auth:v1",
    );
    let first = store
        .record_model_route_snapshot(RecordModelRouteSnapshotRequest {
            run_id,
            runner_id,
            lease_token,
            snapshot: initial.clone(),
        })
        .await
        .unwrap();
    assert_eq!(first.resolved_model_route, Some(initial.clone()));

    let replay = store
        .record_model_route_snapshot(RecordModelRouteSnapshotRequest {
            run_id,
            runner_id,
            lease_token,
            snapshot: initial.clone(),
        })
        .await
        .unwrap();
    assert_eq!(replay.resolved_model_route, Some(initial.clone()));

    let replacement = LoopModelRouteSnapshot::new("nearai", "qwen3-coder", "config:v2", "auth:v2");
    let error = store
        .record_model_route_snapshot(RecordModelRouteSnapshotRequest {
            run_id,
            runner_id,
            lease_token,
            snapshot: replacement,
        })
        .await
        .unwrap_err();
    assert!(matches!(error, TurnError::Conflict { .. }));
    assert_eq!(
        store
            .get_run_state(GetRunStateRequest {
                scope: scope("thread-route-idempotent"),
                run_id,
            })
            .await
            .unwrap()
            .resolved_model_route,
        Some(initial)
    );
}

#[tokio::test]
async fn terminal_record_pruning_bounds_released_admission_reservations() {
    let store = Arc::new(InMemoryTurnStateStore::with_limits(
        InMemoryTurnStateStoreLimits {
            max_terminal_records: 1,
            ..InMemoryTurnStateStoreLimits::default()
        },
    ));
    let coordinator = DefaultTurnCoordinator::new(store.clone());
    let first_run_id = accepted_run_id(
        &coordinator
            .submit_turn(submit_request("thread-a", "idem-submit-a"))
            .await
            .unwrap(),
    );
    let first_runner_id = TurnRunnerId::new();
    let first_lease_token = TurnLeaseToken::new();
    store
        .claim_next_run(ClaimRunRequest {
            runner_id: first_runner_id,
            lease_token: first_lease_token,
            scope_filter: Some(scope("thread-a")),
        })
        .await
        .unwrap()
        .unwrap();
    store
        .complete_run(CompleteRunRequest {
            run_id: first_run_id,
            runner_id: first_runner_id,
            lease_token: first_lease_token,
        })
        .await
        .unwrap();

    let second_run_id = accepted_run_id(
        &coordinator
            .submit_turn(submit_request("thread-b", "idem-submit-b"))
            .await
            .unwrap(),
    );
    let second_runner_id = TurnRunnerId::new();
    let second_lease_token = TurnLeaseToken::new();
    store
        .claim_next_run(ClaimRunRequest {
            runner_id: second_runner_id,
            lease_token: second_lease_token,
            scope_filter: Some(scope("thread-b")),
        })
        .await
        .unwrap()
        .unwrap();
    store
        .complete_run(CompleteRunRequest {
            run_id: second_run_id,
            runner_id: second_runner_id,
            lease_token: second_lease_token,
        })
        .await
        .unwrap();

    let snapshot = store.persistence_snapshot();
    assert!(snapshot.runs.iter().all(|run| run.run_id != first_run_id));
    assert!(
        snapshot
            .admission_reservations
            .iter()
            .all(|reservation| reservation.run_id != first_run_id)
    );
    assert_eq!(snapshot.admission_reservations.len(), 1);
}

#[tokio::test]
async fn actor_user_capacity_uses_submitting_actor_not_thread_scope() {
    let limits = StaticTurnAdmissionLimitProvider::default()
        .with_total_limit(TurnAdmissionAxisKind::ActorUser, 1);
    let store = Arc::new(InMemoryTurnStateStore::with_admission_limit_provider(
        Arc::new(limits),
    ));
    let coordinator = DefaultTurnCoordinator::new(store.clone());
    coordinator
        .submit_turn(submit_request("thread-a", "idem-submit-a"))
        .await
        .unwrap();

    let same_actor_denied = coordinator
        .submit_turn(submit_request("thread-b", "idem-submit-b"))
        .await
        .unwrap_err();
    assert!(matches!(
        same_actor_denied,
        TurnError::AdmissionRejected(AdmissionRejection {
            capacity_denial: Some(TurnAdmissionCapacityDenial {
                axis_kind: TurnAdmissionAxisKind::ActorUser,
                ..
            }),
            ..
        })
    ));

    let mut different_actor = submit_request("thread-c", "idem-submit-c");
    different_actor.actor = TurnActor::new(UserId::new("user2").unwrap());
    coordinator.submit_turn(different_actor).await.unwrap();
    assert_eq!(store.active_admission_reservations().len(), 2);
}

#[tokio::test]
async fn class_capacity_denial_reports_admission_class() {
    let limits = StaticTurnAdmissionLimitProvider::default().with_class_limit(
        TurnAdmissionAxisKind::Tenant,
        TurnAdmissionClass::interactive(),
        1,
    );
    let store = Arc::new(InMemoryTurnStateStore::with_admission_limit_provider(
        Arc::new(limits),
    ));
    let coordinator = DefaultTurnCoordinator::new(store);
    coordinator
        .submit_turn(submit_request("thread-a", "idem-submit-a"))
        .await
        .unwrap();

    let denied = coordinator
        .submit_turn(submit_request("thread-b", "idem-submit-b"))
        .await
        .unwrap_err();
    assert!(matches!(
        denied,
        TurnError::AdmissionRejected(AdmissionRejection {
            capacity_denial: Some(TurnAdmissionCapacityDenial {
                axis_kind: TurnAdmissionAxisKind::Tenant,
                bucket_kind: TurnAdmissionBucketKind::Class,
                admission_class: Some(ref class),
                limit: 1,
                active_count: 1,
                ..
            }),
            ..
        }) if class == &TurnAdmissionClass::interactive()
    ));
}

#[tokio::test]
async fn accepted_submit_reserves_unlimited_buckets_and_replay_does_not_reacquire() {
    let (coordinator, store) = coordinator();
    let first = coordinator
        .submit_turn(submit_request("thread-a", "idem-submit-a"))
        .await
        .unwrap();
    let duplicate = coordinator
        .submit_turn(submit_request("thread-a", "idem-submit-a"))
        .await
        .unwrap();

    assert_eq!(duplicate, first);
    let reservations = store.active_admission_reservations();
    assert_eq!(reservations.len(), 1);
    assert_eq!(reservations[0].buckets.len(), 8);
    assert_eq!(
        reservations[0].admission_class,
        TurnAdmissionClass::interactive()
    );
}

#[tokio::test]
async fn project_none_consumes_unscoped_project_bucket() {
    let limits = StaticTurnAdmissionLimitProvider::default()
        .with_total_limit(TurnAdmissionAxisKind::Project, 1);
    let store = Arc::new(InMemoryTurnStateStore::with_admission_limit_provider(
        Arc::new(limits),
    ));
    let coordinator = DefaultTurnCoordinator::new(store.clone());
    let mut first = submit_request("thread-a", "idem-submit-a");
    first.scope.project_id = None;
    coordinator.submit_turn(first).await.unwrap();
    let mut second = submit_request("thread-b", "idem-submit-b");
    second.scope.project_id = None;

    let denied = coordinator.submit_turn(second).await.unwrap_err();
    assert!(matches!(
        denied,
        TurnError::AdmissionRejected(AdmissionRejection {
            capacity_denial: Some(TurnAdmissionCapacityDenial {
                axis_kind: TurnAdmissionAxisKind::Project,
                bucket_kind: TurnAdmissionBucketKind::Total,
                ..
            }),
            ..
        })
    ));
    assert!(
        store.active_admission_reservations()[0]
            .buckets
            .iter()
            .any(|bucket| matches!(
                bucket.scope,
                TurnAdmissionBucketScope::Project {
                    project_id: None,
                    ..
                }
            ))
    );
}

#[tokio::test]
async fn agent_capacity_is_keyed_by_tenant_and_optional_agent() {
    let limits = StaticTurnAdmissionLimitProvider::default()
        .with_total_limit(TurnAdmissionAxisKind::Agent, 1);
    let store = Arc::new(InMemoryTurnStateStore::with_admission_limit_provider(
        Arc::new(limits),
    ));
    let coordinator = DefaultTurnCoordinator::new(store.clone());
    let first = submit_request("thread-a", "idem-submit-a");
    coordinator.submit_turn(first).await.unwrap();
    let denied = coordinator
        .submit_turn(submit_request("thread-b", "idem-submit-b"))
        .await
        .unwrap_err();
    assert!(matches!(
        denied,
        TurnError::AdmissionRejected(AdmissionRejection {
            capacity_denial: Some(TurnAdmissionCapacityDenial {
                axis_kind: TurnAdmissionAxisKind::Agent,
                ..
            }),
            ..
        })
    ));

    let mut no_agent = submit_request("thread-c", "idem-submit-c");
    no_agent.scope.agent_id = None;
    coordinator.submit_turn(no_agent).await.unwrap();
    assert_eq!(store.active_admission_reservations().len(), 2);
}

#[tokio::test]
async fn admission_limit_provider_unavailable_fails_closed_without_run_or_reservation() {
    let limits = StaticTurnAdmissionLimitProvider::default().unavailable();
    let store = Arc::new(InMemoryTurnStateStore::with_admission_limit_provider(
        Arc::new(limits),
    ));
    let coordinator = DefaultTurnCoordinator::new(store.clone());

    let denied = coordinator
        .submit_turn(submit_request("thread-a", "idem-submit-a"))
        .await
        .unwrap_err();

    assert_eq!(
        denied,
        TurnError::AdmissionRejected(AdmissionRejection::new(
            AdmissionRejectionReason::Unavailable
        ))
    );
    let snapshot = store.persistence_snapshot();
    assert!(snapshot.turns.is_empty());
    assert!(snapshot.runs.is_empty());
    assert!(snapshot.admission_reservations.is_empty());
}

#[tokio::test]
async fn blocked_resume_and_recovery_required_keep_existing_admission_reservation() {
    let limits = StaticTurnAdmissionLimitProvider::default()
        .with_total_limit(TurnAdmissionAxisKind::Tenant, 1);
    let store = Arc::new(InMemoryTurnStateStore::with_admission_limit_provider(
        Arc::new(limits),
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
            scope_filter: Some(scope("thread-a")),
        })
        .await
        .unwrap()
        .unwrap();
    let gate_ref = GateRef::new("gate:approval").unwrap();
    store
        .block_run(BlockRunRequest {
            run_id,
            runner_id,
            lease_token,
            reason: BlockedReason::Approval {
                gate_ref: gate_ref.clone(),
            },
            checkpoint_id: TurnCheckpointId::new(),
            state_ref: block_state_ref(),
        })
        .await
        .unwrap();
    assert_eq!(store.active_admission_reservations().len(), 1);

    let resumed = coordinator
        .resume_turn(ResumeTurnRequest {
            scope: scope("thread-a"),
            actor: actor(),
            run_id,
            gate_resolution_ref: gate_ref,
            precondition: ironclaw_turns::ResumeTurnPrecondition::AnyBlockedGate,
            source_binding_ref: SourceBindingRef::new("source-web-resumed").unwrap(),
            reply_target_binding_ref: ReplyTargetBindingRef::new("reply-web-resumed").unwrap(),
            idempotency_key: IdempotencyKey::new("idem-resume-a").unwrap(),
        })
        .await
        .unwrap();
    assert_eq!(resumed.status, TurnStatus::Queued);
    assert_eq!(store.active_admission_reservations().len(), 1);

    store
        .claim_next_run(ClaimRunRequest {
            runner_id,
            lease_token,
            scope_filter: Some(scope("thread-a")),
        })
        .await
        .unwrap()
        .unwrap();
    store
        .record_recovery_required(ironclaw_turns::runner::RecordRecoveryRequiredRequest {
            run_id,
            runner_id,
            lease_token,
            failure: SanitizedFailure::new("driver_protocol_violation").unwrap(),
        })
        .await
        .unwrap();
    assert_eq!(store.active_admission_reservations().len(), 1);
}

#[tokio::test]
async fn submit_turn_busy_path_is_transient_without_new_run_or_idempotency_record() {
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

    let snapshot = store.persistence_snapshot();
    assert_eq!(snapshot.turns.len(), 1);
    assert_eq!(snapshot.runs.len(), 1);
    assert!(
        snapshot
            .idempotency_records
            .iter()
            .all(|record| record.key != busy_request.idempotency_key),
        "busy submit is transient and must not pin admission/idempotency capacity"
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
        .complete_run(CompleteRunRequest {
            run_id: first_run_id,
            runner_id,
            lease_token,
        })
        .await
        .unwrap();

    let accepted_after_unlock = coordinator.submit_turn(busy_request).await.unwrap();
    assert_ne!(accepted_run_id(&accepted_after_unlock), first_run_id);
    assert_eq!(store.persistence_snapshot().turns.len(), 2);
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
    let state_ref = LoopCheckpointStateRef::new("checkpoint:requested-block-state").unwrap();
    store
        .block_run(BlockRunRequest {
            run_id,
            runner_id,
            lease_token,
            checkpoint_id,
            state_ref: state_ref.clone(),
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
    assert_eq!(checkpoint.state_ref, state_ref);
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
            state_ref: block_state_ref(),
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
        precondition: ironclaw_turns::ResumeTurnPrecondition::AnyBlockedGate,
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
async fn persisted_admission_rejection_and_cancel_replay_envelopes_are_reconstructable() {
    let store = Arc::new(InMemoryTurnStateStore::default());
    let coordinator =
        DefaultTurnCoordinator::new(store.clone()).with_admission_policy(Arc::new(DenyAll));
    let rejected_request = submit_request("thread-a", "idem-submit-rejected");
    let rejected = coordinator
        .submit_turn(rejected_request.clone())
        .await
        .unwrap_err();

    let allowed_coordinator = DefaultTurnCoordinator::new(store.clone());
    let first_run_id = accepted_run_id(
        &allowed_coordinator
            .submit_turn(submit_request("thread-a", "idem-submit-a"))
            .await
            .unwrap(),
    );
    let cancel = allowed_coordinator
        .cancel_run(cancel_request(
            "thread-a",
            first_run_id,
            "idem-cancel-running-a",
        ))
        .await
        .unwrap();

    let snapshot = store.persistence_snapshot();
    let rejection_record = snapshot
        .idempotency_records
        .iter()
        .find(|record| record.key == rejected_request.idempotency_key)
        .expect("admission rejection idempotency record must be persisted");
    assert_eq!(rejection_record.replay_submit().unwrap(), Err(rejected));
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
async fn submit_turn_busy_idempotency_does_not_replay_after_thread_unlocks() {
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

    let accepted_after_unlock = coordinator.submit_turn(busy_request).await.unwrap();
    assert_ne!(accepted_run_id(&accepted_after_unlock), first_run_id);
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

#[test]
fn cancelled_submit_during_profile_resolution_clears_in_flight_idempotency() {
    let store = Arc::new(InMemoryTurnStateStore::default());
    let (started_tx, started_rx) = mpsc::channel();
    let resolver = Arc::new(BlockingRunProfileResolver::new(started_tx));
    let request = submit_request("thread-a", "idem-submit-cancelled-profile-resolution");
    let mut pending = store.submit_turn(
        request.clone(),
        &AllowAllTurnAdmissionPolicy,
        resolver.as_ref(),
    );
    let waker = std::task::Waker::noop();
    let mut context = Context::from_waker(waker);
    assert!(matches!(pending.as_mut().poll(&mut context), Poll::Pending));
    started_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("first submit should start profile resolution");
    drop(pending);

    let retry_store = store.clone();
    let (retry_tx, retry_rx) = mpsc::channel();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let retry = runtime.block_on(retry_store.submit_turn(
            request,
            &AllowAllTurnAdmissionPolicy,
            &InMemoryRunProfileResolver::default(),
        ));
        retry_tx.send(retry).unwrap();
    });
    let retry = retry_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("retry should not wait on a stale in-flight idempotency key")
        .unwrap();

    assert!(matches!(retry, SubmitTurnResponse::Accepted { .. }));
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
            state_ref: block_state_ref(),
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
            precondition: ironclaw_turns::ResumeTurnPrecondition::AnyBlockedGate,
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
            state_ref: block_state_ref(),
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
            precondition: ironclaw_turns::ResumeTurnPrecondition::AnyBlockedGate,
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
async fn admission_policy_rejection_wins_over_busy_thread_metadata() {
    let (coordinator, store) = coordinator();
    coordinator
        .submit_turn(submit_request("thread-a", "idem-submit-a"))
        .await
        .unwrap();
    let unauthorized_coordinator = DefaultTurnCoordinator::new(store.clone())
        .with_admission_policy(Arc::new(DenyUnauthorized));

    let err = unauthorized_coordinator
        .submit_turn(submit_request("thread-a", "idem-submit-unauthorized-busy"))
        .await
        .unwrap_err();

    assert_eq!(
        err,
        TurnError::AdmissionRejected(AdmissionRejection::new(
            AdmissionRejectionReason::Unauthorized
        ))
    );
    assert_eq!(store.persistence_snapshot().turns.len(), 1);
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
async fn cancel_requested_runner_heartbeat_does_not_extend_lease() {
    let limits = InMemoryTurnStateStoreLimits {
        runner_lease_ttl: ChronoDuration::milliseconds(40),
        ..InMemoryTurnStateStoreLimits::default()
    };
    let store = Arc::new(InMemoryTurnStateStore::with_limits(limits));
    let coordinator = DefaultTurnCoordinator::new(store.clone());
    let request = submit_request("thread-cancel-heartbeat", "idem-submit-cancel-heartbeat");
    let scope = request.scope.clone();
    let run_id = accepted_run_id(&coordinator.submit_turn(request).await.unwrap());
    let runner_id = TurnRunnerId::new();
    let lease_token = TurnLeaseToken::new();
    store
        .claim_next_run(ClaimRunRequest {
            runner_id,
            lease_token,
            scope_filter: Some(scope.clone()),
        })
        .await
        .unwrap()
        .unwrap();
    coordinator
        .cancel_run(CancelRunRequest {
            scope,
            actor: TurnActor::new(UserId::new("user1").unwrap()),
            run_id,
            reason: SanitizedCancelReason::UserRequested,
            idempotency_key: IdempotencyKey::new("idem-cancel-heartbeat").unwrap(),
        })
        .await
        .unwrap();

    let heartbeat = store
        .heartbeat(HeartbeatRequest {
            run_id,
            runner_id,
            lease_token,
        })
        .await
        .unwrap_err();
    assert_eq!(
        heartbeat,
        TurnError::InvalidTransition {
            from: TurnStatus::CancelRequested,
            to: TurnStatus::Running,
        }
    );

    std::thread::sleep(Duration::from_millis(60));
    let recovered = store
        .recover_expired_leases(RecoverExpiredLeasesRequest {
            now: Utc::now(),
            scope_filter: None,
        })
        .await
        .unwrap();
    assert_eq!(recovered.recovered.len(), 1);
    assert_eq!(recovered.recovered[0].status, TurnStatus::RecoveryRequired);
}

#[tokio::test]
async fn expired_runner_lease_rejects_heartbeat_and_terminal_completion_before_recovery_sweep() {
    let limits = InMemoryTurnStateStoreLimits {
        runner_lease_ttl: ChronoDuration::milliseconds(-1),
        ..InMemoryTurnStateStoreLimits::default()
    };
    let store = Arc::new(InMemoryTurnStateStore::with_limits(limits));
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

    let heartbeat = store
        .heartbeat(HeartbeatRequest {
            run_id,
            runner_id,
            lease_token,
        })
        .await
        .unwrap_err();
    assert_eq!(
        heartbeat,
        TurnError::Conflict {
            reason: "turn run lease expired".to_string(),
        }
    );

    let completed = store
        .complete_run(CompleteRunRequest {
            run_id,
            runner_id,
            lease_token,
        })
        .await
        .unwrap_err();
    assert_eq!(
        completed,
        TurnError::Conflict {
            reason: "turn run lease expired".to_string(),
        }
    );

    let state = coordinator
        .get_run_state(GetRunStateRequest {
            scope: scope("thread-a"),
            run_id,
        })
        .await
        .unwrap();
    assert_eq!(state.status, TurnStatus::Running);
}

#[tokio::test]
async fn expired_runner_lease_rejects_fail_and_runner_side_cancel_before_recovery_sweep() {
    let limits = InMemoryTurnStateStoreLimits {
        runner_lease_ttl: ChronoDuration::milliseconds(-1),
        ..InMemoryTurnStateStoreLimits::default()
    };
    let store = Arc::new(InMemoryTurnStateStore::with_limits(limits));
    let coordinator = DefaultTurnCoordinator::new(store.clone());

    let failed_run_id = accepted_run_id(
        &coordinator
            .submit_turn(submit_request("thread-a", "idem-submit-a"))
            .await
            .unwrap(),
    );
    let fail_runner_id = TurnRunnerId::new();
    let fail_lease_token = TurnLeaseToken::new();
    store
        .claim_next_run(ClaimRunRequest {
            runner_id: fail_runner_id,
            lease_token: fail_lease_token,
            scope_filter: Some(scope("thread-a")),
        })
        .await
        .unwrap()
        .unwrap();

    let failed = store
        .fail_run(FailRunRequest {
            run_id: failed_run_id,
            runner_id: fail_runner_id,
            lease_token: fail_lease_token,
            failure: SanitizedFailure::new("late_failure").unwrap(),
        })
        .await
        .unwrap_err();
    assert_eq!(
        failed,
        TurnError::Conflict {
            reason: "turn run lease expired".to_string(),
        }
    );

    let cancelled_run_id = accepted_run_id(
        &coordinator
            .submit_turn(submit_request("thread-b", "idem-submit-b"))
            .await
            .unwrap(),
    );
    let cancel_runner_id = TurnRunnerId::new();
    let cancel_lease_token = TurnLeaseToken::new();
    store
        .claim_next_run(ClaimRunRequest {
            runner_id: cancel_runner_id,
            lease_token: cancel_lease_token,
            scope_filter: Some(scope("thread-b")),
        })
        .await
        .unwrap()
        .unwrap();
    coordinator
        .cancel_run(cancel_request(
            "thread-b",
            cancelled_run_id,
            "idem-cancel-b",
        ))
        .await
        .unwrap();

    let cancelled = store
        .cancel_run(CancelRunCompletionRequest {
            run_id: cancelled_run_id,
            runner_id: cancel_runner_id,
            lease_token: cancel_lease_token,
        })
        .await
        .unwrap_err();
    assert_eq!(
        cancelled,
        TurnError::Conflict {
            reason: "turn run lease expired".to_string(),
        }
    );
}

#[tokio::test]
async fn runner_claim_and_heartbeat_persist_lease_expiry() {
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
    let first_heartbeat_at = run
        .last_heartbeat_at
        .expect("claim should record heartbeat timestamp");
    let first_expiry = run
        .lease_expires_at
        .expect("claim should record lease expiry");
    assert!(first_expiry > first_heartbeat_at);

    std::thread::sleep(Duration::from_millis(2));
    store
        .heartbeat(HeartbeatRequest {
            run_id,
            runner_id,
            lease_token,
        })
        .await
        .unwrap();

    let heartbeat = store.persistence_snapshot();
    let run = heartbeat
        .runs
        .iter()
        .find(|record| record.run_id == run_id)
        .unwrap();
    assert!(run.last_heartbeat_at.unwrap() > first_heartbeat_at);
    assert!(run.lease_expires_at.unwrap() > first_expiry);
}

#[tokio::test]
async fn expired_running_lease_enters_recovery_required_and_keeps_thread_locked() {
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
    let lease_expires_at = store
        .persistence_snapshot()
        .runs
        .iter()
        .find(|record| record.run_id == run_id)
        .unwrap()
        .lease_expires_at
        .unwrap();

    let not_yet_expired = store
        .recover_expired_leases(RecoverExpiredLeasesRequest {
            now: lease_expires_at - ChronoDuration::milliseconds(1),
            scope_filter: None,
        })
        .await
        .unwrap();
    assert!(not_yet_expired.recovered.is_empty());

    let recovered = store
        .recover_expired_leases(RecoverExpiredLeasesRequest {
            now: lease_expires_at + ChronoDuration::milliseconds(1),
            scope_filter: None,
        })
        .await
        .unwrap();
    assert_eq!(recovered.recovered.len(), 1);
    assert_eq!(recovered.recovered[0].run_id, run_id);
    assert_eq!(recovered.recovered[0].status, TurnStatus::RecoveryRequired);

    let snapshot = store.persistence_snapshot();
    let run = snapshot
        .runs
        .iter()
        .find(|record| record.run_id == run_id)
        .unwrap();
    assert_eq!(run.status, TurnStatus::RecoveryRequired);
    assert_eq!(run.runner_id, None);
    assert_eq!(run.lease_token, None);
    assert_eq!(run.lease_expires_at, None);
    let lock = snapshot
        .active_locks
        .iter()
        .find(|lock| lock.run_id == run_id)
        .unwrap();
    assert_eq!(lock.status, TurnStatus::RecoveryRequired);

    let busy = coordinator
        .submit_turn(submit_request("thread-a", "idem-submit-after-recovery"))
        .await
        .unwrap_err();
    assert!(matches!(
        busy,
        TurnError::ThreadBusy(ThreadBusy {
            status: TurnStatus::RecoveryRequired,
            ..
        })
    ));
    assert!(
        store
            .claim_next_run(ClaimRunRequest {
                runner_id: TurnRunnerId::new(),
                lease_token: TurnLeaseToken::new(),
                scope_filter: None,
            })
            .await
            .unwrap()
            .is_none(),
        "recovery-required work must not be auto-retried by the normal claim path"
    );
    assert!(store.events().iter().any(|event| {
        event.run_id == run_id
            && event.kind == TurnEventKind::RecoveryRequired
            && event.sanitized_reason.as_deref() == Some("lease_expired")
    }));
}

#[tokio::test]
async fn expired_cancel_requested_lease_enters_recovery_required_and_can_be_cancelled() {
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
    let lease_expires_at = store
        .persistence_snapshot()
        .runs
        .iter()
        .find(|record| record.run_id == run_id)
        .unwrap()
        .lease_expires_at
        .unwrap();

    let cancel = coordinator
        .cancel_run(cancel_request("thread-a", run_id, "idem-cancel-a"))
        .await
        .unwrap();
    assert_eq!(cancel.status, TurnStatus::CancelRequested);

    let recovered = store
        .recover_expired_leases(RecoverExpiredLeasesRequest {
            now: lease_expires_at + ChronoDuration::milliseconds(1),
            scope_filter: None,
        })
        .await
        .unwrap();
    assert_eq!(recovered.recovered.len(), 1);
    assert_eq!(recovered.recovered[0].run_id, run_id);
    assert_eq!(recovered.recovered[0].status, TurnStatus::RecoveryRequired);

    let snapshot = store.persistence_snapshot();
    let run = snapshot
        .runs
        .iter()
        .find(|record| record.run_id == run_id)
        .unwrap();
    assert_eq!(run.status, TurnStatus::RecoveryRequired);
    assert_eq!(run.runner_id, None);
    assert_eq!(run.lease_token, None);
    assert_eq!(run.lease_expires_at, None);
    let busy = coordinator
        .submit_turn(submit_request(
            "thread-a",
            "idem-submit-after-cancel-recovery",
        ))
        .await
        .unwrap_err();
    assert!(matches!(busy, TurnError::ThreadBusy(_)));

    let cancelled = coordinator
        .cancel_run(cancel_request("thread-a", run_id, "idem-cancel-recovered"))
        .await
        .unwrap();
    assert_eq!(cancelled.status, TurnStatus::Cancelled);
    assert!(store.persistence_snapshot().active_locks.is_empty());

    let replacement = coordinator
        .submit_turn(submit_request(
            "thread-a",
            "idem-submit-after-recovered-cancel",
        ))
        .await
        .unwrap();
    assert!(matches!(replacement, SubmitTurnResponse::Accepted { .. }));
}

#[tokio::test]
async fn cancel_recovery_required_run_releases_lock_and_allows_new_submit() {
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
    let lease_expires_at = store
        .persistence_snapshot()
        .runs
        .iter()
        .find(|record| record.run_id == run_id)
        .unwrap()
        .lease_expires_at
        .unwrap();
    store
        .recover_expired_leases(RecoverExpiredLeasesRequest {
            now: lease_expires_at + ChronoDuration::milliseconds(1),
            scope_filter: None,
        })
        .await
        .unwrap();

    let cancelled = coordinator
        .cancel_run(CancelRunRequest {
            scope: scope("thread-a"),
            actor: actor(),
            run_id,
            reason: SanitizedCancelReason::OperatorRequested,
            idempotency_key: IdempotencyKey::new("idem-cancel-recovered").unwrap(),
        })
        .await
        .unwrap();
    assert_eq!(cancelled.status, TurnStatus::Cancelled);
    assert!(!cancelled.already_terminal);
    let snapshot = store.persistence_snapshot();
    assert!(snapshot.active_locks.is_empty());
    let run = snapshot
        .runs
        .iter()
        .find(|record| record.run_id == run_id)
        .unwrap();
    assert_eq!(run.failure, None);

    let replacement = coordinator
        .submit_turn(submit_request("thread-a", "idem-submit-replacement"))
        .await
        .unwrap();
    assert!(matches!(replacement, SubmitTurnResponse::Accepted { .. }));
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
            state_ref: block_state_ref(),
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
        precondition: ironclaw_turns::ResumeTurnPrecondition::BlockedApprovalGate,
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
async fn resume_turn_rejects_unexpected_blocked_status_without_requeueing_run() {
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
            state_ref: block_state_ref(),
            reason: BlockedReason::Approval {
                gate_ref: gate_ref.clone(),
            },
        })
        .await
        .unwrap();

    let err = coordinator
        .resume_turn(ResumeTurnRequest {
            scope: scope("thread-a"),
            actor: actor(),
            run_id,
            gate_resolution_ref: gate_ref.clone(),
            precondition: ironclaw_turns::ResumeTurnPrecondition::BlockedAuthGate,
            source_binding_ref: SourceBindingRef::new("source-web").unwrap(),
            reply_target_binding_ref: ReplyTargetBindingRef::new("reply-web").unwrap(),
            idempotency_key: IdempotencyKey::new("idem-resume-wrong-status").unwrap(),
        })
        .await
        .unwrap_err();

    assert_eq!(
        err,
        TurnError::InvalidTransition {
            from: TurnStatus::BlockedApproval,
            to: TurnStatus::Queued,
        }
    );
    let state = coordinator
        .get_run_state(GetRunStateRequest {
            scope: scope("thread-a"),
            run_id,
        })
        .await
        .unwrap();
    assert_eq!(state.status, TurnStatus::BlockedApproval);
    assert_eq!(state.gate_ref, Some(gate_ref));
}

#[tokio::test]
async fn resume_turn_from_foreign_actor_is_denied_without_requeueing_run() {
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
            state_ref: block_state_ref(),
            reason: BlockedReason::Approval {
                gate_ref: gate_ref.clone(),
            },
        })
        .await
        .unwrap();

    let err = coordinator
        .resume_turn(ResumeTurnRequest {
            scope: scope("thread-a"),
            actor: TurnActor::new(UserId::new("user2").unwrap()),
            run_id,
            gate_resolution_ref: gate_ref.clone(),
            precondition: ironclaw_turns::ResumeTurnPrecondition::AnyBlockedGate,
            source_binding_ref: SourceBindingRef::new("source-web").unwrap(),
            reply_target_binding_ref: ReplyTargetBindingRef::new("reply-web").unwrap(),
            idempotency_key: IdempotencyKey::new("idem-resume-foreign-actor").unwrap(),
        })
        .await
        .unwrap_err();

    assert_eq!(err, TurnError::Unauthorized);
    assert_eq!(err.adapter_status_code(), 403);
    let snapshot = store.persistence_snapshot();
    let run = snapshot
        .runs
        .iter()
        .find(|record| record.run_id == run_id)
        .unwrap();
    assert_eq!(run.status, TurnStatus::BlockedApproval);
    assert_eq!(run.gate_ref, Some(gate_ref));
    assert!(
        store
            .claim_next_run(ClaimRunRequest {
                runner_id: TurnRunnerId::new(),
                lease_token: TurnLeaseToken::new(),
                scope_filter: Some(scope("thread-a")),
            })
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn cancel_run_from_foreign_actor_is_denied_without_mutating_run() {
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
            state_ref: block_state_ref(),
            reason: BlockedReason::Approval {
                gate_ref: gate_ref.clone(),
            },
        })
        .await
        .unwrap();

    let err = coordinator
        .cancel_run(CancelRunRequest {
            scope: scope("thread-a"),
            actor: TurnActor::new(UserId::new("user2").unwrap()),
            run_id,
            reason: SanitizedCancelReason::UserRequested,
            idempotency_key: IdempotencyKey::new("idem-cancel-foreign-actor").unwrap(),
        })
        .await
        .unwrap_err();

    assert_eq!(err, TurnError::Unauthorized);
    assert_eq!(err.adapter_status_code(), 403);
    let state = coordinator
        .get_run_state(GetRunStateRequest {
            scope: scope("thread-a"),
            run_id,
        })
        .await
        .unwrap();
    assert_eq!(state.status, TurnStatus::BlockedApproval);
    assert_eq!(state.gate_ref, Some(gate_ref));

    let cancelled = coordinator
        .cancel_run(CancelRunRequest {
            scope: scope("thread-a"),
            actor: TurnActor::new(UserId::new("user1").unwrap()),
            run_id,
            reason: SanitizedCancelReason::UserRequested,
            idempotency_key: IdempotencyKey::new("idem-cancel-owner").unwrap(),
        })
        .await
        .unwrap();
    assert_eq!(cancelled.status, TurnStatus::Cancelled);
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
            state_ref: block_state_ref(),
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
            precondition: ironclaw_turns::ResumeTurnPrecondition::AnyBlockedGate,
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
            state_ref: block_state_ref(),
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
    assert!(serde_json::from_str::<TurnAdmissionClass>("\"interactive\"").is_ok());
    assert!(serde_json::from_str::<TurnAdmissionClass>("\"\"").is_err());
    assert!(serde_json::from_str::<TurnAdmissionClass>("\"Interactive\"").is_err());
    assert!(serde_json::from_str::<TurnAdmissionClass>("\"admin-system\"").is_err());
    assert!(serde_json::from_str::<TurnAdmissionClass>("\"class\\nsecret\"").is_err());
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

fn assert_no_forbidden_turn_event_content(label: &str, serialized: &str, forbidden: &[&str]) {
    for value in forbidden {
        assert!(
            !serialized.contains(value),
            "{label} leaked forbidden marker {value}"
        );
    }
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

fn block_state_ref() -> LoopCheckpointStateRef {
    LoopCheckpointStateRef::new("checkpoint:block-state").unwrap()
}

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

#[derive(Default)]
struct RecordingWakeNotifier {
    wakes: Mutex<Vec<TurnRunWake>>,
}

impl RecordingWakeNotifier {
    fn wakes(&self) -> Vec<TurnRunWake> {
        self.wakes.lock().unwrap().clone()
    }

    fn clear(&self) {
        self.wakes.lock().unwrap().clear();
    }
}

impl TurnRunWakeNotifier for RecordingWakeNotifier {
    fn notify_queued_run(&self, wake: TurnRunWake) -> Result<(), TurnRunWakeNotifyError> {
        self.wakes.lock().unwrap().push(wake);
        Ok(())
    }
}

struct FailingWakeNotifier;

impl TurnRunWakeNotifier for FailingWakeNotifier {
    fn notify_queued_run(&self, _wake: TurnRunWake) -> Result<(), TurnRunWakeNotifyError> {
        Err(TurnRunWakeNotifyError::DeliveryUnavailable)
    }
}

struct PanickingWakeNotifier;

impl TurnRunWakeNotifier for PanickingWakeNotifier {
    fn notify_queued_run(&self, _wake: TurnRunWake) -> Result<(), TurnRunWakeNotifyError> {
        panic!("test wake notifier panic")
    }
}

struct AtomicLoopExitPort {
    state: Mutex<TurnStatus>,
}

#[async_trait::async_trait]
impl TurnRunTransitionPort for AtomicLoopExitPort {
    async fn claim_next_run(
        &self,
        _request: ClaimRunRequest,
    ) -> Result<Option<ClaimedTurnRun>, TurnError> {
        panic!("cancelled loop-exit application must not claim runs")
    }

    async fn heartbeat(&self, _request: HeartbeatRequest) -> Result<EventCursor, TurnError> {
        panic!("cancelled loop-exit application must not heartbeat")
    }

    async fn recover_expired_leases(
        &self,
        _request: RecoverExpiredLeasesRequest,
    ) -> Result<RecoverExpiredLeasesResponse, TurnError> {
        panic!("cancelled loop-exit application must not recover leases")
    }

    async fn record_model_route_snapshot(
        &self,
        _request: RecordModelRouteSnapshotRequest,
    ) -> Result<TurnRunState, TurnError> {
        panic!("cancelled loop-exit application must not record model route snapshots")
    }

    async fn block_run(&self, _request: BlockRunRequest) -> Result<TurnRunState, TurnError> {
        panic!("cancelled loop-exit application must not block runs")
    }

    async fn complete_run(&self, _request: CompleteRunRequest) -> Result<TurnRunState, TurnError> {
        panic!("cancelled loop-exit application must not complete runs")
    }

    async fn cancel_run(
        &self,
        _request: CancelRunCompletionRequest,
    ) -> Result<TurnRunState, TurnError> {
        panic!("cancelled loop-exit application must use atomic cancelled-exit transition")
    }

    async fn fail_run(&self, _request: FailRunRequest) -> Result<TurnRunState, TurnError> {
        panic!("cancelled loop-exit application must not fail runs")
    }

    async fn record_recovery_required(
        &self,
        _request: ironclaw_turns::runner::RecordRecoveryRequiredRequest,
    ) -> Result<TurnRunState, TurnError> {
        panic!("cancelled loop-exit application must not use a separate recovery transition")
    }

    async fn apply_validated_loop_exit(
        &self,
        request: ApplyValidatedLoopExitRequest,
    ) -> Result<TurnRunState, TurnError> {
        let mut status = self.state.lock().unwrap();
        let next_status = *status;
        *status = TurnStatus::Failed;
        drop(status);
        Ok(TurnRunState {
            scope: scope("thread-a"),
            actor: Some(TurnActor::new(UserId::new("user-thread-a").unwrap())),
            turn_id: ironclaw_turns::TurnId::new(),
            run_id: request.run_id,
            status: next_status,
            accepted_message_ref: AcceptedMessageRef::new("message-thread-a").unwrap(),
            source_binding_ref: SourceBindingRef::new("source-web").unwrap(),
            reply_target_binding_ref: ReplyTargetBindingRef::new("reply-web").unwrap(),
            resolved_run_profile_id: RunProfileId::new("default").unwrap(),
            resolved_run_profile_version: RunProfileVersion::new(1),
            resolved_model_route: None,
            received_at: received_at(),
            checkpoint_id: None,
            gate_ref: None,
            failure: None,
            event_cursor: EventCursor(1),
        })
    }
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

struct DenyUnauthorized;

impl TurnAdmissionPolicy for DenyUnauthorized {
    fn check_submit(&self, _request: &SubmitTurnRequest) -> Result<(), AdmissionRejection> {
        Err(AdmissionRejection::new(
            AdmissionRejectionReason::Unauthorized,
        ))
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

#[tokio::test]
async fn loop_exit_application_completes_after_validation_and_releases_lock() {
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

    let completed = apply_test_loop_exit(
        store.as_ref(),
        run_id,
        runner_id,
        lease_token,
        completed_mapping(),
    )
    .await
    .unwrap();

    assert_eq!(completed.status, TurnStatus::Completed);
    let next = coordinator
        .submit_turn(submit_request("thread-a", "idem-submit-b"))
        .await
        .unwrap();
    assert_ne!(accepted_run_id(&next), run_id);
}

#[tokio::test]
async fn loop_exit_application_blocks_with_checkpoint_and_keeps_lock() {
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
    let gate_ref = LoopGateRef::new("gate:approval-gate").unwrap();
    let state_ref = block_state_ref();

    let blocked = apply_test_loop_exit(
        store.as_ref(),
        run_id,
        runner_id,
        lease_token,
        approval_blocked_mapping(checkpoint_id, state_ref, &gate_ref),
    )
    .await
    .unwrap();

    assert_eq!(blocked.status, TurnStatus::BlockedApproval);
    assert_eq!(blocked.checkpoint_id, Some(checkpoint_id));
    assert_eq!(
        blocked.gate_ref,
        Some(GateRef::new(gate_ref.as_str()).unwrap())
    );
    assert!(matches!(
        coordinator
            .submit_turn(submit_request("thread-a", "idem-submit-b"))
            .await
            .unwrap_err(),
        TurnError::ThreadBusy(_)
    ));
}

#[tokio::test]
async fn invalid_loop_exit_application_records_recovery_required_and_keeps_lock() {
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

    let recovered = apply_test_loop_exit(
        store.as_ref(),
        run_id,
        runner_id,
        lease_token,
        protocol_recovery_mapping(),
    )
    .await
    .unwrap();

    assert_eq!(recovered.status, TurnStatus::RecoveryRequired);
    assert_eq!(
        recovered.failure.as_ref().map(SanitizedFailure::category),
        Some("driver_protocol_violation")
    );
    assert!(matches!(
        coordinator
            .submit_turn(submit_request("thread-a", "idem-submit-b"))
            .await
            .unwrap_err(),
        TurnError::ThreadBusy(_)
    ));
}

#[tokio::test]
async fn loop_exit_application_fails_after_validation_and_releases_lock() {
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

    let failed = apply_test_loop_exit(
        store.as_ref(),
        run_id,
        runner_id,
        lease_token,
        failed_mapping("iteration_limit"),
    )
    .await
    .unwrap();

    assert_eq!(failed.status, TurnStatus::Failed);
    assert_eq!(
        failed.failure.as_ref().map(SanitizedFailure::category),
        Some("iteration_limit")
    );
    let next = coordinator
        .submit_turn(submit_request("thread-a", "idem-submit-b"))
        .await
        .unwrap();
    assert_ne!(accepted_run_id(&next), run_id);
}

#[tokio::test]
async fn loop_exit_application_uses_single_atomic_transition_port_call() {
    let port = AtomicLoopExitPort {
        state: Mutex::new(TurnStatus::Cancelled),
    };

    let state = apply_test_loop_exit(
        &port,
        TurnRunId::new(),
        TurnRunnerId::new(),
        TurnLeaseToken::new(),
        completed_mapping(),
    )
    .await
    .unwrap();

    assert_eq!(state.status, TurnStatus::Cancelled);
}

#[tokio::test]
async fn non_cancelled_loop_exit_after_public_cancel_does_not_terminally_cancel() {
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

    let completed_after_cancel = apply_test_loop_exit(
        store.as_ref(),
        run_id,
        runner_id,
        lease_token,
        completed_mapping(),
    )
    .await
    .unwrap_err();
    assert_eq!(
        completed_after_cancel,
        TurnError::InvalidTransition {
            from: TurnStatus::CancelRequested,
            to: TurnStatus::Completed,
        }
    );

    let recovered_after_cancel = apply_test_loop_exit(
        store.as_ref(),
        run_id,
        runner_id,
        lease_token,
        protocol_recovery_mapping(),
    )
    .await
    .unwrap();
    assert_eq!(recovered_after_cancel.status, TurnStatus::RecoveryRequired);
    assert_eq!(
        recovered_after_cancel
            .failure
            .as_ref()
            .map(SanitizedFailure::category),
        Some("driver_protocol_violation")
    );
    assert!(matches!(
        coordinator
            .submit_turn(submit_request("thread-a", "idem-submit-b"))
            .await
            .unwrap_err(),
        TurnError::ThreadBusy(_)
    ));
}

#[tokio::test]
async fn observed_cancelled_loop_exit_without_recorded_cancel_enters_recovery_required() {
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

    let recovered = apply_test_loop_exit(
        store.as_ref(),
        run_id,
        runner_id,
        lease_token,
        cancelled_mapping(),
    )
    .await
    .unwrap();

    assert_eq!(recovered.status, TurnStatus::RecoveryRequired);
    assert_eq!(
        recovered.failure.as_ref().map(SanitizedFailure::category),
        Some("interrupted_unexpectedly")
    );
    assert!(matches!(
        coordinator
            .submit_turn(submit_request("thread-a", "idem-submit-b"))
            .await
            .unwrap_err(),
        TurnError::ThreadBusy(_)
    ));

    let cancelled = coordinator
        .cancel_run(CancelRunRequest {
            scope: scope("thread-a"),
            actor: actor(),
            run_id,
            reason: SanitizedCancelReason::OperatorRequested,
            idempotency_key: IdempotencyKey::new("idem-cancel-after-unrecorded-interrupt").unwrap(),
        })
        .await
        .unwrap();
    assert_eq!(cancelled.status, TurnStatus::Cancelled);
    let snapshot = store.persistence_snapshot();
    let run = snapshot
        .runs
        .iter()
        .find(|record| record.run_id == run_id)
        .unwrap();
    assert_eq!(run.failure, None);
}

#[tokio::test]
async fn loop_exit_application_cancels_only_after_public_cancel_request() {
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

    let cancelled = apply_test_loop_exit(
        store.as_ref(),
        run_id,
        runner_id,
        lease_token,
        cancelled_mapping(),
    )
    .await
    .unwrap();

    assert_eq!(cancelled.status, TurnStatus::Cancelled);
    let next = coordinator
        .submit_turn(submit_request("thread-a", "idem-submit-b"))
        .await
        .unwrap();
    assert_ne!(accepted_run_id(&next), run_id);
}
