use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_host_api::{AgentId, CapabilityId, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_loop_support::{
    HostManagedModelError, HostManagedModelGateway, HostManagedModelRequest,
    HostManagedModelResponse,
};
use ironclaw_reborn::{
    RebornLoopDriverHostFactory, RebornLoopDriverHostRequest, TextOnlyLoopHostConfig,
    turn_runner::HostFactory,
};
use ironclaw_threads::{
    AcceptInboundMessageRequest, EnsureThreadRequest, InMemorySessionThreadService, MessageContent,
    MessageKind, MessageStatus, SessionThreadService, ThreadHistoryRequest, ThreadScope,
};
use ironclaw_turns::{
    AcceptedMessageRef, CheckpointStateStore, EventCursor, GetCheckpointStateRequest,
    GetLoopCheckpointRequest, InMemoryCheckpointStateStore, InMemoryLoopCheckpointStore,
    InMemoryRunProfileResolver, InMemoryTurnStateStore, InMemoryTurnStateStoreLimits,
    LoopCheckpointRecord, LoopCheckpointStore, PutCheckpointStateRequest, PutLoopCheckpointRequest,
    ReplyTargetBindingRef, RunProfileId, RunProfileResolutionRequest, RunProfileResolver,
    RunProfileVersion, SourceBindingRef, TurnError, TurnLeaseToken, TurnRunId, TurnRunnerId,
    TurnScope, TurnStatus,
    run_profile::{
        AgentLoopDriverHost, AgentLoopHostErrorKind, CapabilityDeniedReasonKind,
        CapabilityInputRef, CapabilityInvocation, CapabilityOutcome, CapabilitySurfaceVersion,
        FinalizeAssistantMessage, InMemoryLoopHostMilestoneSink, LoopCapabilityPort,
        LoopCheckpointKind, LoopCheckpointPort, LoopCheckpointRequest, LoopContextRequest,
        LoopDriverId, LoopDriverNoteKind, LoopHostMilestone, LoopInputCursor, LoopInputCursorToken,
        LoopInputPort, LoopModelRequest, LoopProgressEvent, LoopPromptBundleRequest,
        LoopPromptPort, LoopRunContext, ParentLoopOutput, PromptMode, VisibleCapabilityRequest,
    },
    runner::ClaimedTurnRun,
};

#[tokio::test]
async fn text_only_host_factory_builds_complete_agent_loop_driver_host() {
    let fixture = HostFixture::new("thread-host-complete", "hello reborn").await;
    let host = fixture.build_host().await;
    let host_dyn: &(dyn AgentLoopDriverHost + Send + Sync) = &host;

    assert_eq!(host_dyn.run_context().run_id, fixture.context.run_id);

    let context = host_dyn
        .load_loop_context(LoopContextRequest {
            after: None,
            limit: 8,
        })
        .await
        .unwrap();
    assert_eq!(context.messages.len(), 1);

    let input = host_dyn
        .poll_inputs(LoopInputCursor::origin_for_run(&fixture.context), 8)
        .await
        .unwrap();
    assert!(input.inputs.is_empty());
    host_dyn.ack_inputs(input.next_cursor).await.unwrap();

    let surface = host_dyn
        .visible_capabilities(VisibleCapabilityRequest)
        .await
        .unwrap();
    assert!(surface.descriptors.is_empty());

    let prompt_bundle = host_dyn
        .build_prompt_bundle(LoopPromptBundleRequest {
            mode: PromptMode::TextOnly,
            context_cursor: None,
            surface_version: None,
            checkpoint_state_ref: None,
            max_messages: Some(8),
        })
        .await
        .unwrap();
    assert_eq!(prompt_bundle.messages.len(), 1);

    let model_response = host_dyn
        .stream_model(LoopModelRequest {
            messages: prompt_bundle.messages,
            surface_version: Some(surface.version.clone()),
            model_preference: None,
        })
        .await
        .unwrap();
    let ParentLoopOutput::AssistantReply(reply) = model_response.output else {
        panic!("expected assistant reply");
    };

    let reply_ref = host_dyn
        .finalize_assistant_message(FinalizeAssistantMessage { reply })
        .await
        .unwrap();
    assert!(reply_ref.as_str().starts_with("msg:"));

    let checkpoint_state = fixture
        .stage_checkpoint_state(
            LoopCheckpointKind::BeforeModel,
            b"RAW_CHECKPOINT_PAYLOAD sk-secret",
        )
        .await;
    let checkpoint_id = host_dyn
        .checkpoint(LoopCheckpointRequest {
            kind: LoopCheckpointKind::BeforeModel,
            state_ref: checkpoint_state.state_ref.clone(),
        })
        .await
        .unwrap();
    let _ = checkpoint_id;

    host_dyn
        .emit_loop_progress(
            LoopProgressEvent::driver_note(LoopDriverNoteKind::Planning, "safe driver note")
                .unwrap(),
        )
        .await
        .unwrap();

    let history = fixture
        .thread_service
        .list_thread_history(ThreadHistoryRequest {
            scope: fixture.thread_scope.clone(),
            thread_id: fixture.thread_id.clone(),
        })
        .await
        .unwrap();
    let assistant = history
        .messages
        .iter()
        .find(|message| message.kind == MessageKind::Assistant)
        .expect("assistant reply should be persisted");
    assert_eq!(assistant.status, MessageStatus::Finalized);
    assert_eq!(assistant.content.as_deref(), Some("model says hi"));

    assert_eq!(fixture.gateway.requests().len(), 1);
    assert_eq!(
        fixture.gateway.requests()[0].messages[0].content,
        "hello reborn"
    );

    let milestone_names = fixture.milestone_names();
    assert_eq!(
        milestone_names,
        vec![
            "prompt_bundle_built",
            "model_started",
            "model_completed",
            "assistant_reply_finalized",
            "checkpoint_created",
            "driver_note",
        ]
    );
    assert_public_milestones_hide_raw_payloads(&fixture.milestones());
}

#[tokio::test]
async fn text_only_host_factory_implements_turn_runner_host_factory() {
    let fixture = HostFixture::new("thread-host-turn-runner-factory", "hello runner").await;
    let factory = fixture.factory();

    let host = factory.create_host(&fixture.claimed).await.unwrap();

    assert_eq!(host.run_context().run_id, fixture.context.run_id);
    let context = host
        .load_loop_context(LoopContextRequest {
            after: None,
            limit: 8,
        })
        .await
        .unwrap();
    assert_eq!(context.messages.len(), 1);
}

#[tokio::test]
async fn text_only_host_e2e_flow_persists_checkpoint_mapping_in_turn_state_store() {
    let fixture = HostFixture::new("thread-host-turn-state-e2e", "hello durable host").await;
    let turn_state_store = Arc::new(InMemoryTurnStateStore::default());
    let host = fixture
        .factory_with_loop_checkpoint_store(turn_state_store.clone())
        .build_text_only_host(RebornLoopDriverHostRequest {
            claimed_run: fixture.claimed.clone(),
            loop_run_context: fixture.context.clone(),
        })
        .await
        .unwrap();
    let host_dyn: &(dyn AgentLoopDriverHost + Send + Sync) = &host;

    let surface = host_dyn
        .visible_capabilities(VisibleCapabilityRequest)
        .await
        .unwrap();
    let surface_version = surface.version.clone();
    let prompt_bundle = host_dyn
        .build_prompt_bundle(LoopPromptBundleRequest {
            mode: PromptMode::TextOnly,
            context_cursor: None,
            surface_version: Some(surface_version.clone()),
            checkpoint_state_ref: None,
            max_messages: Some(8),
        })
        .await
        .unwrap();
    let model_response = host_dyn
        .stream_model(LoopModelRequest {
            messages: prompt_bundle.messages,
            surface_version: Some(surface_version.clone()),
            model_preference: None,
        })
        .await
        .unwrap();
    let ParentLoopOutput::AssistantReply(reply) = model_response.output else {
        panic!("expected assistant reply");
    };
    host_dyn
        .finalize_assistant_message(FinalizeAssistantMessage { reply })
        .await
        .unwrap();
    let gateway_requests = fixture.gateway.requests();
    assert_eq!(gateway_requests.len(), 1);
    assert_eq!(gateway_requests[0].run_id, fixture.context.run_id);
    assert_eq!(gateway_requests[0].turn_id, fixture.context.turn_id);
    assert_eq!(
        gateway_requests[0].surface_version.as_ref(),
        Some(&surface_version)
    );

    let checkpoint_state = fixture
        .stage_checkpoint_state(LoopCheckpointKind::BeforeBlock, b"durable resume bytes")
        .await;
    let checkpoint_id = host_dyn
        .checkpoint(LoopCheckpointRequest {
            kind: LoopCheckpointKind::BeforeBlock,
            state_ref: checkpoint_state.state_ref.clone(),
        })
        .await
        .unwrap();

    let snapshot = turn_state_store.persistence_snapshot();
    assert_eq!(snapshot.loop_checkpoints.len(), 1);
    let reopened = InMemoryTurnStateStore::from_persistence_snapshot(
        snapshot,
        InMemoryTurnStateStoreLimits::default(),
    )
    .unwrap();
    let checkpoint_record = reopened
        .get_loop_checkpoint(GetLoopCheckpointRequest {
            scope: fixture.context.scope.clone(),
            turn_id: fixture.context.turn_id,
            run_id: fixture.context.run_id,
            checkpoint_id,
        })
        .await
        .unwrap()
        .expect("checkpoint id should survive turn-state reload");
    assert_eq!(checkpoint_record.state_ref, checkpoint_state.state_ref);

    let history = fixture
        .thread_service
        .list_thread_history(ThreadHistoryRequest {
            scope: fixture.thread_scope.clone(),
            thread_id: fixture.thread_id.clone(),
        })
        .await
        .unwrap();
    assert!(history.messages.iter().any(|message| {
        message.kind == MessageKind::Assistant
            && message.status == MessageStatus::Finalized
            && message.content.as_deref() == Some("model says hi")
    }));
}

#[tokio::test]
async fn text_only_host_prompt_accepts_empty_surface_version() {
    let fixture = HostFixture::new("thread-host-prompt-surface", "hello reborn").await;
    let host = fixture.build_host().await;
    let surface = host
        .visible_capabilities(VisibleCapabilityRequest)
        .await
        .unwrap();

    let prompt_bundle = host
        .build_prompt_bundle(LoopPromptBundleRequest {
            mode: PromptMode::TextOnly,
            context_cursor: None,
            surface_version: Some(surface.version),
            checkpoint_state_ref: None,
            max_messages: Some(8),
        })
        .await
        .unwrap();

    assert_eq!(prompt_bundle.messages.len(), 1);
}

#[tokio::test]
async fn text_only_host_prompt_rejects_stale_surface_version() {
    let fixture = HostFixture::new("thread-host-prompt-stale", "hello reborn").await;
    let host = fixture.build_host().await;

    let error = host
        .build_prompt_bundle(LoopPromptBundleRequest {
            mode: PromptMode::TextOnly,
            context_cursor: None,
            surface_version: Some(CapabilitySurfaceVersion::new("stale:v1").unwrap()),
            checkpoint_state_ref: None,
            max_messages: Some(8),
        })
        .await
        .unwrap_err();

    assert_eq!(error.kind, AgentLoopHostErrorKind::StaleSurface);
}

#[tokio::test]
async fn text_only_host_prompt_rejects_codeact_mode_and_zero_budget() {
    let fixture = HostFixture::new("thread-host-prompt-mode", "hello reborn").await;
    let host = fixture.build_host().await;

    let codeact = host
        .build_prompt_bundle(LoopPromptBundleRequest {
            mode: PromptMode::CodeAct,
            context_cursor: None,
            surface_version: None,
            checkpoint_state_ref: None,
            max_messages: Some(8),
        })
        .await
        .unwrap_err();
    assert_eq!(codeact.kind, AgentLoopHostErrorKind::PolicyDenied);

    let zero_budget = host
        .build_prompt_bundle(LoopPromptBundleRequest {
            mode: PromptMode::TextOnly,
            context_cursor: None,
            surface_version: None,
            checkpoint_state_ref: None,
            max_messages: Some(0),
        })
        .await
        .unwrap_err();
    assert_eq!(zero_budget.kind, AgentLoopHostErrorKind::BudgetExceeded);
}

#[tokio::test]
async fn text_only_host_factory_rejects_scope_mismatch() {
    let fixture = HostFixture::new("thread-host-scope", "hello").await;
    let mut wrong_context = fixture.context.clone();
    wrong_context.run_id = TurnRunId::new();

    let error = fixture
        .factory()
        .build_text_only_host(RebornLoopDriverHostRequest {
            claimed_run: fixture.claimed.clone(),
            loop_run_context: wrong_context,
        })
        .await
        .unwrap_err();

    assert!(error.to_string().contains("claimed run"));
}

#[tokio::test]
async fn text_only_host_factory_rejects_non_running_claimed_run() {
    let fixture = HostFixture::new("thread-host-non-running", "hello").await;
    let mut claimed = fixture.claimed.clone();
    claimed.state.status = TurnStatus::Queued;

    let error = fixture
        .factory()
        .build_text_only_host(RebornLoopDriverHostRequest {
            claimed_run: claimed,
            loop_run_context: fixture.context.clone(),
        })
        .await
        .unwrap_err();

    assert!(error.to_string().contains("must be running"));
}

#[tokio::test]
async fn text_only_host_factory_rejects_thread_scope_mismatch() {
    let fixture = HostFixture::new("thread-host-thread-scope-mismatch", "hello").await;
    let wrong_scope = ThreadScope {
        tenant_id: TenantId::new("tenant-other").unwrap(),
        ..fixture.thread_scope.clone()
    };
    let factory = RebornLoopDriverHostFactory::new(
        Arc::clone(&fixture.thread_service),
        wrong_scope,
        Arc::clone(&fixture.gateway),
        fixture.checkpoint_state_store.clone(),
        fixture.loop_checkpoint_store.clone(),
        fixture.milestone_sink.clone(),
        TextOnlyLoopHostConfig { max_messages: 8 },
    );

    let error = factory
        .build_text_only_host(RebornLoopDriverHostRequest {
            claimed_run: fixture.claimed.clone(),
            loop_run_context: fixture.context.clone(),
        })
        .await
        .unwrap_err();

    assert!(error.to_string().contains("thread scope"));
}

#[tokio::test]
async fn text_only_host_factory_rejects_agentless_turn_scope() {
    let fixture = HostFixture::new("thread-host-agentless-scope", "hello").await;
    let mut context = fixture.context.clone();
    context.scope.agent_id = None;
    let mut claimed = fixture.claimed.clone();
    claimed.state.scope = context.scope.clone();

    let error = fixture
        .factory()
        .build_text_only_host(RebornLoopDriverHostRequest {
            claimed_run: claimed,
            loop_run_context: context,
        })
        .await
        .unwrap_err();

    assert!(error.to_string().contains("agent-scoped thread"));
}

#[tokio::test]
async fn text_only_host_factory_rejects_persisted_profile_identity_mismatch() {
    let fixture = HostFixture::new("thread-host-profile-mismatch", "hello").await;
    let mut claimed = fixture.claimed.clone();
    claimed.state.resolved_run_profile_version = RunProfileVersion::new(999);

    let error = fixture
        .factory()
        .build_text_only_host(RebornLoopDriverHostRequest {
            claimed_run: claimed,
            loop_run_context: fixture.context.clone(),
        })
        .await
        .unwrap_err();

    assert!(error.to_string().contains("profile identity"));
}

#[tokio::test]
async fn text_only_host_factory_rejects_loop_driver_identity_mismatch() {
    let fixture = HostFixture::new("thread-host-driver-mismatch", "hello").await;
    let mut wrong_context = fixture.context.clone();
    wrong_context.loop_driver_id = LoopDriverId::new("other_loop_driver").unwrap();

    let error = fixture
        .factory()
        .build_text_only_host(RebornLoopDriverHostRequest {
            claimed_run: fixture.claimed.clone(),
            loop_run_context: wrong_context,
        })
        .await
        .unwrap_err();

    assert!(error.to_string().contains("driver identity"));
}

#[tokio::test]
async fn no_extra_loop_input_port_rejects_foreign_cursor() {
    let fixture = HostFixture::new("thread-host-input", "hello").await;
    let host = fixture.build_host().await;
    let other_context = LoopRunContext::new(
        fixture.context.scope.clone(),
        fixture.context.turn_id,
        TurnRunId::new(),
        fixture.context.resolved_run_profile.clone(),
    );

    let error = host
        .poll_inputs(
            LoopInputCursor::from_host_token(
                &other_context,
                LoopInputCursorToken::new("input-cursor:foreign").unwrap(),
            ),
            8,
        )
        .await
        .unwrap_err();

    assert_eq!(error.kind, AgentLoopHostErrorKind::ScopeMismatch);
}

#[tokio::test]
async fn no_extra_loop_input_port_ack_rejects_foreign_cursor() {
    let fixture = HostFixture::new("thread-host-input-ack", "hello").await;
    let host = fixture.build_host().await;
    let other_context = LoopRunContext::new(
        fixture.context.scope.clone(),
        fixture.context.turn_id,
        TurnRunId::new(),
        fixture.context.resolved_run_profile.clone(),
    );

    let error = host
        .ack_inputs(LoopInputCursor::from_host_token(
            &other_context,
            LoopInputCursorToken::new("input-cursor:foreign-ack").unwrap(),
        ))
        .await
        .unwrap_err();

    assert_eq!(error.kind, AgentLoopHostErrorKind::ScopeMismatch);
}

#[tokio::test]
async fn text_only_host_checkpoint_port_persists_ref_without_public_payload() {
    let fixture = HostFixture::new("thread-host-checkpoint", "hello").await;
    let host = fixture.build_host().await;
    let checkpoint_state = fixture
        .stage_checkpoint_state(
            LoopCheckpointKind::BeforeSideEffect,
            b"RAW_CHECKPOINT_PAYLOAD sk-secret /host/path tool_input",
        )
        .await;

    let checkpoint_id = host
        .checkpoint(LoopCheckpointRequest {
            kind: LoopCheckpointKind::BeforeSideEffect,
            state_ref: checkpoint_state.state_ref.clone(),
        })
        .await
        .unwrap();
    let checkpoint_record = fixture
        .loop_checkpoint_store
        .get_loop_checkpoint(GetLoopCheckpointRequest {
            scope: fixture.context.scope.clone(),
            turn_id: fixture.context.turn_id,
            run_id: fixture.context.run_id,
            checkpoint_id,
        })
        .await
        .unwrap()
        .expect("returned checkpoint id should resolve to staged state ref");
    assert_eq!(checkpoint_record.state_ref, checkpoint_state.state_ref);
    assert!(
        fixture
            .checkpoint_state_store
            .get_checkpoint_state(GetCheckpointStateRequest {
                scope: fixture.context.scope.clone(),
                turn_id: fixture.context.turn_id,
                run_id: fixture.context.run_id,
                state_ref: checkpoint_record.state_ref,
                schema_id: fixture.context.checkpoint_schema_id.clone(),
                schema_version: fixture.context.checkpoint_schema_version,
                kind: LoopCheckpointKind::BeforeSideEffect,
            })
            .await
            .unwrap()
            .is_some()
    );

    let wire = format!(
        "{}{}",
        serde_json::to_string(&checkpoint_id).unwrap(),
        serde_json::to_string(&fixture.milestones()).unwrap()
    );
    for forbidden in [
        "RAW_CHECKPOINT_PAYLOAD",
        "sk-secret",
        "/host/path",
        "tool_input",
    ] {
        assert!(
            !wire.contains(forbidden),
            "public checkpoint wire leaked {forbidden}"
        );
    }
}

#[tokio::test]
async fn text_only_host_checkpoint_port_rejects_foreign_state_ref() {
    let fixture = HostFixture::new("thread-host-checkpoint-foreign", "hello").await;
    let host = fixture.build_host().await;
    let foreign = HostFixture::new("thread-host-checkpoint-other", "hello").await;
    let foreign_state = foreign
        .stage_checkpoint_state(LoopCheckpointKind::BeforeModel, b"foreign state")
        .await;

    let error = host
        .checkpoint(LoopCheckpointRequest {
            kind: LoopCheckpointKind::BeforeModel,
            state_ref: foreign_state.state_ref,
        })
        .await
        .unwrap_err();

    assert_eq!(error.kind, AgentLoopHostErrorKind::CheckpointRejected);
}

#[tokio::test]
async fn text_only_host_checkpoint_port_rejects_kind_mismatch() {
    let fixture = HostFixture::new("thread-host-checkpoint-kind", "hello").await;
    let host = fixture.build_host().await;
    let state = fixture
        .stage_checkpoint_state(LoopCheckpointKind::BeforeModel, b"model checkpoint")
        .await;

    let error = host
        .checkpoint(LoopCheckpointRequest {
            kind: LoopCheckpointKind::BeforeSideEffect,
            state_ref: state.state_ref,
        })
        .await
        .unwrap_err();

    assert_eq!(error.kind, AgentLoopHostErrorKind::CheckpointRejected);
}

#[tokio::test]
async fn text_only_host_checkpoint_port_maps_store_failures_to_unavailable() {
    let fixture = HostFixture::new("thread-host-checkpoint-store-error", "hello").await;
    let factory = fixture.factory_with_loop_checkpoint_store(Arc::new(FailingLoopCheckpointStore));
    let host = factory
        .build_text_only_host(RebornLoopDriverHostRequest {
            claimed_run: fixture.claimed.clone(),
            loop_run_context: fixture.context.clone(),
        })
        .await
        .unwrap();
    let state = fixture
        .stage_checkpoint_state(LoopCheckpointKind::BeforeBlock, b"state before store error")
        .await;

    let error = host
        .checkpoint(LoopCheckpointRequest {
            kind: LoopCheckpointKind::BeforeBlock,
            state_ref: state.state_ref,
        })
        .await
        .unwrap_err();

    assert_eq!(error.kind, AgentLoopHostErrorKind::Unavailable);
}

#[tokio::test]
async fn text_only_host_empty_capability_surface_denies_invocation() {
    let fixture = HostFixture::new("thread-host-capability", "hello").await;
    let host = fixture.build_host().await;
    let surface = host
        .visible_capabilities(VisibleCapabilityRequest)
        .await
        .unwrap();

    let outcome = host
        .invoke_capability_batch(ironclaw_turns::run_profile::CapabilityBatchInvocation {
            invocations: vec![CapabilityInvocation {
                surface_version: surface.version.clone(),
                capability_id: CapabilityId::new("demo.echo").unwrap(),
                input_ref: CapabilityInputRef::new("input:opaque-tool-input").unwrap(),
            }],
            stop_on_first_suspension: true,
        })
        .await
        .unwrap();

    assert!(matches!(
        outcome.outcomes.as_slice(),
        [CapabilityOutcome::Denied(denied)] if denied.reason_kind == CapabilityDeniedReasonKind::EmptySurface
    ));

    let stale = host
        .invoke_capability(CapabilityInvocation {
            surface_version: CapabilitySurfaceVersion::new("other:v1").unwrap(),
            capability_id: CapabilityId::new("demo.echo").unwrap(),
            input_ref: CapabilityInputRef::new("input:opaque-tool-input").unwrap(),
        })
        .await
        .unwrap_err();
    assert_eq!(stale.kind, AgentLoopHostErrorKind::StaleSurface);
}

struct HostFixture {
    thread_service: Arc<InMemorySessionThreadService>,
    checkpoint_state_store: Arc<InMemoryCheckpointStateStore>,
    loop_checkpoint_store: Arc<InMemoryLoopCheckpointStore>,
    gateway: Arc<RecordingGateway>,
    milestone_sink: Arc<InMemoryLoopHostMilestoneSink>,
    thread_scope: ThreadScope,
    thread_id: ThreadId,
    claimed: ClaimedTurnRun,
    context: LoopRunContext,
}

impl HostFixture {
    async fn new(thread_name: &str, user_content: &str) -> Self {
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        let checkpoint_state_store = Arc::new(InMemoryCheckpointStateStore::default());
        let loop_checkpoint_store = Arc::new(InMemoryLoopCheckpointStore::default());
        let gateway = Arc::new(RecordingGateway::reply("model says hi"));
        let milestone_sink = Arc::new(InMemoryLoopHostMilestoneSink::default());
        let tenant_id = TenantId::new("tenant-text-host").unwrap();
        let agent_id = AgentId::new("agent-text-host").unwrap();
        let project_id = ProjectId::new("project-text-host").unwrap();
        let user_id = UserId::new("user-text-host").unwrap();
        let thread_id = ThreadId::new(thread_name).unwrap();
        let thread_scope = ThreadScope {
            tenant_id: tenant_id.clone(),
            agent_id: agent_id.clone(),
            project_id: Some(project_id.clone()),
            owner_user_id: Some(user_id.clone()),
            mission_id: None,
        };
        thread_service
            .ensure_thread(EnsureThreadRequest {
                scope: thread_scope.clone(),
                thread_id: Some(thread_id.clone()),
                created_by_actor_id: user_id.to_string(),
                title: None,
                metadata_json: None,
            })
            .await
            .unwrap();
        let accepted = thread_service
            .accept_inbound_message(AcceptInboundMessageRequest {
                scope: thread_scope.clone(),
                thread_id: thread_id.clone(),
                actor_id: user_id.to_string(),
                source_binding_id: Some("source-web".to_string()),
                reply_target_binding_id: Some("reply-web".to_string()),
                external_event_id: Some(format!("event-{thread_name}")),
                content: MessageContent::text(user_content),
            })
            .await
            .unwrap();

        let turn_scope = TurnScope::new(
            tenant_id,
            Some(agent_id),
            Some(project_id),
            thread_id.clone(),
        );
        let resolved = InMemoryRunProfileResolver::default()
            .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
            .await
            .unwrap();
        let turn_id = ironclaw_turns::TurnId::new();
        let run_id = TurnRunId::new();
        let state = ironclaw_turns::TurnRunState {
            scope: turn_scope.clone(),
            turn_id,
            run_id,
            status: TurnStatus::Running,
            accepted_message_ref: AcceptedMessageRef::new(format!("accepted-{thread_name}"))
                .unwrap(),
            source_binding_ref: SourceBindingRef::new("source-web").unwrap(),
            reply_target_binding_ref: ReplyTargetBindingRef::new("reply-web").unwrap(),
            resolved_run_profile_id: RunProfileId::default_profile(),
            resolved_run_profile_version: RunProfileVersion::new(1),
            received_at: Utc::now(),
            checkpoint_id: None,
            gate_ref: None,
            failure: None,
            event_cursor: EventCursor(1),
        };
        let claimed = ClaimedTurnRun {
            state,
            resolved_run_profile: resolved.clone(),
            runner_id: TurnRunnerId::new(),
            lease_token: TurnLeaseToken::new(),
        };
        let context = LoopRunContext::new(turn_scope, turn_id, run_id, resolved);
        thread_service
            .mark_message_submitted(
                &thread_scope_from_turn(&context.scope),
                &thread_id,
                accepted.message_id,
                turn_id.to_string(),
                run_id.to_string(),
            )
            .await
            .unwrap();

        Self {
            thread_service,
            checkpoint_state_store,
            loop_checkpoint_store,
            gateway,
            milestone_sink,
            thread_scope,
            thread_id,
            claimed,
            context,
        }
    }

    fn factory(
        &self,
    ) -> RebornLoopDriverHostFactory<InMemorySessionThreadService, RecordingGateway> {
        self.factory_with_loop_checkpoint_store(self.loop_checkpoint_store.clone())
    }

    fn factory_with_loop_checkpoint_store(
        &self,
        loop_checkpoint_store: Arc<dyn LoopCheckpointStore>,
    ) -> RebornLoopDriverHostFactory<InMemorySessionThreadService, RecordingGateway> {
        RebornLoopDriverHostFactory::new(
            Arc::clone(&self.thread_service),
            self.thread_scope.clone(),
            Arc::clone(&self.gateway),
            self.checkpoint_state_store.clone(),
            loop_checkpoint_store,
            self.milestone_sink.clone(),
            TextOnlyLoopHostConfig { max_messages: 8 },
        )
    }

    async fn build_host(&self) -> ironclaw_reborn::RebornLoopDriverHost {
        self.factory()
            .build_text_only_host(RebornLoopDriverHostRequest {
                claimed_run: self.claimed.clone(),
                loop_run_context: self.context.clone(),
            })
            .await
            .unwrap()
    }

    async fn stage_checkpoint_state(
        &self,
        kind: LoopCheckpointKind,
        payload: &[u8],
    ) -> ironclaw_turns::CheckpointStateRecord {
        self.checkpoint_state_store
            .put_checkpoint_state(PutCheckpointStateRequest::new(
                self.context.scope.clone(),
                self.context.turn_id,
                self.context.run_id,
                self.context.checkpoint_schema_id.clone(),
                self.context.checkpoint_schema_version,
                kind,
                payload.to_vec(),
            ))
            .await
            .unwrap()
    }

    fn milestones(&self) -> Vec<LoopHostMilestone> {
        self.milestone_sink.milestones()
    }

    fn milestone_names(&self) -> Vec<&'static str> {
        self.milestones()
            .iter()
            .map(|milestone| milestone.kind.kind_name())
            .collect()
    }
}

fn thread_scope_from_turn(scope: &TurnScope) -> ThreadScope {
    ThreadScope {
        tenant_id: scope.tenant_id.clone(),
        agent_id: scope.agent_id.clone().unwrap(),
        project_id: scope.project_id.clone(),
        owner_user_id: Some(UserId::new("user-text-host").unwrap()),
        mission_id: None,
    }
}

fn assert_public_milestones_hide_raw_payloads(milestones: &[LoopHostMilestone]) {
    // Milestones are public progress metadata: they may carry durable refs and
    // safe summaries, never raw model text, checkpoint bytes, tool input,
    // secrets, or host paths. Drivers must rehydrate content through scoped
    // stores instead of learning it from milestone JSON.
    let wire = serde_json::to_string(milestones).unwrap();
    for forbidden in [
        "RAW_CHECKPOINT_PAYLOAD",
        "sk-secret",
        "/host/path",
        "tool_input",
        "model says hi",
    ] {
        assert!(!wire.contains(forbidden), "milestone leaked {forbidden}");
    }
}

struct FailingLoopCheckpointStore;

#[async_trait]
impl LoopCheckpointStore for FailingLoopCheckpointStore {
    async fn put_loop_checkpoint(
        &self,
        _request: PutLoopCheckpointRequest,
    ) -> Result<LoopCheckpointRecord, TurnError> {
        Err(TurnError::Unavailable {
            reason: "loop checkpoint store offline".to_string(),
        })
    }

    async fn get_loop_checkpoint(
        &self,
        _request: GetLoopCheckpointRequest,
    ) -> Result<Option<LoopCheckpointRecord>, TurnError> {
        Err(TurnError::Unavailable {
            reason: "loop checkpoint store offline".to_string(),
        })
    }
}

struct RecordingGateway {
    requests: Mutex<Vec<HostManagedModelRequest>>,
    response: HostManagedModelResponse,
}

impl RecordingGateway {
    fn reply(content: impl Into<String>) -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
            response: HostManagedModelResponse::assistant_reply(content),
        }
    }

    fn requests(&self) -> Vec<HostManagedModelRequest> {
        self.requests.lock().unwrap().clone()
    }
}

#[async_trait]
impl HostManagedModelGateway for RecordingGateway {
    async fn stream_model(
        &self,
        request: HostManagedModelRequest,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        self.requests.lock().unwrap().push(request);
        Ok(self.response.clone())
    }
}
