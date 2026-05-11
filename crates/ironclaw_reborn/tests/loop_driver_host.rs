use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_host_api::{
    AgentId, ApprovalRequestId, CapabilityDescriptor, CapabilityId, CapabilitySet, EffectKind,
    ExecutionContext, ExtensionId, MountView, PermissionMode, ProcessId, ProjectId,
    ResourceEstimate, ResourceUsage, RuntimeKind, SecretHandle, TenantId, ThreadId, TrustClass,
    UserId,
};
use ironclaw_host_runtime::{
    CancelRuntimeWorkOutcome, CancelRuntimeWorkRequest, CapabilitySurfacePolicy, HostRuntime,
    HostRuntimeError, HostRuntimeHealth, HostRuntimeStatus, RuntimeApprovalGate, RuntimeAuthGate,
    RuntimeBlockedReason, RuntimeCapabilityCompleted, RuntimeCapabilityFailure,
    RuntimeCapabilityOutcome, RuntimeCapabilityRequest, RuntimeCapabilityResumeRequest,
    RuntimeFailureKind, RuntimeGateId, RuntimeProcessHandle, RuntimeResourceGate,
    RuntimeStatusRequest, SurfaceKind, VisibleCapability, VisibleCapabilityAccess,
};
use ironclaw_loop_support::{
    HostManagedModelError, HostManagedModelGateway, HostManagedModelRequest,
    HostManagedModelResponse,
};
use ironclaw_reborn::{
    HostRuntimeLoopCapabilityPort, LoopCapabilityInputResolver, LoopCapabilityResultWriter,
    RebornLoopDriverHostFactory, RebornLoopDriverHostRequest, TextOnlyLoopHostConfig,
    turn_runner::HostFactory,
};
use ironclaw_threads::{
    AcceptInboundMessageRequest, EnsureThreadRequest, InMemorySessionThreadService, MessageContent,
    MessageKind, MessageStatus, SessionThreadService, ThreadHistoryRequest, ThreadScope,
};
use ironclaw_trust::{AuthorityCeiling, EffectiveTrustClass, TrustDecision, TrustProvenance};
use ironclaw_turns::{
    AcceptedMessageRef, CheckpointStateStore, EventCursor, GetCheckpointStateRequest,
    GetLoopCheckpointRequest, InMemoryCheckpointStateStore, InMemoryLoopCheckpointStore,
    InMemoryRunProfileResolver, InMemoryTurnStateStore, InMemoryTurnStateStoreLimits,
    LoopCheckpointRecord, LoopCheckpointStore, LoopResultRef, PutCheckpointStateRequest,
    PutLoopCheckpointRequest, ReplyTargetBindingRef, RunProfileId, RunProfileResolutionRequest,
    RunProfileResolver, RunProfileVersion, SourceBindingRef, TurnError, TurnLeaseToken, TurnRunId,
    TurnRunnerId, TurnScope, TurnStatus,
    run_profile::{
        AgentLoopDriverHost, AgentLoopHostError, AgentLoopHostErrorKind,
        CapabilityDeniedReasonKind, CapabilityInputRef, CapabilityInvocation, CapabilityOutcome,
        CapabilitySurfaceVersion, FinalizeAssistantMessage, InMemoryLoopHostMilestoneSink,
        LoopCapabilityPort, LoopCheckpointKind, LoopCheckpointPort, LoopCheckpointRequest,
        LoopContextRequest, LoopDriverId, LoopDriverNoteKind, LoopHostMilestone, LoopInputCursor,
        LoopInputCursorToken, LoopInputPort, LoopModelRequest, LoopProgressEvent,
        LoopPromptBundleRequest, LoopPromptPort, LoopRunContext, ParentLoopOutput, PromptMode,
        VisibleCapabilityRequest,
    },
    runner::ClaimedTurnRun,
};
use serde_json::{Value, json};

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
async fn text_only_host_routes_capability_invocation_through_host_runtime() {
    let fixture = HostFixture::new("thread-host-runtime-capability", "hello").await;
    let capability_id = CapabilityId::new("demo.echo").unwrap();
    let runtime = Arc::new(RecordingHostRuntime::with_surface(host_runtime_surface([
        capability_descriptor(capability_id.as_str()),
    ])));
    runtime.push_outcome(RuntimeCapabilityOutcome::Completed(Box::new(
        RuntimeCapabilityCompleted {
            capability_id: capability_id.clone(),
            output: json!({"echoed": true}),
            usage: ResourceUsage::default(),
        },
    )));
    let io = Arc::new(InMemoryCapabilityIo::default());
    let input_ref = CapabilityInputRef::new("input:echo-request").unwrap();
    io.put_input(input_ref.clone(), json!({"message": "hello tool"}));

    let capability_port = HostRuntimeLoopCapabilityPort::new(
        runtime.clone(),
        fixture.context.clone(),
        host_runtime_visible_request(&fixture, ["demo"]),
        io.clone(),
        io.clone(),
    )
    .with_milestone_sink(fixture.milestone_sink.clone());
    let host = fixture
        .factory()
        .build_text_only_host_with_capabilities(
            RebornLoopDriverHostRequest {
                claimed_run: fixture.claimed.clone(),
                loop_run_context: fixture.context.clone(),
            },
            Arc::new(capability_port),
        )
        .await
        .unwrap();

    let surface = host
        .visible_capabilities(VisibleCapabilityRequest)
        .await
        .unwrap();
    assert_eq!(surface.descriptors.len(), 1);
    assert_eq!(surface.descriptors[0].capability_id, capability_id);

    let outcome = host
        .invoke_capability(CapabilityInvocation {
            surface_version: surface.version.clone(),
            capability_id: capability_id.clone(),
            input_ref,
        })
        .await
        .unwrap();

    let CapabilityOutcome::Completed(message) = outcome else {
        panic!("expected completed capability outcome");
    };
    assert!(message.result_ref.as_str().starts_with("result:"));
    assert_eq!(message.safe_summary, "capability completed");
    let invocations = runtime.invocations();
    assert_eq!(invocations.len(), 1);
    assert_eq!(invocations[0].capability_id, capability_id);
    assert_eq!(invocations[0].input, json!({"message": "hello tool"}));
    assert_eq!(io.results(), vec![(capability_id, json!({"echoed": true}))]);
    assert!(fixture.milestone_names().contains(&"capability_invoked"));
}

#[tokio::test]
async fn text_only_host_uses_fresh_execution_context_per_capability_invocation() {
    let fixture = HostFixture::new("thread-host-runtime-capability-context", "hello").await;
    let capability_id = CapabilityId::new("demo.echo").unwrap();
    let runtime = Arc::new(RecordingHostRuntime::with_surface(host_runtime_surface([
        capability_descriptor(capability_id.as_str()),
    ])));
    for output in [json!({"call": 1}), json!({"call": 2})] {
        runtime.push_outcome(RuntimeCapabilityOutcome::Completed(Box::new(
            RuntimeCapabilityCompleted {
                capability_id: capability_id.clone(),
                output,
                usage: ResourceUsage::default(),
            },
        )));
    }
    let io = Arc::new(InMemoryCapabilityIo::default());
    let first_input = CapabilityInputRef::new("input:first-call").unwrap();
    let second_input = CapabilityInputRef::new("input:second-call").unwrap();
    io.put_input(first_input.clone(), json!({"call": 1}));
    io.put_input(second_input.clone(), json!({"call": 2}));
    let capability_port = HostRuntimeLoopCapabilityPort::new(
        runtime.clone(),
        fixture.context.clone(),
        host_runtime_visible_request(&fixture, ["demo"]),
        io.clone(),
        io,
    );
    let host = fixture
        .factory()
        .build_text_only_host_with_capabilities(
            RebornLoopDriverHostRequest {
                claimed_run: fixture.claimed.clone(),
                loop_run_context: fixture.context.clone(),
            },
            Arc::new(capability_port),
        )
        .await
        .unwrap();
    let surface = host
        .visible_capabilities(VisibleCapabilityRequest)
        .await
        .unwrap();

    for input_ref in [first_input, second_input] {
        let outcome = host
            .invoke_capability(CapabilityInvocation {
                surface_version: surface.version.clone(),
                capability_id: capability_id.clone(),
                input_ref,
            })
            .await
            .unwrap();
        assert!(matches!(outcome, CapabilityOutcome::Completed(_)));
    }

    let invocations = runtime.invocations();
    assert_eq!(invocations.len(), 2);
    assert_ne!(
        invocations[0].context.invocation_id,
        invocations[1].context.invocation_id
    );
    assert_eq!(
        invocations[0].context.resource_scope.invocation_id,
        invocations[0].context.invocation_id
    );
    assert_eq!(
        invocations[1].context.resource_scope.invocation_id,
        invocations[1].context.invocation_id
    );
}

#[tokio::test]
async fn text_only_host_rejects_outside_surface_capability_before_host_runtime() {
    let fixture = HostFixture::new("thread-host-runtime-capability-deny", "hello").await;
    let visible_id = CapabilityId::new("demo.echo").unwrap();
    let hidden_id = CapabilityId::new("demo.hidden").unwrap();
    let runtime = Arc::new(RecordingHostRuntime::with_surface(host_runtime_surface([
        capability_descriptor(visible_id.as_str()),
    ])));
    let io = Arc::new(InMemoryCapabilityIo::default());
    let capability_port = HostRuntimeLoopCapabilityPort::new(
        runtime.clone(),
        fixture.context.clone(),
        host_runtime_visible_request(&fixture, ["demo"]),
        io.clone(),
        io,
    );
    let host = fixture
        .factory()
        .build_text_only_host_with_capabilities(
            RebornLoopDriverHostRequest {
                claimed_run: fixture.claimed.clone(),
                loop_run_context: fixture.context.clone(),
            },
            Arc::new(capability_port),
        )
        .await
        .unwrap();

    let surface = host
        .visible_capabilities(VisibleCapabilityRequest)
        .await
        .unwrap();
    let denied = host
        .invoke_capability(CapabilityInvocation {
            surface_version: surface.version,
            capability_id: hidden_id,
            input_ref: CapabilityInputRef::new("input:hidden-request").unwrap(),
        })
        .await
        .unwrap();

    assert!(matches!(
        denied,
        CapabilityOutcome::Denied(denied) if denied.reason_kind.as_str() == "outside_visible_surface"
    ));
    assert!(runtime.invocations().is_empty());

    let stale = host
        .invoke_capability(CapabilityInvocation {
            surface_version: CapabilitySurfaceVersion::new("sha256:stale").unwrap(),
            capability_id: visible_id,
            input_ref: CapabilityInputRef::new("input:stale-request").unwrap(),
        })
        .await
        .unwrap_err();
    assert_eq!(stale.kind, AgentLoopHostErrorKind::StaleSurface);
    assert!(runtime.invocations().is_empty());
}

#[tokio::test]
async fn text_only_host_sanitizes_runtime_failure_message_before_driver_output() {
    let fixture = HostFixture::new("thread-host-runtime-capability-failure", "hello").await;
    let capability_id = CapabilityId::new("demo.echo").unwrap();
    let runtime = Arc::new(RecordingHostRuntime::with_surface(host_runtime_surface([
        capability_descriptor(capability_id.as_str()),
    ])));
    runtime.push_outcome(RuntimeCapabilityOutcome::Failed(RuntimeCapabilityFailure {
        capability_id: capability_id.clone(),
        kind: RuntimeFailureKind::Dispatcher,
        message: Some("raw provider error sk-secret /host/path tool_input".to_string()),
    }));
    let io = Arc::new(InMemoryCapabilityIo::default());
    let input_ref = CapabilityInputRef::new("input:failure-request").unwrap();
    io.put_input(input_ref.clone(), json!({"message": "fail"}));
    let capability_port = HostRuntimeLoopCapabilityPort::new(
        runtime,
        fixture.context.clone(),
        host_runtime_visible_request(&fixture, ["demo"]),
        io.clone(),
        io,
    );
    let host = fixture
        .factory()
        .build_text_only_host_with_capabilities(
            RebornLoopDriverHostRequest {
                claimed_run: fixture.claimed.clone(),
                loop_run_context: fixture.context.clone(),
            },
            Arc::new(capability_port),
        )
        .await
        .unwrap();
    let surface = host
        .visible_capabilities(VisibleCapabilityRequest)
        .await
        .unwrap();

    let outcome = host
        .invoke_capability(CapabilityInvocation {
            surface_version: surface.version,
            capability_id,
            input_ref,
        })
        .await
        .unwrap();

    let CapabilityOutcome::Failed(failure) = outcome else {
        panic!("expected failed capability outcome");
    };
    assert_eq!(failure.safe_summary, "capability invocation failed");
}

#[tokio::test]
async fn text_only_host_maps_runtime_suspension_and_process_outcomes() {
    let fixture = HostFixture::new("thread-host-runtime-capability-suspensions", "hello").await;
    let approval_id = CapabilityId::new("demo.approval").unwrap();
    let auth_id = CapabilityId::new("demo.auth").unwrap();
    let resource_id = CapabilityId::new("demo.resource").unwrap();
    let process_id = CapabilityId::new("demo.process").unwrap();
    let runtime = Arc::new(RecordingHostRuntime::with_surface(host_runtime_surface([
        capability_descriptor(approval_id.as_str()),
        capability_descriptor(auth_id.as_str()),
        capability_descriptor(resource_id.as_str()),
        capability_descriptor(process_id.as_str()),
    ])));
    runtime.push_outcome(RuntimeCapabilityOutcome::ApprovalRequired(
        RuntimeApprovalGate {
            approval_request_id: ApprovalRequestId::new(),
            capability_id: approval_id.clone(),
            reason: RuntimeBlockedReason::ApprovalRequired,
        },
    ));
    runtime.push_outcome(RuntimeCapabilityOutcome::AuthRequired(RuntimeAuthGate {
        gate_id: RuntimeGateId::new(),
        capability_id: auth_id.clone(),
        reason: RuntimeBlockedReason::AuthRequired,
        required_secrets: vec![SecretHandle::new("api_key").unwrap()],
    }));
    runtime.push_outcome(RuntimeCapabilityOutcome::ResourceBlocked(
        RuntimeResourceGate {
            gate_id: RuntimeGateId::new(),
            capability_id: resource_id.clone(),
            reason: RuntimeBlockedReason::ResourceLimit,
            estimate: ResourceEstimate::default(),
        },
    ));
    runtime.push_outcome(RuntimeCapabilityOutcome::SpawnedProcess(
        RuntimeProcessHandle {
            process_id: ProcessId::new(),
            capability_id: process_id.clone(),
        },
    ));
    let io = Arc::new(InMemoryCapabilityIo::default());
    let cases = [
        (
            approval_id.clone(),
            CapabilityInputRef::new("input:approval-request").unwrap(),
        ),
        (
            auth_id.clone(),
            CapabilityInputRef::new("input:auth-request").unwrap(),
        ),
        (
            resource_id.clone(),
            CapabilityInputRef::new("input:resource-request").unwrap(),
        ),
        (
            process_id.clone(),
            CapabilityInputRef::new("input:process-request").unwrap(),
        ),
    ];
    for (_, input_ref) in &cases {
        io.put_input(input_ref.clone(), json!({"message": input_ref.as_str()}));
    }
    let capability_port = HostRuntimeLoopCapabilityPort::new(
        runtime.clone(),
        fixture.context.clone(),
        host_runtime_visible_request(&fixture, ["demo"]),
        io.clone(),
        io,
    );
    let host = fixture
        .factory()
        .build_text_only_host_with_capabilities(
            RebornLoopDriverHostRequest {
                claimed_run: fixture.claimed.clone(),
                loop_run_context: fixture.context.clone(),
            },
            Arc::new(capability_port),
        )
        .await
        .unwrap();
    let surface = host
        .visible_capabilities(VisibleCapabilityRequest)
        .await
        .unwrap();

    let mut outcomes = Vec::new();
    for (capability_id, input_ref) in cases {
        outcomes.push(
            host.invoke_capability(CapabilityInvocation {
                surface_version: surface.version.clone(),
                capability_id,
                input_ref,
            })
            .await
            .unwrap(),
        );
    }

    assert!(matches!(
        &outcomes[0],
        CapabilityOutcome::ApprovalRequired { gate_ref, safe_summary }
            if gate_ref.as_str().starts_with("gate:approval-")
                && safe_summary == "capability requires approval"
    ));
    assert!(matches!(
        &outcomes[1],
        CapabilityOutcome::AuthRequired { gate_ref, safe_summary }
            if gate_ref.as_str().starts_with("gate:auth-")
                && safe_summary == "capability requires authentication"
    ));
    assert!(matches!(
        &outcomes[2],
        CapabilityOutcome::ResourceBlocked { gate_ref, safe_summary }
            if gate_ref.as_str().starts_with("gate:resource-")
                && safe_summary == "capability is blocked by resource limits"
    ));
    assert!(matches!(
        &outcomes[3],
        CapabilityOutcome::SpawnedProcess(process)
            if process.process_ref.as_str().starts_with("process:")
                && process.safe_summary == "capability spawned background work"
    ));
    assert!(outcomes.iter().all(CapabilityOutcome::is_suspension));
    assert_eq!(runtime.invocations().len(), 4);
}

#[tokio::test]
async fn text_only_host_batch_stops_on_first_suspension_before_later_invocations() {
    let fixture = HostFixture::new("thread-host-runtime-capability-batch-stop", "hello").await;
    let approval_id = CapabilityId::new("demo.approval").unwrap();
    let echo_id = CapabilityId::new("demo.echo").unwrap();
    let runtime = Arc::new(RecordingHostRuntime::with_surface(host_runtime_surface([
        capability_descriptor(approval_id.as_str()),
        capability_descriptor(echo_id.as_str()),
    ])));
    runtime.push_outcome(RuntimeCapabilityOutcome::ApprovalRequired(
        RuntimeApprovalGate {
            approval_request_id: ApprovalRequestId::new(),
            capability_id: approval_id.clone(),
            reason: RuntimeBlockedReason::ApprovalRequired,
        },
    ));
    runtime.push_outcome(RuntimeCapabilityOutcome::Completed(Box::new(
        RuntimeCapabilityCompleted {
            capability_id: echo_id.clone(),
            output: json!({"should_not_run": true}),
            usage: ResourceUsage::default(),
        },
    )));
    let io = Arc::new(InMemoryCapabilityIo::default());
    let approval_input = CapabilityInputRef::new("input:batch-approval").unwrap();
    let echo_input = CapabilityInputRef::new("input:batch-echo").unwrap();
    io.put_input(approval_input.clone(), json!({"message": "approval"}));
    io.put_input(echo_input.clone(), json!({"message": "echo"}));
    let capability_port = HostRuntimeLoopCapabilityPort::new(
        runtime.clone(),
        fixture.context.clone(),
        host_runtime_visible_request(&fixture, ["demo"]),
        io.clone(),
        io,
    );
    let host = fixture
        .factory()
        .build_text_only_host_with_capabilities(
            RebornLoopDriverHostRequest {
                claimed_run: fixture.claimed.clone(),
                loop_run_context: fixture.context.clone(),
            },
            Arc::new(capability_port),
        )
        .await
        .unwrap();
    let surface = host
        .visible_capabilities(VisibleCapabilityRequest)
        .await
        .unwrap();

    let batch = host
        .invoke_capability_batch(ironclaw_turns::run_profile::CapabilityBatchInvocation {
            invocations: vec![
                CapabilityInvocation {
                    surface_version: surface.version.clone(),
                    capability_id: approval_id,
                    input_ref: approval_input,
                },
                CapabilityInvocation {
                    surface_version: surface.version,
                    capability_id: echo_id,
                    input_ref: echo_input,
                },
            ],
            stop_on_first_suspension: true,
        })
        .await
        .unwrap();

    assert!(batch.stopped_on_suspension);
    assert_eq!(batch.outcomes.len(), 1);
    assert!(matches!(
        batch.outcomes.as_slice(),
        [CapabilityOutcome::ApprovalRequired { .. }]
    ));
    assert_eq!(runtime.invocations().len(), 1);
}

#[tokio::test]
async fn text_only_host_does_not_reinvoke_runtime_after_failed_outcome_retry() {
    let fixture =
        HostFixture::new("thread-host-runtime-capability-failed-idempotency", "hello").await;
    let capability_id = CapabilityId::new("demo.echo").unwrap();
    let runtime = Arc::new(RecordingHostRuntime::with_surface(host_runtime_surface([
        capability_descriptor(capability_id.as_str()),
    ])));
    runtime.push_outcome(RuntimeCapabilityOutcome::Failed(RuntimeCapabilityFailure {
        capability_id: capability_id.clone(),
        kind: RuntimeFailureKind::Dispatcher,
        message: None,
    }));
    let io = Arc::new(InMemoryCapabilityIo::default());
    let input_ref = CapabilityInputRef::new("input:failed-idempotent-request").unwrap();
    io.put_input(input_ref.clone(), json!({"message": "fail once"}));
    let capability_port = HostRuntimeLoopCapabilityPort::new(
        runtime.clone(),
        fixture.context.clone(),
        host_runtime_visible_request(&fixture, ["demo"]),
        io.clone(),
        io.clone(),
    );
    let host = fixture
        .factory()
        .build_text_only_host_with_capabilities(
            RebornLoopDriverHostRequest {
                claimed_run: fixture.claimed.clone(),
                loop_run_context: fixture.context.clone(),
            },
            Arc::new(capability_port),
        )
        .await
        .unwrap();
    let surface = host
        .visible_capabilities(VisibleCapabilityRequest)
        .await
        .unwrap();
    let invocation = CapabilityInvocation {
        surface_version: surface.version,
        capability_id: capability_id.clone(),
        input_ref,
    };

    let first = host.invoke_capability(invocation.clone()).await.unwrap();
    let second = host.invoke_capability(invocation).await.unwrap();

    assert!(matches!(first, CapabilityOutcome::Failed(_)));
    assert_eq!(first, second);
    let invocations = runtime.invocations();
    assert_eq!(invocations.len(), 1);
    assert!(invocations[0].idempotency_key.is_some());
    assert!(io.results().is_empty());
}

#[tokio::test]
async fn text_only_host_prompt_accepts_refetched_surface_version() {
    let fixture = HostFixture::new("thread-host-runtime-capability-prompt-refetch", "hello").await;
    let first_id = CapabilityId::new("demo.echo").unwrap();
    let second_id = CapabilityId::new("demo.other").unwrap();
    let runtime = Arc::new(RecordingHostRuntime::with_surface(
        host_runtime_surface_with_version(
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            [capability_descriptor(first_id.as_str())],
        ),
    ));
    let io = Arc::new(InMemoryCapabilityIo::default());
    let capability_port = HostRuntimeLoopCapabilityPort::new(
        runtime.clone(),
        fixture.context.clone(),
        host_runtime_visible_request(&fixture, ["demo"]),
        io.clone(),
        io,
    );
    let host = fixture
        .factory()
        .build_text_only_host_with_capabilities(
            RebornLoopDriverHostRequest {
                claimed_run: fixture.claimed.clone(),
                loop_run_context: fixture.context.clone(),
            },
            Arc::new(capability_port),
        )
        .await
        .unwrap();

    runtime.set_surface(host_runtime_surface_with_version(
        "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        [capability_descriptor(second_id.as_str())],
    ));
    let refreshed_surface = host
        .visible_capabilities(VisibleCapabilityRequest)
        .await
        .unwrap();

    let prompt = host
        .build_prompt_bundle(LoopPromptBundleRequest {
            mode: PromptMode::TextOnly,
            context_cursor: None,
            surface_version: Some(refreshed_surface.version.clone()),
            checkpoint_state_ref: None,
            max_messages: Some(8),
        })
        .await
        .unwrap();

    assert_eq!(prompt.surface_version, Some(refreshed_surface.version));
}

#[tokio::test]
async fn text_only_host_rejects_concurrent_duplicate_invocation_before_runtime() {
    let fixture = HostFixture::new(
        "thread-host-runtime-capability-concurrent-idempotency",
        "hello",
    )
    .await;
    let capability_id = CapabilityId::new("demo.echo").unwrap();
    let runtime = Arc::new(RecordingHostRuntime::with_surface(host_runtime_surface([
        capability_descriptor(capability_id.as_str()),
    ])));
    for output in [json!({"call": 1}), json!({"call": 2})] {
        runtime.push_outcome(RuntimeCapabilityOutcome::Completed(Box::new(
            RuntimeCapabilityCompleted {
                capability_id: capability_id.clone(),
                output,
                usage: ResourceUsage::default(),
            },
        )));
    }
    let io = Arc::new(InMemoryCapabilityIo::default());
    let input_ref = CapabilityInputRef::new("input:concurrent-idempotent-request").unwrap();
    io.put_input(input_ref.clone(), json!({"message": "once"}));
    let capability_port = HostRuntimeLoopCapabilityPort::new(
        runtime.clone(),
        fixture.context.clone(),
        host_runtime_visible_request(&fixture, ["demo"]),
        io.clone(),
        io,
    );
    let host = fixture
        .factory()
        .build_text_only_host_with_capabilities(
            RebornLoopDriverHostRequest {
                claimed_run: fixture.claimed.clone(),
                loop_run_context: fixture.context.clone(),
            },
            Arc::new(capability_port),
        )
        .await
        .unwrap();
    let surface = host
        .visible_capabilities(VisibleCapabilityRequest)
        .await
        .unwrap();
    let invocation = CapabilityInvocation {
        surface_version: surface.version,
        capability_id: capability_id.clone(),
        input_ref,
    };

    let (first, second) = tokio::join!(
        host.invoke_capability(invocation.clone()),
        host.invoke_capability(invocation)
    );

    let successes = [first.as_ref().ok(), second.as_ref().ok()]
        .into_iter()
        .flatten()
        .filter(|outcome| matches!(outcome, CapabilityOutcome::Completed(_)))
        .count();
    assert_eq!(successes, 1);
    assert_eq!(runtime.invocations().len(), 1);
}

#[tokio::test]
async fn text_only_host_rejects_mismatched_capability_authority_context() {
    let fixture = HostFixture::new("thread-host-runtime-capability-scope-mismatch", "hello").await;
    let capability_id = CapabilityId::new("demo.echo").unwrap();
    let runtime = Arc::new(RecordingHostRuntime::with_surface(host_runtime_surface([
        capability_descriptor(capability_id.as_str()),
    ])));
    let io = Arc::new(InMemoryCapabilityIo::default());
    let mut visible_request = host_runtime_visible_request(&fixture, ["demo"]);
    visible_request.context.tenant_id = TenantId::new("tenant-other").unwrap();
    visible_request.context.resource_scope.tenant_id = visible_request.context.tenant_id.clone();
    let capability_port = HostRuntimeLoopCapabilityPort::new(
        runtime.clone(),
        fixture.context.clone(),
        visible_request,
        io.clone(),
        io,
    );

    let error = fixture
        .factory()
        .build_text_only_host_with_capabilities(
            RebornLoopDriverHostRequest {
                claimed_run: fixture.claimed.clone(),
                loop_run_context: fixture.context.clone(),
            },
            Arc::new(capability_port),
        )
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        ironclaw_reborn::RebornLoopDriverHostError::InvalidRequest { .. }
    ));
    assert!(runtime.invocations().is_empty());
}

#[tokio::test]
async fn text_only_host_does_not_reinvoke_runtime_after_result_write_failure_retry() {
    let fixture = HostFixture::new("thread-host-runtime-capability-idempotency", "hello").await;
    let capability_id = CapabilityId::new("demo.echo").unwrap();
    let runtime = Arc::new(RecordingHostRuntime::with_surface(host_runtime_surface([
        capability_descriptor(capability_id.as_str()),
    ])));
    runtime.push_outcome(RuntimeCapabilityOutcome::Completed(Box::new(
        RuntimeCapabilityCompleted {
            capability_id: capability_id.clone(),
            output: json!({"write": "fails"}),
            usage: ResourceUsage::default(),
        },
    )));
    runtime.push_outcome(RuntimeCapabilityOutcome::Completed(Box::new(
        RuntimeCapabilityCompleted {
            capability_id: capability_id.clone(),
            output: json!({"duplicate": true}),
            usage: ResourceUsage::default(),
        },
    )));
    let io = Arc::new(InMemoryCapabilityIo::default());
    io.fail_next_result_write();
    let input_ref = CapabilityInputRef::new("input:idempotent-request").unwrap();
    io.put_input(input_ref.clone(), json!({"message": "once"}));
    let capability_port = HostRuntimeLoopCapabilityPort::new(
        runtime.clone(),
        fixture.context.clone(),
        host_runtime_visible_request(&fixture, ["demo"]),
        io.clone(),
        io.clone(),
    );
    let host = fixture
        .factory()
        .build_text_only_host_with_capabilities(
            RebornLoopDriverHostRequest {
                claimed_run: fixture.claimed.clone(),
                loop_run_context: fixture.context.clone(),
            },
            Arc::new(capability_port),
        )
        .await
        .unwrap();
    let surface = host
        .visible_capabilities(VisibleCapabilityRequest)
        .await
        .unwrap();
    let invocation = CapabilityInvocation {
        surface_version: surface.version,
        capability_id: capability_id.clone(),
        input_ref,
    };

    let first = host
        .invoke_capability(invocation.clone())
        .await
        .unwrap_err();
    let second = host.invoke_capability(invocation).await.unwrap();

    assert_eq!(first.kind, AgentLoopHostErrorKind::Unavailable);
    assert!(matches!(second, CapabilityOutcome::Completed(_)));
    let invocations = runtime.invocations();
    assert_eq!(invocations.len(), 1);
    assert!(invocations[0].idempotency_key.is_some());
    assert_eq!(
        io.results(),
        vec![(capability_id, json!({"write": "fails"}))]
    );
}

#[tokio::test]
async fn text_only_host_rejects_runtime_outcome_for_different_capability() {
    let fixture = HostFixture::new("thread-host-runtime-capability-mismatch", "hello").await;
    let requested_id = CapabilityId::new("demo.echo").unwrap();
    let returned_id = CapabilityId::new("demo.other").unwrap();
    let runtime = Arc::new(RecordingHostRuntime::with_surface(host_runtime_surface([
        capability_descriptor(requested_id.as_str()),
    ])));
    runtime.push_outcome(RuntimeCapabilityOutcome::Completed(Box::new(
        RuntimeCapabilityCompleted {
            capability_id: returned_id,
            output: json!({"wrong": true}),
            usage: ResourceUsage::default(),
        },
    )));
    let io = Arc::new(InMemoryCapabilityIo::default());
    let input_ref = CapabilityInputRef::new("input:mismatch-request").unwrap();
    io.put_input(input_ref.clone(), json!({"message": "mismatch"}));
    let capability_port = HostRuntimeLoopCapabilityPort::new(
        runtime,
        fixture.context.clone(),
        host_runtime_visible_request(&fixture, ["demo"]),
        io.clone(),
        io.clone(),
    );
    let host = fixture
        .factory()
        .build_text_only_host_with_capabilities(
            RebornLoopDriverHostRequest {
                claimed_run: fixture.claimed.clone(),
                loop_run_context: fixture.context.clone(),
            },
            Arc::new(capability_port),
        )
        .await
        .unwrap();
    let surface = host
        .visible_capabilities(VisibleCapabilityRequest)
        .await
        .unwrap();

    let error = host
        .invoke_capability(CapabilityInvocation {
            surface_version: surface.version,
            capability_id: requested_id,
            input_ref,
        })
        .await
        .unwrap_err();

    assert_eq!(error.kind, AgentLoopHostErrorKind::Internal);
    assert!(io.results().is_empty());
}

#[tokio::test]
async fn text_only_host_rejects_previous_surface_after_refetch() {
    let fixture = HostFixture::new("thread-host-runtime-capability-refetch", "hello").await;
    let first_id = CapabilityId::new("demo.echo").unwrap();
    let second_id = CapabilityId::new("demo.other").unwrap();
    let runtime = Arc::new(RecordingHostRuntime::with_surface(
        host_runtime_surface_with_version(
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            [capability_descriptor(first_id.as_str())],
        ),
    ));
    runtime.push_outcome(RuntimeCapabilityOutcome::Completed(Box::new(
        RuntimeCapabilityCompleted {
            capability_id: first_id.clone(),
            output: json!({"stale": true}),
            usage: ResourceUsage::default(),
        },
    )));
    let io = Arc::new(InMemoryCapabilityIo::default());
    let input_ref = CapabilityInputRef::new("input:old-surface").unwrap();
    io.put_input(input_ref.clone(), json!({"message": "old"}));
    let capability_port = HostRuntimeLoopCapabilityPort::new(
        runtime.clone(),
        fixture.context.clone(),
        host_runtime_visible_request(&fixture, ["demo"]),
        io.clone(),
        io,
    );
    let host = fixture
        .factory()
        .build_text_only_host_with_capabilities(
            RebornLoopDriverHostRequest {
                claimed_run: fixture.claimed.clone(),
                loop_run_context: fixture.context.clone(),
            },
            Arc::new(capability_port),
        )
        .await
        .unwrap();

    let first_surface = host
        .visible_capabilities(VisibleCapabilityRequest)
        .await
        .unwrap();
    runtime.set_surface(host_runtime_surface_with_version(
        "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        [capability_descriptor(second_id.as_str())],
    ));
    let second_surface = host
        .visible_capabilities(VisibleCapabilityRequest)
        .await
        .unwrap();
    assert_ne!(first_surface.version, second_surface.version);

    let stale = host
        .invoke_capability(CapabilityInvocation {
            surface_version: first_surface.version,
            capability_id: first_id,
            input_ref,
        })
        .await
        .unwrap_err();

    assert_eq!(stale.kind, AgentLoopHostErrorKind::StaleSurface);
    assert!(runtime.invocations().is_empty());
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

#[derive(Default)]
struct InMemoryCapabilityIo {
    inputs: Mutex<BTreeMap<String, Value>>,
    results: Mutex<Vec<(CapabilityId, Value)>>,
    fail_result_writes_remaining: Mutex<usize>,
}

impl InMemoryCapabilityIo {
    fn put_input(&self, input_ref: CapabilityInputRef, input: Value) {
        self.inputs
            .lock()
            .unwrap()
            .insert(input_ref.as_str().to_string(), input);
    }

    fn results(&self) -> Vec<(CapabilityId, Value)> {
        self.results.lock().unwrap().clone()
    }

    fn fail_next_result_write(&self) {
        *self.fail_result_writes_remaining.lock().unwrap() += 1;
    }
}

#[async_trait]
impl LoopCapabilityInputResolver for InMemoryCapabilityIo {
    async fn resolve_capability_input(
        &self,
        _run_context: &LoopRunContext,
        input_ref: &CapabilityInputRef,
    ) -> Result<Value, AgentLoopHostError> {
        self.inputs
            .lock()
            .unwrap()
            .get(input_ref.as_str())
            .cloned()
            .ok_or_else(|| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::InvalidInvocation,
                    "capability input ref was not staged for this loop",
                )
            })
    }
}

#[async_trait]
impl LoopCapabilityResultWriter for InMemoryCapabilityIo {
    async fn write_capability_result(
        &self,
        run_context: &LoopRunContext,
        capability_id: &CapabilityId,
        output: Value,
    ) -> Result<LoopResultRef, AgentLoopHostError> {
        let mut remaining_failures = self.fail_result_writes_remaining.lock().unwrap();
        if *remaining_failures > 0 {
            *remaining_failures -= 1;
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::Unavailable,
                "capability result writer is unavailable",
            ));
        }
        drop(remaining_failures);
        self.results
            .lock()
            .unwrap()
            .push((capability_id.clone(), output));
        LoopResultRef::new(format!(
            "result:{}-{}",
            run_context.run_id,
            capability_id.as_str()
        ))
        .map_err(|_| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::Internal,
                "capability result ref could not be represented",
            )
        })
    }
}

struct RecordingHostRuntime {
    surface: Mutex<ironclaw_host_runtime::VisibleCapabilitySurface>,
    outcomes: Mutex<Vec<RuntimeCapabilityOutcome>>,
    invocations: Mutex<Vec<RuntimeCapabilityRequest>>,
}

impl RecordingHostRuntime {
    fn with_surface(surface: ironclaw_host_runtime::VisibleCapabilitySurface) -> Self {
        Self {
            surface: Mutex::new(surface),
            outcomes: Mutex::new(Vec::new()),
            invocations: Mutex::new(Vec::new()),
        }
    }

    fn push_outcome(&self, outcome: RuntimeCapabilityOutcome) {
        self.outcomes.lock().unwrap().push(outcome);
    }

    fn set_surface(&self, surface: ironclaw_host_runtime::VisibleCapabilitySurface) {
        *self.surface.lock().unwrap() = surface;
    }

    fn invocations(&self) -> Vec<RuntimeCapabilityRequest> {
        self.invocations.lock().unwrap().clone()
    }
}

#[async_trait]
impl HostRuntime for RecordingHostRuntime {
    async fn invoke_capability(
        &self,
        request: RuntimeCapabilityRequest,
    ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
        tokio::task::yield_now().await;
        self.invocations.lock().unwrap().push(request);
        Ok(self.outcomes.lock().unwrap().remove(0))
    }

    async fn resume_capability(
        &self,
        _request: RuntimeCapabilityResumeRequest,
    ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
        unreachable!("resume is not used by loop capability tests")
    }

    async fn visible_capabilities(
        &self,
        _request: ironclaw_host_runtime::VisibleCapabilityRequest,
    ) -> Result<ironclaw_host_runtime::VisibleCapabilitySurface, HostRuntimeError> {
        Ok(self.surface.lock().unwrap().clone())
    }

    async fn cancel_work(
        &self,
        _request: CancelRuntimeWorkRequest,
    ) -> Result<CancelRuntimeWorkOutcome, HostRuntimeError> {
        Ok(CancelRuntimeWorkOutcome::default())
    }

    async fn runtime_status(
        &self,
        _request: RuntimeStatusRequest,
    ) -> Result<HostRuntimeStatus, HostRuntimeError> {
        Ok(HostRuntimeStatus::default())
    }

    async fn health(&self) -> Result<HostRuntimeHealth, HostRuntimeError> {
        Ok(HostRuntimeHealth::default())
    }
}

fn host_runtime_surface(
    descriptors: impl IntoIterator<Item = CapabilityDescriptor>,
) -> ironclaw_host_runtime::VisibleCapabilitySurface {
    host_runtime_surface_with_version(
        "sha256:1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef",
        descriptors,
    )
}

fn host_runtime_surface_with_version(
    version: &str,
    descriptors: impl IntoIterator<Item = CapabilityDescriptor>,
) -> ironclaw_host_runtime::VisibleCapabilitySurface {
    ironclaw_host_runtime::VisibleCapabilitySurface {
        version: ironclaw_host_runtime::CapabilitySurfaceVersion::new(version).unwrap(),
        capabilities: descriptors
            .into_iter()
            .map(|descriptor| VisibleCapability {
                descriptor,
                access: VisibleCapabilityAccess::Available,
                estimated_resources: ResourceEstimate::default(),
            })
            .collect(),
    }
}

fn capability_descriptor(id: &str) -> CapabilityDescriptor {
    let provider = id.split('.').next().unwrap_or("demo");
    CapabilityDescriptor {
        id: CapabilityId::new(id).unwrap(),
        provider: ExtensionId::new(provider).unwrap(),
        runtime: RuntimeKind::Wasm,
        trust_ceiling: TrustClass::Sandbox,
        description: format!("Safe description for {id}"),
        parameters_schema: json!({"type": "object"}),
        effects: vec![EffectKind::DispatchCapability],
        default_permission: PermissionMode::Allow,
        resource_profile: None,
    }
}

fn host_runtime_visible_request(
    fixture: &HostFixture,
    trusted_providers: impl IntoIterator<Item = &'static str>,
) -> ironclaw_host_runtime::VisibleCapabilityRequest {
    let user_id = fixture
        .thread_scope
        .owner_user_id
        .clone()
        .unwrap_or_else(|| UserId::new("user-text-host").unwrap());
    let mut context = ExecutionContext::local_default(
        user_id,
        ExtensionId::new("loop-driver").unwrap(),
        RuntimeKind::FirstParty,
        TrustClass::System,
        CapabilitySet::default(),
        MountView::default(),
    )
    .unwrap();
    context.tenant_id = fixture.context.scope.tenant_id.clone();
    context.agent_id = fixture.context.scope.agent_id.clone();
    context.project_id = fixture.context.scope.project_id.clone();
    context.thread_id = Some(fixture.context.thread_id.clone());
    context.resource_scope.tenant_id = context.tenant_id.clone();
    context.resource_scope.agent_id = context.agent_id.clone();
    context.resource_scope.project_id = context.project_id.clone();
    context.resource_scope.thread_id = context.thread_id.clone();

    let provider_trust = trusted_providers
        .into_iter()
        .map(|provider| (ExtensionId::new(provider).unwrap(), trust_decision()))
        .collect::<BTreeMap<_, _>>();

    ironclaw_host_runtime::VisibleCapabilityRequest::new(
        context,
        SurfaceKind::new("agent_loop").unwrap(),
    )
    .with_policy(CapabilitySurfacePolicy::allow_all())
    .with_provider_trust(provider_trust)
}

fn trust_decision() -> TrustDecision {
    TrustDecision {
        effective_trust: EffectiveTrustClass::user_trusted(),
        authority_ceiling: AuthorityCeiling {
            allowed_effects: vec![EffectKind::DispatchCapability],
            max_resource_ceiling: None,
        },
        provenance: TrustProvenance::AdminConfig,
        evaluated_at: Utc::now(),
    }
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
