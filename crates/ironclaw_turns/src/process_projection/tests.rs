use chrono::Utc;
use ironclaw_host_api::{AgentId, ProjectId, ResourceScope, TenantId, ThreadId, UserId};
use ironclaw_processes::{GetProcessSnapshotRequest, ProcessJournalPage};
use std::sync::Arc;

use super::*;
use crate::TurnEventProjectionFromProcessJournal;
use crate::{
    AcceptedMessageRef, CapabilityActivityId, EventCursor, GateRef, ReplyTargetBindingRef,
    RunProfileId, RunProfileVersion, SourceBindingRef, TurnActor, TurnId, TurnRunProfile,
    TurnScope, events::TurnEventProjectionSource,
};

fn scope() -> TurnScope {
    TurnScope::new(
        TenantId::new("tenant-process-journal").expect("tenant"),
        Some(AgentId::new("agent-process-journal").expect("agent")),
        Some(ProjectId::new("project-process-journal").expect("project")),
        ThreadId::new("thread-process-journal").expect("thread"),
    )
}

fn profile() -> TurnRunProfile {
    serde_json::from_value(serde_json::json!({
        "id": "default",
        "version": 1,
        "allow_steering": false,
        "auto_queue_followups": false,
    }))
    .expect("profile")
}

fn record_with_status(status: TurnStatus) -> TurnRunRecord {
    TurnRunRecord {
        run_id: TurnRunId::new(),
        turn_id: TurnId::new(),
        scope: scope(),
        accepted_message_ref: AcceptedMessageRef::new("accepted-process-journal")
            .expect("accepted"),
        source_binding_ref: SourceBindingRef::new("source-process-journal").expect("source"),
        reply_target_binding_ref: ReplyTargetBindingRef::new("reply-process-journal")
            .expect("reply"),
        status,
        profile: profile(),
        resolved_model_route: None,
        model_usage: None,
        checkpoint_id: None,
        gate_ref: GateKind::from_status(status)
            .map(|_| GateRef::new("gate:process-journal").expect("gate")),
        blocked_activity_id: None,
        credential_requirements: Vec::new(),
        failure: None,
        event_cursor: EventCursor(7),
        runner_id: None,
        lease_token: None,
        lease_expires_at: None,
        last_heartbeat_at: None,
        claim_count: 0,
        received_at: Utc::now(),
        parent_run_id: None,
        subagent_depth: 0,
        spawn_tree_root_run_id: None,
        product_context: None,
        resume_disposition: None,
    }
}

#[tokio::test]
async fn retry_rebinds_checkpoint_through_the_real_process_store() {
    use ironclaw_host_api::SanitizedFailure;
    use ironclaw_processes::{
        ClaimProcessesRequest, FailProcessRequest, GetProcessCheckpointRequest,
        ProcessCheckpointId, ProcessCheckpointPayload, ProcessCheckpointPort, ProcessCheckpointRef,
        ProcessJournalSource, ProcessKind, ProcessRuntimePort, ProcessSubmissionPort,
        ProcessTransitionPort, ProcessWorkerId, RecordProcessCheckpointRequest,
        SubmitProcessRequest, in_memory_backed_processes_filesystem,
    };

    let store = Arc::new(ironclaw_processes::ProcessJournalStore::new(
        in_memory_backed_processes_filesystem(),
    ));
    let runtime =
        AgentTurnProcessRuntime::from_process_runtime(store.clone() as Arc<dyn ProcessRuntimePort>);
    let turn_scope = scope();
    let resource_scope = turn_scope.to_resource_scope();
    let actor = TurnActor::new(UserId::new("retry-actor").expect("retry actor"));
    let failed_run_id = TurnRunId::new();
    let failed_process_id = process_id_from_turn_run_id(failed_run_id);
    let checkpoint_id =
        ProcessCheckpointId::from_trusted(TurnCheckpointId::new().as_uuid().to_string());
    let state_ref = ProcessCheckpointRef::from_trusted("source-state");
    let run_profile = profile();
    let metadata = AgentTurnProcessStateMetadata {
        turn_id: TurnId::new(),
        actor: Some(actor.clone()),
        accepted_message_ref: AcceptedMessageRef::new("accepted-retry").expect("accepted"),
        source_binding_ref: SourceBindingRef::new("source-retry").expect("source"),
        reply_target_binding_ref: ReplyTargetBindingRef::new("reply-retry").expect("reply"),
        resolved_run_profile_id: run_profile.id,
        resolved_run_profile_version: run_profile.version,
        resolved_run_profile: None,
        resolved_model_route: None,
        model_usage: None,
        subagent_depth: 0,
        spawn_tree_descendant_cap: None,
        product_context: None,
        resume_disposition: None,
    };
    store
        .submit_process(SubmitProcessRequest {
            process_id: failed_process_id,
            process_kind: ProcessKind::AgentTurn,
            scope: resource_scope.clone(),
            exclusive_within_scope: true,
            operation_id: None,
            owner_user_id: Some(actor.user_id.clone()),
            concurrency_class: None,
            parent_process_id: None,
            root_process_id: None,
            spawn_tree_descendant_cap: None,
            dependency: None,
            checkpoint_ref: Some(ProcessCheckpointRef::from_trusted(checkpoint_id.as_str())),
            input: None,
            created_at: Utc::now(),
            metadata: serde_json::json!({ "agent_turn": metadata }),
        })
        .await
        .expect("submit failed-run precursor");
    store
        .record_process_checkpoint(RecordProcessCheckpointRequest {
            checkpoint_id: checkpoint_id.clone(),
            process_id: failed_process_id,
            scope: resource_scope.clone(),
            state_ref: state_ref.clone(),
            payload: ProcessCheckpointPayload::new(b"checkpoint payload".to_vec())
                .expect("checkpoint payload"),
            created_at: Utc::now(),
            metadata: serde_json::json!({"source": "retry-test"}),
        })
        .await
        .expect("record source checkpoint");
    let claim = store
        .claim_next_processes(ClaimProcessesRequest {
            worker_id: ProcessWorkerId::from_trusted("retry-worker"),
            scope_filter: Some(resource_scope.clone()),
            process_id_filter: Some(failed_process_id),
            process_kind_filter: Some(ProcessKind::AgentTurn),
            max_processes: 1,
        })
        .await
        .expect("claim source run")
        .pop()
        .expect("claimed source run");
    store
        .fail_process(FailProcessRequest {
            process_id: failed_process_id,
            worker_id: claim.worker_id,
            lease_token: claim.lease_token,
            failure: SanitizedFailure::new("retryable_failure").expect("failure"),
            checkpoint_ref: None,
            metadata: None,
        })
        .await
        .expect("fail source run");

    let retried = runtime
        .retry_turn(crate::RetryTurnRequest {
            scope: turn_scope,
            actor,
            run_id: failed_run_id,
            source_binding_ref: SourceBindingRef::new("retry-source").expect("retry source"),
            reply_target_binding_ref: ReplyTargetBindingRef::new("retry-reply")
                .expect("retry reply"),
            idempotency_key: crate::IdempotencyKey::new("retry-operation")
                .expect("idempotency key"),
        })
        .await
        .expect("retry failed run");
    let retried_snapshot = store
        .get_process_snapshot(GetProcessSnapshotRequest {
            scope: resource_scope.clone(),
            process_id: process_id_from_turn_run_id(retried.run_id),
        })
        .await
        .expect("retried snapshot");
    let rebound_ref = retried_snapshot
        .checkpoint_ref
        .expect("retry checkpoint reference");
    let rebound = store
        .get_process_checkpoint(GetProcessCheckpointRequest {
            checkpoint_id: ProcessCheckpointId::from_trusted(rebound_ref.as_str()),
            process_id: retried_snapshot.process_id,
            scope: resource_scope,
        })
        .await
        .expect("read rebound checkpoint")
        .expect("rebound checkpoint");
    assert_eq!(rebound.state_ref, state_ref);
    assert_eq!(rebound.payload.as_bytes(), b"checkpoint payload");
}

#[test]
fn every_turn_status_maps_to_process_lifecycle_status() {
    let cases = [
        (TurnStatus::Queued, ProcessLifecycleStatus::Queued),
        (TurnStatus::Running, ProcessLifecycleStatus::Running),
        (
            TurnStatus::BlockedApproval,
            ProcessLifecycleStatus::Suspended,
        ),
        (TurnStatus::BlockedAuth, ProcessLifecycleStatus::Suspended),
        (
            TurnStatus::BlockedResource,
            ProcessLifecycleStatus::Suspended,
        ),
        (
            TurnStatus::BlockedDependentRun,
            ProcessLifecycleStatus::Suspended,
        ),
        (
            TurnStatus::BlockedExternalTool,
            ProcessLifecycleStatus::Suspended,
        ),
        (
            TurnStatus::CancelRequested,
            ProcessLifecycleStatus::CancelRequested,
        ),
        (TurnStatus::Cancelled, ProcessLifecycleStatus::Cancelled),
        (TurnStatus::Completed, ProcessLifecycleStatus::Completed),
        (TurnStatus::Failed, ProcessLifecycleStatus::Failed),
        (
            TurnStatus::RecoveryRequired,
            ProcessLifecycleStatus::RecoveryRequired,
        ),
    ];

    for (turn_status, process_status) in cases {
        assert_eq!(process_status_from_turn_status(turn_status), process_status);
        assert_eq!(
            process_status_from_turn_status(turn_status).keeps_active_lock(),
            turn_status.keeps_active_lock()
        );
    }
}

#[test]
fn blocked_turn_statuses_map_to_process_suspension_kinds() {
    let cases = [
        (TurnStatus::BlockedApproval, ProcessSuspensionKind::Approval),
        (
            TurnStatus::BlockedAuth,
            ProcessSuspensionKind::Authorization,
        ),
        (TurnStatus::BlockedResource, ProcessSuspensionKind::Resource),
        (
            TurnStatus::BlockedDependentRun,
            ProcessSuspensionKind::AwaitingChildProcess,
        ),
        (
            TurnStatus::BlockedExternalTool,
            ProcessSuspensionKind::ExternalTool,
        ),
    ];

    for (turn_status, suspension_kind) in cases {
        let snapshot = record_with_status(turn_status).to_process_snapshot();
        assert_eq!(snapshot.status, ProcessLifecycleStatus::Suspended);
        assert_eq!(
            snapshot.suspension.expect("suspension").kind,
            suspension_kind
        );
    }
}

#[test]
fn every_turn_event_kind_maps_to_process_journal_kind() {
    let cases = [
        (TurnEventKind::Submitted, ProcessJournalKind::Submitted),
        (TurnEventKind::Resumed, ProcessJournalKind::Resumed),
        (TurnEventKind::RunnerClaimed, ProcessJournalKind::Claimed),
        (
            TurnEventKind::RunnerHeartbeat,
            ProcessJournalKind::Heartbeat,
        ),
        (
            TurnEventKind::RecoveryRequired,
            ProcessJournalKind::RecoveryRequired,
        ),
        (TurnEventKind::Blocked, ProcessJournalKind::Suspended),
        (
            TurnEventKind::CancelRequested,
            ProcessJournalKind::CancelRequested,
        ),
        (TurnEventKind::Cancelled, ProcessJournalKind::Cancelled),
        (TurnEventKind::Completed, ProcessJournalKind::Completed),
        (TurnEventKind::Failed, ProcessJournalKind::Failed),
    ];

    for (turn_kind, process_kind) in cases {
        assert_eq!(
            process_journal_kind_from_turn_event_kind(turn_kind),
            process_kind
        );
    }
}

#[test]
fn lifecycle_event_projects_to_process_journal_entry() {
    let state = crate::TurnRunState {
        scope: scope(),
        actor: Some(TurnActor::new(UserId::new("user:process").expect("user"))),
        turn_id: TurnId::new(),
        run_id: TurnRunId::new(),
        status: TurnStatus::BlockedAuth,
        accepted_message_ref: AcceptedMessageRef::new("accepted-process-journal")
            .expect("accepted"),
        source_binding_ref: SourceBindingRef::new("source-process-journal").expect("source"),
        reply_target_binding_ref: ReplyTargetBindingRef::new("reply-process-journal")
            .expect("reply"),
        resolved_run_profile_id: RunProfileId::default_profile(),
        resolved_run_profile_version: RunProfileVersion::new(1),
        resolved_model_route: None,
        model_usage: None,
        received_at: Utc::now(),
        checkpoint_id: None,
        gate_ref: Some(GateRef::new("gate:process-journal").expect("gate")),
        blocked_activity_id: Some(CapabilityActivityId::new()),
        credential_requirements: Vec::new(),
        failure: None,
        event_cursor: EventCursor(9),
        product_context: None,
        resume_disposition: None,
    };
    let event = TurnLifecycleEvent::from_run_state(
        &state,
        TurnEventKind::Blocked,
        Some("auth_required".to_string()),
    );

    let entry = event.to_process_journal_entry();

    assert_eq!(entry.cursor, ProcessJournalCursor(9));
    assert_eq!(entry.process_id, process_id_from_turn_run_id(state.run_id));
    assert_eq!(entry.status, ProcessLifecycleStatus::Suspended);
    assert_eq!(entry.kind, ProcessJournalKind::Suspended);
    assert_eq!(
        entry.suspension.expect("suspension").kind,
        ProcessSuspensionKind::Authorization
    );
    assert_eq!(entry.sanitized_reason.as_deref(), Some("auth_required"));
}

#[test]
fn claimed_turn_run_projects_to_process_claim() {
    let state = crate::TurnRunState {
        scope: scope(),
        actor: Some(TurnActor::new(UserId::new("user:process").expect("user"))),
        turn_id: TurnId::new(),
        run_id: TurnRunId::new(),
        status: TurnStatus::Running,
        accepted_message_ref: AcceptedMessageRef::new("accepted-process-journal")
            .expect("accepted"),
        source_binding_ref: SourceBindingRef::new("source-process-journal").expect("source"),
        reply_target_binding_ref: ReplyTargetBindingRef::new("reply-process-journal")
            .expect("reply"),
        resolved_run_profile_id: RunProfileId::default_profile(),
        resolved_run_profile_version: RunProfileVersion::new(1),
        resolved_model_route: None,
        model_usage: None,
        received_at: Utc::now(),
        checkpoint_id: None,
        gate_ref: None,
        blocked_activity_id: None,
        credential_requirements: Vec::new(),
        failure: None,
        event_cursor: EventCursor(11),
        product_context: None,
        resume_disposition: None,
    };
    let claimed = ClaimedTurnRun {
        state: state.clone(),
        resolved_run_profile: profile().resolved,
        runner_id: TurnRunnerId::new(),
        lease_token: crate::TurnLeaseToken::new(),
    };

    let process = ClaimedProcess::from(&claimed);

    assert_eq!(
        process.state.process_id,
        process_id_from_turn_run_id(state.run_id)
    );
    assert_eq!(process.state.status, ProcessLifecycleStatus::Running);
    assert_eq!(
        process.state.metadata["agent_turn"]["turn_id"],
        json!(state.turn_id)
    );
}

#[test]
fn claimed_process_round_trips_to_turn_executor_view() {
    let state = crate::TurnRunState {
        scope: scope(),
        actor: Some(TurnActor::new(UserId::new("user:process").expect("user"))),
        turn_id: TurnId::new(),
        run_id: TurnRunId::new(),
        status: TurnStatus::Running,
        accepted_message_ref: AcceptedMessageRef::new("accepted-process-journal")
            .expect("accepted"),
        source_binding_ref: SourceBindingRef::new("source-process-journal").expect("source"),
        reply_target_binding_ref: ReplyTargetBindingRef::new("reply-process-journal")
            .expect("reply"),
        resolved_run_profile_id: RunProfileId::default_profile(),
        resolved_run_profile_version: RunProfileVersion::new(1),
        resolved_model_route: None,
        model_usage: None,
        received_at: Utc::now(),
        checkpoint_id: None,
        gate_ref: None,
        blocked_activity_id: None,
        credential_requirements: Vec::new(),
        failure: None,
        event_cursor: EventCursor(12),
        product_context: None,
        resume_disposition: None,
    };
    let claimed = ClaimedTurnRun {
        state: state.clone(),
        resolved_run_profile: profile().resolved,
        runner_id: TurnRunnerId::new(),
        lease_token: crate::TurnLeaseToken::new(),
    };
    let process_claim = ClaimedProcess::from(&claimed);

    let round_trip = claimed_turn_run_from_process_claim(process_claim).expect("claimed turn view");

    assert_eq!(round_trip.state, state);
    assert_eq!(round_trip.runner_id, claimed.runner_id);
    assert_eq!(round_trip.lease_token, claimed.lease_token);
    assert_eq!(
        round_trip.resolved_run_profile,
        claimed.resolved_run_profile
    );
}

#[tokio::test]
async fn turn_event_projection_can_be_a_view_over_process_journal() {
    let run_id = TurnRunId::new();
    let process_source: Arc<dyn ProcessJournalSource<Error = TurnError>> =
        Arc::new(FakeProcessJournalSource {
            page: ProcessJournalPage {
                entries: vec![ProcessJournalEntry {
                    cursor: ProcessJournalCursor(1),
                    process_id: process_id_from_turn_run_id(run_id),
                    process_kind: ProcessKind::AgentTurn,
                    scope: scope().to_resource_scope(),
                    occurred_at: Some(Utc::now()),
                    owner_user_id: None,
                    status: ProcessLifecycleStatus::Queued,
                    kind: ProcessJournalKind::Submitted,
                    suspension: None,
                    sanitized_reason: None,
                    retryable: None,
                    detail: None,
                    metadata: Value::Null,
                    committed_state: None,
                }],
                next_cursor: ProcessJournalCursor(1),
                truncated: false,
                rebase_required: None,
            },
        });
    let turn_view = TurnEventProjectionFromProcessJournal::new(process_source);

    let page = turn_view
        .read_turn_events_after(&scope(), None, None, 10)
        .await
        .expect("turn view page");
    assert_eq!(page.entries.len(), 1);
    assert_eq!(page.entries[0].run_id, run_id);
    assert_eq!(page.entries[0].kind, TurnEventKind::Submitted);
    assert_eq!(page.entries[0].status, TurnStatus::Queued);
}

struct FakeProcessJournalSource {
    page: ProcessJournalPage,
}

#[async_trait]
impl ProcessJournalSource for FakeProcessJournalSource {
    type Error = TurnError;

    async fn get_process_snapshot(
        &self,
        _request: GetProcessSnapshotRequest,
    ) -> Result<JournaledProcessSnapshot, Self::Error> {
        Err(TurnError::InvalidRequest {
            reason: "fake process journal source does not serve snapshots".to_string(),
        })
    }

    async fn read_process_journal_after(
        &self,
        _scope: &ResourceScope,
        _owner_user_id: Option<&ironclaw_host_api::UserId>,
        _after: Option<ProcessJournalCursor>,
        _limit: usize,
    ) -> Result<ProcessJournalPage, Self::Error> {
        Ok(self.page.clone())
    }

    async fn read_process_journal_log_after(
        &self,
        _after: Option<ProcessJournalCursor>,
        _limit: usize,
    ) -> Result<ProcessJournalPage, Self::Error> {
        Ok(self.page.clone())
    }
}

#[test]
fn runner_outcomes_map_to_process_outcomes() {
    let failure = crate::SanitizedFailure::new("runner_failed").expect("failure");
    let blocked = TurnRunnerOutcome::Blocked {
        checkpoint_id: TurnCheckpointId::new(),
        state_ref: crate::run_profile::LoopCheckpointStateRef::new(
            "checkpoint:state-process-journal".to_string(),
        )
        .expect("state ref"),
        reason: BlockedReason::ExternalTool {
            gate_ref: GateRef::new("gate:process-journal").expect("gate"),
        },
        blocked_activity_id: Some(CapabilityActivityId::new()),
    };

    assert_eq!(
        process_outcome_from_turn_runner_outcome(TurnRunnerOutcome::Completed),
        ProcessOutcome::Completed
    );
    assert_eq!(
        process_outcome_from_turn_runner_outcome(TurnRunnerOutcome::Cancelled),
        ProcessOutcome::Cancelled
    );
    assert!(matches!(
        process_outcome_from_turn_runner_outcome(blocked),
        ProcessOutcome::Suspended { .. }
    ));
    assert_eq!(
        process_outcome_from_turn_runner_outcome(TurnRunnerOutcome::Failed {
            failure: failure.clone()
        }),
        ProcessOutcome::Failed { failure }
    );
}
