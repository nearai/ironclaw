use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use super::port_adapters::HostManagedLoopCheckpointPort;

use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_threads::ThreadScope;
use ironclaw_turns::{
    CheckpointStateRecord, CheckpointStateStore, GetCheckpointStateRequest,
    InMemoryCheckpointStateStore, InMemoryLoopCheckpointStore, InMemoryRunProfileResolver,
    LoopCheckpointStateRef, LoopCheckpointStore, PutCheckpointStateRequest,
    PutLoopCheckpointRequest, RunProfileResolver, TurnActor, TurnCheckpointId, TurnError, TurnId,
    TurnRunId, TurnScope,
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

#[derive(Default)]
struct CountingCheckpointStateStore {
    inner: InMemoryCheckpointStateStore,
    get_calls: AtomicUsize,
}

impl CountingCheckpointStateStore {
    fn get_calls(&self) -> usize {
        self.get_calls.load(Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl CheckpointStateStore for CountingCheckpointStateStore {
    async fn put_checkpoint_state(
        &self,
        request: PutCheckpointStateRequest,
    ) -> Result<CheckpointStateRecord, TurnError> {
        self.inner.put_checkpoint_state(request).await
    }

    async fn get_checkpoint_state(
        &self,
        request: GetCheckpointStateRequest,
    ) -> Result<Option<CheckpointStateRecord>, TurnError> {
        self.get_calls.fetch_add(1, Ordering::SeqCst);
        self.inner.get_checkpoint_state(request).await
    }
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
async fn checkpoint_port_skips_read_back_for_host_staged_ref() {
    let context = test_run_context().await;
    let state_store = Arc::new(CountingCheckpointStateStore::default());
    let checkpoint_store = Arc::new(InMemoryLoopCheckpointStore::default());
    let milestone_sink = Arc::new(InMemoryLoopHostMilestoneSink::default());
    let port = HostManagedLoopCheckpointPort::new(
        context.clone(),
        state_store.clone(),
        checkpoint_store,
        milestone_sink,
    );

    let state_ref = port
        .stage_checkpoint_payload(StageCheckpointPayloadRequest {
            kind: LoopCheckpointKind::BeforeModel,
            schema_id: context.checkpoint_schema_id.as_str().to_string(),
            payload: br#"{"iteration":1}"#.to_vec(),
        })
        .await
        .expect("host staging should write payload");

    port.checkpoint(LoopCheckpointRequest {
        kind: LoopCheckpointKind::BeforeModel,
        state_ref,
        gate_ref: None,
    })
    .await
    .expect("host-staged ref should checkpoint without a read-back");

    assert_eq!(
        state_store.get_calls(),
        0,
        "checkpoint should trust refs returned by this host's stage call"
    );

    let directly_staged = state_store
        .put_checkpoint_state(PutCheckpointStateRequest::new(
            context.scope.clone(),
            context.turn_id,
            context.run_id,
            context.checkpoint_schema_id.clone(),
            context.checkpoint_schema_version,
            LoopCheckpointKind::BeforeModel,
            br#"{"iteration":2}"#.to_vec(),
        ))
        .await
        .expect("direct store staging should work");

    port.checkpoint(LoopCheckpointRequest {
        kind: LoopCheckpointKind::BeforeModel,
        state_ref: directly_staged.state_ref,
        gate_ref: None,
    })
    .await
    .expect("directly staged ref should still be accepted through store verification");

    assert_eq!(
        state_store.get_calls(),
        1,
        "refs not minted by this host must still use durable store verification"
    );
}

#[tokio::test]
async fn checkpoint_port_load_payload_follows_retry_linked_source_run_ref() {
    let source_context = test_run_context().await;
    let retry_context = LoopRunContext::new(
        source_context.scope.clone(),
        source_context.turn_id,
        TurnRunId::new(),
        source_context.resolved_run_profile.clone(),
    );
    let expected_schema_id = retry_context.checkpoint_schema_id.clone();
    let expected_schema_version = retry_context.checkpoint_schema_version;
    let (retry_port, state_store, checkpoint_store) = test_checkpoint_port(retry_context.clone());
    let payload = br#"{"iteration":4,"retry":true}"#.to_vec();

    let staged = state_store
        .put_checkpoint_state(PutCheckpointStateRequest::new(
            source_context.scope.clone(),
            source_context.turn_id,
            source_context.run_id,
            expected_schema_id.clone(),
            expected_schema_version,
            LoopCheckpointKind::BeforeModel,
            payload.clone(),
        ))
        .await
        .expect("stage source run payload");
    let token = staged
        .state_ref
        .as_str()
        .strip_prefix("checkpoint:")
        .expect("state store ref should use checkpoint prefix");
    let source_run_ref =
        LoopCheckpointStateRef::for_run(&source_context, token).expect("source run ref");
    let metadata = checkpoint_store
        .put_loop_checkpoint(PutLoopCheckpointRequest {
            scope: retry_context.scope.clone(),
            turn_id: retry_context.turn_id,
            run_id: retry_context.run_id,
            state_ref: source_run_ref,
            schema_id: expected_schema_id.clone(),
            schema_version: expected_schema_version,
            kind: LoopCheckpointKind::BeforeModel,
            gate_ref: None,
        })
        .await
        .expect("write retry checkpoint link metadata");

    let loaded = retry_port
        .load_checkpoint_payload(LoadCheckpointPayloadRequest {
            checkpoint_id: metadata.checkpoint_id,
            expected_schema_id: expected_schema_id.clone(),
            expected_schema_version,
        })
        .await
        .expect("load retry-linked checkpoint payload");

    assert_eq!(loaded.kind, LoopCheckpointKind::BeforeModel);
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

#[tokio::test]
async fn checkpoint_write_rejects_foreign_run_scoped_state_ref() {
    // Regression: the checkpoint WRITE path must only stage refs scoped to the
    // current run. A `checkpoint:{other_run}:{token}` ref is a read-only
    // retry-resume link; accepting it on write would index the record against a
    // foreign run's payload and later fail to load. (CodeRabbit PR #4841.)
    let context = test_run_context().await;
    let foreign_run = TurnRunId::new();
    let (port, _state_store, _checkpoint_store) = test_checkpoint_port(context);

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
