use std::sync::Arc;

use super::port_adapters::HostManagedLoopCheckpointPort;

use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_threads::ThreadScope;
use ironclaw_turns::{
    InMemoryCheckpointStateStore, InMemoryLoopCheckpointStore, InMemoryRunProfileResolver,
    LoopCheckpointStateRef, LoopCheckpointStore, PutLoopCheckpointRequest, RunProfileResolver,
    TurnActor, TurnCheckpointId, TurnId, TurnRunId, TurnScope,
    run_profile::{
        AgentLoopHostErrorKind, CheckpointSchemaId, InMemoryLoopHostMilestoneSink,
        LoadCheckpointPayloadRequest, LoopCheckpointKind, LoopCheckpointPort,
        LoopCheckpointRequest, LoopRunContext, RunProfileResolutionRequest,
        StageCheckpointPayloadRequest,
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

fn test_checkpoint_port(
    context: LoopRunContext,
) -> (
    HostManagedLoopCheckpointPort,
    Arc<InMemoryCheckpointStateStore>,
    Arc<InMemoryLoopCheckpointStore>,
) {
    let state_store = Arc::new(InMemoryCheckpointStateStore::default());
    let checkpoint_store = Arc::new(InMemoryLoopCheckpointStore::default());
    let milestone_sink = Arc::new(InMemoryLoopHostMilestoneSink::default());
    let port = HostManagedLoopCheckpointPort::new(
        context,
        state_store.clone(),
        checkpoint_store.clone(),
        milestone_sink,
    );
    (port, state_store, checkpoint_store)
}

#[tokio::test]
async fn checkpoint_port_load_payload_roundtrips_staged_payload() {
    let context = test_run_context().await;
    let expected_schema_id = context.checkpoint_schema_id.clone();
    let expected_schema_version = context.checkpoint_schema_version;
    let (port, _state_store, _checkpoint_store) = test_checkpoint_port(context);
    let payload = br#"{"iteration":3}"#.to_vec();

    let state_ref = port
        .stage_checkpoint_payload(StageCheckpointPayloadRequest {
            kind: LoopCheckpointKind::BeforeSideEffect,
            schema_id: expected_schema_id.as_str().to_string(),
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
    let (port, _state_store, _checkpoint_store) = test_checkpoint_port(context);
    let state_ref = port
        .stage_checkpoint_payload(StageCheckpointPayloadRequest {
            kind: LoopCheckpointKind::BeforeModel,
            schema_id: expected_schema_id.as_str().to_string(),
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
    let (port, _state_store, _checkpoint_store) = test_checkpoint_port(context);
    let state_ref = port
        .stage_checkpoint_payload(StageCheckpointPayloadRequest {
            kind: LoopCheckpointKind::BeforeModel,
            schema_id: expected_schema_id.as_str().to_string(),
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
    let (port, _state_store, _checkpoint_store) = test_checkpoint_port(context);

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

#[tokio::test]
async fn checkpoint_port_load_payload_missing_state_record_is_unavailable() {
    let context = test_run_context().await;
    let expected_schema_id = context.checkpoint_schema_id.clone();
    let expected_schema_version = context.checkpoint_schema_version;
    let (port, _state_store, checkpoint_store) = test_checkpoint_port(context.clone());
    let missing_state_ref =
        LoopCheckpointStateRef::for_run(&context, "missing-state").expect("valid ref");
    let metadata = checkpoint_store
        .put_loop_checkpoint(PutLoopCheckpointRequest {
            scope: context.scope.clone(),
            turn_id: context.turn_id,
            run_id: context.run_id,
            state_ref: missing_state_ref,
            schema_id: expected_schema_id.clone(),
            schema_version: expected_schema_version,
            kind: LoopCheckpointKind::BeforeBlock,
            gate_ref: None,
        })
        .await
        .expect("write checkpoint metadata");

    let error = port
        .load_checkpoint_payload(LoadCheckpointPayloadRequest {
            checkpoint_id: metadata.checkpoint_id,
            expected_schema_id,
            expected_schema_version,
        })
        .await
        .expect_err("missing state payload must reject");

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
