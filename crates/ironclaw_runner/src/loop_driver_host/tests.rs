use std::sync::Arc;

use super::port_adapters::{HostManagedLoopCheckpointPort, HostManagedLoopProgressPort};

use ironclaw_host_api::{AgentId, FailureKind, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_threads::ThreadScope;
use ironclaw_turns::test_support::in_memory_loop_checkpoint_store;
use ironclaw_turns::{
    InMemoryRunProfileResolver, LoopCheckpointStateRef, ProcessLoopCheckpointStore,
    RunProfileResolver, TurnActor, TurnCheckpointId, TurnId, TurnRunId, TurnScope,
    run_profile::{
        AgentLoopHostErrorKind, CheckpointSchemaId, InMemoryLoopHostMilestoneSink,
        LoadCheckpointPayloadRequest, LoopCheckpointKind, LoopCheckpointPort,
        LoopCheckpointRequest, LoopHostMilestoneKind, LoopHostMilestoneSink, LoopProgressEvent,
        LoopProgressPort, LoopRecoveryClass, LoopRecoveryDisposition, LoopRecoveryStage,
        LoopRunContext, LoopSafeSummary, RunProfileResolutionRequest,
        StageCheckpointPayloadRequest, SystemInferenceTaskId,
    },
};

async fn test_run_context() -> LoopRunContext {
    let tenant_id = TenantId::new("tenant-surf-prompt-test").unwrap();
    let agent_id = AgentId::new("agent-surf-prompt-test").unwrap();
    let project_id = ProjectId::new("project-surf-prompt-test").unwrap();
    let thread_id = ThreadId::new("thread-surf-prompt-test").unwrap();
    let turn_scope = TurnScope::new(tenant_id, Some(agent_id), Some(project_id), thread_id);
    let resolved = InMemoryRunProfileResolver::default()
        .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
        .await
        .unwrap();
    LoopRunContext::new(turn_scope, TurnId::new(), TurnRunId::new(), resolved)
}

#[tokio::test]
async fn recovery_progress_adapter_preserves_sequence_and_typed_labels() {
    let context = test_run_context().await;
    let sink = Arc::new(InMemoryLoopHostMilestoneSink::default());
    let milestone_sink: Arc<dyn LoopHostMilestoneSink> = sink.clone();
    let port = HostManagedLoopProgressPort::new(context.clone(), milestone_sink);

    port.emit_loop_progress(LoopProgressEvent::FailureRecovered {
        sequence: 7,
        stage: LoopRecoveryStage::Capability,
        class: LoopRecoveryClass::Capability(FailureKind::Backend),
        disposition: LoopRecoveryDisposition::Retried,
    })
    .await
    .expect("recovery progress must reach the durable milestone seam");

    let milestones = sink.milestones();
    assert_eq!(milestones.len(), 1);
    let milestone = &milestones[0];
    assert_eq!(milestone.scope, context.scope);
    assert_eq!(milestone.turn_id, context.turn_id);
    assert_eq!(milestone.run_id, context.run_id);
    assert!(matches!(
        milestone.kind,
        LoopHostMilestoneKind::FailureRecovered {
            sequence: 7,
            stage: LoopRecoveryStage::Capability,
            class: LoopRecoveryClass::Capability(FailureKind::Backend),
            disposition: LoopRecoveryDisposition::Retried,
        }
    ));
}

#[tokio::test]
async fn compaction_redaction_progress_adapter_preserves_count_and_reason() {
    let context = test_run_context().await;
    let sink = Arc::new(InMemoryLoopHostMilestoneSink::default());
    let milestone_sink: Arc<dyn LoopHostMilestoneSink> = sink.clone();
    let port = HostManagedLoopProgressPort::new(context.clone(), milestone_sink);
    let task_id = SystemInferenceTaskId::new();

    port.emit_loop_progress(LoopProgressEvent::CompactionLeakDetected {
        task_id,
        reason_kind: LoopSafeSummary::new("redacted").expect("valid safe reason"),
        redacted_leak_count: 2,
    })
    .await
    .expect("compaction redaction progress must reach the durable milestone seam");

    let milestones = sink.milestones();
    assert_eq!(milestones.len(), 1);
    let milestone = &milestones[0];
    assert_eq!(milestone.scope, context.scope);
    assert_eq!(milestone.turn_id, context.turn_id);
    assert_eq!(milestone.run_id, context.run_id);
    assert!(matches!(
        &milestone.kind,
        LoopHostMilestoneKind::CompactionLeakDetected {
            task_id: emitted_task_id,
            reason_kind,
            redacted_leak_count: 2,
        } if *emitted_task_id == task_id && reason_kind.as_str() == "redacted"
    ));
}

fn test_checkpoint_port(
    context: LoopRunContext,
) -> (
    HostManagedLoopCheckpointPort,
    Arc<ProcessLoopCheckpointStore>,
) {
    let checkpoint_store = Arc::new(in_memory_loop_checkpoint_store());
    let milestone_sink = Arc::new(InMemoryLoopHostMilestoneSink::default());
    let port =
        HostManagedLoopCheckpointPort::new(context, checkpoint_store.clone(), milestone_sink);
    (port, checkpoint_store)
}

#[tokio::test]
async fn checkpoint_port_load_payload_roundtrips_staged_payload() {
    let context = test_run_context().await;
    let expected_schema_id = context.checkpoint_schema_id.clone();
    let expected_schema_version = context.checkpoint_schema_version;
    let (port, _checkpoint_store) = test_checkpoint_port(context);
    let payload = br#"{"iteration":3}"#.to_vec();

    let state_ref = port
        .stage_checkpoint_payload(StageCheckpointPayloadRequest {
            kind: LoopCheckpointKind::BeforeSideEffect,
            schema_id: expected_schema_id.clone(),
            payload: payload.clone(),
        })
        .await
        .expect("stage checkpoint payload");
    let checkpoint_id = port
        .checkpoint(LoopCheckpointRequest {
            kind: LoopCheckpointKind::BeforeSideEffect,
            state_ref,
            gate_ref: None,
        })
        .await
        .expect("write checkpoint metadata");

    let loaded = port
        .load_checkpoint_payload(LoadCheckpointPayloadRequest {
            checkpoint_id,
            expected_schema_id: expected_schema_id.clone(),
            expected_schema_version,
        })
        .await
        .expect("load checkpoint payload");

    assert_eq!(loaded.kind, LoopCheckpointKind::BeforeSideEffect);
    assert_eq!(loaded.schema_id, expected_schema_id);
    assert_eq!(loaded.schema_version, expected_schema_version);
    assert_eq!(loaded.payload.as_bytes(), payload.as_slice());
}

#[tokio::test]
async fn checkpoint_port_load_payload_rejects_schema_mismatch() {
    let context = test_run_context().await;
    let expected_schema_id = context.checkpoint_schema_id.clone();
    let expected_schema_version = context.checkpoint_schema_version;
    let (port, _checkpoint_store) = test_checkpoint_port(context);
    let state_ref = port
        .stage_checkpoint_payload(StageCheckpointPayloadRequest {
            kind: LoopCheckpointKind::BeforeModel,
            schema_id: expected_schema_id.clone(),
            payload: b"{}".to_vec(),
        })
        .await
        .expect("stage checkpoint payload");
    let checkpoint_id = port
        .checkpoint(LoopCheckpointRequest {
            kind: LoopCheckpointKind::BeforeModel,
            state_ref,
            gate_ref: None,
        })
        .await
        .expect("write checkpoint metadata");

    let error = port
        .load_checkpoint_payload(LoadCheckpointPayloadRequest {
            checkpoint_id,
            expected_schema_id: CheckpointSchemaId::new("different_checkpoint_schema")
                .expect("valid schema"),
            expected_schema_version,
        })
        .await
        .expect_err("schema mismatch must reject");

    assert_eq!(error.kind, AgentLoopHostErrorKind::Invalid);
}

#[tokio::test]
async fn checkpoint_port_load_payload_rejects_schema_version_mismatch() {
    let context = test_run_context().await;
    let expected_schema_id = context.checkpoint_schema_id.clone();
    let stored_schema_version = context.checkpoint_schema_version;
    let (port, _checkpoint_store) = test_checkpoint_port(context);
    let state_ref = port
        .stage_checkpoint_payload(StageCheckpointPayloadRequest {
            kind: LoopCheckpointKind::BeforeModel,
            schema_id: expected_schema_id.clone(),
            payload: b"{}".to_vec(),
        })
        .await
        .expect("stage checkpoint payload");
    let checkpoint_id = port
        .checkpoint(LoopCheckpointRequest {
            kind: LoopCheckpointKind::BeforeModel,
            state_ref,
            gate_ref: None,
        })
        .await
        .expect("write checkpoint metadata");

    // Load with a bumped schema version — stored = N, expected = N+1.
    let bumped_version = ironclaw_turns::RunProfileVersion::new(stored_schema_version.as_u64() + 1);

    let error = port
        .load_checkpoint_payload(LoadCheckpointPayloadRequest {
            checkpoint_id,
            expected_schema_id,
            expected_schema_version: bumped_version,
        })
        .await
        .expect_err("schema version mismatch must reject");

    assert_eq!(error.kind, AgentLoopHostErrorKind::Invalid);
}

#[tokio::test]
async fn checkpoint_port_load_payload_missing_metadata_is_unavailable() {
    let context = test_run_context().await;
    let expected_schema_id = context.checkpoint_schema_id.clone();
    let expected_schema_version = context.checkpoint_schema_version;
    let (port, _checkpoint_store) = test_checkpoint_port(context);

    let error = port
        .load_checkpoint_payload(LoadCheckpointPayloadRequest {
            checkpoint_id: TurnCheckpointId::new(),
            expected_schema_id,
            expected_schema_version,
        })
        .await
        .expect_err("missing metadata must reject");

    assert_eq!(error.kind, AgentLoopHostErrorKind::Unavailable);
}

fn thread_scope_for(context: &LoopRunContext, owner: Option<UserId>) -> ThreadScope {
    ThreadScope {
        tenant_id: context.scope.tenant_id.clone(),
        agent_id: context
            .scope
            .agent_id
            .clone()
            .expect("test run context is agent-scoped"),
        project_id: context.scope.project_id.clone(),
        owner_user_id: owner,
        mission_id: None,
    }
}

#[tokio::test]
async fn validate_thread_scope_rejects_owner_mismatch() {
    // Defense in depth for the thread-owner MountView divergence: the thread
    // store keys threads by owner, so a host thread scope whose owner differs
    // from the run's authenticated actor silently reads the wrong
    // `owners/<user>` subtree and fails with `UnknownThread`. Fail loud here
    // instead.
    let context = test_run_context()
        .await
        .with_actor(TurnActor::new(UserId::new("local-user").unwrap()));
    let thread_scope = thread_scope_for(&context, Some(UserId::new("reborn-cli").unwrap()));

    let error = super::validate_thread_scope(&thread_scope, &context)
        .expect_err("owner mismatch must be rejected");
    assert!(matches!(
        error,
        super::RebornLoopDriverHostError::ScopeMismatch { .. }
    ));
}

#[tokio::test]
async fn validate_thread_scope_accepts_matching_owner() {
    let context = test_run_context()
        .await
        .with_actor(TurnActor::new(UserId::new("local-user").unwrap()));
    let thread_scope = thread_scope_for(&context, Some(UserId::new("local-user").unwrap()));

    super::validate_thread_scope(&thread_scope, &context).expect("matching owner must validate");
}

#[tokio::test]
async fn validate_thread_scope_skips_owner_check_without_actor() {
    // When the run carries no actor (system/legacy turns), the owner axis
    // cannot be cross-checked; the guard must not reject these.
    let context = test_run_context().await;
    let thread_scope = thread_scope_for(&context, Some(UserId::new("local-user").unwrap()));

    super::validate_thread_scope(&thread_scope, &context)
        .expect("absent actor must skip the owner check");
}

#[tokio::test]
async fn checkpoint_write_rejects_foreign_run_scoped_state_ref() {
    // Regression: the checkpoint WRITE path must only stage refs scoped to the
    // current run. A `checkpoint:{other_run}:{token}` ref is a read-only
    // retry-resume link; accepting it on write would index the record against a
    // foreign run's payload and later fail to load. (CodeRabbit PR #4841.)
    let context = test_run_context().await;
    let foreign_run = TurnRunId::new();
    let (port, _checkpoint_store) = test_checkpoint_port(context);

    let foreign_ref =
        LoopCheckpointStateRef::new(format!("checkpoint:{foreign_run}:retry_state")).unwrap();

    let error = port
        .checkpoint(LoopCheckpointRequest {
            kind: LoopCheckpointKind::BeforeModel,
            state_ref: foreign_ref,
            gate_ref: None,
        })
        .await
        .expect_err("foreign run-scoped checkpoint ref must be rejected on write");

    assert_eq!(error.kind, AgentLoopHostErrorKind::CheckpointRejected);
}
