use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_authorization::GrantAuthorizer;
use ironclaw_extensions::{ExtensionManifest, ExtensionPackage, ExtensionRegistry, ManifestSource};
use ironclaw_filesystem::LocalFilesystem;
use ironclaw_host_api::{
    AgentId, ApprovalRequestId, CapabilityDescriptor, CapabilityGrant, CapabilityGrantId,
    CapabilityId, CapabilitySet, EffectKind, ExecutionContext, ExtensionId, GrantConstraints,
    HostPortCatalog, MountView, NetworkPolicy, PackageId, PermissionMode, Principal, ProcessId,
    ProjectId, ResourceEstimate, ResourceUsage, RuntimeKind, SecretHandle, TenantId, ThreadId,
    TrustClass, UserId, VirtualPath,
};
use ironclaw_host_runtime::{
    CancelRuntimeWorkOutcome, CancelRuntimeWorkRequest, CapabilitySurfacePolicy, HostRuntime,
    HostRuntimeError, HostRuntimeHealth, HostRuntimeServices, HostRuntimeStatus,
    RuntimeApprovalGate, RuntimeAuthGate, RuntimeBlockedReason, RuntimeCapabilityCompleted,
    RuntimeCapabilityFailure, RuntimeCapabilityOutcome, RuntimeCapabilityRequest,
    RuntimeCapabilityResumeRequest, RuntimeCapabilityUnknown, RuntimeFailureKind, RuntimeGateId,
    RuntimeProcessHandle, RuntimeResourceGate, RuntimeStatusRequest, SurfaceKind,
    VisibleCapability, VisibleCapabilityAccess,
};
use ironclaw_loop_support::{
    CapabilityAllowSet, CapabilityResolveError, CapabilitySurfaceProfileResolver,
    EmptyLoopCapabilityPort, HostIdentityContextBuildError, HostIdentityContextCandidate,
    HostIdentityContextSource, HostIdentityMessageContent, HostInputBatch, HostInputEnvelope,
    HostInputQueue, HostInputQueueError, HostManagedModelError, HostManagedModelErrorKind,
    HostManagedModelGateway, HostManagedModelRequest, HostManagedModelResponse,
    HostRuntimeLoopCapabilityPort, HostSkillContextBuildError, HostSkillContextCandidate,
    HostSkillContextSource, IdentityApplicability, IdentityFileName, LoopCapabilityInputResolver,
    LoopCapabilityResultWriter, ProductLiveCancellationProbe, RunCancellationFactory,
    RunCancellationHandle, identity_message_ref,
};
use ironclaw_processes::ProcessServices;
use ironclaw_reborn::driver_registry::{
    DriverKind, DriverRegistry, DriverRequirements, LoopDriverRegistryKey,
};
use ironclaw_reborn::loop_driver_host::{
    LoopCapabilityPortFactory, RebornLoopDriverHost, RebornLoopDriverHostFactory,
    RebornLoopDriverHostRequest, TextOnlyLoopHostConfig,
};
use ironclaw_reborn::loop_exit_applier::{
    BlockedEvidenceRequest, CompletionEvidenceRequest, FailureEvidenceRequest,
    FinalCheckpointEvidenceRequest, LoopExitApplier, LoopExitEvidencePort,
    ThreadCheckpointLoopExitEvidencePort,
};
use ironclaw_reborn::model_routes::{
    ModelRoute, ModelRoutePolicy, ModelRouteResolver, ModelSelectionMode, ModelSlot,
    StaticModelRouteResolver,
};
use ironclaw_reborn::planned_driver_factory::default_planned_run_profile_resolver;
use ironclaw_reborn::runtime::{
    DefaultPlannedRuntimeConfig, DefaultPlannedRuntimeParts, build_default_planned_runtime,
    build_product_live_planned_runtime,
};
use ironclaw_reborn::text_loop_driver::TextOnlyModelReplyDriver;
use ironclaw_reborn::turn_runner::{
    HostFactory, HostFactoryError, TurnRunnerWakeReceiver, TurnRunnerWorker, TurnRunnerWorkerConfig,
};
use ironclaw_resources::InMemoryResourceGovernor;
use ironclaw_scripts::{
    ScriptBackend, ScriptBackendOutput, ScriptBackendRequest, ScriptRuntime, ScriptRuntimeConfig,
};
use ironclaw_skills::SkillTrust;
use ironclaw_threads::{
    AcceptInboundMessageRequest, EnsureThreadRequest, InMemorySessionThreadService, MessageContent,
    MessageKind, MessageStatus, SessionThreadService, ThreadHistoryRequest, ThreadMessageId,
    ThreadScope,
};
use ironclaw_trust::{
    AdminConfig, AdminEntry, AuthorityCeiling, EffectiveTrustClass, HostTrustAssignment,
    HostTrustPolicy, TrustDecision, TrustProvenance,
};
use ironclaw_turns::{
    AcceptedMessageRef, AgentLoopDriver, AgentLoopDriverDescriptor, AgentLoopDriverError,
    AgentLoopDriverResumeRequest, AgentLoopDriverRunRequest, CancelRunRequest, CancelRunResponse,
    CheckpointStateStore, DefaultTurnCoordinator, EventCursor, GetCheckpointStateRequest,
    GetLoopCheckpointRequest, GetRunStateRequest, IdempotencyKey, InMemoryCheckpointStateStore,
    InMemoryLoopCheckpointStore, InMemoryRunProfileResolver, InMemoryTurnStateStore,
    InMemoryTurnStateStoreLimits, LoopBlocked, LoopBlockedKind, LoopCheckpointRecord,
    LoopCheckpointStore, LoopCompleted, LoopCompletionKind, LoopExit, LoopExitId, LoopGateRef,
    LoopMessageRef, LoopResultRef, PutCheckpointStateRequest, PutLoopCheckpointRequest,
    ReplyTargetBindingRef, ResumeTurnRequest, RunProfileId, RunProfileResolutionRequest,
    RunProfileResolver, RunProfileVersion, SourceBindingRef, SubmitTurnRequest, SubmitTurnResponse,
    TurnActor, TurnAdmissionPolicy, TurnCoordinator, TurnError, TurnId, TurnLeaseToken, TurnRunId,
    TurnRunState, TurnRunnerId, TurnScope, TurnStateStore, TurnStatus,
    run_profile::{
        AgentLoopDriverHost, AgentLoopHostError, AgentLoopHostErrorKind, AssistantReply,
        BatchPolicyKind, CapabilityDeniedReasonKind, CapabilityFailureKind, CapabilityInputRef,
        CapabilityInvocation, CapabilityOutcome, CapabilitySurfaceVersion,
        FinalizeAssistantMessage, InMemoryLoopHostMilestoneSink, InstructionSafetyContext,
        LoopCancelReasonKind, LoopCancellationPort, LoopCapabilityPort, LoopCheckpointKind,
        LoopCheckpointPort, LoopCheckpointRequest, LoopCheckpointStateRef, LoopContextRequest,
        LoopDriverId, LoopDriverNoteKind, LoopGateKind, LoopHostMilestone, LoopHostMilestoneKind,
        LoopInlineMessage, LoopInlineMessageRole, LoopInput, LoopInputAckToken, LoopInputCursor,
        LoopInputCursorToken, LoopInputPort, LoopModelBudgetAccountant, LoopModelGatewayError,
        LoopModelPort, LoopModelRequest, LoopModelRouteSnapshot, LoopProgressEvent,
        LoopPromptBundleRequest, LoopPromptPort, LoopRunContext, LoopSafeSummary, ModelCallOutcome,
        NoOpBudgetAccountant, NoOpPolicyGuard, ParentLoopOutput, PersonalContextPolicy, PromptMode,
        SkillVisibility, StageCheckpointPayloadRequest, VisibleCapabilityRequest,
    },
    runner::{ClaimRunRequest, ClaimedTurnRun, TurnRunTransitionPort},
};
use serde_json::{Value, json};

fn driver_requirements_for(
    descriptor: &AgentLoopDriverDescriptor,
    requirements: DriverRequirements,
) -> HashMap<LoopDriverRegistryKey, DriverRequirements> {
    HashMap::from([(
        LoopDriverRegistryKey::from_descriptor(descriptor).unwrap(),
        requirements,
    )])
}

fn turn_state_store_dyn(store: &Arc<InMemoryTurnStateStore>) -> Arc<dyn TurnStateStore> {
    Arc::clone(store) as Arc<dyn TurnStateStore>
}

fn test_safety_context() -> InstructionSafetyContext {
    InstructionSafetyContext::new("policy:test", "test safety context")
        .expect("test safety context")
}

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
            mode: ironclaw_turns::run_profile::PromptMode::TextOnly,
        })
        .await
        .unwrap();
    assert_eq!(context.messages.len(), 1);

    let input = host_dyn
        .poll_inputs(LoopInputCursor::origin_for_run(&fixture.context), 8)
        .await
        .unwrap();
    assert!(input.inputs.is_empty());
    host_dyn
        .ack_inputs(input.input_acks.into_iter().map(|ack| ack.token).collect())
        .await
        .unwrap();

    let surface = host_dyn
        .visible_capabilities(VisibleCapabilityRequest)
        .await
        .unwrap();
    assert!(surface.descriptors.is_empty());

    let prompt_bundle = host_dyn
        .build_prompt_bundle(LoopPromptBundleRequest {
            mode: PromptMode::TextOnly,
            context_cursor: None,
            surface_version: Some(surface.version.clone()),
            checkpoint_state_ref: None,
            max_messages: Some(8),
            inline_messages: Vec::new(),
            capability_view: None,
        })
        .await
        .unwrap();
    assert_eq!(prompt_bundle.messages.len(), 1);
    assert!(prompt_bundle.instruction_fingerprint.is_some());

    let model_response = host_dyn
        .stream_model(LoopModelRequest {
            messages: prompt_bundle.messages,
            surface_version: Some(surface.version.clone()),
            model_preference: None,
            capability_view: None,
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
async fn text_only_host_factory_sanitizes_gateway_error_summaries() {
    let fixture = HostFixture::new(
        "thread-host-model-error-redaction",
        "RAW_PROMPT_TEXT_SENTINEL user text",
    )
    .await;
    fixture
        .gateway
        .set_response(Err(HostManagedModelError::safe(
            HostManagedModelErrorKind::PolicyDenied,
            "RAW_PROVIDER_SECRET invalid api key sk-provider-secret /host/path tool_input",
        )));
    let host = fixture.build_host().await;
    let prompt_bundle = host
        .build_prompt_bundle(LoopPromptBundleRequest {
            mode: PromptMode::TextOnly,
            context_cursor: None,
            surface_version: None,
            checkpoint_state_ref: None,
            max_messages: Some(8),
            inline_messages: Vec::new(),
            capability_view: None,
        })
        .await
        .unwrap();

    let error = host
        .stream_model(LoopModelRequest {
            messages: prompt_bundle.messages,
            surface_version: None,
            model_preference: None,
            capability_view: None,
        })
        .await
        .unwrap_err();

    assert_eq!(error.kind, AgentLoopHostErrorKind::PolicyDenied);
    assert_eq!(error.safe_summary, "model profile is not permitted");
    let wire = format!(
        "{}{:?}{}",
        serde_json::to_string(&error).unwrap(),
        error,
        serde_json::to_string(&fixture.milestones()).unwrap()
    );
    for forbidden in [
        "RAW_PROVIDER_SECRET",
        "RAW_PROMPT_TEXT_SENTINEL",
        "invalid api key",
        "sk-provider-secret",
        "/host/path",
        "tool_input",
    ] {
        assert!(!wire.contains(forbidden), "model error leaked {forbidden}");
    }
}

#[tokio::test]
async fn text_only_host_factory_invokes_model_budget_accountant() {
    let fixture = HostFixture::new("thread-host-model-accounting", "hello accounting").await;
    let accountant = Arc::new(RecordingBudgetAccountant::default());
    let factory = fixture
        .factory()
        .with_model_budget_accountant(accountant.clone());
    let host = factory
        .build_text_only_host(RebornLoopDriverHostRequest {
            claimed_run: fixture.claimed.clone(),
            loop_run_context: fixture.context.clone(),
        })
        .await
        .unwrap();
    let prompt_bundle = host
        .build_prompt_bundle(LoopPromptBundleRequest {
            mode: PromptMode::TextOnly,
            context_cursor: None,
            surface_version: None,
            checkpoint_state_ref: None,
            max_messages: Some(8),
            inline_messages: Vec::new(),
            capability_view: None,
        })
        .await
        .unwrap();

    host.stream_model(LoopModelRequest {
        messages: prompt_bundle.messages,
        surface_version: None,
        model_preference: None,
        capability_view: None,
    })
    .await
    .unwrap();

    assert!(accountant.was_pre_called());
    assert!(accountant.was_post_called());
    assert!(!accountant.post_saw_failure());
    assert_eq!(fixture.gateway.requests().len(), 1);
}

#[tokio::test]
async fn progress_port_routes_loop_progress_milestones() {
    let fixture = HostFixture::new("thread-progress-route", "hello progress").await;
    let host = fixture.build_host().await;
    let host_dyn: &(dyn AgentLoopDriverHost + Send + Sync) = &host;

    host_dyn
        .emit_loop_progress(LoopProgressEvent::IterationStarted { iteration: 2 })
        .await
        .unwrap();
    host_dyn
        .emit_loop_progress(LoopProgressEvent::CapabilityBatchStarted {
            iteration: 2,
            call_count: 3,
            policy: BatchPolicyKind::Parallel,
        })
        .await
        .unwrap();
    host_dyn
        .emit_loop_progress(LoopProgressEvent::CapabilityBatchCompleted {
            iteration: 2,
            result_count: 1,
            denied_count: 1,
            gated_count: 1,
            failed_count: 0,
        })
        .await
        .unwrap();
    host_dyn
        .emit_loop_progress(LoopProgressEvent::GateBlocked {
            iteration: 2,
            gate_kind: LoopGateKind::Approval,
        })
        .await
        .unwrap();

    let milestones = fixture.milestones();
    assert!(matches!(
        milestones[0].kind,
        LoopHostMilestoneKind::IterationStarted { iteration: 2 }
    ));
    assert!(matches!(
        milestones[1].kind,
        LoopHostMilestoneKind::CapabilityBatchStarted {
            iteration: 2,
            call_count: 3,
            policy: BatchPolicyKind::Parallel,
        }
    ));
    assert!(matches!(
        milestones[2].kind,
        LoopHostMilestoneKind::CapabilityBatchCompleted {
            iteration: 2,
            result_count: 1,
            denied_count: 1,
            gated_count: 1,
            failed_count: 0,
        }
    ));
    assert!(matches!(
        milestones[3].kind,
        LoopHostMilestoneKind::GateBlocked {
            iteration: 2,
            gate_kind: LoopGateKind::Approval,
        }
    ));
    assert!(milestones.iter().all(|milestone| {
        milestone.scope == fixture.context.scope
            && milestone.turn_id == fixture.context.turn_id
            && milestone.run_id == fixture.context.run_id
    }));
}

#[tokio::test]
async fn progress_port_checkpoint_written_does_not_double_emit() {
    let fixture = HostFixture::new("thread-progress-checkpoint", "hello progress").await;
    let host = fixture.build_host().await;
    let host_dyn: &(dyn AgentLoopDriverHost + Send + Sync) = &host;

    host_dyn
        .emit_loop_progress(LoopProgressEvent::CheckpointWritten {
            iteration: 0,
            kind: LoopCheckpointKind::BeforeModel,
        })
        .await
        .unwrap();

    assert!(fixture.milestones().is_empty());
}

#[tokio::test]
async fn progress_port_prompt_bundle_built_does_not_double_emit() {
    let fixture = HostFixture::new("thread-progress-prompt", "hello progress").await;
    let host = fixture.build_host().await;
    let host_dyn: &(dyn AgentLoopDriverHost + Send + Sync) = &host;

    let prompt_bundle = host_dyn
        .build_prompt_bundle(LoopPromptBundleRequest {
            mode: PromptMode::TextOnly,
            context_cursor: None,
            surface_version: None,
            checkpoint_state_ref: None,
            max_messages: Some(8),
            inline_messages: Vec::new(),
            capability_view: None,
        })
        .await
        .unwrap();
    host_dyn
        .emit_loop_progress(LoopProgressEvent::PromptBundleBuilt {
            iteration: 0,
            bundle_ref: prompt_bundle.bundle_ref,
            mode: PromptMode::TextOnly,
            surface_version: prompt_bundle.surface_version,
            message_count: prompt_bundle.messages.len() as u32,
            identity_message_count: prompt_bundle.identity_message_count,
            instruction_snippet_count: prompt_bundle.instruction_snippet_count,
        })
        .await
        .unwrap();

    assert_eq!(fixture.milestone_names(), vec!["prompt_bundle_built"]);
}

#[tokio::test]
async fn progress_event_serde_roundtrip_all_variants() {
    let fixture = HostFixture::new("thread-progress-serde", "hello progress").await;
    let context = fixture.context.clone();
    let bundle_ref = ironclaw_turns::run_profile::LoopPromptBundleRef::for_run(&context, "bundle")
        .expect("bundle ref");
    let surface_version = CapabilitySurfaceVersion::new("surface:v1").expect("surface version");

    let events = vec![
        LoopProgressEvent::driver_note(LoopDriverNoteKind::Planning, "safe note").unwrap(),
        LoopProgressEvent::IterationStarted { iteration: 1 },
        LoopProgressEvent::PromptBundleBuilt {
            iteration: 1,
            bundle_ref,
            mode: PromptMode::TextOnly,
            surface_version: Some(surface_version),
            message_count: 4,
            identity_message_count: 1,
            instruction_snippet_count: 2,
        },
        LoopProgressEvent::CapabilityBatchStarted {
            iteration: 1,
            call_count: 2,
            policy: BatchPolicyKind::Sequential,
        },
        LoopProgressEvent::CapabilityBatchCompleted {
            iteration: 1,
            result_count: 1,
            denied_count: 0,
            gated_count: 1,
            failed_count: 0,
        },
        LoopProgressEvent::GateBlocked {
            iteration: 1,
            gate_kind: LoopGateKind::ResourceWait,
        },
        LoopProgressEvent::CheckpointWritten {
            iteration: 1,
            kind: LoopCheckpointKind::Final,
        },
    ];

    for event in events {
        let value = serde_json::to_value(&event).expect("serialize");
        let restored: LoopProgressEvent = serde_json::from_value(value).expect("deserialize");
        assert_eq!(restored, event);
    }
}

#[tokio::test]
async fn text_only_model_reply_driver_runs_prompt_model_transcript_path() {
    let mut fixture = HostFixture::new(
        "thread-driver-happy",
        "RAW_PROMPT_TEXT_SENTINEL sk-prompt-secret /host/path tool_input",
    )
    .await;
    let driver = TextOnlyModelReplyDriver::default();
    assign_driver_to_fixture(&mut fixture, driver.descriptor());
    let host = fixture.build_host().await;

    let exit = driver
        .run(driver_request(&fixture.context), &host)
        .await
        .unwrap();

    let LoopExit::Completed(completed) = exit else {
        panic!("expected completed final reply exit");
    };
    assert_eq!(completed.completion_kind, LoopCompletionKind::FinalReply);
    assert_eq!(completed.result_refs, Vec::new());
    assert_eq!(completed.reply_message_refs.len(), 1);
    let reply_ref = completed.reply_message_refs[0].clone();

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
        .expect("driver must persist assistant reply through transcript port");
    assert_eq!(assistant.status, MessageStatus::Finalized);
    assert_eq!(assistant.content.as_deref(), Some("model says hi"));
    assert_eq!(reply_ref.as_str(), format!("msg:{}", assistant.message_id));

    let requests = fixture.gateway.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].messages.len(), 1);
    assert_eq!(
        requests[0].messages[0].content,
        "RAW_PROMPT_TEXT_SENTINEL sk-prompt-secret /host/path tool_input"
    );
    assert_eq!(
        fixture.milestone_names(),
        vec![
            "prompt_bundle_built",
            "model_started",
            "model_completed",
            "assistant_reply_finalized",
        ]
    );
    assert_public_milestones_hide_raw_payloads(&fixture.milestones());
    assert_driver_public_outputs_hide_raw_payloads(&completed);
}

#[tokio::test]
async fn text_only_model_reply_driver_redacts_credential_marker_reply_text() {
    let mut fixture = HostFixture::new("thread-driver-marker-reply", "hello config").await;
    fixture.gateway.set_response(Ok(HostManagedModelResponse {
        safe_text_deltas: vec!["Use OPENAI_API_KEY in the environment".to_string()],
        output: ParentLoopOutput::AssistantReply(AssistantReply {
            content: "Use OPENAI_API_KEY in the environment".to_string(),
        }),
    }));
    let driver = TextOnlyModelReplyDriver::default();
    assign_driver_to_fixture(&mut fixture, driver.descriptor());
    let host = fixture.build_host().await;

    driver
        .run(driver_request(&fixture.context), &host)
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
        .expect("driver must persist assistant reply through transcript port");
    assert_eq!(assistant.status, MessageStatus::Finalized);
    assert_eq!(
        assistant.content.as_deref(),
        Some("Use [redacted] in the environment")
    );
}

#[tokio::test]
async fn text_only_model_reply_driver_sanitizes_model_failures_and_skips_transcript_write() {
    let mut fixture = HostFixture::new(
        "thread-driver-model-error",
        "RAW_PROMPT_TEXT_SENTINEL sk-prompt-secret /host/path tool_input",
    )
    .await;
    fixture.gateway.fail_with_model_error(
        HostManagedModelErrorKind::PolicyDenied,
        "RAW_PROVIDER_ERROR invalid api key sk-provider-secret /host/path tool_input",
    );
    let driver = TextOnlyModelReplyDriver::default();
    assign_driver_to_fixture(&mut fixture, driver.descriptor());
    let host = fixture.build_host().await;

    let error = driver
        .run(driver_request(&fixture.context), &host)
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        AgentLoopDriverError::Failed { ref reason_kind } if reason_kind == "model_error"
    ));
    assert_driver_error_hides_raw_payloads(&error);
    assert_no_assistant_message(&fixture).await;
    assert_eq!(
        fixture.milestone_names(),
        vec!["prompt_bundle_built", "model_started", "model_failed"]
    );
    assert_public_milestones_hide_raw_payloads(&fixture.milestones());
}

#[tokio::test]
async fn text_only_model_reply_driver_rejects_capability_calls_without_dispatching_tools() {
    let mut fixture = HostFixture::new("thread-driver-capability-call", "hello needs tool").await;
    fixture.gateway.respond_with_capability_calls();
    let driver = TextOnlyModelReplyDriver::default();
    assign_driver_to_fixture(&mut fixture, driver.descriptor());
    let host = fixture.build_host().await;

    let error = driver
        .run(driver_request(&fixture.context), &host)
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        AgentLoopDriverError::Failed { ref reason_kind } if reason_kind == "invalid_model_output"
    ));
    assert_driver_error_hides_raw_payloads(&error);
    assert_no_assistant_message(&fixture).await;
    assert_eq!(
        fixture.milestone_names(),
        vec!["prompt_bundle_built", "model_started", "model_completed"]
    );
}

#[tokio::test]
async fn text_only_model_reply_driver_rejects_profiles_not_assigned_to_driver() {
    let fixture = HostFixture::new("thread-driver-profile-mismatch", "hello mismatch").await;
    let host = fixture.build_host().await;
    let driver = TextOnlyModelReplyDriver::default();

    let error = driver
        .run(driver_request(&fixture.context), &host)
        .await
        .unwrap_err();

    assert!(matches!(error, AgentLoopDriverError::InvalidRequest { .. }));
    assert!(fixture.gateway.requests().is_empty());
    assert!(fixture.milestones().is_empty());
    assert_driver_error_hides_raw_payloads(&error);
}

#[tokio::test]
async fn text_only_host_factory_includes_safety_context_in_prompt_bundle() {
    let fixture = HostFixture::new("thread-host-safety-context", "hello safety").await;
    let host = fixture
        .factory()
        .with_safety_context(
            InstructionSafetyContext::new("safety:prompt-write", "prompt write safety enforced")
                .unwrap(),
        )
        .build_text_only_host(RebornLoopDriverHostRequest {
            claimed_run: fixture.claimed.clone(),
            loop_run_context: fixture.context.clone(),
        })
        .await
        .unwrap();
    let host_dyn: &(dyn AgentLoopDriverHost + Send + Sync) = &host;

    let prompt_bundle = host_dyn
        .build_prompt_bundle(LoopPromptBundleRequest {
            mode: PromptMode::TextOnly,
            context_cursor: None,
            surface_version: None,
            checkpoint_state_ref: None,
            max_messages: Some(8),
            inline_messages: Vec::new(),
            capability_view: None,
        })
        .await
        .unwrap();
    host_dyn
        .stream_model(LoopModelRequest {
            messages: prompt_bundle.messages,
            surface_version: None,
            model_preference: None,
            capability_view: None,
        })
        .await
        .unwrap();

    let requests = fixture.gateway.requests();
    assert_eq!(requests.len(), 1);
    assert!(
        requests[0]
            .messages
            .iter()
            .any(|message| message.content == "prompt write safety enforced")
    );
}

#[tokio::test]
async fn turn_runner_worker_completes_queued_run_after_turn_store_reopen() {
    let fixture = HostFixture::new_unsubmitted(
        "thread-runner-restart-e2e",
        "hello after turn store restart",
    )
    .await;
    let original_turn_store = InMemoryTurnStateStore::default();
    let resolver = InMemoryRunProfileResolver::default();
    let resolved = resolver
        .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
        .await
        .unwrap();
    let descriptor = resolved.loop_driver.clone();
    let run_id = queue_fixture_turn(
        &fixture,
        &original_turn_store,
        &resolver,
        "idem-runner-restart-e2e",
    )
    .await;
    let snapshot = original_turn_store.persistence_snapshot();
    drop(original_turn_store);

    let reopened_turn_store = Arc::new(
        InMemoryTurnStateStore::from_persistence_snapshot(
            snapshot,
            InMemoryTurnStateStoreLimits::default(),
        )
        .unwrap(),
    );
    let mut registry = DriverRegistry::new();
    registry
        .register_driver(
            Arc::new(TextOnlyFinalReplyDriver { descriptor }),
            DriverRequirements::all_required(),
            DriverKind::Reference,
        )
        .unwrap();

    let (_wake_sender, wake_receiver) = TurnRunnerWakeReceiver::new();
    let worker = TurnRunnerWorker::new(
        TurnRunnerWorkerConfig {
            heartbeat_interval: std::time::Duration::from_millis(20),
            poll_interval: std::time::Duration::from_millis(10),
            scope_filter: Some(fixture.context.scope.clone()),
        },
        reopened_turn_store.clone(),
        loop_exit_applier_for_fixture(&fixture, reopened_turn_store.clone()),
        Arc::new(registry),
        Arc::new(fixture.factory_with_loop_checkpoint_store(reopened_turn_store.clone())),
        wake_receiver,
    );

    let cancel = tokio_util::sync::CancellationToken::new();
    let cancel_clone = cancel.clone();
    let handle = tokio::spawn(async move { worker.run(cancel_clone).await });

    let completed_state = wait_for_run_status(
        reopened_turn_store.as_ref(),
        &fixture.context.scope,
        run_id,
        TurnStatus::Completed,
        "reopened turn store worker should complete queued run",
    )
    .await;
    cancel.cancel();
    handle.await.unwrap();

    assert_eq!(completed_state.run_id, run_id);
    assert!(completed_state.failure.is_none());
    let requests = fixture.gateway.requests();
    assert_eq!(
        requests.len(),
        1,
        "restart path must not duplicate model calls"
    );
    assert_eq!(requests[0].run_id, run_id);
    assert!(
        requests[0]
            .messages
            .iter()
            .any(|message| message.content == "hello after turn store restart"),
        "restart prompt should include original user content: {:?}",
        requests[0].messages
    );
    let expected_run_id = run_id.to_string();
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
            && message.turn_run_id.as_deref() == Some(expected_run_id.as_str())
    }));
}

#[cfg(feature = "libsql-restart-tests")]
#[tokio::test]
async fn turn_runner_worker_completes_after_libsql_turn_and_thread_services_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let thread_db_path = dir.path().join("threads.db");
    let turn_db_path = dir.path().join("turns.db");
    let tenant_id = TenantId::new("tenant-libsql-restart").unwrap();
    let agent_id = AgentId::new("agent-libsql-restart").unwrap();
    let project_id = ProjectId::new("project-libsql-restart").unwrap();
    let user_id = UserId::new("user-libsql-restart").unwrap();
    let thread_id = ThreadId::new("thread-libsql-restart").unwrap();
    let thread_scope = ThreadScope {
        tenant_id: tenant_id.clone(),
        agent_id: agent_id.clone(),
        project_id: Some(project_id.clone()),
        owner_user_id: None,
        mission_id: None,
    };
    let turn_scope = TurnScope::new(
        tenant_id,
        Some(agent_id),
        Some(project_id),
        thread_id.clone(),
    );
    let resolver = InMemoryRunProfileResolver::default();
    let resolved = resolver
        .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
        .await
        .unwrap();
    let descriptor = resolved.loop_driver.clone();

    let run_id = {
        let thread_db = Arc::new(
            libsql::Builder::new_local(&thread_db_path)
                .build()
                .await
                .unwrap(),
        );
        let turn_db = Arc::new(
            libsql::Builder::new_local(&turn_db_path)
                .build()
                .await
                .unwrap(),
        );
        let thread_service = build_libsql_thread_service(thread_db).await;
        let turn_store = libsql_filesystem_turn_store(turn_db).await;
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
                external_event_id: Some("event-libsql-restart".to_string()),
                content: MessageContent::text("hello after libsql restart"),
            })
            .await
            .unwrap();
        let submit = turn_store
            .submit_turn(
                SubmitTurnRequest {
                    scope: turn_scope.clone(),
                    actor: TurnActor::new(user_id),
                    accepted_message_ref: AcceptedMessageRef::new("accepted-libsql-restart")
                        .unwrap(),
                    source_binding_ref: SourceBindingRef::new("source-web").unwrap(),
                    reply_target_binding_ref: ReplyTargetBindingRef::new("reply-web").unwrap(),
                    requested_run_profile: None,
                    idempotency_key: IdempotencyKey::new("idem-libsql-restart").unwrap(),
                    received_at: Utc::now(),
                },
                &ironclaw_turns::AllowAllTurnAdmissionPolicy,
                &resolver,
            )
            .await
            .unwrap();
        let SubmitTurnResponse::Accepted {
            turn_id,
            run_id,
            status,
            ..
        } = submit;
        assert_eq!(status, TurnStatus::Queued);
        thread_service
            .mark_message_submitted(
                &thread_scope,
                &thread_id,
                accepted.message_id,
                turn_id.to_string(),
                run_id.to_string(),
            )
            .await
            .unwrap();
        run_id
    };

    let thread_db = Arc::new(
        libsql::Builder::new_local(&thread_db_path)
            .build()
            .await
            .unwrap(),
    );
    let turn_db = Arc::new(
        libsql::Builder::new_local(&turn_db_path)
            .build()
            .await
            .unwrap(),
    );
    let thread_service = Arc::new(build_libsql_thread_service(thread_db).await);
    let turn_store = Arc::new(libsql_filesystem_turn_store(turn_db).await);
    let loop_checkpoint_store: Arc<dyn LoopCheckpointStore> = turn_store.clone();
    let transition_port: Arc<dyn ironclaw_turns::runner::TurnRunTransitionPort> =
        turn_store.clone();
    let evidence: Arc<dyn LoopExitEvidencePort> =
        Arc::new(ThreadCheckpointLoopExitEvidencePort::new(
            thread_service.clone(),
            turn_store.clone(),
            loop_checkpoint_store.clone(),
        ));
    let applier = Arc::new(LoopExitApplier::new(transition_port, evidence));
    let gateway = Arc::new(RecordingGateway::reply("model says hi"));
    let milestone_sink = Arc::new(InMemoryLoopHostMilestoneSink::default());
    let factory = RebornLoopDriverHostFactory::new(
        thread_service.clone(),
        thread_scope.clone(),
        gateway.clone(),
        Arc::new(InMemoryCheckpointStateStore::default()),
        turn_store.clone(),
        loop_checkpoint_store,
        milestone_sink,
        TextOnlyLoopHostConfig {
            max_messages: 8,
            require_model_route_snapshot: false,
        },
    );
    let mut registry = DriverRegistry::new();
    registry
        .register_driver(
            Arc::new(TextOnlyFinalReplyDriver { descriptor }),
            DriverRequirements::all_required(),
            DriverKind::Reference,
        )
        .unwrap();
    let (_wake_sender, wake_receiver) = TurnRunnerWakeReceiver::new();
    let worker = TurnRunnerWorker::new(
        TurnRunnerWorkerConfig {
            heartbeat_interval: std::time::Duration::from_millis(20),
            poll_interval: std::time::Duration::from_millis(10),
            scope_filter: Some(turn_scope.clone()),
        },
        turn_store.clone(),
        applier,
        Arc::new(registry),
        Arc::new(factory),
        wake_receiver,
    );

    let cancel = tokio_util::sync::CancellationToken::new();
    let cancel_clone = cancel.clone();
    let handle = tokio::spawn(async move { worker.run(cancel_clone).await });
    let completed_state = wait_for_run_status(
        turn_store.as_ref(),
        &turn_scope,
        run_id,
        TurnStatus::Completed,
        "reopened libSQL turn/thread services should complete queued run",
    )
    .await;
    cancel.cancel();
    handle.await.unwrap();

    assert_eq!(completed_state.run_id, run_id);
    assert!(completed_state.failure.is_none());
    let requests = gateway.requests();
    assert_eq!(
        requests.len(),
        1,
        "restarted worker must not duplicate model calls"
    );
    assert_eq!(requests[0].run_id, run_id);
    assert!(
        requests[0]
            .messages
            .iter()
            .any(|message| message.content == "hello after libsql restart"),
        "restart prompt should include original user content: {:?}",
        requests[0].messages
    );
    let history = thread_service
        .list_thread_history(ThreadHistoryRequest {
            scope: thread_scope,
            thread_id,
        })
        .await
        .unwrap();
    let expected_run_id = run_id.to_string();
    assert!(history.messages.iter().any(|message| {
        message.kind == MessageKind::Assistant
            && message.status == MessageStatus::Finalized
            && message.content.as_deref() == Some("model says hi")
            && message.turn_run_id.as_deref() == Some(expected_run_id.as_str())
    }));
}

/// Build the libSQL-backed [`FilesystemSessionThreadService`] used by the
/// restart test. The same `libsql::Database` handle drives the underlying
/// [`LibSqlRootFilesystem`]; reopening the database file from a sibling
/// process (or reconstructing it across a process-restart, as this test
/// simulates) exposes the same records through a fresh
/// `FilesystemSessionThreadService` instance. The `/threads` mount alias
/// resolves to a fixed top-level `VirtualPath` for the test; production
/// composition routes the alias through the per-invocation
/// [`MountView`](ironclaw_host_api::MountView) so tenant isolation is
/// structural rather than something this test has to thread through paths.
#[cfg(feature = "libsql-restart-tests")]
async fn build_libsql_thread_service(
    db: Arc<libsql::Database>,
) -> ironclaw_threads::FilesystemSessionThreadService<ironclaw_filesystem::LibSqlRootFilesystem> {
    use ironclaw_filesystem::{LibSqlRootFilesystem, ScopedFilesystem};
    use ironclaw_host_api::{MountAlias, MountGrant, MountPermissions};

    let fs = LibSqlRootFilesystem::new(db);
    fs.run_migrations().await.unwrap();
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/threads").unwrap(),
        VirtualPath::new("/threads").unwrap(),
        MountPermissions::read_write_list_delete(),
    )])
    .unwrap();
    let scoped = Arc::new(ScopedFilesystem::with_fixed_view(Arc::new(fs), mounts));
    ironclaw_threads::FilesystemSessionThreadService::new(scoped)
}

/// Construct a [`FilesystemTurnStateStore`] backed by [`LibSqlRootFilesystem`]
/// over the supplied libSQL database. The on-disk shape is the same single
/// `/turns/state.json` snapshot the production composition uses; the libSQL
/// backend just provides durability for the underlying filesystem record.
/// Mounts `/turns` at the canonical
/// `/engine/tenants/<tenant>/users/<user>/turns` target so the per-invocation
/// `MountView` shape lines up with the production wiring.
#[cfg(feature = "libsql-restart-tests")]
async fn libsql_filesystem_turn_store(
    db: Arc<libsql::Database>,
) -> ironclaw_turns::FilesystemTurnStateStore<ironclaw_filesystem::LibSqlRootFilesystem> {
    use ironclaw_filesystem::{LibSqlRootFilesystem, ScopedFilesystem};
    use ironclaw_host_api::{MountAlias, MountGrant, MountPermissions, MountView, VirtualPath};
    let filesystem = Arc::new(LibSqlRootFilesystem::new(db));
    filesystem.run_migrations().await.unwrap();
    let view = MountView::new(vec![MountGrant::new(
        MountAlias::new("/turns").unwrap(),
        VirtualPath::new("/engine/tenants/tenant-libsql-restart/users/user-libsql-restart/turns")
            .unwrap(),
        MountPermissions::read_write_list_delete(),
    )])
    .unwrap();
    let scoped = Arc::new(ScopedFilesystem::with_fixed_view(filesystem, view));
    ironclaw_turns::FilesystemTurnStateStore::new(scoped)
}

#[tokio::test]
async fn turn_runner_worker_drives_full_text_only_model_transcript_completion_after_missed_wake() {
    let fixture = HostFixture::new_unsubmitted("thread-runner-e2e", "hello full runner").await;
    let turn_store = Arc::new(InMemoryTurnStateStore::default());
    let resolver = InMemoryRunProfileResolver::default();
    let resolved = resolver
        .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
        .await
        .unwrap();
    let descriptor = resolved.loop_driver.clone();

    let run_id =
        queue_fixture_turn(&fixture, turn_store.as_ref(), &resolver, "idem-runner-e2e").await;

    let mut registry = DriverRegistry::new();
    registry
        .register_driver(
            Arc::new(TextOnlyFinalReplyDriver { descriptor }),
            DriverRequirements::all_required(),
            DriverKind::Reference,
        )
        .unwrap();

    let (_wake_sender, wake_receiver) = TurnRunnerWakeReceiver::new();
    let worker = TurnRunnerWorker::new(
        TurnRunnerWorkerConfig {
            heartbeat_interval: std::time::Duration::from_millis(20),
            poll_interval: std::time::Duration::from_millis(10),
            scope_filter: Some(fixture.context.scope.clone()),
        },
        turn_store.clone(),
        loop_exit_applier_for_fixture(&fixture, turn_store.clone()),
        Arc::new(registry),
        Arc::new(fixture.factory_with_loop_checkpoint_store(turn_store.clone())),
        wake_receiver,
    );

    let cancel = tokio_util::sync::CancellationToken::new();
    let cancel_clone = cancel.clone();
    let handle = tokio::spawn(async move { worker.run(cancel_clone).await });

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let state = turn_store
            .get_run_state(GetRunStateRequest {
                scope: fixture.context.scope.clone(),
                run_id,
            })
            .await
            .unwrap();
        if state.status == TurnStatus::Completed {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "worker should complete queued run via fallback polling after missed wake"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    cancel.cancel();
    handle.await.unwrap();

    let final_state = turn_store
        .get_run_state(GetRunStateRequest {
            scope: fixture.context.scope.clone(),
            run_id,
        })
        .await
        .unwrap();
    assert_eq!(final_state.status, TurnStatus::Completed);

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
    assert_eq!(fixture.gateway.requests().len(), 1);
    assert!(
        fixture.gateway.requests()[0]
            .messages
            .iter()
            .any(|message| message.content == "hello full runner"),
        "runner prompt should include original user content: {:?}",
        fixture.gateway.requests()[0].messages
    );
    assert_eq!(
        fixture.milestone_names(),
        vec![
            "prompt_bundle_built",
            "model_started",
            "model_completed",
            "assistant_reply_finalized",
        ]
    );
}

#[tokio::test]
async fn turn_runner_worker_drives_script_capability_through_real_host_runtime() {
    let fixture = HostFixture::new_unsubmitted(
        "thread-runner-script-capability-e2e",
        "hello script capability runner",
    )
    .await;
    let turn_store = Arc::new(InMemoryTurnStateStore::default());
    let resolver = InMemoryRunProfileResolver::default();
    let resolved = resolver
        .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
        .await
        .unwrap();
    let descriptor = resolved.loop_driver.clone();
    let input_ref = CapabilityInputRef::new("input:runner-script-capability-happy-path").unwrap();
    let input = json!({"message": "reborn runner script capability happy path"});
    let io = Arc::new(InMemoryCapabilityIo::default());
    io.put_input(input_ref.clone(), input.clone());
    let run_id = queue_fixture_turn(
        &fixture,
        turn_store.as_ref(),
        &resolver,
        "idem-runner-script",
    )
    .await;

    let mut registry = DriverRegistry::new();
    registry
        .register_driver(
            Arc::new(ScriptCapabilityFinalReplyDriver {
                descriptor,
                capability_id: e2e_script_capability_id(),
                input_ref,
            }),
            DriverRequirements::all_required(),
            DriverKind::Reference,
        )
        .unwrap();

    let runtime: Arc<dyn HostRuntime + Send + Sync> = Arc::new(
        HostRuntimeServices::new(
            Arc::new(e2e_registry_with_manifest(E2E_SCRIPT_MANIFEST)),
            Arc::new(LocalFilesystem::new()),
            Arc::new(InMemoryResourceGovernor::new()),
            Arc::new(GrantAuthorizer::new()),
            ProcessServices::in_memory(),
            ironclaw_host_runtime::CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        )
        .with_trust_policy(Arc::new(e2e_trust_policy()))
        .with_script_runtime(Arc::new(ScriptRuntime::new(
            ScriptRuntimeConfig::for_testing(),
            E2eEchoScriptBackend,
        )))
        .host_runtime_for_local_testing(),
    );
    let factory = CapabilityHostFactory {
        thread_service: fixture.thread_service.clone(),
        thread_scope: fixture.thread_scope.clone(),
        model_gateway: fixture.gateway.clone(),
        checkpoint_state_store: fixture.checkpoint_state_store.clone(),
        loop_checkpoint_store: turn_store.clone(),
        milestone_sink: fixture.milestone_sink.clone(),
        runtime,
        visible_request: host_runtime_visible_request_with_dispatch_grant(
            &fixture,
            e2e_script_capability_id(),
        ),
        io: io.clone(),
    };

    let (_wake_sender, wake_receiver) = TurnRunnerWakeReceiver::new();
    let worker = TurnRunnerWorker::new(
        TurnRunnerWorkerConfig {
            heartbeat_interval: std::time::Duration::from_millis(20),
            poll_interval: std::time::Duration::from_millis(10),
            scope_filter: Some(fixture.context.scope.clone()),
        },
        turn_store.clone(),
        loop_exit_applier_for_fixture(&fixture, turn_store.clone()),
        Arc::new(registry),
        Arc::new(factory),
        wake_receiver,
    );

    let cancel = tokio_util::sync::CancellationToken::new();
    let cancel_clone = cancel.clone();
    let handle = tokio::spawn(async move { worker.run(cancel_clone).await });

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let state = turn_store
            .get_run_state(GetRunStateRequest {
                scope: fixture.context.scope.clone(),
                run_id,
            })
            .await
            .unwrap();
        if state.status == TurnStatus::Completed {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "worker should complete queued run through script capability and final reply; last status={:?} failure={:?} milestones={:?}",
            state.status,
            state.failure,
            fixture.milestone_names()
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    cancel.cancel();
    handle.await.unwrap();

    let expected_result_ref = format!("result:{run_id}-{}", e2e_script_capability_id().as_str());
    assert_eq!(io.results(), vec![(e2e_script_capability_id(), input)]);
    assert_eq!(io.result_refs(), vec![expected_result_ref]);
    assert_eq!(fixture.gateway.requests().len(), 1);
    assert!(
        fixture.gateway.requests()[0]
            .messages
            .iter()
            .any(|message| message.content == "hello script capability runner"),
        "script capability prompt should include original user content: {:?}",
        fixture.gateway.requests()[0].messages
    );
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
    let milestone_names = fixture.milestone_names();
    assert!(milestone_names.contains(&"capability_invoked"));
    assert!(milestone_names.contains(&"prompt_bundle_built"));
    assert!(milestone_names.contains(&"model_started"));
    assert!(milestone_names.contains(&"model_completed"));
    assert!(milestone_names.contains(&"assistant_reply_finalized"));
    assert!(
        milestone_names
            .iter()
            .position(|name| *name == "capability_invoked")
            .expect("capability_invoked milestone should be present")
            < milestone_names
                .iter()
                .position(|name| *name == "assistant_reply_finalized")
                .expect("assistant_reply_finalized milestone should be present"),
        "capability must be invoked before final reply is persisted: {milestone_names:?}"
    );
}

#[tokio::test]
async fn turn_runner_rejects_driver_fabricated_approval_block_without_durable_gate_evidence() {
    let fixture = HostFixture::new_unsubmitted(
        "thread-runner-approval-fail-closed-e2e",
        "hello approval fail closed",
    )
    .await;
    let turn_store = Arc::new(InMemoryTurnStateStore::default());
    let resolver = InMemoryRunProfileResolver::default();
    let resolved = resolver
        .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
        .await
        .unwrap();
    let descriptor = resolved.loop_driver.clone();
    let run_id = queue_fixture_turn(
        &fixture,
        turn_store.as_ref(),
        &resolver,
        "idem-approval-fail-closed",
    )
    .await;

    let mut registry = DriverRegistry::new();
    registry
        .register_driver(
            Arc::new(ApprovalBlockThenFinalReplyDriver { descriptor }),
            DriverRequirements::all_required(),
            DriverKind::Reference,
        )
        .unwrap();

    let (_wake_sender, wake_receiver) = TurnRunnerWakeReceiver::new();
    let worker = TurnRunnerWorker::new(
        TurnRunnerWorkerConfig {
            heartbeat_interval: std::time::Duration::from_millis(20),
            poll_interval: std::time::Duration::from_millis(10),
            scope_filter: Some(fixture.context.scope.clone()),
        },
        turn_store.clone(),
        loop_exit_applier_for_fixture(&fixture, turn_store.clone()),
        Arc::new(registry),
        Arc::new(fixture.factory_with_loop_checkpoint_store(turn_store.clone())),
        wake_receiver,
    );

    let cancel = tokio_util::sync::CancellationToken::new();
    let cancel_clone = cancel.clone();
    let handle = tokio::spawn(async move { worker.run(cancel_clone).await });

    let recovery_state = wait_for_run_status(
        turn_store.as_ref(),
        &fixture.context.scope,
        run_id,
        TurnStatus::RecoveryRequired,
        "production-like evidence must reject fabricated approval block",
    )
    .await;
    cancel.cancel();
    handle.await.unwrap();

    assert_eq!(recovery_state.run_id, run_id);
    assert_eq!(recovery_state.gate_ref, None);
    assert_eq!(
        recovery_state.failure.expect("failure").category(),
        "driver_protocol_violation"
    );
    assert!(fixture.gateway.requests().is_empty());
    let history = fixture
        .thread_service
        .list_thread_history(ThreadHistoryRequest {
            scope: fixture.thread_scope.clone(),
            thread_id: fixture.thread_id.clone(),
        })
        .await
        .unwrap();
    assert!(
        !history
            .messages
            .iter()
            .any(|message| message.kind == MessageKind::Assistant),
        "fail-closed blocked evidence path must not finalize assistant replies"
    );
}

#[tokio::test]
async fn turn_runner_blocks_on_approval_then_coordinator_resume_completes_same_run() {
    let fixture =
        HostFixture::new_unsubmitted("thread-runner-approval-resume-e2e", "hello approval resume")
            .await;
    let turn_store = Arc::new(InMemoryTurnStateStore::default());
    let resolver = InMemoryRunProfileResolver::default();
    let resolved = resolver
        .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
        .await
        .unwrap();
    let descriptor = resolved.loop_driver.clone();
    let run_id = queue_fixture_turn(
        &fixture,
        turn_store.as_ref(),
        &resolver,
        "idem-approval-resume",
    )
    .await;

    let mut registry = DriverRegistry::new();
    registry
        .register_driver(
            Arc::new(ApprovalBlockThenFinalReplyDriver { descriptor }),
            DriverRequirements::all_required(),
            DriverKind::Reference,
        )
        .unwrap();

    let (_wake_sender, wake_receiver) = TurnRunnerWakeReceiver::new();
    let worker = TurnRunnerWorker::new(
        TurnRunnerWorkerConfig {
            heartbeat_interval: std::time::Duration::from_millis(20),
            poll_interval: std::time::Duration::from_millis(10),
            scope_filter: Some(fixture.context.scope.clone()),
        },
        turn_store.clone(),
        Arc::new(LoopExitApplier::new(
            turn_store.clone(),
            Arc::new(AlwaysVerifiedLoopExitEvidence),
        )),
        Arc::new(registry),
        Arc::new(fixture.factory_with_loop_checkpoint_store(turn_store.clone())),
        wake_receiver,
    );

    let cancel = tokio_util::sync::CancellationToken::new();
    let cancel_clone = cancel.clone();
    let handle = tokio::spawn(async move { worker.run(cancel_clone).await });

    let blocked_state = wait_for_run_status(
        turn_store.as_ref(),
        &fixture.context.scope,
        run_id,
        TurnStatus::BlockedApproval,
        "runner should block run on approval gate",
    )
    .await;
    let checkpoint_id = blocked_state
        .checkpoint_id
        .expect("blocked run must retain checkpoint for resume");
    let gate_ref = blocked_state
        .gate_ref
        .clone()
        .expect("blocked run must retain approval gate ref");

    let coordinator = DefaultTurnCoordinator::new(turn_store.clone());
    let resume = coordinator
        .resume_turn(ResumeTurnRequest {
            scope: fixture.context.scope.clone(),
            actor: TurnActor::new(UserId::new("user-text-host").unwrap()),
            run_id,
            gate_resolution_ref: gate_ref.clone(),
            source_binding_ref: SourceBindingRef::new("source-web-resumed").unwrap(),
            reply_target_binding_ref: ReplyTargetBindingRef::new("reply-web-resumed").unwrap(),
            idempotency_key: IdempotencyKey::new("resume-approval-once").unwrap(),
        })
        .await
        .unwrap();
    assert_eq!(resume.run_id, run_id);
    assert_eq!(resume.status, TurnStatus::Queued);
    assert_eq!(
        turn_store
            .events()
            .last()
            .expect("resume should emit lifecycle event")
            .kind,
        ironclaw_turns::TurnEventKind::Resumed,
        "coordinator resume must resolve the matching gate before the run can be queued"
    );

    let completed_state = wait_for_run_status(
        turn_store.as_ref(),
        &fixture.context.scope,
        run_id,
        TurnStatus::Completed,
        "resumed approval-blocked run should complete through driver resume",
    )
    .await;
    cancel.cancel();
    handle.await.unwrap();

    assert_eq!(completed_state.run_id, run_id);
    assert_eq!(completed_state.checkpoint_id, Some(checkpoint_id));
    assert_eq!(completed_state.gate_ref, None);
    let expected_run_id = run_id.to_string();
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
            && message.turn_run_id.as_deref() == Some(expected_run_id.as_str())
    }));
    assert_eq!(fixture.gateway.requests().len(), 1);
    assert!(
        fixture.gateway.requests()[0]
            .messages
            .iter()
            .any(|message| message.content == "hello approval resume"),
        "approval resume prompt should include original user content: {:?}",
        fixture.gateway.requests()[0].messages
    );
    let milestone_names = fixture.milestone_names();
    assert!(milestone_names.contains(&"prompt_bundle_built"));
    assert!(milestone_names.contains(&"assistant_reply_finalized"));
}

#[tokio::test]
async fn text_only_host_e2e_keeps_persisted_model_route_through_full_flow() {
    let fixture = HostFixture::new("thread-host-route-e2e", "hello routed e2e").await;
    let persisted_route = LoopModelRouteSnapshot::new(
        "openrouter",
        "anthropic/claude-sonnet-4",
        "config:v1",
        "auth:v1",
    );
    let stale_current_route = ModelRoute::new("nearai", "qwen3-coder").unwrap();
    let resolver = Arc::new(
        StaticModelRouteResolver::new(ModelRoutePolicy::new(
            ModelSelectionMode::DeveloperAnyConfigured,
        ))
        .with_route(ModelSlot::Default, stale_current_route),
    );
    let mut claimed = fixture.claimed.clone();
    claimed.state.resolved_model_route = Some(persisted_route.clone());
    let context = fixture
        .context
        .clone()
        .with_resolved_model_route(persisted_route.clone());
    let host = fixture
        .factory()
        .with_model_route_resolver(resolver)
        .build_text_only_host(RebornLoopDriverHostRequest {
            claimed_run: claimed,
            loop_run_context: context,
        })
        .await
        .unwrap();
    let host_dyn: &(dyn AgentLoopDriverHost + Send + Sync) = &host;

    let prompt_bundle = host_dyn
        .build_prompt_bundle(LoopPromptBundleRequest {
            mode: PromptMode::TextOnly,
            context_cursor: None,
            surface_version: None,
            checkpoint_state_ref: None,
            max_messages: Some(8),
            inline_messages: Vec::new(),
            capability_view: None,
        })
        .await
        .unwrap();
    let model_response = host_dyn
        .stream_model(LoopModelRequest {
            messages: prompt_bundle.messages,
            surface_version: None,
            model_preference: None,
            capability_view: None,
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
    let checkpoint_state = fixture
        .stage_checkpoint_state(
            LoopCheckpointKind::BeforeModel,
            b"durable route e2e checkpoint",
        )
        .await;
    let checkpoint_id = host_dyn
        .checkpoint(LoopCheckpointRequest {
            kind: LoopCheckpointKind::BeforeModel,
            state_ref: checkpoint_state.state_ref.clone(),
        })
        .await
        .unwrap();

    let requests = fixture.gateway.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].resolved_model_route, Some(persisted_route));
    assert_eq!(requests[0].messages[0].content, "hello routed e2e");
    assert!(reply_ref.as_str().starts_with("msg:"));
    assert!(
        fixture
            .loop_checkpoint_store
            .get_loop_checkpoint(GetLoopCheckpointRequest {
                scope: fixture.context.scope.clone(),
                turn_id: fixture.context.turn_id,
                run_id: fixture.context.run_id,
                checkpoint_id,
            })
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(
        fixture.milestone_names(),
        vec![
            "prompt_bundle_built",
            "model_started",
            "model_completed",
            "assistant_reply_finalized",
            "checkpoint_created",
        ]
    );
}

#[tokio::test]
async fn turn_runner_worker_records_recovery_when_real_host_factory_rejects_claimed_scope() {
    let fixture = HostFixture::new_unsubmitted("thread-runner-host-edge", "hello edge").await;
    let turn_store = Arc::new(InMemoryTurnStateStore::default());
    let resolver = InMemoryRunProfileResolver::default();
    let resolved = resolver
        .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
        .await
        .unwrap();
    let descriptor = resolved.loop_driver.clone();
    let run_id =
        queue_fixture_turn(&fixture, turn_store.as_ref(), &resolver, "idem-runner-edge").await;

    let mut registry = DriverRegistry::new();
    registry
        .register_driver(
            Arc::new(TextOnlyFinalReplyDriver { descriptor }),
            DriverRequirements::all_required(),
            DriverKind::Reference,
        )
        .unwrap();

    let wrong_thread_scope = ThreadScope {
        tenant_id: TenantId::new("tenant-other").unwrap(),
        ..fixture.thread_scope.clone()
    };
    let rejecting_factory = RebornLoopDriverHostFactory::new(
        Arc::clone(&fixture.thread_service),
        wrong_thread_scope,
        Arc::clone(&fixture.gateway),
        fixture.checkpoint_state_store.clone(),
        turn_store.clone(),
        turn_store.clone(),
        fixture.milestone_sink.clone(),
        TextOnlyLoopHostConfig {
            max_messages: 8,
            require_model_route_snapshot: false,
        },
    );

    let (_wake_sender, wake_receiver) = TurnRunnerWakeReceiver::new();
    let worker = TurnRunnerWorker::new(
        TurnRunnerWorkerConfig {
            heartbeat_interval: std::time::Duration::from_millis(20),
            poll_interval: std::time::Duration::from_millis(10),
            scope_filter: Some(fixture.context.scope.clone()),
        },
        turn_store.clone(),
        loop_exit_applier_for_fixture(&fixture, turn_store.clone()),
        Arc::new(registry),
        Arc::new(rejecting_factory),
        wake_receiver,
    );

    let cancel = tokio_util::sync::CancellationToken::new();
    let cancel_clone = cancel.clone();
    let handle = tokio::spawn(async move { worker.run(cancel_clone).await });

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let state = turn_store
            .get_run_state(GetRunStateRequest {
                scope: fixture.context.scope.clone(),
                run_id,
            })
            .await
            .unwrap();
        if state.status == TurnStatus::RecoveryRequired {
            assert!(state.failure.is_some());
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "host factory scope rejection should record RecoveryRequired"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    cancel.cancel();
    handle.await.unwrap();

    assert!(fixture.gateway.requests().is_empty());
    let history = fixture
        .thread_service
        .list_thread_history(ThreadHistoryRequest {
            scope: fixture.thread_scope.clone(),
            thread_id: fixture.thread_id.clone(),
        })
        .await
        .unwrap();
    assert!(
        !history
            .messages
            .iter()
            .any(|message| message.kind == MessageKind::Assistant)
    );
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
            mode: ironclaw_turns::run_profile::PromptMode::TextOnly,
        })
        .await
        .unwrap();
    assert_eq!(context.messages.len(), 1);
}

#[tokio::test]
async fn text_only_host_factory_create_host_uses_claimed_model_route_snapshot() {
    let fixture = HostFixture::new("thread-host-claimed-model-route", "hello routed host").await;
    let persisted_snapshot = LoopModelRouteSnapshot::new(
        "openrouter",
        "anthropic/claude-sonnet-4",
        "config:v1",
        "auth:v1",
    );
    let mut claimed = fixture.claimed.clone();
    claimed.state.resolved_model_route = Some(persisted_snapshot.clone());
    let resolver = Arc::new(
        StaticModelRouteResolver::new(ModelRoutePolicy::new(
            ModelSelectionMode::DeveloperAnyConfigured,
        ))
        .with_route(
            ModelSlot::Default,
            ModelRoute::new("openrouter", "anthropic/claude-sonnet-4").unwrap(),
        ),
    );

    let host = fixture
        .factory()
        .with_model_route_resolver(resolver)
        .create_host(&claimed)
        .await
        .unwrap();

    assert_eq!(
        host.run_context().resolved_model_route,
        Some(persisted_snapshot)
    );
}

#[tokio::test]
async fn planned_host_factory_create_host_uses_profiled_capabilities() {
    let fixture = HostFixture::new("thread-host-planned-profiled-capabilities", "hello").await;
    let allowed_id = CapabilityId::new("demo.allowed").unwrap();
    let denied_id = CapabilityId::new("demo.denied").unwrap();
    let runtime = Arc::new(RecordingHostRuntime::with_surface(host_runtime_surface([
        capability_descriptor(allowed_id.as_str()),
        capability_descriptor(denied_id.as_str()),
    ])));
    let io = Arc::new(InMemoryCapabilityIo::default());
    let capability_factory = Arc::new(TestHostRuntimeCapabilityFactory {
        runtime: runtime.clone(),
        visible_request: host_runtime_visible_request(&fixture, ["demo"]),
        io,
        milestone_sink: fixture.milestone_sink.clone(),
    });
    let surface_resolver = Arc::new(StaticCapabilitySurfaceProfileResolver::new(
        CapabilityAllowSet::allowlist([allowed_id.clone()]),
    ));
    let planned = default_planned_run_profile_resolver()
        .expect("planned default profile resolver")
        .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
        .await
        .unwrap();
    let mut claimed = fixture.claimed.clone();
    claimed.state.resolved_run_profile_id = planned.profile_id.clone();
    claimed.state.resolved_run_profile_version = planned.loop_driver.version;
    claimed.resolved_run_profile = planned;

    let host = fixture
        .factory()
        .with_driver_requirements(driver_requirements_for(
            &claimed.resolved_run_profile.loop_driver,
            DriverRequirements::all_required(),
        ))
        .with_profiled_capability_port_factory(capability_factory, surface_resolver)
        .create_host(&claimed)
        .await
        .unwrap();

    let surface = host
        .visible_capabilities(VisibleCapabilityRequest)
        .await
        .unwrap();
    assert_eq!(surface.descriptors.len(), 1);
    assert_eq!(surface.descriptors[0].capability_id, allowed_id);

    let outcome = host
        .invoke_capability(CapabilityInvocation {
            surface_version: surface.version,
            capability_id: denied_id,
            input_ref: CapabilityInputRef::new("input:denied-from-planned-host").unwrap(),
        })
        .await
        .unwrap();

    assert!(matches!(
        outcome,
        CapabilityOutcome::Denied(denied)
            if denied.reason_kind.as_str() == "surface_profile_denied"
    ));
    assert!(runtime.invocations().is_empty());
}

#[tokio::test]
async fn planned_host_factory_create_host_requires_profiled_capabilities() {
    let fixture = HostFixture::new("thread-host-planned-missing-capabilities", "hello").await;
    let planned = default_planned_run_profile_resolver()
        .expect("planned default profile resolver")
        .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
        .await
        .unwrap();
    let mut claimed = fixture.claimed.clone();
    claimed.state.resolved_run_profile_id = planned.profile_id.clone();
    claimed.state.resolved_run_profile_version = planned.loop_driver.version;
    claimed.resolved_run_profile = planned;

    let error = match fixture
        .factory()
        .with_driver_requirements(driver_requirements_for(
            &claimed.resolved_run_profile.loop_driver,
            DriverRequirements::all_required(),
        ))
        .create_host(&claimed)
        .await
    {
        Ok(_) => panic!("planned hosts must fail closed without profiled capabilities"),
        Err(error) => error,
    };

    assert!(error.reason.contains("profiled capability port factory"));
    assert!(error.reason.contains("capability-required driver host"));
    assert!(!error.reason.contains("planned driver host"));
}

#[tokio::test]
async fn planned_host_factory_fails_closed_when_driver_requirements_are_missing() {
    let fixture = HostFixture::new("thread-host-planned-missing-requirements", "hello").await;
    let planned = default_planned_run_profile_resolver()
        .expect("planned default profile resolver")
        .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
        .await
        .unwrap();
    let mut claimed = fixture.claimed.clone();
    claimed.state.resolved_run_profile_id = planned.profile_id.clone();
    claimed.state.resolved_run_profile_version = planned.loop_driver.version;
    claimed.resolved_run_profile = planned;

    let error = match fixture.factory().create_host(&claimed).await {
        Ok(_) => panic!("non-text-only hosts must fail closed without driver requirements"),
        Err(error) => error,
    };

    assert_eq!(
        error.reason,
        "loop driver requirements metadata is unavailable; cannot determine capability requirements"
    );
}

#[tokio::test]
async fn planned_host_factory_sanitizes_capability_profile_resolver_errors() {
    let fixture = HostFixture::new("thread-host-planned-profile-resolver-error", "hello").await;
    let allowed_id = CapabilityId::new("demo.allowed").unwrap();
    let runtime = Arc::new(RecordingHostRuntime::with_surface(host_runtime_surface([
        capability_descriptor(allowed_id.as_str()),
    ])));
    let io = Arc::new(InMemoryCapabilityIo::default());
    let capability_factory = Arc::new(TestHostRuntimeCapabilityFactory {
        runtime,
        visible_request: host_runtime_visible_request(&fixture, ["demo"]),
        io,
        milestone_sink: fixture.milestone_sink.clone(),
    });
    let surface_resolver = Arc::new(FailingCapabilitySurfaceProfileResolver::internal(
        "RAW_SECRET_TOKEN /tmp/private/trace",
    ));
    let planned = default_planned_run_profile_resolver()
        .expect("planned default profile resolver")
        .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
        .await
        .unwrap();
    let mut claimed = fixture.claimed.clone();
    claimed.state.resolved_run_profile_id = planned.profile_id.clone();
    claimed.state.resolved_run_profile_version = planned.loop_driver.version;
    claimed.resolved_run_profile = planned;

    let error = match fixture
        .factory()
        .with_driver_requirements(driver_requirements_for(
            &claimed.resolved_run_profile.loop_driver,
            DriverRequirements::all_required(),
        ))
        .with_profiled_capability_port_factory(capability_factory, surface_resolver)
        .create_host(&claimed)
        .await
    {
        Ok(_) => panic!("resolver failures must reject host creation"),
        Err(error) => error,
    };

    assert_eq!(
        error.reason,
        "invalid loop driver host request: capability surface profile could not be resolved"
    );
    assert!(!error.reason.contains("RAW_SECRET_TOKEN"));
    assert!(!error.reason.contains("/tmp/private"));
}

#[tokio::test]
async fn profiled_capability_surface_resolver_errors_are_sanitized() {
    let fixture = HostFixture::new("thread-planned-host-sanitized-surface", "hello").await;
    let planned = default_planned_run_profile_resolver()
        .expect("planned default profile resolver")
        .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
        .await
        .unwrap();
    let mut claimed = fixture.claimed.clone();
    claimed.state.resolved_run_profile_id = planned.profile_id.clone();
    claimed.state.resolved_run_profile_version = planned.loop_driver.version;
    claimed.resolved_run_profile = planned;
    let loop_run_context = LoopRunContext::new(
        fixture.context.scope.clone(),
        fixture.context.turn_id,
        fixture.context.run_id,
        claimed.resolved_run_profile.clone(),
    );
    let request = RebornLoopDriverHostRequest {
        claimed_run: claimed,
        loop_run_context,
    };

    let error = fixture
        .factory()
        .build_text_only_host_with_profiled_capabilities(
            request,
            Arc::new(EmptyLoopCapabilityPort),
            Arc::new(FailingCapabilitySurfaceProfileResolver::internal(
                "raw resolver failure sk-secret /host/path tool_input",
            )),
        )
        .await
        .expect_err("resolver failure should fail host construction");
    let reason = error.to_string();

    assert!(
        reason.contains("capability surface profile could not be resolved"),
        "reason: {reason}"
    );
    assert!(!reason.contains("sk-secret"), "reason: {reason}");
    assert!(!reason.contains("/host/path"), "reason: {reason}");
    assert!(!reason.contains("tool_input"), "reason: {reason}");
}

#[tokio::test]
async fn default_planned_runtime_composes_no_profile_coordinator_and_profiled_host_factory() {
    let fixture = HostFixture::new_unsubmitted("thread-runtime-planned-default", "hello").await;
    let turn_store = Arc::new(InMemoryTurnStateStore::default());
    let allowed_id = CapabilityId::new("demo.allowed").unwrap();
    let denied_id = CapabilityId::new("demo.denied").unwrap();
    let runtime = Arc::new(RecordingHostRuntime::with_surface(host_runtime_surface([
        capability_descriptor(allowed_id.as_str()),
        capability_descriptor(denied_id.as_str()),
    ])));
    let io = Arc::new(InMemoryCapabilityIo::default());
    let capability_factory = Arc::new(TestHostRuntimeCapabilityFactory {
        runtime: runtime.clone(),
        visible_request: host_runtime_visible_request(&fixture, ["demo"]),
        io,
        milestone_sink: fixture.milestone_sink.clone(),
    });
    let surface_resolver = Arc::new(StaticCapabilitySurfaceProfileResolver::new(
        CapabilityAllowSet::allowlist([allowed_id.clone()]),
    ));
    let evidence = Arc::new(ThreadCheckpointLoopExitEvidencePort::new(
        fixture.thread_service.clone(),
        turn_store.clone(),
        turn_store.clone(),
    ));
    let composition = build_default_planned_runtime(DefaultPlannedRuntimeParts {
        turn_state: turn_store.clone(),
        thread_service: fixture.thread_service.clone(),
        thread_scope: fixture.thread_scope.clone(),
        model_gateway: fixture.gateway.clone(),
        checkpoint_state_store: fixture.checkpoint_state_store.clone(),
        loop_checkpoint_store: turn_store.clone(),
        milestone_sink: fixture.milestone_sink.clone(),
        capability_factory,
        capability_surface_resolver: surface_resolver,
        loop_exit_evidence: evidence,
        config: DefaultPlannedRuntimeConfig {
            worker: TurnRunnerWorkerConfig {
                heartbeat_interval: std::time::Duration::from_millis(20),
                poll_interval: std::time::Duration::from_millis(10),
                scope_filter: Some(fixture.context.scope.clone()),
            },
            ..DefaultPlannedRuntimeConfig::default()
        },
        model_route_resolver: None,
        cancellation_factory: None,
        skill_context_source: None,
        input_queue: None,
        identity_context_source: Arc::new(StaticIdentityContextSource::new(Vec::new())),
        model_policy_guard: None,
        model_budget_accountant: None,
        safety_context: None,
    })
    .unwrap();

    let SubmitTurnResponse::Accepted { run_id, status, .. } = composition
        .coordinator
        .submit_turn(SubmitTurnRequest {
            scope: fixture.context.scope.clone(),
            actor: TurnActor::new(UserId::new("user-text-host").unwrap()),
            accepted_message_ref: AcceptedMessageRef::new("accepted-runtime-planned").unwrap(),
            source_binding_ref: SourceBindingRef::new("source-web").unwrap(),
            reply_target_binding_ref: ReplyTargetBindingRef::new("reply-web").unwrap(),
            requested_run_profile: None,
            idempotency_key: IdempotencyKey::new("idem-runtime-planned").unwrap(),
            received_at: Utc::now(),
        })
        .await
        .unwrap();
    assert_eq!(status, TurnStatus::Queued);

    let claimed = turn_store
        .claim_next_run(ClaimRunRequest {
            runner_id: composition.worker.runner_id(),
            lease_token: TurnLeaseToken::new(),
            scope_filter: Some(fixture.context.scope.clone()),
        })
        .await
        .unwrap()
        .expect("submitted planned run should be claimable");
    assert_eq!(claimed.state.run_id, run_id);
    assert_eq!(
        claimed.resolved_run_profile.profile_id.as_str(),
        "reborn-planned-default"
    );

    let host = composition
        .host_factory
        .create_host(&claimed)
        .await
        .unwrap();
    let surface = host
        .visible_capabilities(VisibleCapabilityRequest)
        .await
        .unwrap();
    assert_eq!(surface.descriptors.len(), 1);
    assert_eq!(surface.descriptors[0].capability_id, allowed_id);

    let outcome = host
        .invoke_capability(CapabilityInvocation {
            surface_version: surface.version,
            capability_id: denied_id,
            input_ref: CapabilityInputRef::new("input:runtime-denied").unwrap(),
        })
        .await
        .unwrap();
    assert!(matches!(
        outcome,
        CapabilityOutcome::Denied(denied)
            if denied.reason_kind.as_str() == "surface_profile_denied"
    ));
    assert!(runtime.invocations().is_empty());
}

// Identity source is now required by the `DefaultPlannedRuntimeParts` type
// signature, so the previous fail-closed runtime gate is enforced at compile
// time. The dynamic gate test has been retired alongside the dead
// `is_none()` check; the "builds when all required adapters are present" test
// below still proves the happy-path readiness contract.

#[tokio::test]
async fn product_live_runtime_builds_when_all_required_adapters_are_present() {
    let fixture = HostFixture::new_unsubmitted("thread-product-live-ready", "hello").await;
    let turn_store = Arc::new(InMemoryTurnStateStore::default());
    let runtime = Arc::new(RecordingHostRuntime::with_surface(host_runtime_surface([
        capability_descriptor("demo.allowed"),
    ])));
    let io = Arc::new(InMemoryCapabilityIo::default());
    let capability_factory = Arc::new(TestHostRuntimeCapabilityFactory {
        runtime,
        visible_request: host_runtime_visible_request(&fixture, ["demo"]),
        io,
        milestone_sink: fixture.milestone_sink.clone(),
    });
    let model_route_resolver: Arc<dyn ModelRouteResolver> = Arc::new(
        StaticModelRouteResolver::new(ModelRoutePolicy::new(
            ModelSelectionMode::DeveloperAnyConfigured,
        ))
        .with_route(
            ModelSlot::Default,
            ModelRoute::new("nearai", "qwen3-coder").unwrap(),
        ),
    );

    let composition = build_product_live_planned_runtime(DefaultPlannedRuntimeParts {
        turn_state: turn_store.clone(),
        thread_service: fixture.thread_service.clone(),
        thread_scope: fixture.thread_scope.clone(),
        model_gateway: fixture.gateway.clone(),
        checkpoint_state_store: fixture.checkpoint_state_store.clone(),
        loop_checkpoint_store: turn_store.clone(),
        milestone_sink: fixture.milestone_sink.clone(),
        capability_factory,
        capability_surface_resolver: Arc::new(StaticCapabilitySurfaceProfileResolver::new(
            CapabilityAllowSet::allowlist([CapabilityId::new("demo.allowed").unwrap()]),
        )),
        loop_exit_evidence: Arc::new(ThreadCheckpointLoopExitEvidencePort::new(
            fixture.thread_service.clone(),
            turn_state_store_dyn(&turn_store),
            turn_store,
        )),
        config: DefaultPlannedRuntimeConfig::default(),
        model_route_resolver: Some(model_route_resolver),
        cancellation_factory: Some(Arc::new(ReadyRunCancellationFactory::default())),
        skill_context_source: None,
        input_queue: Some(Arc::new(EmptyHostInputQueue)),
        identity_context_source: Arc::new(EmptyIdentityContextSource),
        model_policy_guard: Some(Arc::new(NoOpPolicyGuard)),
        model_budget_accountant: Some(Arc::new(NoOpBudgetAccountant)),
        safety_context: Some(test_safety_context()),
    })
    .expect("all product-live adapters should satisfy readiness");

    let resolved = composition
        .run_profile_resolver
        .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
        .await
        .unwrap();
    assert_eq!(resolved.profile_id.as_str(), "reborn-planned-default");
}

/// Build a fully-populated `DefaultPlannedRuntimeParts` for product-live
/// readiness gate tests. Callers below override individual fields with `None`
/// to assert the fail-closed branch fires.
async fn product_live_parts_for_gate_test(
    thread_label: &'static str,
) -> DefaultPlannedRuntimeParts<
    InMemoryTurnStateStore,
    InMemorySessionThreadService,
    RecordingGateway,
> {
    let fixture = HostFixture::new_unsubmitted(thread_label, "hello").await;
    let turn_store = Arc::new(InMemoryTurnStateStore::default());
    let runtime = Arc::new(RecordingHostRuntime::with_surface(host_runtime_surface([
        capability_descriptor("demo.allowed"),
    ])));
    let io = Arc::new(InMemoryCapabilityIo::default());
    let capability_factory = Arc::new(TestHostRuntimeCapabilityFactory {
        runtime,
        visible_request: host_runtime_visible_request(&fixture, ["demo"]),
        io,
        milestone_sink: fixture.milestone_sink.clone(),
    });
    let model_route_resolver: Arc<dyn ModelRouteResolver> = Arc::new(
        StaticModelRouteResolver::new(ModelRoutePolicy::new(
            ModelSelectionMode::DeveloperAnyConfigured,
        ))
        .with_route(
            ModelSlot::Default,
            ModelRoute::new("nearai", "qwen3-coder").unwrap(),
        ),
    );
    DefaultPlannedRuntimeParts {
        turn_state: turn_store.clone(),
        thread_service: fixture.thread_service.clone(),
        thread_scope: fixture.thread_scope.clone(),
        model_gateway: fixture.gateway.clone(),
        checkpoint_state_store: fixture.checkpoint_state_store.clone(),
        loop_checkpoint_store: turn_store.clone(),
        milestone_sink: fixture.milestone_sink.clone(),
        capability_factory,
        capability_surface_resolver: Arc::new(StaticCapabilitySurfaceProfileResolver::new(
            CapabilityAllowSet::allowlist([CapabilityId::new("demo.allowed").unwrap()]),
        )),
        loop_exit_evidence: Arc::new(ThreadCheckpointLoopExitEvidencePort::new(
            fixture.thread_service.clone(),
            turn_state_store_dyn(&turn_store),
            turn_store,
        )),
        config: DefaultPlannedRuntimeConfig::default(),
        model_route_resolver: Some(model_route_resolver),
        cancellation_factory: Some(Arc::new(ReadyRunCancellationFactory::default())),
        skill_context_source: None,
        input_queue: Some(Arc::new(EmptyHostInputQueue)),
        identity_context_source: Arc::new(EmptyIdentityContextSource),
        model_policy_guard: Some(Arc::new(NoOpPolicyGuard)),
        model_budget_accountant: Some(Arc::new(NoOpBudgetAccountant)),
        safety_context: Some(test_safety_context()),
    }
}

#[tokio::test]
async fn product_live_runtime_fails_closed_without_model_policy_guard() {
    let mut parts = product_live_parts_for_gate_test("thread-gate-policy-guard").await;
    parts.model_policy_guard = None;
    let error = build_product_live_planned_runtime(parts)
        .err()
        .expect("missing model_policy_guard must fail closed");
    assert!(
        error.to_string().contains("model_policy_guard"),
        "error should name the missing component: {error}"
    );
}

#[tokio::test]
async fn product_live_runtime_fails_closed_without_model_budget_accountant() {
    let mut parts = product_live_parts_for_gate_test("thread-gate-budget-accountant").await;
    parts.model_budget_accountant = None;
    let error = build_product_live_planned_runtime(parts)
        .err()
        .expect("missing model_budget_accountant must fail closed");
    assert!(
        error.to_string().contains("model_budget_accountant"),
        "error should name the missing component: {error}"
    );
}

#[tokio::test]
async fn product_live_runtime_fails_closed_without_safety_context() {
    let mut parts = product_live_parts_for_gate_test("thread-gate-safety-context").await;
    parts.safety_context = None;
    let error = build_product_live_planned_runtime(parts)
        .err()
        .expect("missing safety_context must fail closed");
    assert!(
        error.to_string().contains("safety_context"),
        "error should name the missing component: {error}"
    );
}

#[tokio::test]
async fn text_only_host_factory_threads_model_route_snapshot_to_gateway() {
    let fixture = HostFixture::new("thread-host-model-route", "hello routed host").await;
    let route = ModelRoute::new("nearai", "qwen3-coder").unwrap();
    let resolver = Arc::new(
        StaticModelRouteResolver::new(ModelRoutePolicy::new(
            ModelSelectionMode::DeveloperAnyConfigured,
        ))
        .with_route(ModelSlot::Default, route),
    );
    let host = fixture
        .factory()
        .with_model_route_resolver(resolver)
        .build_text_only_host(RebornLoopDriverHostRequest {
            claimed_run: fixture.claimed.clone(),
            loop_run_context: fixture.context.clone(),
        })
        .await
        .unwrap();
    let host_dyn: &(dyn AgentLoopDriverHost + Send + Sync) = &host;
    let snapshot = host_dyn
        .run_context()
        .resolved_model_route
        .clone()
        .expect("factory should attach model route snapshot");

    host_dyn
        .stream_model(LoopModelRequest {
            messages: host_dyn
                .build_prompt_bundle(LoopPromptBundleRequest {
                    mode: PromptMode::TextOnly,
                    context_cursor: None,
                    surface_version: None,
                    checkpoint_state_ref: None,
                    max_messages: Some(8),
                    inline_messages: Vec::new(),
                    capability_view: None,
                })
                .await
                .unwrap()
                .messages,
            surface_version: None,
            model_preference: None,
            capability_view: None,
        })
        .await
        .unwrap();

    let requests = fixture.gateway.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].resolved_model_route, Some(snapshot));
}

#[tokio::test]
async fn text_only_host_factory_reuses_existing_model_route_snapshot_without_reresolving() {
    let fixture = HostFixture::new("thread-host-model-route-reuse", "hello routed host").await;
    let persisted_route = ModelRoute::new("openrouter", "anthropic/claude-sonnet-4").unwrap();
    let replacement_route = ModelRoute::new("nearai", "qwen3-coder").unwrap();
    let resolver = Arc::new(
        StaticModelRouteResolver::new(
            ModelRoutePolicy::new(ModelSelectionMode::ManagedOnly)
                .with_approved_route(persisted_route.clone()),
        )
        .with_route(ModelSlot::Default, replacement_route),
    );
    let persisted_snapshot = LoopModelRouteSnapshot::new(
        "openrouter",
        "anthropic/claude-sonnet-4",
        "config:v1",
        "auth:v1",
    );
    let context = fixture
        .context
        .clone()
        .with_resolved_model_route(persisted_snapshot.clone());
    let mut claimed = fixture.claimed.clone();
    claimed.state.resolved_model_route = Some(persisted_snapshot.clone());

    let host = fixture
        .factory()
        .with_model_route_resolver(resolver)
        .build_text_only_host(RebornLoopDriverHostRequest {
            claimed_run: claimed,
            loop_run_context: context,
        })
        .await
        .unwrap();

    let host_dyn: &(dyn AgentLoopDriverHost + Send + Sync) = &host;
    assert_eq!(
        host_dyn.run_context().resolved_model_route,
        Some(persisted_snapshot)
    );
}

#[tokio::test]
async fn text_only_host_factory_rejects_persisted_model_route_snapshot_without_resolver() {
    let fixture =
        HostFixture::new("thread-host-model-route-no-resolver", "hello routed host").await;
    let persisted_snapshot = LoopModelRouteSnapshot::new(
        "openrouter",
        "anthropic/claude-sonnet-4",
        "config:v1",
        "auth:v1",
    );
    let context = fixture
        .context
        .clone()
        .with_resolved_model_route(persisted_snapshot.clone());
    let mut claimed = fixture.claimed.clone();
    claimed.state.resolved_model_route = Some(persisted_snapshot);

    let error = fixture
        .factory()
        .build_text_only_host(RebornLoopDriverHostRequest {
            claimed_run: claimed,
            loop_run_context: context,
        })
        .await
        .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("model route resolver is required")
    );
}

#[tokio::test]
async fn text_only_host_factory_rejects_persisted_model_route_snapshot_denied_by_policy() {
    let fixture = HostFixture::new("thread-host-model-route-denied", "hello routed host").await;
    let allowed_route = ModelRoute::new("nearai", "qwen3-coder").unwrap();
    let resolver = Arc::new(
        StaticModelRouteResolver::new(
            ModelRoutePolicy::new(ModelSelectionMode::ManagedOnly)
                .with_approved_route(allowed_route.clone()),
        )
        .with_route(ModelSlot::Default, allowed_route),
    );
    let denied_snapshot = LoopModelRouteSnapshot::new(
        "openrouter",
        "anthropic/claude-sonnet-4",
        "config:v1",
        "auth:v1",
    );
    let context = fixture
        .context
        .clone()
        .with_resolved_model_route(denied_snapshot.clone());
    let mut claimed = fixture.claimed.clone();
    claimed.state.resolved_model_route = Some(denied_snapshot);

    let error = fixture
        .factory()
        .with_model_route_resolver(resolver)
        .build_text_only_host(RebornLoopDriverHostRequest {
            claimed_run: claimed,
            loop_run_context: context,
        })
        .await
        .unwrap_err();

    assert!(error.to_string().contains("model route resolution failed"));
}

#[tokio::test]
async fn text_only_host_factory_rejects_unpersisted_context_model_route_snapshot() {
    let fixture = HostFixture::new("thread-host-model-route-injected", "hello routed host").await;
    let context = fixture
        .context
        .clone()
        .with_resolved_model_route(LoopModelRouteSnapshot::new(
            "openrouter",
            "anthropic/claude-sonnet-4",
            "config:v1",
            "auth:v1",
        ));

    let error = fixture
        .factory()
        .build_text_only_host(RebornLoopDriverHostRequest {
            claimed_run: fixture.claimed.clone(),
            loop_run_context: context,
        })
        .await
        .unwrap_err();

    assert!(error.to_string().contains("was not persisted"));
}

#[tokio::test]
async fn text_only_host_factory_rejects_mismatched_persisted_model_route_snapshot() {
    let fixture = HostFixture::new("thread-host-model-route-mismatch", "hello routed host").await;
    let context = fixture
        .context
        .clone()
        .with_resolved_model_route(LoopModelRouteSnapshot::new(
            "openrouter",
            "anthropic/claude-sonnet-4",
            "config:v1",
            "auth:v1",
        ));
    let mut claimed = fixture.claimed.clone();
    claimed.state.resolved_model_route = Some(LoopModelRouteSnapshot::new(
        "nearai",
        "qwen3-coder",
        "config:v1",
        "auth:v1",
    ));

    let error = fixture
        .factory()
        .build_text_only_host(RebornLoopDriverHostRequest {
            claimed_run: claimed,
            loop_run_context: context,
        })
        .await
        .unwrap_err();

    assert!(error.to_string().contains("does not match claimed run"));
}

#[tokio::test]
async fn text_only_host_factory_rejects_invalid_persisted_model_route_snapshot() {
    let fixture = HostFixture::new("thread-host-model-route-invalid", "hello routed host").await;
    let invalid_snapshot = LoopModelRouteSnapshot::new(
        "openrouter",
        "anthropic/secret-model",
        "config:v1",
        "auth:v1",
    );
    let context = fixture
        .context
        .clone()
        .with_resolved_model_route(invalid_snapshot.clone());
    let mut claimed = fixture.claimed.clone();
    claimed.state.resolved_model_route = Some(invalid_snapshot);

    let error = fixture
        .factory()
        .build_text_only_host(RebornLoopDriverHostRequest {
            claimed_run: claimed,
            loop_run_context: context,
        })
        .await
        .unwrap_err();

    assert!(error.to_string().contains("forbidden marker"));
}

#[tokio::test]
async fn text_only_host_factory_fails_fast_when_model_route_snapshot_required_without_resolver() {
    let fixture = HostFixture::new("thread-host-model-route-required", "hello routed host").await;
    let error = fixture
        .factory_with_config(TextOnlyLoopHostConfig {
            max_messages: 8,
            require_model_route_snapshot: true,
        })
        .build_text_only_host(RebornLoopDriverHostRequest {
            claimed_run: fixture.claimed.clone(),
            loop_run_context: fixture.context.clone(),
        })
        .await
        .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("model route resolver is required")
    );
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
            inline_messages: Vec::new(),
            capability_view: None,
        })
        .await
        .unwrap();
    let model_response = host_dyn
        .stream_model(LoopModelRequest {
            messages: prompt_bundle.messages,
            surface_version: Some(surface_version.clone()),
            model_preference: None,
            capability_view: None,
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
            inline_messages: Vec::new(),
            capability_view: None,
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
            inline_messages: Vec::new(),
            capability_view: None,
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
            inline_messages: Vec::new(),
            capability_view: None,
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
            inline_messages: Vec::new(),
            capability_view: None,
        })
        .await
        .unwrap_err();
    assert_eq!(zero_budget.kind, AgentLoopHostErrorKind::BudgetExceeded);
}

#[tokio::test]
async fn text_only_host_prompt_rejects_inline_messages() {
    let fixture = HostFixture::new("thread-host-prompt-inline", "hello reborn").await;
    let host = fixture.build_host().await;

    let error = host
        .build_prompt_bundle(LoopPromptBundleRequest {
            mode: PromptMode::TextOnly,
            context_cursor: None,
            surface_version: None,
            checkpoint_state_ref: None,
            max_messages: Some(8),
            capability_view: None,
            inline_messages: vec![LoopInlineMessage {
                role: LoopInlineMessageRole::User,
                safe_body: LoopSafeSummary::new("safe inline nudge").unwrap(),
            }],
        })
        .await
        .unwrap_err();

    assert_eq!(error.kind, AgentLoopHostErrorKind::PolicyDenied);
    assert_eq!(
        error.safe_summary,
        "inline_messages not yet supported by this prompt builder"
    );
    assert!(fixture.gateway.requests().is_empty());
    assert!(fixture.milestones().is_empty());
}

#[tokio::test]
async fn text_only_host_prompt_rejects_foreign_context_and_checkpoint_refs() {
    let fixture = HostFixture::new("thread-host-prompt-scope-refs", "hello reborn").await;
    let host = fixture.build_host().await;
    let other_context = LoopRunContext::new(
        fixture.context.scope.clone(),
        fixture.context.turn_id,
        TurnRunId::new(),
        fixture.context.resolved_run_profile.clone(),
    );

    let foreign_cursor = host
        .build_prompt_bundle(LoopPromptBundleRequest {
            mode: PromptMode::TextOnly,
            context_cursor: Some(LoopInputCursor::from_host_token(
                &other_context,
                LoopInputCursorToken::new("input-cursor:foreign-prompt").unwrap(),
            )),
            surface_version: None,
            checkpoint_state_ref: None,
            max_messages: Some(8),
            inline_messages: Vec::new(),
            capability_view: None,
        })
        .await
        .unwrap_err();
    assert_eq!(foreign_cursor.kind, AgentLoopHostErrorKind::ScopeMismatch);

    let foreign_checkpoint = host
        .build_prompt_bundle(LoopPromptBundleRequest {
            mode: PromptMode::TextOnly,
            context_cursor: None,
            surface_version: None,
            checkpoint_state_ref: Some(LoopCheckpointStateRef::new("checkpoint:foreign").unwrap()),
            max_messages: Some(8),
            inline_messages: Vec::new(),
            capability_view: None,
        })
        .await
        .unwrap_err();
    assert_eq!(
        foreign_checkpoint.kind,
        AgentLoopHostErrorKind::ScopeMismatch
    );

    let malformed_checkpoint = host
        .build_prompt_bundle(LoopPromptBundleRequest {
            mode: PromptMode::TextOnly,
            context_cursor: None,
            surface_version: None,
            checkpoint_state_ref: Some(
                LoopCheckpointStateRef::new(format!("checkpoint:{}:bad!", fixture.context.run_id))
                    .unwrap(),
            ),
            max_messages: Some(8),
            inline_messages: Vec::new(),
            capability_view: None,
        })
        .await
        .unwrap_err();
    assert_eq!(
        malformed_checkpoint.kind,
        AgentLoopHostErrorKind::InvalidInvocation
    );
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
async fn text_only_host_factory_threads_identity_source_to_prompt_and_model() {
    let fixture = HostFixture::new("thread-host-identity", "hello reborn").await;
    let source = Arc::new(StaticIdentityContextSource::new(vec![trusted_identity(
        "AGENTS.md",
        "factory identity content",
        IdentityApplicability::Always,
    )]));
    let host = fixture
        .factory()
        .with_identity_context_source(source)
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
    let prompt_bundle = host_dyn
        .build_prompt_bundle(LoopPromptBundleRequest {
            mode: PromptMode::TextOnly,
            context_cursor: None,
            surface_version: Some(surface.version.clone()),
            checkpoint_state_ref: None,
            max_messages: Some(8),
            inline_messages: Vec::new(),
            capability_view: None,
        })
        .await
        .unwrap();

    assert_eq!(prompt_bundle.messages[0].role, "system");
    assert!(
        prompt_bundle.messages[0]
            .content_ref
            .as_str()
            .starts_with("msg:identity.agents.md.")
    );

    host_dyn
        .stream_model(LoopModelRequest {
            messages: prompt_bundle.messages,
            surface_version: Some(surface.version),
            model_preference: None,
            capability_view: None,
        })
        .await
        .unwrap();

    let requests = fixture.gateway.requests();
    assert_eq!(requests[0].messages[0].content, "factory identity content");
}

#[tokio::test]
async fn text_only_host_factory_excludes_personal_identity_when_profile_excludes_it() {
    let fixture = HostFixture::new("thread-host-personal-excluded", "hello reborn").await;
    assert_eq!(
        fixture.context.resolved_run_profile.personal_context_policy,
        PersonalContextPolicy::Excluded
    );
    let source = Arc::new(StaticIdentityContextSource::new(vec![trusted_identity(
        "USER.md",
        "private user profile",
        IdentityApplicability::OnPersonalContextAllowed,
    )]));
    let host = fixture
        .factory()
        .with_identity_context_source(source)
        .build_text_only_host(RebornLoopDriverHostRequest {
            claimed_run: fixture.claimed.clone(),
            loop_run_context: fixture.context.clone(),
        })
        .await
        .unwrap();
    let host_dyn: &(dyn AgentLoopDriverHost + Send + Sync) = &host;

    let prompt_bundle = host_dyn
        .build_prompt_bundle(LoopPromptBundleRequest {
            mode: PromptMode::TextOnly,
            context_cursor: None,
            surface_version: None,
            checkpoint_state_ref: None,
            max_messages: Some(8),
            inline_messages: Vec::new(),
            capability_view: None,
        })
        .await
        .unwrap();

    assert_eq!(prompt_bundle.identity_message_count, 0);
    assert!(
        prompt_bundle
            .messages
            .iter()
            .all(|message| !message.content_ref.as_str().starts_with("msg:identity."))
    );
}

#[tokio::test]
async fn text_only_host_default_cancellation_factory_observes_durable_cancel_request() {
    let fixture = HostFixture::new("thread-host-default-cancel", "hello").await;
    let mut durable_state = fixture.claimed.state.clone();
    durable_state.status = TurnStatus::CancelRequested;
    let factory = RebornLoopDriverHostFactory::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        Arc::clone(&fixture.gateway),
        fixture.checkpoint_state_store.clone(),
        Arc::new(StaticTurnStateStore::new(durable_state)),
        fixture.loop_checkpoint_store.clone(),
        fixture.milestone_sink.clone(),
        TextOnlyLoopHostConfig {
            max_messages: 8,
            require_model_route_snapshot: false,
        },
    );

    assert!(factory.cancellation_observation_kind().is_live_capable());
    let host = factory
        .build_text_only_host(RebornLoopDriverHostRequest {
            claimed_run: fixture.claimed.clone(),
            loop_run_context: fixture.context.clone(),
        })
        .await
        .unwrap();

    let signal = host.observe_cancellation().expect("cancel signal");
    assert_eq!(signal.reason_kind, LoopCancelReasonKind::UserRequested);
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
        fixture.turn_state_store.clone(),
        fixture.loop_checkpoint_store.clone(),
        fixture.milestone_sink.clone(),
        TextOnlyLoopHostConfig {
            max_messages: 8,
            require_model_route_snapshot: false,
        },
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
async fn no_extra_loop_input_port_accepts_empty_ack_batch() {
    let fixture = HostFixture::new("thread-host-input-ack", "hello").await;
    let host = fixture.build_host().await;

    host.ack_inputs(Vec::new()).await.unwrap();
}

#[tokio::test]
async fn no_extra_loop_input_port_rejects_unissued_ack_token() {
    let fixture = HostFixture::new("thread-host-input-ack-forged", "hello").await;
    let host = fixture.build_host().await;

    let error = host
        .ack_inputs(vec![LoopInputAckToken::new("input-ack:forged").unwrap()])
        .await
        .unwrap_err();

    assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
}

/// Regression test for the `with_input_queue` factory composition path.
///
/// Proves that `RebornLoopDriverHostFactory::with_input_queue` actually wires the
/// provided `HostInputQueue` into the host's `LoopInputPort` — i.e. the factory
/// does not silently drop the queue. If the wiring were ever broken, `poll_inputs`
/// would return an empty batch (the `NoExtraLoopInputPort` default) instead of
/// delivering the steering message.
#[tokio::test]
async fn input_queue_wired_through_factory_drains_steering_message() {
    let fixture = HostFixture::new("thread-host-input-queue-factory", "hello input queue").await;

    let steering_ref = LoopMessageRef::new("msg:steering-factory-test").unwrap();
    let queue = Arc::new(SingleMessageQueue::new(LoopInput::Steering {
        message_ref: steering_ref.clone(),
    }));

    let host = fixture
        .factory()
        .with_input_queue(queue as Arc<dyn HostInputQueue>)
        .build_text_only_host(RebornLoopDriverHostRequest {
            claimed_run: fixture.claimed.clone(),
            loop_run_context: fixture.context.clone(),
        })
        .await
        .unwrap();

    let batch = host
        .poll_inputs(LoopInputCursor::origin_for_run(&fixture.context), 8)
        .await
        .unwrap();

    assert_eq!(
        batch.inputs,
        vec![LoopInput::Steering {
            message_ref: steering_ref
        }],
        "factory should wire input_queue into the host's LoopInputPort"
    );
    assert_eq!(batch.input_acks.len(), 1, "one ack token should be issued");
}

/// A minimal in-memory `HostInputQueue` that serves exactly one input item,
/// then returns empty batches.
struct SingleMessageQueue {
    input: Mutex<Option<LoopInput>>,
}

impl SingleMessageQueue {
    fn new(input: LoopInput) -> Self {
        Self {
            input: Mutex::new(Some(input)),
        }
    }
}

#[async_trait]
impl HostInputQueue for SingleMessageQueue {
    async fn next_after(
        &self,
        _run_id: TurnRunId,
        after: ironclaw_turns::run_profile::LoopInputCursorToken,
        _limit: usize,
    ) -> Result<HostInputBatch, HostInputQueueError> {
        let pending = self.input.lock().expect("queue lock").take();
        match pending {
            Some(input) => {
                let cursor =
                    ironclaw_turns::run_profile::LoopInputCursorToken::new("input-cursor:1")
                        .unwrap();
                let ack_token =
                    ironclaw_turns::run_profile::LoopInputAckToken::new("input-ack:1").unwrap();
                Ok(HostInputBatch {
                    inputs: vec![HostInputEnvelope {
                        input,
                        cursor: cursor.clone(),
                        ack_token,
                    }],
                    next_cursor: cursor,
                })
            }
            None => Ok(HostInputBatch {
                inputs: Vec::new(),
                next_cursor: after,
            }),
        }
    }

    async fn ack_consumed(
        &self,
        _run_id: TurnRunId,
        _tokens: Vec<ironclaw_turns::run_profile::LoopInputAckToken>,
    ) -> Result<(), HostInputQueueError> {
        Ok(())
    }
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
async fn text_only_host_stage_checkpoint_payload_returns_ref_usable_by_checkpoint() {
    let fixture = HostFixture::new("thread-host-stage-payload", "hello").await;
    let host = fixture.build_host().await;
    let host_dyn: &(dyn AgentLoopDriverHost + Send + Sync) = &host;

    // Two-step write: stage payload bytes, then write metadata referencing
    // the returned state ref. The kind must round-trip through both calls or
    // the read-side will reject the staged payload on resume.
    let state_ref = host_dyn
        .stage_checkpoint_payload(StageCheckpointPayloadRequest {
            kind: LoopCheckpointKind::BeforeSideEffect,
            schema_id: fixture.context.checkpoint_schema_id.as_str().to_string(),
            payload: b"durable resume bytes".to_vec(),
        })
        .await
        .expect("stage_checkpoint_payload should succeed for matching schema_id");

    // The returned ref must be run-scoped: `checkpoint:{run_id}:{token}`.
    assert!(
        state_ref.is_for_run(&fixture.context),
        "stage_checkpoint_payload must return a run-scoped LoopCheckpointStateRef"
    );

    let checkpoint_id = host_dyn
        .checkpoint(LoopCheckpointRequest {
            kind: LoopCheckpointKind::BeforeSideEffect,
            state_ref: state_ref.clone(),
        })
        .await
        .expect("checkpoint should accept the staged state_ref");

    let stored = fixture
        .loop_checkpoint_store
        .get_loop_checkpoint(GetLoopCheckpointRequest {
            scope: fixture.context.scope.clone(),
            turn_id: fixture.context.turn_id,
            run_id: fixture.context.run_id,
            checkpoint_id,
        })
        .await
        .unwrap()
        .expect("checkpoint id should resolve to the staged state ref");
    assert_eq!(stored.state_ref, state_ref);
    assert_eq!(stored.kind, LoopCheckpointKind::BeforeSideEffect);

    // The underlying state store indexes by the un-scoped `checkpoint:{token}`
    // key (generated by `new_state_ref`). Reconstruct it for the direct store
    // lookup: strip `checkpoint:{run_id}:` to get the token, then prefix with
    // `checkpoint:`.
    let run_scoped_prefix = format!("checkpoint:{}:", fixture.context.run_id);
    let token = state_ref
        .as_str()
        .strip_prefix(&run_scoped_prefix)
        .expect("state_ref should start with the run-scoped prefix");
    let store_ref = LoopCheckpointStateRef::new(format!("checkpoint:{token}"))
        .expect("store key must be a valid LoopCheckpointStateRef");

    // The read-side `get_checkpoint_state` authenticates `(state_ref, kind)`
    // together, so a kind mismatch must reject the staged payload.
    let with_correct_kind = fixture
        .checkpoint_state_store
        .get_checkpoint_state(GetCheckpointStateRequest {
            scope: fixture.context.scope.clone(),
            turn_id: fixture.context.turn_id,
            run_id: fixture.context.run_id,
            state_ref: store_ref.clone(),
            schema_id: fixture.context.checkpoint_schema_id.clone(),
            schema_version: fixture.context.checkpoint_schema_version,
            kind: LoopCheckpointKind::BeforeSideEffect,
        })
        .await
        .unwrap();
    assert!(with_correct_kind.is_some());

    let with_wrong_kind = fixture
        .checkpoint_state_store
        .get_checkpoint_state(GetCheckpointStateRequest {
            scope: fixture.context.scope.clone(),
            turn_id: fixture.context.turn_id,
            run_id: fixture.context.run_id,
            state_ref: store_ref,
            schema_id: fixture.context.checkpoint_schema_id.clone(),
            schema_version: fixture.context.checkpoint_schema_version,
            kind: LoopCheckpointKind::BeforeModel,
        })
        .await
        .unwrap();
    assert!(with_wrong_kind.is_none());
}

#[tokio::test]
async fn text_only_host_stage_checkpoint_payload_rejects_foreign_schema_id() {
    let fixture = HostFixture::new("thread-host-stage-foreign-schema", "hello").await;
    let host = fixture.build_host().await;
    let host_dyn: &(dyn AgentLoopDriverHost + Send + Sync) = &host;

    let error = host_dyn
        .stage_checkpoint_payload(StageCheckpointPayloadRequest {
            kind: LoopCheckpointKind::BeforeModel,
            schema_id: "some_other_schema_v1".to_string(),
            payload: b"payload bytes".to_vec(),
        })
        .await
        .expect_err("staging with a foreign schema_id must be rejected");
    assert_eq!(error.kind, AgentLoopHostErrorKind::CheckpointRejected);
}

#[tokio::test]
async fn text_only_host_skill_context_does_not_expand_capability_surface() {
    let fixture = HostFixture::new("thread-host-skill-capability", "hello").await;
    let source = Arc::new(StaticSkillContextSource::new(vec![
        HostSkillContextCandidate::new(
            skill_md(
                "installed-alpha",
                "installed skill description",
                "installed prompt must not imply tool authority",
            ),
            Some(SkillTrust::Installed),
            Some(SkillVisibility::Visible),
        ),
    ]));
    let host = fixture
        .factory()
        .with_skill_context_source(source)
        .build_text_only_host(RebornLoopDriverHostRequest {
            claimed_run: fixture.claimed.clone(),
            loop_run_context: fixture.context.clone(),
        })
        .await
        .unwrap();

    let prompt_bundle = host
        .build_prompt_bundle(LoopPromptBundleRequest {
            mode: PromptMode::TextOnly,
            context_cursor: None,
            surface_version: None,
            checkpoint_state_ref: None,
            max_messages: Some(8),
            inline_messages: Vec::new(),
            capability_view: None,
        })
        .await
        .unwrap();
    assert_eq!(prompt_bundle.messages.len(), 2);

    let surface = host
        .visible_capabilities(VisibleCapabilityRequest)
        .await
        .unwrap();
    assert!(surface.descriptors.is_empty());
    let outcome = host
        .invoke_capability_batch(ironclaw_turns::run_profile::CapabilityBatchInvocation {
            invocations: vec![CapabilityInvocation {
                surface_version: surface.version,
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
}

#[tokio::test]
async fn text_only_host_prompt_bundle_includes_surface_metadata_and_still_streams_model() {
    let fixture = HostFixture::new("thread-host-surface-prompt", "hello").await;
    let capability_id = CapabilityId::new("demo.echo").unwrap();
    let runtime = Arc::new(RecordingHostRuntime::with_surface(host_runtime_surface([
        capability_descriptor(capability_id.as_str()),
    ])));
    let capability_port = HostRuntimeLoopCapabilityPort::new(
        runtime,
        fixture.context.clone(),
        host_runtime_visible_request(&fixture, ["demo"]),
        Arc::new(InMemoryCapabilityIo::default()),
        Arc::new(InMemoryCapabilityIo::default()),
        fixture.milestone_sink.clone(),
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
    let prompt_bundle = host
        .build_prompt_bundle(LoopPromptBundleRequest {
            mode: PromptMode::TextOnly,
            context_cursor: None,
            surface_version: Some(surface.version.clone()),
            checkpoint_state_ref: None,
            max_messages: Some(8),
            inline_messages: Vec::new(),
            capability_view: None,
        })
        .await
        .unwrap();

    assert!(prompt_bundle.instruction_fingerprint.is_some());
    assert_eq!(prompt_bundle.surface_version, Some(surface.version.clone()));
    assert_eq!(prompt_bundle.messages.len(), 2);

    host.stream_model(LoopModelRequest {
        messages: prompt_bundle.messages,
        surface_version: Some(surface.version),
        model_preference: None,
        capability_view: None,
    })
    .await
    .unwrap();
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
        fixture.milestone_sink.clone(),
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
async fn text_only_host_profiled_capabilities_filter_surface_and_invocation() {
    let fixture = HostFixture::new("thread-host-runtime-capability-profile", "hello").await;
    let allowed_id = CapabilityId::new("demo.allowed").unwrap();
    let denied_id = CapabilityId::new("demo.denied").unwrap();
    let runtime = Arc::new(RecordingHostRuntime::with_surface(host_runtime_surface([
        capability_descriptor(allowed_id.as_str()),
        capability_descriptor(denied_id.as_str()),
    ])));
    let io = Arc::new(InMemoryCapabilityIo::default());
    let capability_port = HostRuntimeLoopCapabilityPort::new(
        runtime.clone(),
        fixture.context.clone(),
        host_runtime_visible_request(&fixture, ["demo"]),
        io.clone(),
        io,
        fixture.milestone_sink.clone(),
    );
    let resolver = Arc::new(StaticCapabilitySurfaceProfileResolver::new(
        CapabilityAllowSet::allowlist([allowed_id.clone()]),
    ));

    let host = fixture
        .factory()
        .build_text_only_host_with_profiled_capabilities(
            RebornLoopDriverHostRequest {
                claimed_run: fixture.claimed.clone(),
                loop_run_context: fixture.context.clone(),
            },
            Arc::new(capability_port),
            resolver,
        )
        .await
        .unwrap();

    let surface = host
        .visible_capabilities(VisibleCapabilityRequest)
        .await
        .unwrap();
    assert_eq!(surface.descriptors.len(), 1);
    assert_eq!(surface.descriptors[0].capability_id, allowed_id);

    let outcome = host
        .invoke_capability(CapabilityInvocation {
            surface_version: surface.version,
            capability_id: denied_id,
            input_ref: CapabilityInputRef::new("input:denied-profile").unwrap(),
        })
        .await
        .unwrap();

    assert!(matches!(
        outcome,
        CapabilityOutcome::Denied(denied)
            if denied.reason_kind.as_str() == "surface_profile_denied"
    ));
    assert!(runtime.invocations().is_empty());
}

// Fix 4 (henrypark): composition test — host profile filter wins over strategy `All`.
// This gates the host-wiring/cutover boundary: when the host-level CapabilitySurfaceProfileFilter
// restricts to only `tool_a`, invoking `tool_b` must be denied even though the strategy
// (via `CapabilityAllowSet::All`) would normally permit everything.
#[tokio::test]
async fn default_strategy_filter_all_loses_to_host_profile_filter() {
    let fixture = HostFixture::new("thread-host-profile-filter-wins", "hello").await;
    let tool_a_id = CapabilityId::new("demo.tool_a").unwrap();
    let tool_b_id = CapabilityId::new("demo.tool_b").unwrap();

    // The host runtime exposes both capabilities.
    let runtime = Arc::new(RecordingHostRuntime::with_surface(host_runtime_surface([
        capability_descriptor(tool_a_id.as_str()),
        capability_descriptor(tool_b_id.as_str()),
    ])));
    let io = Arc::new(InMemoryCapabilityIo::default());

    // Build a raw capability port wired to the host runtime (no profile filter yet).
    let capability_port = HostRuntimeLoopCapabilityPort::new(
        runtime.clone(),
        fixture.context.clone(),
        host_runtime_visible_request(&fixture, ["demo"]),
        io.clone(),
        io,
        fixture.milestone_sink.clone(),
    );

    // The profile resolver only allows tool_a — this is the host-level filter.
    // The strategy effectively resolves to `CapabilityAllowSet::All` for any
    // capability not explicitly blocked, but the host filter wraps the port and
    // takes precedence.
    let resolver = Arc::new(StaticCapabilitySurfaceProfileResolver::new(
        CapabilityAllowSet::allowlist([tool_a_id.clone()]),
    ));

    let host = fixture
        .factory()
        .build_text_only_host_with_profiled_capabilities(
            RebornLoopDriverHostRequest {
                claimed_run: fixture.claimed.clone(),
                loop_run_context: fixture.context.clone(),
            },
            Arc::new(capability_port),
            resolver,
        )
        .await
        .unwrap();

    // Surface should only expose tool_a (host filter applied).
    let surface = host
        .visible_capabilities(VisibleCapabilityRequest)
        .await
        .unwrap();
    assert_eq!(surface.descriptors.len(), 1);
    assert_eq!(surface.descriptors[0].capability_id, tool_a_id);

    // Invoking tool_b must be denied — the host profile filter wins over the
    // strategy's implicit `All` permit.
    let outcome = host
        .invoke_capability(CapabilityInvocation {
            surface_version: surface.version,
            capability_id: tool_b_id,
            input_ref: CapabilityInputRef::new("input:tool-b-denied").unwrap(),
        })
        .await
        .unwrap();

    assert!(
        matches!(
            &outcome,
            CapabilityOutcome::Denied(denied)
                if denied.reason_kind.as_str() == "surface_profile_denied"
        ),
        "expected surface_profile_denied, got {outcome:?}"
    );
    // The host runtime must not have been called for the denied invocation.
    assert!(
        runtime.invocations().is_empty(),
        "host runtime must not be invoked for a profile-denied capability"
    );
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
    let visible_request = host_runtime_visible_request(&fixture, ["demo"]);
    let capability_port = HostRuntimeLoopCapabilityPort::new(
        runtime.clone(),
        fixture.context.clone(),
        visible_request.clone(),
        io.clone(),
        io,
        fixture.milestone_sink.clone(),
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
    for invocation in invocations {
        assert_eq!(
            invocation.context.tenant_id,
            visible_request.context.tenant_id
        );
        assert_eq!(invocation.context.user_id, visible_request.context.user_id);
        assert_eq!(
            invocation.context.agent_id,
            visible_request.context.agent_id
        );
        assert_eq!(
            invocation.context.project_id,
            visible_request.context.project_id
        );
        assert_eq!(
            invocation.context.thread_id,
            visible_request.context.thread_id
        );
        assert_eq!(
            invocation.context.extension_id,
            ExtensionId::new(fixture.context.loop_driver_id.as_str()).unwrap()
        );
        assert_eq!(invocation.context.runtime, RuntimeKind::Wasm);
        assert_eq!(invocation.context.trust, TrustClass::UserTrusted);
        assert_eq!(invocation.context.grants, visible_request.context.grants);
        assert_eq!(invocation.context.mounts, visible_request.context.mounts);
    }
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
        fixture.milestone_sink.clone(),
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
        fixture.milestone_sink.clone(),
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
        fixture.milestone_sink.clone(),
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
async fn text_only_host_maps_explicit_unknown_runtime_outcome_to_failure() {
    let fixture = HostFixture::new("thread-host-runtime-capability-unknown", "hello").await;
    let capability_id = CapabilityId::new("demo.echo").unwrap();
    let runtime = Arc::new(RecordingHostRuntime::with_surface(host_runtime_surface([
        capability_descriptor(capability_id.as_str()),
    ])));
    runtime.push_outcome(RuntimeCapabilityOutcome::Unknown(
        RuntimeCapabilityUnknown {
            capability_id: capability_id.clone(),
            kind: "streaming".to_string(),
            message: Some("streaming outcomes are not supported by this loop port".to_string()),
        },
    ));
    let io = Arc::new(InMemoryCapabilityIo::default());
    let input_ref = CapabilityInputRef::new("input:unknown-outcome").unwrap();
    io.put_input(input_ref.clone(), json!({"message": "unknown"}));
    let capability_port = HostRuntimeLoopCapabilityPort::new(
        runtime,
        fixture.context.clone(),
        host_runtime_visible_request(&fixture, ["demo"]),
        io.clone(),
        io,
        fixture.milestone_sink.clone(),
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
    assert_eq!(
        failure.error_kind,
        CapabilityFailureKind::Unknown(
            ironclaw_turns::run_profile::CapabilityFailureKindValue::new("streaming")
                .expect("valid failure kind")
        )
    );
    assert_eq!(
        failure.safe_summary,
        "streaming outcomes are not supported by this loop port"
    );
}

#[tokio::test]
async fn text_only_host_preserves_host_runtime_error_kind_and_summary() {
    let fixture = HostFixture::new("thread-host-runtime-capability-host-error", "hello").await;
    let capability_id = CapabilityId::new("demo.echo").unwrap();
    let runtime = Arc::new(RecordingHostRuntime::with_surface(host_runtime_surface([
        capability_descriptor(capability_id.as_str()),
    ])));
    runtime.push_error(HostRuntimeError::invalid_request(
        "capability input schema invalid",
    ));
    runtime.push_error(HostRuntimeError::unavailable(
        "resource governor temporarily unavailable",
    ));
    let io = Arc::new(InMemoryCapabilityIo::default());
    let first_input = CapabilityInputRef::new("input:host-error-invalid").unwrap();
    let second_input = CapabilityInputRef::new("input:host-error-unavailable").unwrap();
    io.put_input(first_input.clone(), json!({"message": "invalid"}));
    io.put_input(second_input.clone(), json!({"message": "unavailable"}));
    let capability_port = HostRuntimeLoopCapabilityPort::new(
        runtime,
        fixture.context.clone(),
        host_runtime_visible_request(&fixture, ["demo"]),
        io.clone(),
        io,
        fixture.milestone_sink.clone(),
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

    let invalid = host
        .invoke_capability(CapabilityInvocation {
            surface_version: surface.version.clone(),
            capability_id: capability_id.clone(),
            input_ref: first_input,
        })
        .await
        .unwrap_err();
    let unavailable = host
        .invoke_capability(CapabilityInvocation {
            surface_version: surface.version,
            capability_id,
            input_ref: second_input,
        })
        .await
        .unwrap_err();

    assert_eq!(invalid.kind, AgentLoopHostErrorKind::InvalidInvocation);
    assert_eq!(invalid.safe_summary, "capability input schema invalid");
    assert_eq!(unavailable.kind, AgentLoopHostErrorKind::Unavailable);
    assert_eq!(
        unavailable.safe_summary,
        "resource governor temporarily unavailable"
    );
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
        fixture.milestone_sink.clone(),
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
        fixture.milestone_sink.clone(),
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
        fixture.milestone_sink.clone(),
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
            inline_messages: Vec::new(),
            capability_view: None,
        })
        .await
        .unwrap();

    assert_eq!(prompt.surface_version, Some(refreshed_surface.version));
}

#[tokio::test]
async fn text_only_host_waits_for_concurrent_duplicate_invocation_result() {
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
        fixture.milestone_sink.clone(),
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

    let first = first.unwrap();
    let second = second.unwrap();
    assert!(matches!(first, CapabilityOutcome::Completed(_)));
    assert_eq!(first, second);
    assert_eq!(runtime.invocations().len(), 1);
}

#[tokio::test]
async fn text_only_host_bounds_completed_dispatch_records() {
    let fixture = HostFixture::new(
        "thread-host-runtime-capability-bounded-idempotency",
        "hello",
    )
    .await;
    let capability_id = CapabilityId::new("demo.echo").unwrap();
    let runtime = Arc::new(RecordingHostRuntime::with_surface(host_runtime_surface([
        capability_descriptor(capability_id.as_str()),
    ])));
    let io = Arc::new(InMemoryCapabilityIo::default());
    let invocation_count = 130;
    let mut input_refs = Vec::new();
    for index in 0..invocation_count {
        runtime.push_outcome(RuntimeCapabilityOutcome::Completed(Box::new(
            RuntimeCapabilityCompleted {
                capability_id: capability_id.clone(),
                output: json!({"call": index}),
                usage: ResourceUsage::default(),
            },
        )));
        let input_ref = CapabilityInputRef::new(format!("input:bounded-{index}")).unwrap();
        io.put_input(input_ref.clone(), json!({"call": index}));
        input_refs.push(input_ref);
    }
    runtime.push_outcome(RuntimeCapabilityOutcome::Completed(Box::new(
        RuntimeCapabilityCompleted {
            capability_id: capability_id.clone(),
            output: json!({"call": "retried-after-eviction"}),
            usage: ResourceUsage::default(),
        },
    )));
    let capability_port = HostRuntimeLoopCapabilityPort::new(
        runtime.clone(),
        fixture.context.clone(),
        host_runtime_visible_request(&fixture, ["demo"]),
        io.clone(),
        io,
        fixture.milestone_sink.clone(),
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

    for input_ref in input_refs.iter().cloned() {
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
    let retried = host
        .invoke_capability(CapabilityInvocation {
            surface_version: surface.version,
            capability_id,
            input_ref: input_refs[0].clone(),
        })
        .await
        .unwrap();

    assert!(matches!(retried, CapabilityOutcome::Completed(_)));
    assert_eq!(runtime.invocations().len(), invocation_count + 1);
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
        fixture.milestone_sink.clone(),
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
        ironclaw_reborn::loop_driver_host::RebornLoopDriverHostError::InvalidRequest { .. }
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
        fixture.milestone_sink.clone(),
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
        fixture.milestone_sink.clone(),
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
        fixture.milestone_sink.clone(),
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

#[tokio::test]
async fn text_only_host_e2e_invokes_script_capability_through_real_host_runtime() {
    let fixture = HostFixture::new("thread-host-runtime-e2e-script", "hello e2e").await;
    let runtime: Arc<dyn HostRuntime + Send + Sync> = Arc::new(
        HostRuntimeServices::new(
            Arc::new(e2e_registry_with_manifest(E2E_SCRIPT_MANIFEST)),
            Arc::new(LocalFilesystem::new()),
            Arc::new(InMemoryResourceGovernor::new()),
            Arc::new(GrantAuthorizer::new()),
            ProcessServices::in_memory(),
            ironclaw_host_runtime::CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        )
        .with_trust_policy(Arc::new(e2e_trust_policy()))
        .with_script_runtime(Arc::new(ScriptRuntime::new(
            ScriptRuntimeConfig::for_testing(),
            E2eEchoScriptBackend,
        )))
        .host_runtime_for_local_testing(),
    );
    let io = Arc::new(InMemoryCapabilityIo::default());
    let input_ref = CapabilityInputRef::new("input:e2e-script-happy-path").unwrap();
    let input = json!({"message": "reborn adapter e2e happy path"});
    io.put_input(input_ref.clone(), input.clone());
    let capability_port = HostRuntimeLoopCapabilityPort::new(
        runtime,
        fixture.context.clone(),
        host_runtime_visible_request_with_dispatch_grant(&fixture, e2e_script_capability_id()),
        io.clone(),
        io.clone(),
        fixture.milestone_sink.clone(),
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
    assert_eq!(surface.descriptors.len(), 1);
    assert_eq!(
        surface.descriptors[0].capability_id,
        e2e_script_capability_id()
    );
    assert_eq!(surface.descriptors[0].runtime, RuntimeKind::Script);

    let outcome = host
        .invoke_capability(CapabilityInvocation {
            surface_version: surface.version,
            capability_id: e2e_script_capability_id(),
            input_ref,
        })
        .await
        .unwrap();

    let CapabilityOutcome::Completed(completed) = outcome else {
        panic!("expected completed script capability through host runtime");
    };
    assert!(completed.result_ref.as_str().starts_with("result:"));
    assert_eq!(completed.safe_summary, "capability completed");
    assert_eq!(io.results(), vec![(e2e_script_capability_id(), input)]);
    assert!(fixture.milestone_names().contains(&"capability_invoked"));
}

#[tokio::test]
async fn text_only_host_denies_capability_without_provider_trust_before_host_runtime() {
    let fixture = HostFixture::new("thread-host-runtime-capability-missing-trust", "hello").await;
    let capability_id = CapabilityId::new("demo.echo").unwrap();
    let runtime = Arc::new(RecordingHostRuntime::with_surface(host_runtime_surface([
        capability_descriptor(capability_id.as_str()),
    ])));
    let io = Arc::new(InMemoryCapabilityIo::default());
    let input_ref = CapabilityInputRef::new("input:missing-provider-trust").unwrap();
    io.put_input(input_ref.clone(), json!({"message": "must not dispatch"}));
    let capability_port = HostRuntimeLoopCapabilityPort::new(
        runtime.clone(),
        fixture.context.clone(),
        host_runtime_visible_request(&fixture, []),
        io.clone(),
        io,
        fixture.milestone_sink.clone(),
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
            capability_id,
            input_ref,
        })
        .await
        .unwrap();

    assert!(matches!(
        denied,
        CapabilityOutcome::Denied(denied)
            if denied.reason_kind.as_str() == "missing_provider_trust"
                && denied.safe_summary == "capability provider trust is unavailable"
    ));
    assert!(runtime.invocations().is_empty());
}

#[tokio::test]
async fn text_only_host_allows_retry_after_missing_capability_input_is_staged() {
    let fixture = HostFixture::new("thread-host-runtime-capability-input-retry", "hello").await;
    let capability_id = CapabilityId::new("demo.echo").unwrap();
    let runtime = Arc::new(RecordingHostRuntime::with_surface(host_runtime_surface([
        capability_descriptor(capability_id.as_str()),
    ])));
    runtime.push_outcome(RuntimeCapabilityOutcome::Completed(Box::new(
        RuntimeCapabilityCompleted {
            capability_id: capability_id.clone(),
            output: json!({"retried": true}),
            usage: ResourceUsage::default(),
        },
    )));
    let io = Arc::new(InMemoryCapabilityIo::default());
    let input_ref = CapabilityInputRef::new("input:stage-after-miss").unwrap();
    let capability_port = HostRuntimeLoopCapabilityPort::new(
        runtime.clone(),
        fixture.context.clone(),
        host_runtime_visible_request(&fixture, ["demo"]),
        io.clone(),
        io.clone(),
        fixture.milestone_sink.clone(),
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
        input_ref: input_ref.clone(),
    };

    let missing = host
        .invoke_capability(invocation.clone())
        .await
        .unwrap_err();
    assert_eq!(missing.kind, AgentLoopHostErrorKind::InvalidInvocation);
    assert!(runtime.invocations().is_empty());

    io.put_input(input_ref, json!({"message": "now staged"}));
    let retried = host.invoke_capability(invocation).await.unwrap();

    assert!(matches!(retried, CapabilityOutcome::Completed(_)));
    assert_eq!(runtime.invocations().len(), 1);
    assert_eq!(
        io.results(),
        vec![(capability_id, json!({"retried": true}))]
    );
}

#[derive(Clone)]
struct StaticSkillContextSource {
    candidates: Vec<HostSkillContextCandidate>,
}

impl StaticSkillContextSource {
    fn new(candidates: Vec<HostSkillContextCandidate>) -> Self {
        Self { candidates }
    }
}

#[async_trait]
impl HostSkillContextSource for StaticSkillContextSource {
    async fn load_skill_context_candidates(
        &self,
        _run_context: &LoopRunContext,
    ) -> Result<Vec<HostSkillContextCandidate>, HostSkillContextBuildError> {
        Ok(self.candidates.clone())
    }
}

#[derive(Clone)]
struct StaticIdentityContextSource {
    candidates: Vec<HostIdentityContextCandidate>,
    content_by_ref: HashMap<String, HostIdentityMessageContent>,
}

impl StaticIdentityContextSource {
    fn new(candidates: Vec<(HostIdentityContextCandidate, String)>) -> Self {
        let mut context_candidates = Vec::with_capacity(candidates.len());
        let mut content_by_ref = HashMap::new();
        for (candidate, content) in candidates {
            if let Some(message_ref) = candidate.message_ref.as_ref() {
                content_by_ref.insert(
                    message_ref.as_str().to_string(),
                    HostIdentityMessageContent {
                        name: candidate.name.clone(),
                        content,
                    },
                );
            }
            context_candidates.push(candidate);
        }
        Self {
            candidates: context_candidates,
            content_by_ref,
        }
    }
}

#[async_trait]
impl HostIdentityContextSource for StaticIdentityContextSource {
    async fn load_identity_candidates(
        &self,
        _run_context: &LoopRunContext,
        _mode: PromptMode,
    ) -> Result<Vec<HostIdentityContextCandidate>, HostIdentityContextBuildError> {
        Ok(self.candidates.clone())
    }

    async fn resolve_identity_message_content(
        &self,
        _run_context: &LoopRunContext,
        message_ref: &ironclaw_turns::LoopMessageRef,
    ) -> Result<Option<HostIdentityMessageContent>, HostIdentityContextBuildError> {
        Ok(self.content_by_ref.get(message_ref.as_str()).cloned())
    }
}

fn trusted_identity(
    name: &str,
    content: &str,
    applies_when: IdentityApplicability,
) -> (HostIdentityContextCandidate, String) {
    let name = IdentityFileName::new(name).unwrap();
    let message_ref = identity_message_ref(&name, content).unwrap();
    (
        HostIdentityContextCandidate::new_trusted(
            name.clone(),
            message_ref,
            format!("identity file {} available", name.as_str()),
            applies_when,
            content.len(),
        ),
        content.to_string(),
    )
}

struct StaticCapabilitySurfaceProfileResolver {
    allow_set: CapabilityAllowSet,
}

impl StaticCapabilitySurfaceProfileResolver {
    fn new(allow_set: CapabilityAllowSet) -> Self {
        Self { allow_set }
    }
}

#[async_trait]
impl CapabilitySurfaceProfileResolver for StaticCapabilitySurfaceProfileResolver {
    async fn resolve(
        &self,
        _run_context: &LoopRunContext,
    ) -> Result<CapabilityAllowSet, CapabilityResolveError> {
        Ok(self.allow_set.clone())
    }
}

struct FailingCapabilitySurfaceProfileResolver {
    reason: String,
}

impl FailingCapabilitySurfaceProfileResolver {
    fn internal(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

#[async_trait]
impl CapabilitySurfaceProfileResolver for FailingCapabilitySurfaceProfileResolver {
    async fn resolve(
        &self,
        _run_context: &LoopRunContext,
    ) -> Result<CapabilityAllowSet, CapabilityResolveError> {
        Err(CapabilityResolveError::internal(self.reason.clone()))
    }
}

struct EmptyHostInputQueue;

#[async_trait]
impl HostInputQueue for EmptyHostInputQueue {
    async fn next_after(
        &self,
        _run_id: TurnRunId,
        after: LoopInputCursorToken,
        _limit: usize,
    ) -> Result<HostInputBatch, HostInputQueueError> {
        Ok(HostInputBatch {
            inputs: Vec::<HostInputEnvelope>::new(),
            next_cursor: after,
        })
    }

    async fn ack_consumed(
        &self,
        _run_id: TurnRunId,
        _tokens: Vec<LoopInputAckToken>,
    ) -> Result<(), HostInputQueueError> {
        Ok(())
    }
}

struct EmptyIdentityContextSource;

#[async_trait]
impl HostIdentityContextSource for EmptyIdentityContextSource {
    async fn load_identity_candidates(
        &self,
        _run_context: &LoopRunContext,
        _mode: PromptMode,
    ) -> Result<Vec<HostIdentityContextCandidate>, HostIdentityContextBuildError> {
        Ok(Vec::new())
    }
}

#[derive(Default)]
struct ReadyRunCancellationFactory {
    handles: Arc<Mutex<HashMap<TurnRunId, RunCancellationHandle>>>,
}

#[async_trait]
impl RunCancellationFactory for ReadyRunCancellationFactory {
    async fn handle_for_run(
        &self,
        _scope: &TurnScope,
        run_id: TurnRunId,
    ) -> Result<RunCancellationHandle, AgentLoopHostError> {
        let handle = RunCancellationHandle::default();
        self.handles
            .lock()
            .expect("ready cancellation lock poisoned")
            .insert(run_id, handle.clone());
        Ok(handle)
    }

    fn product_live_cancellation_probe(&self) -> Option<Box<dyn ProductLiveCancellationProbe>> {
        let run_id = TurnRunId::new();
        let handle = RunCancellationHandle::default();
        self.handles
            .lock()
            .expect("ready cancellation lock poisoned")
            .insert(run_id, handle);
        Some(Box::new(ReadyRunCancellationProbe {
            handles: Arc::clone(&self.handles),
            run_id,
        }))
    }

    fn is_product_cancellation_observed(
        &self,
        run_id: TurnRunId,
    ) -> Result<bool, AgentLoopHostError> {
        Ok(self
            .handles
            .lock()
            .expect("ready cancellation lock poisoned")
            .get(&run_id)
            .map(|handle| handle.is_requested())
            .unwrap_or(false))
    }
}

struct ReadyRunCancellationProbe {
    handles: Arc<Mutex<HashMap<TurnRunId, RunCancellationHandle>>>,
    run_id: TurnRunId,
}

impl ReadyRunCancellationProbe {
    fn handle_for(&self) -> Result<RunCancellationHandle, AgentLoopHostError> {
        self.handles
            .lock()
            .expect("ready cancellation lock poisoned")
            .get(&self.run_id)
            .cloned()
            .ok_or_else(|| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Unavailable,
                    "product cancellation probe handle was not retained",
                )
            })
    }
}

impl ProductLiveCancellationProbe for ReadyRunCancellationProbe {
    fn request_cancellation(
        &self,
        reason_kind: LoopCancelReasonKind,
    ) -> Result<(), AgentLoopHostError> {
        self.handle_for()?.request(reason_kind);
        Ok(())
    }

    fn is_cancellation_observed(&self) -> Result<bool, AgentLoopHostError> {
        Ok(self.handle_for()?.is_requested())
    }
}

struct TestHostRuntimeCapabilityFactory {
    runtime: Arc<RecordingHostRuntime>,
    visible_request: ironclaw_host_runtime::VisibleCapabilityRequest,
    io: Arc<InMemoryCapabilityIo>,
    milestone_sink: Arc<InMemoryLoopHostMilestoneSink>,
}

#[async_trait]
impl LoopCapabilityPortFactory for TestHostRuntimeCapabilityFactory {
    async fn create_capability_port(
        &self,
        run_context: &LoopRunContext,
    ) -> Result<Arc<dyn LoopCapabilityPort>, AgentLoopHostError> {
        let port = HostRuntimeLoopCapabilityPort::new(
            self.runtime.clone(),
            run_context.clone(),
            self.visible_request.clone(),
            self.io.clone(),
            self.io.clone(),
            self.milestone_sink.clone(),
        );
        Ok(Arc::new(port))
    }
}

fn skill_md(name: &str, description: &str, prompt: &str) -> String {
    format!(
        "---\nname: {name}\ndescription: {description}\nactivation:\n  keywords: [{name}]\n---\n\n{prompt}\n"
    )
}

/// In-memory capability I/O fixture.
///
/// `results` captures the structured capability output, while `result_refs`
/// captures the materialized ref returned to the driver so e2e tests can assert
/// both payload persistence and ref propagation.
#[derive(Default)]
struct InMemoryCapabilityIo {
    inputs: Mutex<BTreeMap<String, Value>>,
    results: Mutex<Vec<(CapabilityId, Value)>>,
    result_refs: Mutex<Vec<String>>,
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

    fn result_refs(&self) -> Vec<String> {
        self.result_refs.lock().unwrap().clone()
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
        let result_ref = format!("result:{}-{}", run_context.run_id, capability_id.as_str());
        self.result_refs.lock().unwrap().push(result_ref.clone());
        LoopResultRef::new(result_ref).map_err(|_| {
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
    errors: Mutex<Vec<HostRuntimeError>>,
    invocations: Mutex<Vec<RuntimeCapabilityRequest>>,
}

impl RecordingHostRuntime {
    fn with_surface(surface: ironclaw_host_runtime::VisibleCapabilitySurface) -> Self {
        Self {
            surface: Mutex::new(surface),
            outcomes: Mutex::new(Vec::new()),
            errors: Mutex::new(Vec::new()),
            invocations: Mutex::new(Vec::new()),
        }
    }

    fn push_outcome(&self, outcome: RuntimeCapabilityOutcome) {
        self.outcomes.lock().unwrap().push(outcome);
    }

    fn push_error(&self, error: HostRuntimeError) {
        self.errors.lock().unwrap().push(error);
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
        let mut errors = self.errors.lock().unwrap();
        if !errors.is_empty() {
            return Err(errors.remove(0));
        }
        drop(errors);
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
        ExtensionId::new(fixture.context.loop_driver_id.as_str()).unwrap(),
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

fn host_runtime_visible_request_with_dispatch_grant(
    fixture: &HostFixture,
    capability_id: CapabilityId,
) -> ironclaw_host_runtime::VisibleCapabilityRequest {
    let mut request = host_runtime_visible_request(fixture, ["script"]);
    request.context.grants.grants.push(CapabilityGrant {
        id: CapabilityGrantId::new(),
        capability: capability_id,
        grantee: Principal::Extension(request.context.extension_id.clone()),
        issued_by: Principal::HostRuntime,
        constraints: GrantConstraints {
            allowed_effects: vec![EffectKind::DispatchCapability],
            mounts: MountView::default(),
            network: NetworkPolicy::default(),
            secrets: Vec::new(),
            resource_ceiling: None,
            expires_at: None,
            max_invocations: None,
        },
    });
    request
}

fn e2e_registry_with_manifest(manifest: &str) -> ExtensionRegistry {
    let mut registry = ExtensionRegistry::new();
    let manifest = ExtensionManifest::parse(
        manifest,
        ManifestSource::InstalledLocal,
        &HostPortCatalog::empty(),
    )
    .unwrap();
    let package = ExtensionPackage::from_manifest(
        manifest,
        VirtualPath::new("/system/extensions/script").unwrap(),
    )
    .unwrap();
    registry.insert(package).unwrap();
    registry
}

fn e2e_trust_policy() -> HostTrustPolicy {
    HostTrustPolicy::new(vec![Box::new(AdminConfig::with_entries(vec![
        AdminEntry::for_local_manifest(
            PackageId::new("script").unwrap(),
            "/system/extensions/script/manifest.toml".to_string(),
            None,
            HostTrustAssignment::user_trusted(),
            vec![EffectKind::DispatchCapability],
            None,
        ),
    ]))])
    .unwrap()
}

fn e2e_script_capability_id() -> CapabilityId {
    CapabilityId::new("script.echo").unwrap()
}

struct E2eEchoScriptBackend;

impl ScriptBackend for E2eEchoScriptBackend {
    fn execute(&self, request: ScriptBackendRequest) -> Result<ScriptBackendOutput, String> {
        let value = serde_json::from_str(&request.stdin_json).map_err(|error| error.to_string())?;
        Ok(ScriptBackendOutput::json(value))
    }
}

const E2E_SCRIPT_MANIFEST: &str = r#"
id = "script"
name = "Script Echo"
version = "0.1.0"
description = "Script echo test extension"
trust = "third_party"

[runtime]
kind = "script"
runner = "sandboxed_process"
command = "echo-script"
args = []

[[capabilities]]
id = "script.echo"
description = "Echo text through Reborn adapter e2e"
effects = ["dispatch_capability"]
default_permission = "allow"
parameters_schema = { type = "object" }
"#;

/// Test-only evidence port that bypasses all durable evidence checks.
///
/// Use only when the test asserts behavior outside evidence verification; use
/// `ThreadCheckpointLoopExitEvidencePort` when the evidence path itself matters.
struct AlwaysVerifiedLoopExitEvidence;

#[async_trait]
impl LoopExitEvidencePort for AlwaysVerifiedLoopExitEvidence {
    async fn verify_completion_refs(
        &self,
        _request: CompletionEvidenceRequest<'_>,
    ) -> Result<bool, TurnError> {
        Ok(true)
    }

    async fn verify_final_checkpoint(
        &self,
        _request: FinalCheckpointEvidenceRequest<'_>,
    ) -> Result<bool, TurnError> {
        Ok(true)
    }

    async fn verify_blocked_evidence(
        &self,
        _request: BlockedEvidenceRequest<'_>,
    ) -> Result<bool, TurnError> {
        Ok(true)
    }

    async fn verify_failure_evidence(
        &self,
        _request: FailureEvidenceRequest<'_>,
    ) -> Result<bool, TurnError> {
        Ok(true)
    }

    async fn is_cancellation_observed(
        &self,
        _scope: &TurnScope,
        _turn_id: TurnId,
        _run_id: TurnRunId,
    ) -> Result<bool, TurnError> {
        Ok(true)
    }

    async fn latest_checkpoint_kind(
        &self,
        _scope: &TurnScope,
        _turn_id: TurnId,
        _run_id: TurnRunId,
    ) -> Result<Option<LoopCheckpointKind>, TurnError> {
        Ok(Some(LoopCheckpointKind::BeforeBlock))
    }
}

async fn wait_for_run_status(
    store: &dyn TurnStateStore,
    scope: &TurnScope,
    run_id: TurnRunId,
    expected: TurnStatus,
    failure_message: &'static str,
) -> ironclaw_turns::TurnRunState {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let state = store
            .get_run_state(GetRunStateRequest {
                scope: scope.clone(),
                run_id,
            })
            .await
            .unwrap();
        if state.status == expected {
            return state;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "{failure_message}; last status={:?} failure={:?}",
            state.status,
            state.failure
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

struct CapabilityHostFactory {
    thread_service: Arc<InMemorySessionThreadService>,
    thread_scope: ThreadScope,
    model_gateway: Arc<RecordingGateway>,
    checkpoint_state_store: Arc<InMemoryCheckpointStateStore>,
    loop_checkpoint_store: Arc<dyn LoopCheckpointStore>,
    milestone_sink: Arc<InMemoryLoopHostMilestoneSink>,
    runtime: Arc<dyn HostRuntime + Send + Sync>,
    visible_request: ironclaw_host_runtime::VisibleCapabilityRequest,
    io: Arc<InMemoryCapabilityIo>,
}

#[async_trait]
impl HostFactory for CapabilityHostFactory {
    async fn create_host(
        &self,
        claimed: &ClaimedTurnRun,
    ) -> Result<Box<dyn AgentLoopDriverHost + Send + Sync>, HostFactoryError> {
        let mut loop_run_context = LoopRunContext::new(
            claimed.state.scope.clone(),
            claimed.state.turn_id,
            claimed.state.run_id,
            claimed.resolved_run_profile.clone(),
        );
        if let Some(snapshot) = claimed.state.resolved_model_route.clone() {
            loop_run_context = loop_run_context.with_resolved_model_route(snapshot);
        }
        let capability_port = HostRuntimeLoopCapabilityPort::new(
            self.runtime.clone(),
            loop_run_context.clone(),
            self.visible_request.clone(),
            self.io.clone(),
            self.io.clone(),
            self.milestone_sink.clone(),
        );
        RebornLoopDriverHostFactory::new(
            self.thread_service.clone(),
            self.thread_scope.clone(),
            self.model_gateway.clone(),
            self.checkpoint_state_store.clone(),
            Arc::new(StaticTurnStateStore::new(claimed.state.clone())),
            self.loop_checkpoint_store.clone(),
            self.milestone_sink.clone(),
            TextOnlyLoopHostConfig {
                max_messages: 8,
                require_model_route_snapshot: false,
            },
        )
        .build_text_only_host_with_capabilities(
            RebornLoopDriverHostRequest {
                claimed_run: claimed.clone(),
                loop_run_context,
            },
            Arc::new(capability_port),
        )
        .await
        .map(|host| Box::new(host) as Box<dyn AgentLoopDriverHost + Send + Sync>)
        .map_err(|error| HostFactoryError::new(error.to_string()))
    }
}

struct ScriptCapabilityFinalReplyDriver {
    descriptor: AgentLoopDriverDescriptor,
    capability_id: CapabilityId,
    input_ref: CapabilityInputRef,
}

#[async_trait]
impl AgentLoopDriver for ScriptCapabilityFinalReplyDriver {
    fn descriptor(&self) -> AgentLoopDriverDescriptor {
        self.descriptor.clone()
    }

    async fn run(
        &self,
        _request: AgentLoopDriverRunRequest,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
    ) -> Result<LoopExit, AgentLoopDriverError> {
        let surface = host
            .visible_capabilities(VisibleCapabilityRequest)
            .await
            .map_err(driver_host_error)?;
        let capability = host
            .invoke_capability(CapabilityInvocation {
                surface_version: surface.version.clone(),
                capability_id: self.capability_id.clone(),
                input_ref: self.input_ref.clone(),
            })
            .await
            .map_err(driver_host_error)?;
        let CapabilityOutcome::Completed(completed) = capability else {
            return Err(AgentLoopDriverError::Failed {
                reason_kind: "script_capability_did_not_complete".to_string(),
            });
        };
        let prompt_bundle = host
            .build_prompt_bundle(LoopPromptBundleRequest {
                mode: PromptMode::TextOnly,
                context_cursor: None,
                surface_version: Some(surface.version.clone()),
                checkpoint_state_ref: None,
                max_messages: Some(8),
                inline_messages: Vec::new(),
                capability_view: None,
            })
            .await
            .map_err(driver_host_error)?;
        let model_response = host
            .stream_model(LoopModelRequest {
                messages: prompt_bundle.messages,
                surface_version: Some(surface.version),
                model_preference: None,
                capability_view: None,
            })
            .await
            .map_err(driver_host_error)?;
        let ParentLoopOutput::AssistantReply(reply) = model_response.output else {
            return Err(AgentLoopDriverError::Failed {
                reason_kind: "unexpected_model_output".to_string(),
            });
        };
        let reply_ref = host
            .finalize_assistant_message(FinalizeAssistantMessage { reply })
            .await
            .map_err(driver_host_error)?;

        let _result_ref = completed.result_ref;
        Ok(LoopExit::Completed(LoopCompleted {
            completion_kind: LoopCompletionKind::FinalReply,
            reply_message_refs: vec![reply_ref],
            result_refs: vec![],
            final_checkpoint_id: None,
            usage_summary_ref: None,
            exit_id: LoopExitId::new("exit:turn-runner-script-capability-e2e").unwrap(),
        }))
    }

    async fn resume(
        &self,
        request: AgentLoopDriverResumeRequest,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
    ) -> Result<LoopExit, AgentLoopDriverError> {
        self.run(
            AgentLoopDriverRunRequest {
                turn_id: request.turn_id,
                run_id: request.run_id,
                resolved_run_profile: request.resolved_run_profile,
            },
            host,
        )
        .await
    }
}

struct ApprovalBlockThenFinalReplyDriver {
    descriptor: AgentLoopDriverDescriptor,
}

#[async_trait]
impl AgentLoopDriver for ApprovalBlockThenFinalReplyDriver {
    fn descriptor(&self) -> AgentLoopDriverDescriptor {
        self.descriptor.clone()
    }

    async fn run(
        &self,
        _request: AgentLoopDriverRunRequest,
        _host: &(dyn AgentLoopDriverHost + Send + Sync),
    ) -> Result<LoopExit, AgentLoopDriverError> {
        Ok(LoopExit::Blocked(LoopBlocked {
            kind: LoopBlockedKind::Approval,
            gate_ref: LoopGateRef::new("gate:approval-resume-e2e").unwrap(),
            checkpoint_id: ironclaw_turns::TurnCheckpointId::new(),
            state_ref: LoopCheckpointStateRef::new("checkpoint:approval-resume-state").unwrap(),
            exit_id: LoopExitId::new("exit:approval-resume-blocked").unwrap(),
        }))
    }

    async fn resume(
        &self,
        request: AgentLoopDriverResumeRequest,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
    ) -> Result<LoopExit, AgentLoopDriverError> {
        TextOnlyFinalReplyDriver {
            descriptor: self.descriptor.clone(),
        }
        .run(
            AgentLoopDriverRunRequest {
                turn_id: request.turn_id,
                run_id: request.run_id,
                resolved_run_profile: request.resolved_run_profile,
            },
            host,
        )
        .await
    }
}

struct TextOnlyFinalReplyDriver {
    descriptor: AgentLoopDriverDescriptor,
}

#[async_trait]
impl AgentLoopDriver for TextOnlyFinalReplyDriver {
    fn descriptor(&self) -> AgentLoopDriverDescriptor {
        self.descriptor.clone()
    }

    async fn run(
        &self,
        _request: AgentLoopDriverRunRequest,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
    ) -> Result<LoopExit, AgentLoopDriverError> {
        let surface = host
            .visible_capabilities(VisibleCapabilityRequest)
            .await
            .map_err(driver_host_error)?;
        let prompt_bundle = host
            .build_prompt_bundle(LoopPromptBundleRequest {
                mode: PromptMode::TextOnly,
                context_cursor: None,
                surface_version: Some(surface.version.clone()),
                checkpoint_state_ref: None,
                max_messages: Some(8),
                inline_messages: Vec::new(),
                capability_view: None,
            })
            .await
            .map_err(driver_host_error)?;
        let model_response = host
            .stream_model(LoopModelRequest {
                messages: prompt_bundle.messages,
                surface_version: Some(surface.version),
                model_preference: None,
                capability_view: None,
            })
            .await
            .map_err(driver_host_error)?;
        let ParentLoopOutput::AssistantReply(reply) = model_response.output else {
            return Err(AgentLoopDriverError::Failed {
                reason_kind: "unexpected_model_output".to_string(),
            });
        };
        let reply_ref = host
            .finalize_assistant_message(FinalizeAssistantMessage { reply })
            .await
            .map_err(driver_host_error)?;

        Ok(LoopExit::Completed(LoopCompleted {
            completion_kind: LoopCompletionKind::FinalReply,
            reply_message_refs: vec![reply_ref],
            result_refs: vec![],
            final_checkpoint_id: None,
            usage_summary_ref: None,
            exit_id: LoopExitId::new("exit:turn-runner-e2e").unwrap(),
        }))
    }

    async fn resume(
        &self,
        request: AgentLoopDriverResumeRequest,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
    ) -> Result<LoopExit, AgentLoopDriverError> {
        self.run(
            AgentLoopDriverRunRequest {
                turn_id: request.turn_id,
                run_id: request.run_id,
                resolved_run_profile: request.resolved_run_profile,
            },
            host,
        )
        .await
    }
}

fn driver_host_error(
    error: ironclaw_turns::run_profile::AgentLoopHostError,
) -> AgentLoopDriverError {
    AgentLoopDriverError::Failed {
        reason_kind: format!("{:?}", error.kind),
    }
}

fn loop_exit_applier_for_fixture(
    fixture: &HostFixture,
    turn_store: Arc<InMemoryTurnStateStore>,
) -> Arc<LoopExitApplier> {
    let loop_checkpoint_store: Arc<dyn LoopCheckpointStore> = turn_store.clone();
    let evidence = Arc::new(ThreadCheckpointLoopExitEvidencePort::new(
        Arc::clone(&fixture.thread_service),
        turn_store.clone(),
        loop_checkpoint_store,
    ));
    Arc::new(LoopExitApplier::new(turn_store, evidence))
}

struct StaticTurnStateStore {
    state: Mutex<TurnRunState>,
}

impl StaticTurnStateStore {
    fn new(state: TurnRunState) -> Self {
        Self {
            state: Mutex::new(state),
        }
    }
}

#[async_trait]
impl TurnStateStore for StaticTurnStateStore {
    async fn submit_turn(
        &self,
        _request: SubmitTurnRequest,
        _admission_policy: &dyn TurnAdmissionPolicy,
        _run_profile_resolver: &dyn RunProfileResolver,
    ) -> Result<SubmitTurnResponse, TurnError> {
        panic!("submit_turn should not be called by static test turn state store")
    }

    async fn resume_turn(
        &self,
        _request: ResumeTurnRequest,
    ) -> Result<ironclaw_turns::ResumeTurnResponse, TurnError> {
        panic!("resume_turn should not be called by static test turn state store")
    }

    async fn request_cancel(
        &self,
        _request: CancelRunRequest,
    ) -> Result<CancelRunResponse, TurnError> {
        panic!("request_cancel should not be called by static test turn state store")
    }

    async fn get_run_state(&self, _request: GetRunStateRequest) -> Result<TurnRunState, TurnError> {
        Ok(self.state.lock().unwrap().clone())
    }
}

async fn queue_fixture_turn(
    fixture: &HostFixture,
    turn_store: &InMemoryTurnStateStore,
    resolver: &dyn RunProfileResolver,
    idempotency_key: &str,
) -> TurnRunId {
    let submit = turn_store
        .submit_turn(
            SubmitTurnRequest {
                scope: fixture.context.scope.clone(),
                actor: TurnActor::new(UserId::new("user-text-host").unwrap()),
                accepted_message_ref: AcceptedMessageRef::new(format!(
                    "accepted-{idempotency_key}"
                ))
                .unwrap(),
                source_binding_ref: SourceBindingRef::new("source-web").unwrap(),
                reply_target_binding_ref: ReplyTargetBindingRef::new("reply-web").unwrap(),
                requested_run_profile: None,
                idempotency_key: IdempotencyKey::new(idempotency_key).unwrap(),
                received_at: Utc::now(),
            },
            &ironclaw_turns::AllowAllTurnAdmissionPolicy,
            resolver,
        )
        .await
        .unwrap();
    let SubmitTurnResponse::Accepted {
        turn_id,
        run_id,
        status,
        ..
    } = submit;
    assert_eq!(status, TurnStatus::Queued);

    fixture
        .thread_service
        .mark_message_submitted(
            &fixture.thread_scope,
            &fixture.thread_id,
            fixture.accepted_message_id,
            turn_id.to_string(),
            run_id.to_string(),
        )
        .await
        .unwrap();
    run_id
}

struct HostFixture {
    thread_service: Arc<InMemorySessionThreadService>,
    checkpoint_state_store: Arc<InMemoryCheckpointStateStore>,
    turn_state_store: Arc<StaticTurnStateStore>,
    loop_checkpoint_store: Arc<InMemoryLoopCheckpointStore>,
    gateway: Arc<RecordingGateway>,
    milestone_sink: Arc<InMemoryLoopHostMilestoneSink>,
    thread_scope: ThreadScope,
    thread_id: ThreadId,
    accepted_message_id: ThreadMessageId,
    claimed: ClaimedTurnRun,
    context: LoopRunContext,
}

impl HostFixture {
    async fn new(thread_name: &str, user_content: &str) -> Self {
        Self::new_with_submission_state(thread_name, user_content, true).await
    }

    async fn new_unsubmitted(thread_name: &str, user_content: &str) -> Self {
        Self::new_with_submission_state(thread_name, user_content, false).await
    }

    async fn new_with_submission_state(
        thread_name: &str,
        user_content: &str,
        mark_submitted: bool,
    ) -> Self {
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
            owner_user_id: None,
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
            actor: Some(TurnActor::new(user_id.clone())),
            turn_id,
            run_id,
            status: TurnStatus::Running,
            accepted_message_ref: AcceptedMessageRef::new(format!("accepted-{thread_name}"))
                .unwrap(),
            source_binding_ref: SourceBindingRef::new("source-web").unwrap(),
            reply_target_binding_ref: ReplyTargetBindingRef::new("reply-web").unwrap(),
            resolved_run_profile_id: RunProfileId::default_profile(),
            resolved_run_profile_version: RunProfileVersion::new(1),
            resolved_model_route: None,
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
        let turn_state_store = Arc::new(StaticTurnStateStore::new(claimed.state.clone()));
        let context = LoopRunContext::new(turn_scope, turn_id, run_id, resolved);
        if mark_submitted {
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
        }

        Self {
            thread_service,
            checkpoint_state_store,
            turn_state_store,
            loop_checkpoint_store,
            gateway,
            milestone_sink,
            thread_scope,
            thread_id,
            accepted_message_id: accepted.message_id,
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
        self.factory_with_config_and_loop_checkpoint_store(
            TextOnlyLoopHostConfig {
                max_messages: 8,
                require_model_route_snapshot: false,
            },
            loop_checkpoint_store,
        )
    }

    fn factory_with_config(
        &self,
        config: TextOnlyLoopHostConfig,
    ) -> RebornLoopDriverHostFactory<InMemorySessionThreadService, RecordingGateway> {
        self.factory_with_config_and_loop_checkpoint_store(
            config,
            self.loop_checkpoint_store.clone(),
        )
    }

    fn factory_with_config_and_loop_checkpoint_store(
        &self,
        config: TextOnlyLoopHostConfig,
        loop_checkpoint_store: Arc<dyn LoopCheckpointStore>,
    ) -> RebornLoopDriverHostFactory<InMemorySessionThreadService, RecordingGateway> {
        RebornLoopDriverHostFactory::new(
            Arc::clone(&self.thread_service),
            self.thread_scope.clone(),
            Arc::clone(&self.gateway),
            self.checkpoint_state_store.clone(),
            self.turn_state_store.clone(),
            loop_checkpoint_store,
            self.milestone_sink.clone(),
            config,
        )
    }

    async fn build_host(&self) -> RebornLoopDriverHost {
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
        owner_user_id: None,
        mission_id: None,
    }
}

fn assign_driver_to_fixture(fixture: &mut HostFixture, descriptor: AgentLoopDriverDescriptor) {
    fixture.context.resolved_run_profile.loop_driver = descriptor.clone();
    fixture.context.loop_driver_id = descriptor.id.clone();
    fixture.context.loop_driver_version = descriptor.version;
    fixture.claimed.resolved_run_profile = fixture.context.resolved_run_profile.clone();
}

fn driver_request(context: &LoopRunContext) -> AgentLoopDriverRunRequest {
    AgentLoopDriverRunRequest {
        turn_id: context.turn_id,
        run_id: context.run_id,
        resolved_run_profile: context.resolved_run_profile.clone(),
    }
}

async fn assert_no_assistant_message(fixture: &HostFixture) {
    let history = fixture
        .thread_service
        .list_thread_history(ThreadHistoryRequest {
            scope: fixture.thread_scope.clone(),
            thread_id: fixture.thread_id.clone(),
        })
        .await
        .unwrap();
    assert!(
        !history
            .messages
            .iter()
            .any(|message| message.kind == MessageKind::Assistant)
    );
}

fn assert_driver_error_hides_raw_payloads(error: &AgentLoopDriverError) {
    assert_serialized_or_debug_hides_raw_payloads(&format!("{error:?}"));
}

fn assert_driver_public_outputs_hide_raw_payloads<T: serde::Serialize>(value: &T) {
    let wire = serde_json::to_string(value).unwrap();
    assert_serialized_or_debug_hides_raw_payloads(&wire);
}

fn assert_serialized_or_debug_hides_raw_payloads(wire: &str) {
    for forbidden in [
        "RAW_CHECKPOINT_PAYLOAD",
        "RAW_PROMPT_TEXT_SENTINEL",
        "RAW_PROVIDER_ERROR",
        "invalid api key",
        "sk-secret",
        "sk-prompt-secret",
        "sk-provider-secret",
        "/host/path",
        "tool_input",
        "model says hi",
    ] {
        assert!(
            !wire.contains(forbidden),
            "public output leaked {forbidden}"
        );
    }
}

fn assert_public_milestones_hide_raw_payloads(milestones: &[LoopHostMilestone]) {
    // Milestones are public progress metadata: they may carry durable refs and
    // safe summaries, never raw model text, checkpoint bytes, tool input,
    // secrets, or host paths. Drivers must rehydrate content through scoped
    // stores instead of learning it from milestone JSON.
    let wire = serde_json::to_string(milestones).unwrap();
    assert_serialized_or_debug_hides_raw_payloads(&wire);
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
    response: Mutex<Result<HostManagedModelResponse, HostManagedModelError>>,
}

impl RecordingGateway {
    fn reply(content: impl Into<String>) -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
            response: Mutex::new(Ok(HostManagedModelResponse::assistant_reply(content))),
        }
    }

    fn set_response(&self, response: Result<HostManagedModelResponse, HostManagedModelError>) {
        *self.response.lock().unwrap() = response;
    }

    fn fail_with_model_error(
        &self,
        kind: HostManagedModelErrorKind,
        raw_detail: impl Into<String>,
    ) {
        *self.response.lock().unwrap() = Err(HostManagedModelError::new(kind, raw_detail));
    }

    fn respond_with_capability_calls(&self) {
        *self.response.lock().unwrap() = Ok(HostManagedModelResponse {
            safe_text_deltas: Vec::new(),
            output: ParentLoopOutput::CapabilityCalls(vec![
                ironclaw_turns::run_profile::CapabilityCallCandidate {
                    surface_version: CapabilitySurfaceVersion::new("empty:v1").unwrap(),
                    capability_id: CapabilityId::new("demo.echo").unwrap(),
                    input_ref: CapabilityInputRef::new("input:opaque-tool-call").unwrap(),
                    provider_replay: None,
                },
            ]),
        });
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
        self.response.lock().unwrap().clone()
    }
}

#[derive(Default)]
struct RecordingBudgetAccountant {
    pre_calls: Mutex<usize>,
    post_calls: Mutex<Vec<bool>>,
}

impl RecordingBudgetAccountant {
    fn was_pre_called(&self) -> bool {
        *self.pre_calls.lock().unwrap() > 0
    }

    fn was_post_called(&self) -> bool {
        !self.post_calls.lock().unwrap().is_empty()
    }

    fn post_saw_failure(&self) -> bool {
        self.post_calls.lock().unwrap().iter().any(|failed| *failed)
    }
}

#[async_trait]
impl LoopModelBudgetAccountant for RecordingBudgetAccountant {
    async fn pre_model_call(
        &self,
        _context: &LoopRunContext,
        _request: &LoopModelRequest,
    ) -> Result<(), LoopModelGatewayError> {
        *self.pre_calls.lock().unwrap() += 1;
        Ok(())
    }

    async fn post_model_call(
        &self,
        _context: &LoopRunContext,
        _request: &LoopModelRequest,
        outcome: ModelCallOutcome<'_>,
    ) -> Result<(), LoopModelGatewayError> {
        self.post_calls
            .lock()
            .unwrap()
            .push(matches!(outcome, ModelCallOutcome::Failure(_)));
        Ok(())
    }
}
