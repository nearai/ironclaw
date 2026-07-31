use chrono::Utc;
use ironclaw_host_api::{
    ids::{AgentId, ProjectId, TenantId, ThreadId, UserId},
    resource::ResourceScope,
};
use ironclaw_processes::{GetProcessSnapshotRequest, ProcessJournalPage, ProcessSnapshotSource};
use std::sync::Arc;

use super::*;
use crate::TurnEventProjectionFromProcessJournal;
use crate::{
    AcceptedMessageRef, AllowAllTurnAdmissionPolicy, CapabilityActivityId, EventCursor,
    IdempotencyKey, ReplyTargetBindingRef, RunProfileId, RunProfileVersion, SourceBindingRef,
    TurnActor, TurnGateRef, TurnId, TurnRunProfile, TurnScope, events::TurnEventProjectionSource,
};
use ironclaw_loop_contracts::InMemoryRunProfileResolver;

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
            .map(|_| TurnGateRef::new("gate:process-journal").expect("gate")),
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

fn agent_turn_metadata(
    actor: TurnActor,
    turn_id: TurnId,
    subagent_depth: u32,
) -> AgentTurnProcessStateMetadata {
    let run_profile = profile();
    AgentTurnProcessStateMetadata {
        turn_id,
        actor: Some(actor),
        accepted_message_ref: AcceptedMessageRef::new("accepted-runtime-test")
            .expect("accepted message"),
        source_binding_ref: SourceBindingRef::new("source-runtime-test").expect("source binding"),
        reply_target_binding_ref: ReplyTargetBindingRef::new("reply-runtime-test")
            .expect("reply binding"),
        resolved_run_profile_id: run_profile.id,
        resolved_run_profile_version: run_profile.version,
        resolved_run_profile: None,
        resolved_model_route: None,
        model_usage: None,
        subagent_depth,
        spawn_tree_descendant_cap: None,
        product_context: None,
        resume_disposition: None,
    }
}

fn child_request(
    parent_scope: TurnScope,
    parent_run_id: TurnRunId,
    child_scope: TurnScope,
    actor: TurnActor,
    requested_run_id: TurnRunId,
    idempotency_key: &str,
) -> SubmitChildRunRequest {
    SubmitChildRunRequest {
        parent_scope,
        parent_run_id,
        child_scope,
        actor,
        accepted_message_ref: AcceptedMessageRef::new("accepted-child").expect("accepted child"),
        source_binding_ref: SourceBindingRef::new("source-child").expect("source child"),
        reply_target_binding_ref: ReplyTargetBindingRef::new("reply-child").expect("reply child"),
        requested_run_profile: None,
        idempotency_key: IdempotencyKey::new(idempotency_key).expect("idempotency key"),
        received_at: Utc::now(),
        requested_run_id: Some(requested_run_id),
        spawn_tree_descendant_cap: 8,
        process_dependency: None,
        process_input: None,
    }
}

async fn submit_agent_process<F>(
    store: &ironclaw_processes::ProcessJournalStore<F>,
    turn_scope: &TurnScope,
    actor: &TurnActor,
    run_id: TurnRunId,
    turn_id: TurnId,
    checkpoint_ref: Option<ironclaw_processes::ProcessCheckpointRef>,
) where
    F: ironclaw_filesystem::RootFilesystem + Send + Sync + 'static,
{
    use ironclaw_processes::{ProcessKind, ProcessSubmissionPort, SubmitProcessRequest};

    store
        .submit_process(SubmitProcessRequest {
            process_id: process_id_from_turn_run_id(run_id),
            process_kind: ProcessKind::AgentTurn,
            scope: turn_scope.to_resource_scope(),
            exclusive_within_scope: true,
            operation_id: None,
            owner_user_id: Some(actor.user_id.clone()),
            concurrency_class: None,
            parent_process_id: None,
            root_process_id: None,
            spawn_tree_descendant_cap: None,
            dependency: None,
            checkpoint_ref,
            input: None,
            created_at: Utc::now(),
            metadata: serde_json::json!({
                "agent_turn": agent_turn_metadata(actor.clone(), turn_id, 0)
            }),
        })
        .await
        .expect("submit agent process");
}

async fn fail_agent_process<F>(
    store: &ironclaw_processes::ProcessJournalStore<F>,
    turn_scope: &TurnScope,
    run_id: TurnRunId,
    checkpoint_ref: Option<ironclaw_processes::ProcessCheckpointRef>,
) where
    F: ironclaw_filesystem::RootFilesystem + Send + Sync + 'static,
{
    fail_agent_process_with_category(
        store,
        turn_scope,
        run_id,
        checkpoint_ref,
        "runtime_test_failure",
    )
    .await;
}

async fn fail_agent_process_with_category<F>(
    store: &ironclaw_processes::ProcessJournalStore<F>,
    turn_scope: &TurnScope,
    run_id: TurnRunId,
    checkpoint_ref: Option<ironclaw_processes::ProcessCheckpointRef>,
    failure_category: &str,
) where
    F: ironclaw_filesystem::RootFilesystem + Send + Sync + 'static,
{
    use ironclaw_host_api::turn::SanitizedFailure;
    use ironclaw_processes::{
        ClaimProcessesRequest, FailProcessRequest, ProcessKind, ProcessTransitionPort,
        ProcessWorkerId,
    };

    let claim = store
        .claim_next_processes(ClaimProcessesRequest {
            worker_id: ProcessWorkerId::from_trusted("runtime-test-worker"),
            scope_filter: Some(turn_scope.to_resource_scope()),
            process_id_filter: Some(process_id_from_turn_run_id(run_id)),
            process_kind_filter: Some(ProcessKind::AgentTurn),
            max_processes: 1,
        })
        .await
        .expect("claim agent process")
        .pop()
        .expect("claimed agent process");
    store
        .fail_process(FailProcessRequest {
            process_id: process_id_from_turn_run_id(run_id),
            worker_id: claim.worker_id,
            lease_token: claim.lease_token,
            failure: SanitizedFailure::new(failure_category).expect("failure"),
            recovery: ironclaw_processes::ProcessFailureRecovery::Terminal,
            checkpoint_ref,
            metadata: None,
        })
        .await
        .expect("fail agent process");
}

#[tokio::test]
async fn resume_rejects_a_running_claim_without_clearing_its_lease() {
    use ironclaw_processes::{
        ClaimProcessesRequest, ProcessKind, ProcessRuntimePort, ProcessTransitionPort,
        ProcessWorkerId, in_memory_backed_process_store,
    };

    let store = Arc::new(in_memory_backed_process_store());
    let runtime =
        AgentTurnProcessRuntime::from_process_runtime(store.clone() as Arc<dyn ProcessRuntimePort>);
    let turn_scope = scope();
    let actor = TurnActor::new(UserId::new("running-resume-user").expect("user"));
    let run_id = TurnRunId::new();
    submit_agent_process(
        store.as_ref(),
        &turn_scope,
        &actor,
        run_id,
        TurnId::new(),
        None,
    )
    .await;
    let claim = store
        .claim_next_processes(ClaimProcessesRequest {
            worker_id: ProcessWorkerId::from_trusted("running-resume-worker"),
            scope_filter: Some(turn_scope.to_resource_scope()),
            process_id_filter: Some(process_id_from_turn_run_id(run_id)),
            process_kind_filter: Some(ProcessKind::AgentTurn),
            max_processes: 1,
        })
        .await
        .expect("claim running turn")
        .pop()
        .expect("running turn claim");

    let error = runtime
        .resume_turn(crate::ResumeTurnRequest {
            scope: turn_scope.clone(),
            actor,
            run_id,
            gate_resolution_ref: TurnGateRef::new("gate:stale-resume").expect("gate"),
            source_binding_ref: SourceBindingRef::new("source:stale-resume").expect("source"),
            reply_target_binding_ref: ReplyTargetBindingRef::new("reply:stale-resume")
                .expect("reply"),
            idempotency_key: IdempotencyKey::new("stale-resume").expect("idempotency"),
            precondition: crate::ResumeTurnPrecondition::default(),
            resume_disposition: None,
        })
        .await
        .expect_err("a stale resume must not requeue running work");
    assert!(matches!(
        error,
        crate::TurnError::InvalidTransition {
            from: TurnStatus::Running,
            to: TurnStatus::Queued
        }
    ));

    let snapshot = store
        .get_process_snapshot(GetProcessSnapshotRequest {
            scope: turn_scope.to_resource_scope(),
            process_id: process_id_from_turn_run_id(run_id),
        })
        .await
        .expect("load running turn after stale resume");
    assert_eq!(snapshot.status, ProcessLifecycleStatus::Running);
    assert_eq!(
        snapshot.lease.as_ref().map(|lease| &lease.lease_token),
        Some(&claim.lease_token)
    );
}

#[tokio::test]
async fn foreign_actor_cannot_resume_or_cancel_and_leaves_process_unchanged() {
    use ironclaw_processes::{
        ClaimProcessesRequest, ProcessKind, ProcessRuntimePort, ProcessSuspension,
        ProcessSuspensionKind, ProcessTransitionPort, ProcessWorkerId, SuspendProcessRequest,
        in_memory_backed_process_store,
    };

    let store = Arc::new(in_memory_backed_process_store());
    let runtime =
        AgentTurnProcessRuntime::from_process_runtime(store.clone() as Arc<dyn ProcessRuntimePort>);
    let turn_scope = scope();
    let owner = TurnActor::new(UserId::new("foreign-test-owner").expect("owner"));
    let intruder = TurnActor::new(UserId::new("foreign-test-intruder").expect("intruder"));
    let run_id = TurnRunId::new();
    submit_agent_process(
        store.as_ref(),
        &turn_scope,
        &owner,
        run_id,
        TurnId::new(),
        None,
    )
    .await;
    let claim = store
        .claim_next_processes(ClaimProcessesRequest {
            worker_id: ProcessWorkerId::from_trusted("foreign-test-worker"),
            scope_filter: Some(turn_scope.to_resource_scope()),
            process_id_filter: Some(process_id_from_turn_run_id(run_id)),
            process_kind_filter: Some(ProcessKind::AgentTurn),
            max_processes: 1,
        })
        .await
        .expect("claim")
        .pop()
        .expect("claimed");
    let gate_ref = TurnGateRef::new("gate:foreign-test").expect("gate");
    store
        .suspend_process(SuspendProcessRequest {
            process_id: claim.state.process_id,
            worker_id: claim.worker_id,
            lease_token: claim.lease_token,
            checkpoint_ref: ProcessCheckpointRef::from_trusted(
                TurnCheckpointId::new().as_uuid().to_string(),
            ),
            suspension: ProcessSuspension {
                kind: ProcessSuspensionKind::Approval,
                gate_ref: Some(
                    ironclaw_host_api::turn::TurnGateRef::new(gate_ref.as_str())
                        .expect("turn gate"),
                ),
                activity_id: None,
                credential_requirements: Vec::new(),
                detail: None,
            },
            metadata: None,
        })
        .await
        .expect("suspend");
    let before = store
        .get_process_snapshot(GetProcessSnapshotRequest {
            scope: turn_scope.to_resource_scope(),
            process_id: process_id_from_turn_run_id(run_id),
        })
        .await
        .expect("snapshot before denials");

    let resume = runtime
        .resume_turn(crate::ResumeTurnRequest {
            scope: turn_scope.clone(),
            actor: intruder.clone(),
            run_id,
            gate_resolution_ref: gate_ref,
            source_binding_ref: SourceBindingRef::new("foreign-source").expect("source"),
            reply_target_binding_ref: ReplyTargetBindingRef::new("foreign-reply").expect("reply"),
            idempotency_key: IdempotencyKey::new("foreign-resume").expect("idempotency"),
            precondition: crate::ResumeTurnPrecondition::default(),
            resume_disposition: None,
        })
        .await;
    assert!(
        matches!(resume, Err(TurnError::Unauthorized)),
        "unexpected foreign resume result: {resume:?}"
    );
    let cancel = runtime
        .cancel_run(crate::CancelRunRequest {
            scope: turn_scope.clone(),
            actor: intruder,
            run_id,
            idempotency_key: IdempotencyKey::new("foreign-cancel").expect("idempotency"),
            reason: crate::SanitizedCancelReason::UserRequested,
        })
        .await;
    assert!(
        matches!(cancel, Err(TurnError::Unauthorized)),
        "unexpected foreign cancel result: {cancel:?}"
    );

    let after = store
        .get_process_snapshot(GetProcessSnapshotRequest {
            scope: turn_scope.to_resource_scope(),
            process_id: process_id_from_turn_run_id(run_id),
        })
        .await
        .expect("snapshot after denials");
    assert_eq!(after, before);
}

#[tokio::test]
async fn child_submission_persists_computed_lineage_and_is_idempotent() {
    use ironclaw_processes::{
        ProcessJournalSource, ProcessKind, ProcessRuntimePort, ProcessSubmissionPort,
        SubmitProcessRequest, in_memory_backed_process_store,
    };

    let store = Arc::new(in_memory_backed_process_store());
    let runtime =
        AgentTurnProcessRuntime::from_process_runtime(store.clone() as Arc<dyn ProcessRuntimePort>);
    let parent_scope = scope();
    let child_scope = TurnScope::new(
        parent_scope.tenant_id.clone(),
        parent_scope.agent_id.clone(),
        parent_scope.project_id.clone(),
        ThreadId::new("thread-process-journal-child").expect("child thread"),
    );
    let actor = TurnActor::new(UserId::new("child-actor").expect("child actor"));
    let parent_run_id = TurnRunId::new();
    let parent_process_id = process_id_from_turn_run_id(parent_run_id);
    store
        .submit_process(SubmitProcessRequest {
            process_id: parent_process_id,
            process_kind: ProcessKind::AgentTurn,
            scope: parent_scope.to_resource_scope(),
            exclusive_within_scope: true,
            operation_id: None,
            owner_user_id: Some(actor.user_id.clone()),
            concurrency_class: None,
            parent_process_id: None,
            root_process_id: None,
            spawn_tree_descendant_cap: None,
            dependency: None,
            checkpoint_ref: None,
            input: None,
            created_at: Utc::now(),
            metadata: serde_json::json!({
                "agent_turn": agent_turn_metadata(actor.clone(), TurnId::new(), 2)
            }),
        })
        .await
        .expect("submit parent");

    let child_run_id = TurnRunId::new();
    let request = child_request(
        parent_scope,
        parent_run_id,
        child_scope.clone(),
        actor.clone(),
        child_run_id,
        "child-submit-idempotency",
    );
    let resolver = InMemoryRunProfileResolver::default();
    let first = runtime
        .submit_child_turn(request.clone(), &AllowAllTurnAdmissionPolicy, &resolver)
        .await
        .expect("submit child");
    let replay = runtime
        .submit_child_turn(request, &AllowAllTurnAdmissionPolicy, &resolver)
        .await
        .expect("replay child");
    assert_eq!(first, replay);

    let snapshot = store
        .get_process_snapshot(GetProcessSnapshotRequest {
            scope: child_scope.to_resource_scope(),
            process_id: process_id_from_turn_run_id(child_run_id),
        })
        .await
        .expect("child snapshot");
    assert_eq!(snapshot.parent_process_id, Some(parent_process_id));
    assert_eq!(snapshot.root_process_id, Some(parent_process_id));
    let metadata = agent_turn_metadata_from_process_snapshot(&snapshot).expect("child metadata");
    assert_eq!(metadata.subagent_depth, 3);
    assert_eq!(metadata.spawn_tree_descendant_cap, Some(8));
    assert_eq!(metadata.actor, Some(actor));

    let children = store
        .process_snapshots(&child_scope.to_resource_scope())
        .await
        .expect("child snapshots");
    assert_eq!(
        children.len(),
        1,
        "idempotent replay must not duplicate child"
    );
}

#[tokio::test]
async fn child_submission_rejects_depth_overflow_without_persisting_a_child() {
    use ironclaw_processes::{
        ProcessKind, ProcessRuntimePort, ProcessSubmissionPort, SubmitProcessRequest,
        in_memory_backed_process_store,
    };

    let store = Arc::new(in_memory_backed_process_store());
    let runtime =
        AgentTurnProcessRuntime::from_process_runtime(store.clone() as Arc<dyn ProcessRuntimePort>);
    let parent_scope = scope();
    let child_scope = TurnScope::new(
        parent_scope.tenant_id.clone(),
        parent_scope.agent_id.clone(),
        parent_scope.project_id.clone(),
        ThreadId::new("thread-overflow-child").expect("child thread"),
    );
    let actor = TurnActor::new(UserId::new("overflow-actor").expect("overflow actor"));
    let parent_run_id = TurnRunId::new();
    store
        .submit_process(SubmitProcessRequest {
            process_id: process_id_from_turn_run_id(parent_run_id),
            process_kind: ProcessKind::AgentTurn,
            scope: parent_scope.to_resource_scope(),
            exclusive_within_scope: true,
            operation_id: None,
            owner_user_id: Some(actor.user_id.clone()),
            concurrency_class: None,
            parent_process_id: None,
            root_process_id: None,
            spawn_tree_descendant_cap: None,
            dependency: None,
            checkpoint_ref: None,
            input: None,
            created_at: Utc::now(),
            metadata: serde_json::json!({
                "agent_turn": agent_turn_metadata(actor.clone(), TurnId::new(), u32::MAX)
            }),
        })
        .await
        .expect("submit parent");

    let child_run_id = TurnRunId::new();
    let error = runtime
        .submit_child_turn(
            child_request(
                parent_scope,
                parent_run_id,
                child_scope.clone(),
                actor,
                child_run_id,
                "child-depth-overflow",
            ),
            &AllowAllTurnAdmissionPolicy,
            &InMemoryRunProfileResolver::default(),
        )
        .await
        .expect_err("overflow must reject child");
    assert!(matches!(
        error,
        TurnError::InvalidRequest { reason } if reason.contains("depth would overflow")
    ));
    assert!(
        store
            .process_snapshots(&child_scope.to_resource_scope())
            .await
            .expect("child snapshots")
            .is_empty()
    );
}

#[tokio::test]
async fn retry_rejects_wrong_actor_and_non_terminal_runs_without_creating_processes() {
    use ironclaw_processes::{
        ProcessRuntimePort, ProcessSnapshotSource, in_memory_backed_process_store,
    };

    let store = Arc::new(in_memory_backed_process_store());
    let runtime =
        AgentTurnProcessRuntime::from_process_runtime(store.clone() as Arc<dyn ProcessRuntimePort>);
    let turn_scope = scope();
    let actor = TurnActor::new(UserId::new("retry-owner").expect("retry owner"));
    let run_id = TurnRunId::new();
    submit_agent_process(
        store.as_ref(),
        &turn_scope,
        &actor,
        run_id,
        TurnId::new(),
        None,
    )
    .await;

    let request = RetryTurnRequest {
        scope: turn_scope.clone(),
        actor: TurnActor::new(UserId::new("retry-intruder").expect("retry intruder")),
        run_id,
        source_binding_ref: SourceBindingRef::new("retry-wrong-source").expect("source"),
        reply_target_binding_ref: ReplyTargetBindingRef::new("retry-wrong-reply").expect("reply"),
        idempotency_key: IdempotencyKey::new("retry-wrong-actor").expect("idempotency"),
    };
    assert!(matches!(
        runtime.retry_turn(request).await,
        Err(TurnError::Unauthorized)
    ));

    let request = RetryTurnRequest {
        scope: turn_scope.clone(),
        actor,
        run_id,
        source_binding_ref: SourceBindingRef::new("retry-queued-source").expect("source"),
        reply_target_binding_ref: ReplyTargetBindingRef::new("retry-queued-reply").expect("reply"),
        idempotency_key: IdempotencyKey::new("retry-queued").expect("idempotency"),
    };
    assert!(matches!(
        runtime.retry_turn(request).await,
        Err(TurnError::RunNotRetryable { run_id: rejected }) if rejected == run_id
    ));
    assert_eq!(
        store
            .process_snapshots(&turn_scope.to_resource_scope())
            .await
            .expect("snapshots")
            .len(),
        1
    );
}

#[tokio::test]
async fn retry_rejects_checkpoint_rejection_without_creating_a_process() {
    use ironclaw_processes::{
        ProcessRuntimePort, ProcessSnapshotSource, in_memory_backed_process_store,
    };

    let store = Arc::new(in_memory_backed_process_store());
    let runtime =
        AgentTurnProcessRuntime::from_process_runtime(store.clone() as Arc<dyn ProcessRuntimePort>);
    let turn_scope = scope();
    let actor = TurnActor::new(UserId::new("retry-checkpoint-owner").expect("retry owner"));
    let run_id = TurnRunId::new();
    submit_agent_process(
        store.as_ref(),
        &turn_scope,
        &actor,
        run_id,
        TurnId::new(),
        None,
    )
    .await;
    fail_agent_process_with_category(
        store.as_ref(),
        &turn_scope,
        run_id,
        None,
        ironclaw_loop_contracts::LoopFailureKind::CheckpointRejected.as_str(),
    )
    .await;

    let result = runtime
        .retry_turn(RetryTurnRequest {
            scope: turn_scope.clone(),
            actor,
            run_id,
            source_binding_ref: SourceBindingRef::new("retry-checkpoint-source").expect("source"),
            reply_target_binding_ref: ReplyTargetBindingRef::new("retry-checkpoint-reply")
                .expect("reply"),
            idempotency_key: IdempotencyKey::new("retry-checkpoint").expect("idempotency"),
        })
        .await;

    assert!(matches!(
        result,
        Err(TurnError::RunNotRetryable { run_id: rejected }) if rejected == run_id
    ));
    assert_eq!(
        store
            .process_snapshots(&turn_scope.to_resource_scope())
            .await
            .expect("snapshots")
            .len(),
        1,
        "a deterministic checkpoint rejection must not create a retry process"
    );
}

#[tokio::test]
async fn retry_rejects_superseded_runs_and_missing_checkpoint_payloads() {
    use ironclaw_processes::{
        ProcessCheckpointRef, ProcessRuntimePort, ProcessSnapshotSource,
        in_memory_backed_process_store,
    };

    let store = Arc::new(in_memory_backed_process_store());
    let runtime =
        AgentTurnProcessRuntime::from_process_runtime(store.clone() as Arc<dyn ProcessRuntimePort>);
    let turn_scope = scope();
    let actor = TurnActor::new(UserId::new("retry-owner").expect("retry owner"));
    let turn_id = TurnId::new();
    let original_run_id = TurnRunId::new();
    submit_agent_process(
        store.as_ref(),
        &turn_scope,
        &actor,
        original_run_id,
        turn_id,
        None,
    )
    .await;
    fail_agent_process(store.as_ref(), &turn_scope, original_run_id, None).await;

    let newer_run_id = TurnRunId::new();
    submit_agent_process(
        store.as_ref(),
        &turn_scope,
        &actor,
        newer_run_id,
        turn_id,
        None,
    )
    .await;
    let superseded = RetryTurnRequest {
        scope: turn_scope.clone(),
        actor: actor.clone(),
        run_id: original_run_id,
        source_binding_ref: SourceBindingRef::new("retry-superseded-source").expect("source"),
        reply_target_binding_ref: ReplyTargetBindingRef::new("retry-superseded-reply")
            .expect("reply"),
        idempotency_key: IdempotencyKey::new("retry-superseded").expect("idempotency"),
    };
    assert!(matches!(
        runtime.retry_turn(superseded).await,
        Err(TurnError::RunNotRetryable { run_id }) if run_id == original_run_id
    ));

    fail_agent_process(store.as_ref(), &turn_scope, newer_run_id, None).await;
    let missing_checkpoint_run_id = TurnRunId::new();
    let missing_checkpoint_ref =
        ProcessCheckpointRef::from_trusted(TurnCheckpointId::new().as_uuid().to_string());
    submit_agent_process(
        store.as_ref(),
        &turn_scope,
        &actor,
        missing_checkpoint_run_id,
        TurnId::new(),
        Some(missing_checkpoint_ref.clone()),
    )
    .await;
    fail_agent_process(
        store.as_ref(),
        &turn_scope,
        missing_checkpoint_run_id,
        Some(missing_checkpoint_ref),
    )
    .await;
    let missing_checkpoint = RetryTurnRequest {
        scope: turn_scope.clone(),
        actor,
        run_id: missing_checkpoint_run_id,
        source_binding_ref: SourceBindingRef::new("retry-missing-source").expect("source"),
        reply_target_binding_ref: ReplyTargetBindingRef::new("retry-missing-reply").expect("reply"),
        idempotency_key: IdempotencyKey::new("retry-missing-checkpoint").expect("idempotency"),
    };
    let error = runtime
        .retry_turn(missing_checkpoint)
        .await
        .expect_err("missing checkpoint payload must reject retry");
    assert!(
        matches!(
            &error,
            TurnError::RunNotRetryable { run_id } if *run_id == missing_checkpoint_run_id
        ),
        "unexpected retry error: {error:?}"
    );
    assert_eq!(
        store
            .process_snapshots(&turn_scope.to_resource_scope())
            .await
            .expect("snapshots")
            .len(),
        3,
        "rejected retries must not create another process"
    );
}

#[tokio::test]
async fn retry_rejects_final_checkpoint_without_creating_a_process() {
    use ironclaw_processes::{
        ProcessCheckpointId, ProcessCheckpointPayload, ProcessCheckpointPort, ProcessCheckpointRef,
        ProcessRuntimePort, ProcessSnapshotSource, RecordProcessCheckpointRequest,
        in_memory_backed_process_store,
    };

    let store = Arc::new(in_memory_backed_process_store());
    let runtime =
        AgentTurnProcessRuntime::from_process_runtime(store.clone() as Arc<dyn ProcessRuntimePort>);
    let turn_scope = scope();
    let actor = TurnActor::new(UserId::new("retry-final-owner").expect("owner"));
    let run_id = TurnRunId::new();
    let checkpoint_id =
        ProcessCheckpointId::from_trusted(TurnCheckpointId::new().as_uuid().to_string());
    let checkpoint_ref = ProcessCheckpointRef::from_trusted(checkpoint_id.as_str());
    submit_agent_process(
        store.as_ref(),
        &turn_scope,
        &actor,
        run_id,
        TurnId::new(),
        Some(checkpoint_ref.clone()),
    )
    .await;
    store
        .record_process_checkpoint(RecordProcessCheckpointRequest {
            checkpoint_id,
            process_id: process_id_from_turn_run_id(run_id),
            scope: turn_scope.to_resource_scope(),
            state_ref: ProcessCheckpointRef::from_trusted("retry-final-state"),
            payload: ProcessCheckpointPayload::new(b"final checkpoint".to_vec()).expect("payload"),
            created_at: Utc::now(),
            link_to_process: true,
            metadata: serde_json::json!({
                "kind": ironclaw_loop_contracts::LoopCheckpointKind::Final,
            }),
        })
        .await
        .expect("record final checkpoint");
    fail_agent_process(store.as_ref(), &turn_scope, run_id, Some(checkpoint_ref)).await;

    let result = runtime
        .retry_turn(RetryTurnRequest {
            scope: turn_scope.clone(),
            actor,
            run_id,
            source_binding_ref: SourceBindingRef::new("retry-final-source").expect("source"),
            reply_target_binding_ref: ReplyTargetBindingRef::new("retry-final-reply")
                .expect("reply"),
            idempotency_key: IdempotencyKey::new("retry-final").expect("idempotency"),
        })
        .await;
    assert!(matches!(
        result,
        Err(TurnError::RunNotRetryable { run_id: rejected }) if rejected == run_id
    ));
    assert_eq!(
        store
            .process_snapshots(&turn_scope.to_resource_scope())
            .await
            .expect("snapshots")
            .len(),
        1
    );
}

#[tokio::test]
async fn retry_rebinds_checkpoint_through_the_real_process_store() {
    use ironclaw_host_api::turn::SanitizedFailure;
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
            link_to_process: true,
            metadata: serde_json::json!({
                "source": "retry-test",
                "kind": ironclaw_loop_contracts::LoopCheckpointKind::BeforeModel,
            }),
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
            recovery: ironclaw_processes::ProcessFailureRecovery::Terminal,
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
        gate_ref: Some(TurnGateRef::new("gate:process-journal").expect("gate")),
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
        subagent_depth: 3,
        spawn_tree_descendant_cap: Some(17),
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
    assert_eq!(
        process.state.metadata["agent_turn"]["subagent_depth"],
        json!(3)
    );
    assert_eq!(
        process.state.metadata["agent_turn"]["spawn_tree_descendant_cap"],
        json!(17)
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
        subagent_depth: 4,
        spawn_tree_descendant_cap: Some(23),
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
    assert_eq!(round_trip.subagent_depth, 4);
    assert_eq!(round_trip.spawn_tree_descendant_cap, Some(23));
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
        _owner_user_id: Option<&ironclaw_host_api::ids::UserId>,
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
        state_ref: ironclaw_loop_contracts::LoopCheckpointStateRef::new(
            "checkpoint:state-process-journal".to_string(),
        )
        .expect("state ref"),
        reason: BlockedReason::ExternalTool {
            gate_ref: TurnGateRef::new("gate:process-journal").expect("gate"),
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
