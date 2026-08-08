use std::collections::HashMap;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

use async_trait::async_trait;
use ironclaw_host_api::{
    decision::DenyReason,
    dispatch::DispatchInputIssueCode,
    ids::{
        AgentId, CapabilityId, MissionId, ProjectId, ProviderToolName, RunId, TenantId, ThreadId,
        UserId,
    },
    path::ScopedPath,
    resolution::{Resolution, ResolutionBatch},
    resource::ResourceScope,
};
use ironclaw_loop_contracts::{
    AgentLoopHostError, AgentLoopHostErrorKind, AgentLoopHostErrorReasonKind,
    AppendCapabilityResultRef, AssistantReply, BeginAssistantDraft, CapabilityDeniedReasonKind,
    CapabilityInputIssue, CapabilityInputRef, CapabilitySurfaceVersion,
    EphemeralInstructionMaterializationStore, FinalizeAssistantMessage,
    InMemoryLoopHostMilestoneSink, InMemoryRunProfileResolver, LoopCapabilityPort,
    LoopContextBundle, LoopContextCompactionKind, LoopContextMessage, LoopContextPort,
    LoopContextRequest, LoopContextSnippet, LoopDriverNoteKind, LoopHostMilestoneKind,
    LoopHostMilestoneSink, LoopInputCursor, LoopInputCursorToken, LoopModelCapabilityView,
    LoopModelMessage, LoopModelPort, LoopModelRequest, LoopModelRouteSnapshot, LoopModelUsage,
    LoopPromptBundle, LoopPromptBundleAuthority, LoopPromptBundleRef, LoopPromptBundleRequest,
    LoopPromptPort, LoopRequest, LoopRequestBatch, LoopRunContext, LoopTranscriptPort,
    ModelProfileId, ModelVisibleToolObservation, ObservationTrust, ParentLoopOutput,
    PersonalContextPolicy, PromptMode, PromptSkillContextMetadata, ProviderToolCallReference,
    ProviderToolCallReplay, ProviderToolDefinition, RunProfileResolutionRequest,
    RunProfileResolver, SkillName, SkillTrustLevel, SkillVisibility, ToolObservationDetail,
    ToolObservationStatus, UpdateAssistantDraft, VisibleCapabilityRequest,
    VisibleCapabilitySurface, resolution,
};
use ironclaw_loop_host::{
    EmptyLoopCapabilityPort, HostIdentityContextBuildError, HostIdentityContextCandidate,
    HostIdentityContextSource, HostIdentityMessageContent, HostManagedModelCallDiagnostic,
    HostManagedModelCallDiagnosticCapture, HostManagedModelCallDiagnosticOutcome,
    HostManagedModelError, HostManagedModelErrorKind, HostManagedModelGateway,
    HostManagedModelMessageRole, HostManagedModelRequest, HostManagedModelResponse,
    HostManagedPromptDiagnosticCapture, HostManagedPromptDiagnosticSink,
    HostManagedToolResultContent, HostSkillContextBuildError, HostSkillContextCandidate,
    HostSkillContextSource, IdentityApplicability, IdentityBudget, IdentityFileName,
    LoopAttachmentReadError, LoopAttachmentReadPort, PromptContextTokenBudget, ProviderModelId,
    SkillBundleContextSource, SkillBundleDescriptor, SkillBundleId, SkillBundleSource,
    SkillBundleSourceError, SkillFilePath, SkillSourceKind, ThreadBackedLoopContextPort,
    ThreadBackedLoopModelPort, ThreadBackedLoopTranscriptPort, ThreadContextWindowCache,
    build_skill_run_snapshot, identity_message_ref,
};
use ironclaw_outbound::{
    OutboundError, OutboundStateStore, ReplyAttachmentHandle, ReplyAttachmentIntent,
    ReplyAttachmentIntentPort,
};
use ironclaw_skills::SkillTrust;
use ironclaw_threads::{
    AcceptInboundMessageRequest, AcceptedInboundMessage, AcceptedInboundMessageReplay,
    AppendAssistantDraftRequest, AppendCapabilityDisplayPreviewRequest,
    AppendFinalizedAssistantMessageRequest, AppendToolResultReferenceRequest, AttachmentKind,
    AttachmentRef, ContextMessage, ContextMessages, ContextWindow, CreateSummaryArtifactRequest,
    EnsureThreadRequest, InMemorySessionThreadService, LoadContextMessagesRequest, MessageContent,
    MessageKind, MessageStatus, ProviderToolCallReferenceEnvelope, RedactMessageRequest,
    ReplayAcceptedInboundMessageRequest, SessionThreadError, SessionThreadRecord,
    SessionThreadService, SummaryArtifact, SummaryModelContextPolicy, ThreadHistory,
    ThreadHistoryRequest, ThreadMessageId, ThreadMessageRecord, ThreadScope,
    ToolResultReferenceEnvelope, ToolResultSafeSummary, UpdateAssistantDraftRequest,
    UpdateToolResultReferenceRequest,
};
use ironclaw_turns::HostManagedLoopPromptPort;
use ironclaw_turns::{LoopMessageRef, LoopResultRef, TurnActor, TurnId, TurnRunId, TurnScope};
use tracing_test::traced_test;

#[tokio::test]
async fn thread_context_port_loads_policy_filtered_transcript_messages() {
    let fixture = ThreadFixture::new().await;
    let adapter = ThreadBackedLoopContextPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
        16,
    );

    let bundle = adapter
        .load_loop_context(LoopContextRequest {
            after: None,
            limit: 16,
            mode: ironclaw_loop_contracts::PromptMode::TextOnly,
        })
        .await
        .unwrap();

    assert_eq!(bundle.messages.len(), 1);
    assert_eq!(bundle.messages[0].role, "user");
    assert_eq!(bundle.messages[0].safe_summary, "user message available");
    assert!(!bundle.messages[0].safe_summary.contains("hello reborn"));
    assert_eq!(
        bundle.messages[0]
            .message_ref
            .as_ref()
            .expect("message_ref")
            .as_str(),
        format!("msg:{}", fixture.user_message_id).as_str()
    );
    let compaction = bundle.messages[0]
        .compaction
        .as_ref()
        .expect("model-visible transcript message should carry compaction metadata");
    assert_eq!(compaction.sequence, 1);
    assert_eq!(compaction.kind, LoopContextCompactionKind::User);
    assert!(compaction.estimated_tokens > 0);
    assert!(bundle.memory_snippets.is_empty());
}

#[tokio::test]
async fn thread_context_port_applies_prompt_token_budget_to_scanned_messages() {
    let fixture = ThreadFixture::new_with_user_content("old short").await;
    fixture
        .accept_user_message("event-2", &"large ".repeat(32))
        .await;
    fixture.accept_user_message("event-3", "latest short").await;
    let adapter = ThreadBackedLoopContextPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
        16,
    )
    .with_prompt_context_token_budget(PromptContextTokenBudget::new(6, 0, 0));

    let bundle = adapter
        .load_loop_context(LoopContextRequest {
            after: None,
            limit: 16,
            mode: ironclaw_loop_contracts::PromptMode::TextOnly,
        })
        .await
        .unwrap();

    assert_eq!(bundle.messages.len(), 1);
    let compaction = bundle.messages[0]
        .compaction
        .as_ref()
        .expect("budget-admitted message should retain compaction metadata");
    assert_eq!(compaction.sequence, 3);
    assert_eq!(
        bundle
            .compaction_message_index
            .iter()
            .map(|entry| entry.sequence)
            .collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
}

#[tokio::test]
async fn prompt_port_default_scan_reaches_past_old_sixteen_message_tail() {
    let fixture = ThreadFixture::new_with_user_content("message 1").await;
    for sequence in 2..=17 {
        fixture
            .accept_user_message(&format!("event-{sequence}"), &format!("message {sequence}"))
            .await;
    }
    let context_port = Arc::new(ThreadBackedLoopContextPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
        128,
    ));
    let prompt_port = HostManagedLoopPromptPort::new(
        fixture.run_context.clone(),
        context_port,
        Arc::new(InMemoryLoopHostMilestoneSink::default()),
    );

    let prompt_bundle = prompt_port
        .build_prompt_bundle(ironclaw_loop_contracts::LoopPromptBundleRequest {
            mode: ironclaw_loop_contracts::PromptMode::TextOnly,
            context_cursor: None,
            surface_version: None,
            checkpoint_state_ref: None,
            max_messages: Some(128),
            inline_messages: Vec::new(),
            capability_view: None,
        })
        .await
        .unwrap();

    assert_eq!(prompt_bundle.compaction_message_index.len(), 17);
    assert_eq!(prompt_bundle.compaction_message_index[0].sequence, 1);
    assert_eq!(prompt_bundle.compaction_message_index[16].sequence, 17);
}

#[tokio::test]
async fn model_port_empty_request_applies_prompt_token_budget_to_context_fallback() {
    let fixture = ThreadFixture::new_with_user_content("old short").await;
    fixture
        .accept_user_message("event-2", &"large ".repeat(32))
        .await;
    fixture.accept_user_message("event-3", "latest short").await;
    let gateway = Arc::new(RecordingGateway::reply("model says hi"));
    let port = ThreadBackedLoopModelPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
        gateway.clone(),
        16,
    )
    .with_prompt_context_token_budget(PromptContextTokenBudget::new(6, 0, 0));
    issue_prompt_grant(&fixture.run_context, &[]);

    port.stream_model(LoopModelRequest {
        inline_messages: Vec::new(),
        messages: Vec::new(),
        surface_version: None,
        model_preference: None,
        fallback_index: 0,
        iteration: 0,
        capability_view: None,
    })
    .await
    .unwrap();

    let calls = gateway.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].messages.len(), 1);
    assert_eq!(calls[0].messages[0].content, "latest short");
}

#[tokio::test]
async fn model_port_records_resolved_prompt_with_fallback_model_at_the_host_boundary() {
    let fixture = ThreadFixture::new_with_user_content("diagnostic prompt body").await;
    let gateway = Arc::new(RecordingGateway::reply_with_usage_and_fallback(
        "model says hi",
        LoopModelUsage {
            input_tokens: 21,
            output_tokens: 8,
            cache_read_input_tokens: 5,
            cache_creation_input_tokens: 3,
        },
        2,
    ));
    let sink = Arc::new(RecordingPromptDiagnosticSink::default());
    let messages = user_model_messages(&fixture);
    let bundle = LoopPromptBundle {
        bundle_ref: LoopPromptBundleRef::for_run(&fixture.run_context, "diagnostic-bundle")
            .expect("bundle"),
        messages: messages.clone(),
        surface_version: None,
        compaction_message_index: Vec::new(),
        instruction_fingerprint: None,
        identity_message_count: 0,
        instruction_snippet_count: 2,
    };
    LoopPromptBundleAuthority::shared()
        .issue_bundle_with_diagnostic_metadata(
            &fixture.run_context,
            &bundle,
            Some(ironclaw_loop_contracts::LoopPromptDiagnosticMetadata {
                identity_message_count: 0,
                instruction_snippet_count: 2,
                active_skills: vec![SkillName::new("workspace-search").expect("skill name")],
            }),
        )
        .expect("prompt grant");

    ThreadBackedLoopModelPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
        gateway,
        16,
    )
    .with_prompt_context_token_budget(PromptContextTokenBudget::new(64_000, 4_000, 0))
    .with_prompt_diagnostic_sink(sink.clone())
    .stream_model(LoopModelRequest {
        inline_messages: Vec::new(),
        messages,
        surface_version: None,
        model_preference: None,
        fallback_index: 2,
        iteration: 7,
        capability_view: Some(LoopModelCapabilityView {
            visible_capability_ids: vec![CapabilityId::new("filesystem.read").expect("capability")],
        }),
    })
    .await
    .expect("model response");

    let captures = sink.captures.lock().expect("captures");
    assert_eq!(captures.len(), 1);
    assert_eq!(captures[0].messages[0].content, "diagnostic prompt body");
    assert_eq!(captures[0].instruction_snippet_count, 2);
    assert_eq!(captures[0].active_skills[0].as_str(), "workspace-search");
    assert_eq!(captures[0].capability_ids[0].as_str(), "filesystem.read");
    assert_eq!(
        captures[0]
            .effective_model
            .as_ref()
            .map(ProviderModelId::as_str),
        Some("fallback-provider-model")
    );
    assert_eq!(captures[0].context_limit, 64_000);
    drop(captures);
    let model_calls = sink.model_calls.lock().expect("model calls");
    assert_eq!(model_calls.len(), 2);
    let started = model_call_diagnostic(&model_calls[0]);
    let completed = model_call_diagnostic(&model_calls[1]);
    assert!(matches!(
        model_calls[0],
        HostManagedModelCallDiagnosticCapture::Started(_)
    ));
    assert_eq!(started.call_id, completed.call_id);
    assert_eq!(completed.iteration, 7);
    assert_eq!(completed.requested_model, "interactive_model");
    assert_eq!(
        completed.effective_model.as_deref(),
        Some("provider-model-from-response")
    );
    let Some(HostManagedModelCallDiagnosticOutcome::Succeeded { usage }) =
        model_call_outcome(&model_calls[1])
    else {
        panic!("completed model call should succeed");
    };
    assert_eq!(usage.as_ref().map(|usage| usage.input_tokens), Some(21));
}

#[tokio::test]
async fn model_port_keeps_effective_model_unavailable_without_provider_evidence() {
    let fixture = ThreadFixture::new_with_user_content("diagnostic prompt body").await;
    let gateway = Arc::new(MissingDiagnosticModelGateway);
    let sink = Arc::new(RecordingPromptDiagnosticSink::default());
    let messages = user_model_messages(&fixture);
    issue_prompt_grant(&fixture.run_context, &messages);

    ThreadBackedLoopModelPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
        gateway,
        16,
    )
    .with_prompt_diagnostic_sink(sink.clone())
    .stream_model(LoopModelRequest {
        inline_messages: Vec::new(),
        messages,
        surface_version: None,
        model_preference: None,
        fallback_index: 2,
        iteration: 7,
        capability_view: None,
    })
    .await
    .expect("model response");

    let model_calls = sink.model_calls.lock().expect("model calls");
    assert_eq!(model_calls.len(), 2);
    let started = model_call_diagnostic(&model_calls[0]);
    let completed = model_call_diagnostic(&model_calls[1]);
    assert_eq!(started.call_id, completed.call_id);
    assert_eq!(completed.requested_model, "interactive_model");
    assert_eq!(started.effective_model, None);
    assert_eq!(completed.effective_model, None);
}

#[tokio::test]
async fn model_port_retains_usage_reported_by_failed_calls() {
    let fixture = ThreadFixture::new_with_user_content("failed diagnostic prompt").await;
    let usage = LoopModelUsage {
        input_tokens: 34,
        output_tokens: 2,
        cache_read_input_tokens: 8,
        cache_creation_input_tokens: 1,
    };
    let gateway = Arc::new(RecordingGateway::model_error_with_usage(
        HostManagedModelErrorKind::ProviderUnavailable,
        "model provider unavailable",
        usage,
    ));
    let sink = Arc::new(RecordingPromptDiagnosticSink::default());
    let messages = user_model_messages(&fixture);
    issue_prompt_grant(&fixture.run_context, &messages);

    let error = ThreadBackedLoopModelPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
        gateway,
        16,
    )
    .with_prompt_diagnostic_sink(sink.clone())
    .stream_model(LoopModelRequest {
        inline_messages: Vec::new(),
        messages,
        surface_version: None,
        model_preference: None,
        fallback_index: 0,
        iteration: 0,
        capability_view: None,
    })
    .await
    .expect_err("model call fails");
    assert_eq!(error.kind, AgentLoopHostErrorKind::Unavailable);

    let model_calls = sink.model_calls.lock().expect("model calls");
    assert_eq!(model_calls.len(), 2);
    let started = model_call_diagnostic(&model_calls[0]);
    let completed = model_call_diagnostic(&model_calls[1]);
    assert_eq!(started.call_id, completed.call_id);
    let Some(HostManagedModelCallDiagnosticOutcome::Failed {
        usage: completed_usage,
        failure_summary,
    }) = model_call_outcome(&model_calls[1])
    else {
        panic!("completed model call should fail");
    };
    assert_eq!(*completed_usage, Some(usage));
    assert_eq!(
        completed.effective_model.as_deref(),
        Some("provider-model-from-error")
    );
    assert_eq!(failure_summary, "model provider unavailable");
}

#[tokio::test]
async fn model_port_keeps_omitted_usage_unavailable() {
    let fixture = ThreadFixture::new_with_user_content("usage unavailable prompt").await;
    let gateway = Arc::new(RecordingGateway::reply("model says hi"));
    let sink = Arc::new(RecordingPromptDiagnosticSink::default());
    let messages = user_model_messages(&fixture);
    issue_prompt_grant(&fixture.run_context, &messages);

    ThreadBackedLoopModelPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
        gateway,
        16,
    )
    .with_prompt_diagnostic_sink(sink.clone())
    .stream_model(LoopModelRequest {
        inline_messages: Vec::new(),
        messages,
        surface_version: None,
        model_preference: None,
        fallback_index: 0,
        iteration: 0,
        capability_view: None,
    })
    .await
    .expect("model call succeeds");

    let model_calls = sink.model_calls.lock().expect("model calls");
    assert_eq!(model_calls.len(), 2);
    assert_eq!(
        model_call_diagnostic(&model_calls[0]).call_id,
        model_call_diagnostic(&model_calls[1]).call_id
    );
    let Some(HostManagedModelCallDiagnosticOutcome::Succeeded { usage }) =
        model_call_outcome(&model_calls[1])
    else {
        panic!("completed model call should succeed");
    };
    assert_eq!(*usage, None);
}

#[tokio::test]
async fn model_port_records_full_capability_surface_when_request_has_no_view() {
    let fixture = ThreadFixture::new().await;
    let messages = user_model_messages(&fixture);
    issue_prompt_grant(&fixture.run_context, &messages);
    let capability_id = CapabilityId::new("demo.full_surface").expect("capability");
    let gateway = Arc::new(RecordingGateway::reply("model says hi"));
    let sink = Arc::new(RecordingPromptDiagnosticSink::default());

    ThreadBackedLoopModelPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
        gateway.clone(),
        16,
    )
    .with_capability_port(Arc::new(StaticToolDefinitionPort::new(vec![
        provider_tool_definition(capability_id.clone(), "demo__full_surface"),
    ])))
    .with_prompt_diagnostic_sink(sink.clone())
    .stream_model(LoopModelRequest {
        inline_messages: Vec::new(),
        messages,
        surface_version: None,
        model_preference: None,
        fallback_index: 0,
        iteration: 0,
        capability_view: None,
    })
    .await
    .expect("model response");

    let captures = sink.captures.lock().expect("captures");
    assert_eq!(captures.len(), 1);
    assert_eq!(captures[0].capability_ids, vec![capability_id]);
    assert_eq!(
        gateway.tool_definition_calls()[0][0].name.as_str(),
        "demo__full_surface"
    );
}

#[traced_test]
#[tokio::test]
async fn model_port_continues_when_diagnostic_capability_lookup_fails() {
    let fixture = ThreadFixture::new().await;
    let messages = user_model_messages(&fixture);
    issue_prompt_grant(&fixture.run_context, &messages);
    let capability_id = CapabilityId::new("demo.transient_surface").expect("capability");
    let capabilities = Arc::new(StaticToolDefinitionPort::failing_first_lookup(vec![
        provider_tool_definition(capability_id, "demo__transient_surface"),
    ]));
    let gateway = Arc::new(RecordingGateway::reply("model says hi"));
    let sink = Arc::new(RecordingPromptDiagnosticSink::default());

    let response = ThreadBackedLoopModelPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
        gateway.clone(),
        16,
    )
    .with_capability_port(capabilities.clone())
    .with_prompt_diagnostic_sink(sink.clone())
    .stream_model(LoopModelRequest {
        inline_messages: Vec::new(),
        messages,
        surface_version: None,
        model_preference: None,
        fallback_index: 0,
        iteration: 0,
        capability_view: None,
    })
    .await
    .expect("diagnostic failure must not fail the model request");

    assert!(matches!(
        response.output,
        ParentLoopOutput::AssistantReply(AssistantReply { ref content })
            if content == "model says hi"
    ));
    assert_eq!(capabilities.tool_definition_calls(), 2);
    assert!(
        sink.captures
            .lock()
            .expect("captures")
            .first()
            .is_some_and(|capture| capture.capability_ids.is_empty())
    );
    assert_eq!(gateway.tool_definition_calls().len(), 1);
    assert!(logs_contain(
        "prompt diagnostics could not capture capability ids"
    ));
}

#[tokio::test]
async fn prompt_and_model_ports_share_cached_context_window_for_one_request() {
    let fixture = GatedThreadFixture::new().await;
    let context_window_cache = Arc::new(ThreadContextWindowCache::default());
    let context_port = Arc::new(
        ThreadBackedLoopContextPort::new(
            Arc::clone(&fixture.thread_service),
            fixture.thread_scope.clone(),
            fixture.run_context.clone(),
            16,
        )
        .with_context_window_cache(Arc::clone(&context_window_cache)),
    );
    let prompt_port = HostManagedLoopPromptPort::new(
        fixture.run_context.clone(),
        context_port,
        Arc::new(InMemoryLoopHostMilestoneSink::default()),
    );
    let prompt_bundle = prompt_port
        .build_prompt_bundle(LoopPromptBundleRequest {
            mode: PromptMode::TextOnly,
            context_cursor: None,
            surface_version: None,
            checkpoint_state_ref: None,
            max_messages: Some(16),
            inline_messages: Vec::new(),
            capability_view: None,
        })
        .await
        .unwrap();
    let gateway = Arc::new(RecordingGateway::reply("model says hi"));
    let model_port = ThreadBackedLoopModelPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
        gateway,
        16,
    )
    .with_context_window_cache(context_window_cache);

    model_port
        .stream_model(LoopModelRequest {
            inline_messages: Vec::new(),
            messages: prompt_bundle.messages,
            surface_version: prompt_bundle.surface_version,
            model_preference: None,
            fallback_index: 0,
            iteration: 0,
            capability_view: None,
        })
        .await
        .unwrap();

    assert_eq!(fixture.thread_service.context_window_loads(), 1);
}

#[tokio::test]
async fn model_port_reuses_smaller_prompt_context_window_for_explicit_prompt_refs() {
    let fixture = GatedThreadFixture::new().await;
    let context_window_cache = Arc::new(ThreadContextWindowCache::default());
    let context_port = Arc::new(
        ThreadBackedLoopContextPort::new(
            Arc::clone(&fixture.thread_service),
            fixture.thread_scope.clone(),
            fixture.run_context.clone(),
            16,
        )
        .with_context_window_cache(Arc::clone(&context_window_cache)),
    );
    let prompt_port = HostManagedLoopPromptPort::new(
        fixture.run_context.clone(),
        context_port,
        Arc::new(InMemoryLoopHostMilestoneSink::default()),
    );
    let prompt_bundle = prompt_port
        .build_prompt_bundle(LoopPromptBundleRequest {
            mode: PromptMode::TextOnly,
            context_cursor: None,
            surface_version: None,
            checkpoint_state_ref: None,
            max_messages: Some(16),
            inline_messages: Vec::new(),
            capability_view: None,
        })
        .await
        .unwrap();
    let gateway = Arc::new(RecordingGateway::reply("model says hi"));
    let model_port = ThreadBackedLoopModelPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
        gateway,
        128,
    )
    .with_context_window_cache(context_window_cache);

    model_port
        .stream_model(LoopModelRequest {
            inline_messages: Vec::new(),
            messages: prompt_bundle.messages,
            surface_version: prompt_bundle.surface_version,
            model_preference: None,
            fallback_index: 0,
            iteration: 0,
            capability_view: None,
        })
        .await
        .unwrap();

    assert_eq!(fixture.thread_service.context_window_loads(), 1);
}

#[tokio::test]
async fn context_window_cache_does_not_cross_thread_scope_boundaries() {
    let fixture = ThreadFixture::new().await;
    let message_id = ThreadMessageId::new();
    let mission_a = MissionId::new("mission-cache-a").unwrap();
    let mission_b = MissionId::new("mission-cache-b").unwrap();
    let scope_a = ThreadScope {
        mission_id: Some(mission_a.clone()),
        ..fixture.thread_scope.clone()
    };
    let scope_b = ThreadScope {
        mission_id: Some(mission_b.clone()),
        ..fixture.thread_scope.clone()
    };
    let scoped_service = Arc::new(StaticContextThreadService::with_scoped_context_messages(
        vec![
            (
                Some(mission_a),
                ContextMessage {
                    message_id: Some(message_id),
                    summary_id: None,
                    sequence: 1,
                    kind: MessageKind::User,
                    tool_result_provider_call: None,
                    content: "mission a transcript".to_string(),
                    image_attachments: Vec::new(),
                },
            ),
            (
                Some(mission_b),
                ContextMessage {
                    message_id: Some(message_id),
                    summary_id: None,
                    sequence: 1,
                    kind: MessageKind::User,
                    tool_result_provider_call: None,
                    content: "mission b transcript".to_string(),
                    image_attachments: Vec::new(),
                },
            ),
        ],
    ));
    let context_window_cache = Arc::new(ThreadContextWindowCache::default());
    let context_port = Arc::new(
        ThreadBackedLoopContextPort::new(
            Arc::clone(&scoped_service),
            scope_a,
            fixture.run_context.clone(),
            16,
        )
        .with_context_window_cache(Arc::clone(&context_window_cache)),
    );
    let prompt_port = HostManagedLoopPromptPort::new(
        fixture.run_context.clone(),
        context_port,
        Arc::new(InMemoryLoopHostMilestoneSink::default()),
    );
    let prompt_bundle = prompt_port
        .build_prompt_bundle(LoopPromptBundleRequest {
            mode: PromptMode::TextOnly,
            context_cursor: None,
            surface_version: None,
            checkpoint_state_ref: None,
            max_messages: Some(16),
            inline_messages: Vec::new(),
            capability_view: None,
        })
        .await
        .unwrap();
    let gateway = Arc::new(RecordingGateway::reply("model says hi"));
    let model_port = ThreadBackedLoopModelPort::new(
        Arc::clone(&scoped_service),
        scope_b,
        fixture.run_context,
        Arc::clone(&gateway),
        16,
    )
    .with_context_window_cache(context_window_cache);

    model_port
        .stream_model(LoopModelRequest {
            inline_messages: Vec::new(),
            messages: prompt_bundle.messages,
            surface_version: prompt_bundle.surface_version,
            model_preference: None,
            fallback_index: 0,
            iteration: 0,
            capability_view: None,
        })
        .await
        .unwrap();

    assert_eq!(scoped_service.context_window_loads(), 2);
    let calls = gateway.calls.lock().unwrap();
    assert_eq!(calls[0].messages[0].content, "mission b transcript");
}

#[tokio::test]
async fn thread_context_port_rejects_run_actor_owner_mismatch() {
    // Defense in depth for the thread-owner MountView divergence: the store
    // keys threads by owner, so reading a thread whose scope owner differs
    // from the run's authenticated actor silently targets the wrong
    // `owners/<user>` subtree. The port must fail loud before that read.
    let fixture = ThreadFixture::new().await;
    let mismatched_run_context = fixture
        .run_context
        .clone()
        .with_actor(TurnActor::new(UserId::new("intruder-user").unwrap()));
    let adapter = ThreadBackedLoopContextPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        mismatched_run_context,
        16,
    );

    let error = adapter
        .load_loop_context(LoopContextRequest {
            after: None,
            limit: 16,
            mode: ironclaw_loop_contracts::PromptMode::TextOnly,
        })
        .await
        .expect_err("owner mismatch must be rejected before the thread read");

    assert_eq!(error.kind, AgentLoopHostErrorKind::ScopeMismatch);
}

#[tokio::test]
async fn thread_context_port_accepts_explicit_owner_with_distinct_actor() {
    // Shared routes pin the transcript to an explicit subject while preserving
    // the submitting actor for identity/policy decisions.
    let fixture = ThreadFixture::new().await;
    let explicit_scope = TurnScope::new_with_owner(
        fixture.run_context.scope.tenant_id.clone(),
        fixture.run_context.scope.agent_id.clone(),
        fixture.run_context.scope.project_id.clone(),
        fixture.thread_id.clone(),
        fixture.thread_scope.owner_user_id.clone(),
    );
    let run_context = LoopRunContext::new(
        explicit_scope,
        fixture.run_context.turn_id,
        fixture.run_context.run_id,
        fixture.run_context.resolved_run_profile,
    )
    .with_actor(TurnActor::new(UserId::new("room-participant").unwrap()));
    let adapter = ThreadBackedLoopContextPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        run_context,
        16,
    );

    let bundle = adapter
        .load_loop_context(LoopContextRequest {
            after: None,
            limit: 16,
            mode: ironclaw_loop_contracts::PromptMode::TextOnly,
        })
        .await
        .expect("explicit owner should allow actor/owner divergence");

    assert_eq!(bundle.messages.len(), 1);
}

#[tokio::test]
async fn thread_context_port_accepts_matching_run_actor_owner() {
    // The same path must still succeed when the run actor owns the thread.
    let fixture = ThreadFixture::new().await;
    let matched_run_context = fixture
        .run_context
        .clone()
        .with_actor(TurnActor::new(UserId::new("user-loop-support").unwrap()));
    let adapter = ThreadBackedLoopContextPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        matched_run_context,
        16,
    );

    let bundle = adapter
        .load_loop_context(LoopContextRequest {
            after: None,
            limit: 16,
            mode: ironclaw_loop_contracts::PromptMode::TextOnly,
        })
        .await
        .expect("matching run actor owner must load context");

    assert_eq!(bundle.messages.len(), 1);
}

#[tokio::test]
async fn thread_context_port_preserves_summary_replacements_as_system_messages() {
    let fixture = ThreadFixture::new().await;
    fixture
        .thread_service
        .create_summary_artifact(CreateSummaryArtifactRequest {
            scope: fixture.thread_scope.clone(),
            thread_id: fixture.thread_id.clone(),
            start_sequence: 1,
            end_sequence: 1,
            summary_kind: ironclaw_threads::SummaryKind::Compaction,
            content: MessageContent::text("summarized hello"),
            model_context_policy: Some(SummaryModelContextPolicy::ReplaceRangeWhenSelected),
        })
        .await
        .unwrap();
    let adapter = ThreadBackedLoopContextPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
        16,
    );

    let bundle = adapter
        .load_loop_context(LoopContextRequest {
            after: None,
            limit: 16,
            mode: ironclaw_loop_contracts::PromptMode::TextOnly,
        })
        .await
        .unwrap();

    assert_eq!(bundle.messages.len(), 1);
    assert_eq!(bundle.messages[0].role, "system");
    assert_eq!(
        bundle.messages[0].safe_summary,
        "summary artifact available"
    );
    assert!(!bundle.messages[0].safe_summary.contains("summarized hello"));
    assert!(
        bundle.messages[0]
            .message_ref
            .as_ref()
            .expect("message_ref")
            .as_str()
            .starts_with("msg:summary-")
    );
    assert!(bundle.instruction_snippets.is_empty());
}

#[tokio::test]
async fn thread_context_port_builds_skill_instruction_snippets_from_real_skill_md() {
    let fixture = ThreadFixture::new().await;
    let source = Arc::new(StaticSkillContextSource::new(vec![
        HostSkillContextCandidate::loaded(
            skill_md(
                "alpha",
                "safe alpha description",
                "Use alpha prompt content.",
            ),
            Some(SkillTrust::Trusted),
            Some(SkillVisibility::Visible),
        ),
    ]));
    let adapter = ThreadBackedLoopContextPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
        16,
    )
    .with_skill_context_source(source);

    let bundle = adapter
        .load_loop_context(LoopContextRequest {
            after: None,
            limit: 16,
            mode: ironclaw_loop_contracts::PromptMode::TextOnly,
        })
        .await
        .unwrap();

    assert_eq!(bundle.instruction_snippets.len(), 1);
    let snippet = &bundle.instruction_snippets[0];
    assert_eq!(snippet.snippet_ref, "skill:alpha");
    assert!(snippet.safe_summary.contains("safe alpha description"));
    assert!(!snippet.safe_summary.contains("Use alpha prompt content."));
    assert!(snippet.model_content.contains("safe alpha description"));
    assert!(snippet.model_content.contains("Use alpha prompt content."));
    assert!(!snippet.safe_summary.contains("/tmp"));
    assert!(!snippet.model_content.contains("/tmp"));
}

#[tokio::test]
async fn thread_context_port_builds_skill_instruction_snippets_from_skill_bundle_context_source() {
    let fixture = ThreadFixture::new().await;
    let bundle_source = Arc::new(StaticSkillBundleSource::new(vec![
        skill_bundle_descriptor(
            SkillSourceKind::System,
            "alpha",
            Some(SkillTrust::Trusted),
            Some(SkillVisibility::Visible),
        )
        .with_description("safe alpha description"),
        skill_bundle_descriptor(
            SkillSourceKind::User,
            "bravo",
            Some(SkillTrust::Installed),
            Some(SkillVisibility::Visible),
        )
        .with_description("safe bravo description"),
    ]));
    let source = Arc::new(SkillBundleContextSource::new(Arc::clone(&bundle_source)));
    let adapter = ThreadBackedLoopContextPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
        16,
    )
    .with_skill_context_source(source);

    let bundle = adapter
        .load_loop_context(LoopContextRequest {
            after: None,
            limit: 16,
            mode: PromptMode::TextOnly,
        })
        .await
        .unwrap();

    assert_eq!(
        bundle
            .instruction_snippets
            .iter()
            .map(|snippet| snippet.snippet_ref.as_str())
            .collect::<Vec<_>>(),
        vec!["skill:alpha", "skill:bravo"]
    );
    assert!(
        bundle.instruction_snippets[0]
            .safe_summary
            .contains("safe alpha description")
    );
    assert!(
        !bundle.instruction_snippets[0]
            .safe_summary
            .contains("Use trusted alpha prompt content.")
    );
    assert!(
        bundle.instruction_snippets[0]
            .model_content
            .contains("safe alpha description")
    );
    assert!(
        !bundle.instruction_snippets[0]
            .model_content
            .contains("Use trusted alpha prompt content.")
    );
    assert!(
        bundle.instruction_snippets[1]
            .safe_summary
            .contains("safe bravo description")
    );
    assert!(
        !bundle.instruction_snippets[1]
            .safe_summary
            .contains("RAW_INSTALLED_PROMPT_SENTINEL")
    );
    assert!(
        !bundle.instruction_snippets[1]
            .model_content
            .contains("RAW_INSTALLED_PROMPT_SENTINEL")
    );
    assert!(bundle_source.reads().is_empty());
}

#[tokio::test]
async fn thread_context_port_skill_bundle_source_fails_closed_when_visibility_missing() {
    let fixture = ThreadFixture::new().await;
    let bundle_source = Arc::new(StaticSkillBundleSource::new(vec![skill_bundle_descriptor(
        SkillSourceKind::User,
        "alpha",
        Some(SkillTrust::Trusted),
        None,
    )]));
    let source = Arc::new(SkillBundleContextSource::new(Arc::clone(&bundle_source)));
    let adapter = ThreadBackedLoopContextPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
        16,
    )
    .with_skill_context_source(source);

    let error = adapter
        .load_loop_context(LoopContextRequest {
            after: None,
            limit: 16,
            mode: PromptMode::TextOnly,
        })
        .await
        .unwrap_err();

    assert_eq!(error.kind, AgentLoopHostErrorKind::PolicyDenied);
    assert!(bundle_source.reads().is_empty());
}

#[tokio::test]
async fn context_port_populates_identity_when_source_set() {
    let fixture = ThreadFixture::new().await;
    let source = Arc::new(StaticIdentityContextSource::new(vec![trusted_identity(
        "AGENTS.md",
        "agent instructions",
        IdentityApplicability::Always,
    )]));
    let adapter = ThreadBackedLoopContextPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
        16,
    )
    .with_identity_context_source(source);

    let bundle = adapter
        .load_loop_context(LoopContextRequest {
            after: None,
            limit: 16,
            mode: ironclaw_loop_contracts::PromptMode::TextOnly,
        })
        .await
        .unwrap();

    assert_eq!(bundle.identity_messages.len(), 1);
    assert_eq!(
        bundle.identity_messages[0].safe_summary,
        "identity file AGENTS.md available"
    );
    assert!(bundle.identity_messages[0].message_ref.is_some());
}

#[tokio::test]
async fn context_port_applies_identity_budget_to_trusted_content() {
    let fixture = ThreadFixture::new().await;
    let source = Arc::new(StaticIdentityContextSource::new(vec![trusted_identity(
        "AGENTS.md",
        "trusted identity content that exceeds the tiny budget",
        IdentityApplicability::Always,
    )]));
    let adapter = ThreadBackedLoopContextPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
        16,
    )
    .with_identity_context_source(source)
    .with_identity_budget(IdentityBudget::new(4).unwrap());

    let bundle = adapter
        .load_loop_context(LoopContextRequest {
            after: None,
            limit: 16,
            mode: PromptMode::TextOnly,
        })
        .await
        .unwrap();

    assert!(bundle.identity_messages.is_empty());
}

#[tokio::test]
async fn context_port_empty_identity_when_source_unset() {
    let fixture = ThreadFixture::new().await;
    let adapter = ThreadBackedLoopContextPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
        16,
    );

    let bundle = adapter
        .load_loop_context(LoopContextRequest {
            after: None,
            limit: 16,
            mode: ironclaw_loop_contracts::PromptMode::TextOnly,
        })
        .await
        .unwrap();

    assert!(bundle.identity_messages.is_empty());
}

#[tokio::test]
async fn context_port_caches_stable_identity_within_run() {
    let fixture = ThreadFixture::new().await;
    let source = Arc::new(StaticIdentityContextSource::new(vec![trusted_identity(
        "AGENTS.md",
        "agent instructions",
        IdentityApplicability::Always,
    )]));
    let adapter = ThreadBackedLoopContextPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
        16,
    )
    .with_identity_context_source(source.clone());

    let first = adapter
        .load_loop_context(LoopContextRequest {
            after: None,
            limit: 16,
            mode: ironclaw_loop_contracts::PromptMode::TextOnly,
        })
        .await
        .unwrap();
    let second = adapter
        .load_loop_context(LoopContextRequest {
            after: None,
            limit: 16,
            mode: ironclaw_loop_contracts::PromptMode::TextOnly,
        })
        .await
        .unwrap();

    assert_eq!(first.identity_messages, second.identity_messages);
    assert_eq!(source.load_calls(), 1);
}

#[tokio::test]
async fn context_port_caches_identity_candidates_per_prompt_mode() {
    let fixture = ThreadFixture::new().await;
    let source = Arc::new(ModeAwareIdentityContextSource::new(
        Vec::new(),
        vec![
            trusted_identity(
                "TOOLS.md",
                "codeact-only tool identity",
                IdentityApplicability::OnCodeAct,
            )
            .0,
        ],
    ));
    let adapter = ThreadBackedLoopContextPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
        16,
    )
    .with_identity_context_source(source.clone());

    let text_only = adapter
        .load_loop_context(LoopContextRequest {
            after: None,
            limit: 16,
            mode: PromptMode::TextOnly,
        })
        .await
        .unwrap();
    let codeact = adapter
        .load_loop_context(LoopContextRequest {
            after: None,
            limit: 16,
            mode: PromptMode::CodeAct,
        })
        .await
        .unwrap();

    assert!(text_only.identity_messages.is_empty());
    assert_eq!(codeact.identity_messages.len(), 1);
    assert_eq!(
        codeact.identity_messages[0].safe_summary,
        "identity file TOOLS.md available"
    );
    assert_eq!(source.load_calls(), 2);
}

#[tokio::test]
async fn context_port_defense_gate_excludes_personal_identity_from_host_source_by_default_policy() {
    let fixture = ThreadFixture::new().await;
    let source = Arc::new(StaticIdentityContextSource::new(vec![
        trusted_identity(
            "AGENTS.md",
            "stable identity",
            IdentityApplicability::Always,
        ),
        personal_identity("USER.md", "private user profile"),
    ]));
    let adapter = ThreadBackedLoopContextPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
        16,
    )
    .with_identity_context_source(source);

    let bundle = adapter
        .load_loop_context(LoopContextRequest {
            after: None,
            limit: 16,
            mode: PromptMode::TextOnly,
        })
        .await
        .unwrap();

    assert_eq!(bundle.identity_messages.len(), 1);
    assert_eq!(
        bundle.identity_messages[0].safe_summary,
        "identity file AGENTS.md available"
    );
}

#[tokio::test]
async fn context_port_includes_personal_identity_when_profile_allows_it() {
    let fixture = ThreadFixture::new().await;
    let mut run_context = fixture.run_context.clone();
    run_context.resolved_run_profile.personal_context_policy = PersonalContextPolicy::Allowed;
    let source = Arc::new(StaticIdentityContextSource::new(vec![personal_identity(
        "USER.md",
        "private user profile",
    )]));
    let adapter = ThreadBackedLoopContextPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        run_context,
        16,
    )
    .with_identity_context_source(source);

    let bundle = adapter
        .load_loop_context(LoopContextRequest {
            after: None,
            limit: 16,
            mode: PromptMode::TextOnly,
        })
        .await
        .unwrap();

    assert_eq!(bundle.identity_messages.len(), 1);
    assert_eq!(
        bundle.identity_messages[0].safe_summary,
        "identity file USER.md available"
    );
}

#[tokio::test]
async fn context_port_includes_personal_identity_in_codeact_mode_covering_codeact_prompt_mode() {
    let fixture = ThreadFixture::new().await;
    let mut run_context = fixture.run_context.clone();
    run_context.resolved_run_profile.personal_context_policy = PersonalContextPolicy::Allowed;
    let source = Arc::new(StaticIdentityContextSource::new(vec![personal_identity(
        "USER.md",
        "private user profile",
    )]));
    let milestones = Arc::new(InMemoryLoopHostMilestoneSink::default());
    let milestone_sink: Arc<dyn LoopHostMilestoneSink> = milestones.clone();
    let adapter = ThreadBackedLoopContextPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        run_context,
        16,
    )
    .with_identity_context_source(source)
    .with_milestone_sink(milestone_sink);

    let bundle = adapter
        .load_loop_context(LoopContextRequest {
            after: None,
            limit: 16,
            mode: PromptMode::CodeAct,
        })
        .await
        .unwrap();

    assert_eq!(bundle.identity_messages.len(), 1);
    assert_eq!(
        bundle.identity_messages[0].safe_summary,
        "identity file USER.md available"
    );
    let recorded = wait_for_in_memory_milestones(&milestones, 1).await;
    assert_eq!(recorded.len(), 1);
    assert!(matches!(
        &recorded[0].kind,
        LoopHostMilestoneKind::DriverNote { kind, safe_summary }
            if *kind == LoopDriverNoteKind::Context
                && safe_summary.as_str() == "personal context admitted count 1 sources USER.md"
    ));
}

#[tokio::test]
async fn context_port_emits_safe_milestone_when_personal_identity_is_admitted() {
    let fixture = ThreadFixture::new().await;
    let mut run_context = fixture.run_context.clone();
    run_context.resolved_run_profile.personal_context_policy = PersonalContextPolicy::Allowed;
    let source = Arc::new(StaticIdentityContextSource::new(vec![
        personal_identity("USER.md", "private user profile"),
        personal_identity(
            "context/assistant-directives.md",
            "private assistant directive",
        ),
    ]));
    let milestones = Arc::new(InMemoryLoopHostMilestoneSink::default());
    let milestone_sink: Arc<dyn LoopHostMilestoneSink> = milestones.clone();
    let adapter = ThreadBackedLoopContextPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        run_context,
        16,
    )
    .with_identity_context_source(source)
    .with_milestone_sink(milestone_sink);

    let bundle = adapter
        .load_loop_context(LoopContextRequest {
            after: None,
            limit: 16,
            mode: PromptMode::TextOnly,
        })
        .await
        .unwrap();
    let second_bundle = adapter
        .load_loop_context(LoopContextRequest {
            after: None,
            limit: 16,
            mode: PromptMode::TextOnly,
        })
        .await
        .unwrap();

    assert_eq!(bundle.identity_messages.len(), 2);
    assert_eq!(second_bundle.identity_messages.len(), 2);
    let recorded = wait_for_in_memory_milestones(&milestones, 1).await;
    assert_eq!(recorded.len(), 1);
    assert!(matches!(
        &recorded[0].kind,
        LoopHostMilestoneKind::DriverNote { kind, safe_summary }
            if *kind == LoopDriverNoteKind::Context
                && safe_summary.as_str()
                    == "personal context admitted count 2 sources USER.md assistant-directives.md"
    ));
    let wire = serde_json::to_string(&recorded).unwrap();
    assert!(wire.contains("USER.md"));
    assert!(wire.contains("assistant-directives.md"));
    assert!(!wire.contains("private user profile"));
    assert!(!wire.contains("private assistant directive"));
    assert!(!wire.contains("context/assistant-directives.md"));
}

#[tokio::test]
async fn context_port_does_not_emit_personal_context_milestone_when_context_load_fails() {
    let fixture = ThreadFixture::new().await;
    let mut run_context = fixture.run_context.clone();
    run_context.resolved_run_profile.personal_context_policy = PersonalContextPolicy::Allowed;
    let source = Arc::new(StaticIdentityContextSource::new(vec![personal_identity(
        "USER.md",
        "private user profile",
    )]));
    let milestones = Arc::new(InMemoryLoopHostMilestoneSink::default());
    let milestone_sink: Arc<dyn LoopHostMilestoneSink> = milestones.clone();
    let adapter = ThreadBackedLoopContextPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        run_context,
        16,
    )
    .with_identity_context_source(source)
    .with_skill_context_source(Arc::new(DelayedFailingSkillContextSource {
        delay: Duration::from_millis(25),
    }))
    .with_milestone_sink(milestone_sink);

    let error = adapter
        .load_loop_context(LoopContextRequest {
            after: None,
            limit: 16,
            mode: PromptMode::TextOnly,
        })
        .await
        .expect_err("skill context failure should fail context loading");

    assert_eq!(error.kind, AgentLoopHostErrorKind::Unavailable);
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        milestones.milestones().is_empty(),
        "personal context admission should only publish after the full context bundle loads"
    );
}

#[traced_test]
#[tokio::test]
async fn context_port_survives_personal_context_admitted_milestone_sink_failure() {
    let fixture = ThreadFixture::new().await;
    let mut run_context = fixture.run_context.clone();
    run_context.resolved_run_profile.personal_context_policy = PersonalContextPolicy::Allowed;
    let source = Arc::new(StaticIdentityContextSource::new(vec![personal_identity(
        "USER.md",
        "private user profile",
    )]));
    let milestone_sink = Arc::new(FailOnceMilestoneSink::default());
    let adapter = ThreadBackedLoopContextPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        run_context,
        16,
    )
    .with_identity_context_source(source)
    .with_milestone_sink(milestone_sink.clone());

    let first = adapter
        .load_loop_context(LoopContextRequest {
            after: None,
            limit: 16,
            mode: PromptMode::TextOnly,
        })
        .await
        .unwrap();
    assert_eq!(first.identity_messages.len(), 1);
    wait_for_fail_once_attempts(&milestone_sink, 1).await;
    assert!(milestone_sink.milestones().is_empty());
    assert!(logs_contain(
        "failed to emit personal context admitted milestone"
    ));

    let second = adapter
        .load_loop_context(LoopContextRequest {
            after: None,
            limit: 16,
            mode: PromptMode::TextOnly,
        })
        .await
        .unwrap();
    assert_eq!(second.identity_messages.len(), 1);

    wait_for_fail_once_attempts(&milestone_sink, 2).await;
    let milestones = milestone_sink.milestones();
    assert_eq!(milestones.len(), 1);
    assert!(matches!(
        &milestones[0].kind,
        LoopHostMilestoneKind::DriverNote { kind, safe_summary }
            if *kind == LoopDriverNoteKind::Context
                && safe_summary.as_str() == "personal context admitted count 1 sources USER.md"
    ));
}

#[tokio::test]
async fn context_port_milestone_counts_only_budget_admitted_personal_identity() {
    let fixture = ThreadFixture::new().await;
    let mut run_context = fixture.run_context.clone();
    run_context.resolved_run_profile.personal_context_policy = PersonalContextPolicy::Allowed;
    let source = Arc::new(StaticIdentityContextSource::new(vec![
        personal_identity("USER.md", "p"),
        personal_identity(
            "context/assistant-directives.md",
            "private assistant directive that exceeds the tiny budget",
        ),
    ]));
    let milestones = Arc::new(InMemoryLoopHostMilestoneSink::default());
    let milestone_sink: Arc<dyn LoopHostMilestoneSink> = milestones.clone();
    let adapter = ThreadBackedLoopContextPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        run_context,
        16,
    )
    .with_identity_context_source(source)
    .with_identity_budget(IdentityBudget::new(3).unwrap())
    .with_milestone_sink(milestone_sink);

    let bundle = adapter
        .load_loop_context(LoopContextRequest {
            after: None,
            limit: 16,
            mode: PromptMode::TextOnly,
        })
        .await
        .unwrap();

    assert_eq!(bundle.identity_messages.len(), 1);
    let recorded = wait_for_in_memory_milestones(&milestones, 1).await;
    assert_eq!(recorded.len(), 1);
    assert!(matches!(
        &recorded[0].kind,
        LoopHostMilestoneKind::DriverNote { kind, safe_summary }
            if *kind == LoopDriverNoteKind::Context
                && safe_summary.as_str() == "personal context admitted count 1 sources USER.md"
    ));
    let wire = serde_json::to_string(&recorded).unwrap();
    assert!(!wire.contains("assistant-directives.md"));
}

#[tokio::test]
async fn context_port_dedupes_personal_context_admitted_milestone_under_concurrent_loads() {
    let fixture = ThreadFixture::new().await;
    let mut run_context = fixture.run_context.clone();
    run_context.resolved_run_profile.personal_context_policy = PersonalContextPolicy::Allowed;
    let source = Arc::new(StaticIdentityContextSource::new(vec![personal_identity(
        "USER.md",
        "private user profile",
    )]));
    let milestones = Arc::new(InMemoryLoopHostMilestoneSink::default());
    let milestone_sink: Arc<dyn LoopHostMilestoneSink> = milestones.clone();
    let adapter = ThreadBackedLoopContextPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        run_context,
        16,
    )
    .with_identity_context_source(source)
    .with_milestone_sink(milestone_sink);

    let first = adapter.load_loop_context(LoopContextRequest {
        after: None,
        limit: 16,
        mode: PromptMode::TextOnly,
    });
    let second = adapter.load_loop_context(LoopContextRequest {
        after: None,
        limit: 16,
        mode: PromptMode::TextOnly,
    });
    let (first, second) = tokio::join!(first, second);

    assert_eq!(first.unwrap().identity_messages.len(), 1);
    assert_eq!(second.unwrap().identity_messages.len(), 1);
    let recorded = wait_for_in_memory_milestones(&milestones, 1).await;
    assert_eq!(recorded.len(), 1);
}

#[tokio::test]
async fn context_port_does_not_emit_milestone_when_no_personal_context_admitted() {
    let fixture = ThreadFixture::new().await;
    let source = Arc::new(StaticIdentityContextSource::new(vec![trusted_identity(
        "AGENTS.md",
        "stable identity",
        IdentityApplicability::Always,
    )]));
    let milestones = Arc::new(InMemoryLoopHostMilestoneSink::default());
    let milestone_sink: Arc<dyn LoopHostMilestoneSink> = milestones.clone();
    let adapter = ThreadBackedLoopContextPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
        16,
    )
    .with_identity_context_source(source)
    .with_milestone_sink(milestone_sink);

    let _bundle = adapter
        .load_loop_context(LoopContextRequest {
            after: None,
            limit: 16,
            mode: PromptMode::TextOnly,
        })
        .await
        .unwrap();

    let recorded = milestones.milestones();
    assert!(recorded.is_empty());
}

#[tokio::test]
async fn prompt_and_model_ports_materialize_trusted_identity_content() {
    let fixture = ThreadFixture::new().await;
    let source = Arc::new(StaticIdentityContextSource::new(vec![trusted_identity(
        "AGENTS.md",
        "trusted identity content",
        IdentityApplicability::Always,
    )]));
    let context_port = Arc::new(
        ThreadBackedLoopContextPort::new(
            Arc::clone(&fixture.thread_service),
            fixture.thread_scope.clone(),
            fixture.run_context.clone(),
            16,
        )
        .with_identity_context_source(source.clone()),
    );
    let milestones = Arc::new(InMemoryLoopHostMilestoneSink::default());
    let materialization_store = Arc::new(EphemeralInstructionMaterializationStore::default());
    let prompt_port =
        HostManagedLoopPromptPort::new(fixture.run_context.clone(), context_port, milestones)
            .with_instruction_materialization_store(materialization_store);
    let prompt_bundle = prompt_port
        .build_prompt_bundle(ironclaw_loop_contracts::LoopPromptBundleRequest {
            mode: ironclaw_loop_contracts::PromptMode::TextOnly,
            context_cursor: None,
            surface_version: None,
            checkpoint_state_ref: None,
            max_messages: None,
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

    let gateway = Arc::new(RecordingGateway::reply("model says hi"));
    let model_port = ThreadBackedLoopModelPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
        gateway.clone(),
        16,
    )
    .with_identity_context_source(source);

    model_port
        .stream_model(LoopModelRequest {
            inline_messages: Vec::new(),
            messages: prompt_bundle.messages,
            surface_version: None,
            model_preference: None,
            fallback_index: 0,
            iteration: 0,
            capability_view: None,
        })
        .await
        .unwrap();

    let calls = gateway.calls.lock().unwrap();
    assert_eq!(
        calls[0].messages[0].role,
        HostManagedModelMessageRole::System
    );
    assert_eq!(calls[0].messages[0].content, "trusted identity content");
}

#[tokio::test]
async fn model_port_limits_provider_tool_definitions_to_model_visible_capability_view() {
    let fixture = ThreadFixture::new().await;
    let messages = user_model_messages(&fixture);
    issue_prompt_grant(&fixture.run_context, &messages);

    let allowed_id = CapabilityId::new("demo.allowed").unwrap();
    let hidden_id = CapabilityId::new("demo.hidden").unwrap();
    let gateway = Arc::new(RecordingGateway::reply("model says hi"));
    let model_port = ThreadBackedLoopModelPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
        gateway.clone(),
        16,
    )
    .with_capability_port(Arc::new(StaticToolDefinitionPort::new(vec![
        provider_tool_definition(allowed_id.clone(), "demo__allowed"),
        provider_tool_definition(hidden_id, "demo__hidden"),
    ])));

    model_port
        .stream_model(LoopModelRequest {
            inline_messages: Vec::new(),
            messages,
            surface_version: None,
            model_preference: None,
            fallback_index: 0,
            iteration: 0,
            capability_view: Some(LoopModelCapabilityView {
                visible_capability_ids: vec![allowed_id],
            }),
        })
        .await
        .unwrap();

    let tool_definition_calls = gateway.tool_definition_calls();
    assert_eq!(tool_definition_calls.len(), 1);
    assert_eq!(
        tool_definition_calls[0]
            .iter()
            .map(|definition| definition.name.as_str())
            .collect::<Vec<_>>(),
        vec!["demo__allowed"]
    );
}

#[tokio::test]
async fn model_port_maps_invalid_model_output_to_recoverable_model_error() {
    let fixture = ThreadFixture::new().await;
    let messages = user_model_messages(&fixture);
    issue_prompt_grant(&fixture.run_context, &messages);

    let gateway = Arc::new(RecordingGateway::model_error(
        HostManagedModelErrorKind::InvalidOutput,
        "model returned a tool call outside the advertised capability surface",
    ));
    let model_port = ThreadBackedLoopModelPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
        gateway.clone(),
        16,
    );

    let error = model_port
        .stream_model(LoopModelRequest {
            inline_messages: Vec::new(),
            messages,
            surface_version: None,
            model_preference: None,
            fallback_index: 0,
            iteration: 0,
            capability_view: None,
        })
        .await
        .unwrap_err();

    assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidOutput);
    assert_eq!(
        error.safe_summary,
        "model returned a tool call outside the advertised capability surface"
    );
    assert_eq!(gateway.calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn model_port_preserves_capability_info_for_filtered_capability_view() {
    let fixture = ThreadFixture::new().await;
    let messages = user_model_messages(&fixture);
    issue_prompt_grant(&fixture.run_context, &messages);

    let allowed_id = CapabilityId::new("demo.allowed").unwrap();
    let hidden_id = CapabilityId::new("demo.hidden").unwrap();
    let gateway = Arc::new(RecordingGateway::reply("model says hi"));
    let model_port = ThreadBackedLoopModelPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
        gateway.clone(),
        16,
    )
    .with_capability_port(Arc::new(StaticToolDefinitionPort::new(vec![
        provider_tool_definition(
            CapabilityId::new("ironclaw.loop.capability_info").unwrap(),
            "capability_info",
        ),
        provider_tool_definition(allowed_id.clone(), "demo__allowed"),
        provider_tool_definition(hidden_id, "demo__hidden"),
    ])));

    model_port
        .stream_model(LoopModelRequest {
            inline_messages: Vec::new(),
            messages,
            surface_version: None,
            model_preference: None,
            fallback_index: 0,
            iteration: 0,
            capability_view: Some(LoopModelCapabilityView {
                visible_capability_ids: vec![allowed_id],
            }),
        })
        .await
        .unwrap();

    let tool_definition_calls = gateway.tool_definition_calls();
    assert_eq!(tool_definition_calls.len(), 1);
    assert_eq!(
        tool_definition_calls[0]
            .iter()
            .map(|definition| definition.name.as_str())
            .collect::<Vec<_>>(),
        vec!["capability_info", "demo__allowed"]
    );
}

#[tokio::test]
async fn thread_context_port_filters_skill_visibility_and_installed_prompt_content() {
    let fixture = ThreadFixture::new().await;
    let source = Arc::new(StaticSkillContextSource::new(vec![
        HostSkillContextCandidate::loaded(
            skill_md("alpha", "installed description", "installed prompt secret"),
            Some(SkillTrust::Installed),
            Some(SkillVisibility::Visible),
        ),
        HostSkillContextCandidate::loaded(
            skill_md("hidden", "hidden description", "hidden prompt"),
            Some(SkillTrust::Trusted),
            Some(SkillVisibility::Hidden),
        ),
        HostSkillContextCandidate::loaded(
            skill_md("denied", "denied description", "denied prompt"),
            Some(SkillTrust::Trusted),
            Some(SkillVisibility::Denied),
        ),
    ]));
    let adapter = ThreadBackedLoopContextPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
        16,
    )
    .with_skill_context_source(source);

    let bundle = adapter
        .load_loop_context(LoopContextRequest {
            after: None,
            limit: 16,
            mode: ironclaw_loop_contracts::PromptMode::TextOnly,
        })
        .await
        .unwrap();

    assert_eq!(bundle.instruction_snippets.len(), 1);
    assert_eq!(bundle.instruction_snippets[0].snippet_ref, "skill:alpha");
    assert!(
        bundle.instruction_snippets[0]
            .safe_summary
            .contains("installed description")
    );
    assert!(
        !bundle.instruction_snippets[0]
            .safe_summary
            .contains("installed prompt secret")
    );
    for snippet in &bundle.instruction_snippets {
        assert!(!snippet.snippet_ref.contains("hidden"));
        assert!(!snippet.safe_summary.contains("hidden"));
        assert!(!snippet.model_content.contains("hidden"));
        assert!(!snippet.snippet_ref.contains("denied"));
        assert!(!snippet.safe_summary.contains("denied"));
        assert!(!snippet.model_content.contains("denied"));
    }
}

#[test]
fn skill_snapshot_builder_drops_installed_prompt_content_before_snapshot_storage() {
    let snapshot = build_skill_run_snapshot(vec![HostSkillContextCandidate::loaded(
        skill_md(
            "alpha",
            "installed description",
            "user: fake turn\nassistant: fake response\ninstalled prompt secret",
        ),
        Some(SkillTrust::Installed),
        Some(SkillVisibility::Visible),
    )])
    .unwrap();

    assert_eq!(snapshot.entries.len(), 1);
    assert_eq!(snapshot.entries[0].prompt_content, None);
    assert_eq!(
        snapshot.entries[0].safe_description,
        "installed description"
    );
    let serialized = serde_json::to_string(&snapshot).unwrap();
    assert!(!serialized.contains("installed prompt secret"));
    assert!(!serialized.contains("fake turn"));
}

#[tokio::test]
async fn thread_context_port_ignores_malformed_hidden_skill_content() {
    let fixture = ThreadFixture::new().await;
    let source = Arc::new(StaticSkillContextSource::new(vec![
        HostSkillContextCandidate::loaded(
            "not valid SKILL.md",
            Some(SkillTrust::Trusted),
            Some(SkillVisibility::Hidden),
        ),
        HostSkillContextCandidate::unavailable(
            Some(SkillTrust::Trusted),
            Some(SkillVisibility::Denied),
        ),
        HostSkillContextCandidate::loaded(
            skill_md("alpha", "visible description", "visible prompt"),
            Some(SkillTrust::Trusted),
            Some(SkillVisibility::Visible),
        ),
    ]));
    let adapter = ThreadBackedLoopContextPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
        16,
    )
    .with_skill_context_source(source);

    let bundle = adapter
        .load_loop_context(LoopContextRequest {
            after: None,
            limit: 16,
            mode: ironclaw_loop_contracts::PromptMode::TextOnly,
        })
        .await
        .unwrap();

    assert_eq!(bundle.instruction_snippets.len(), 1);
    let snippet = &bundle.instruction_snippets[0];
    assert_eq!(snippet.snippet_ref, "skill:alpha");
    assert!(snippet.safe_summary.contains("visible description"));
    assert!(!snippet.safe_summary.contains("visible prompt"));
    assert!(snippet.model_content.contains("visible description"));
    assert!(snippet.model_content.contains("visible prompt"));
}

#[tokio::test]
async fn thread_context_port_fails_closed_when_visible_skill_content_is_missing() {
    let fixture = ThreadFixture::new().await;
    let source = Arc::new(StaticSkillContextSource::new(vec![
        HostSkillContextCandidate::unavailable(
            Some(SkillTrust::Trusted),
            Some(SkillVisibility::Visible),
        ),
    ]));
    let adapter = ThreadBackedLoopContextPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
        16,
    )
    .with_skill_context_source(source);

    let error = adapter
        .load_loop_context(LoopContextRequest {
            after: None,
            limit: 16,
            mode: ironclaw_loop_contracts::PromptMode::TextOnly,
        })
        .await
        .unwrap_err();

    assert_eq!(error.kind, AgentLoopHostErrorKind::Unavailable);
}

#[tokio::test]
async fn thread_context_port_fails_closed_when_skill_policy_data_is_missing() {
    let fixture = ThreadFixture::new().await;
    let source = Arc::new(StaticSkillContextSource::new(vec![
        HostSkillContextCandidate::loaded(
            skill_md(
                "alpha",
                "safe alpha description",
                "Use alpha prompt content.",
            ),
            None,
            Some(SkillVisibility::Visible),
        ),
    ]));
    let adapter = ThreadBackedLoopContextPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
        16,
    )
    .with_skill_context_source(source);

    let error = adapter
        .load_loop_context(LoopContextRequest {
            after: None,
            limit: 16,
            mode: ironclaw_loop_contracts::PromptMode::TextOnly,
        })
        .await
        .unwrap_err();

    assert_eq!(error.kind, AgentLoopHostErrorKind::PolicyDenied);
    assert!(!serde_json::to_string(&error).unwrap().contains("alpha"));
}

#[tokio::test]
async fn prompt_and_model_ports_send_selected_skill_context_to_gateway() {
    let fixture = ThreadFixture::new().await;
    let source = Arc::new(StaticSkillContextSource::new(vec![
        HostSkillContextCandidate::loaded(
            skill_md(
                "alpha",
                "safe alpha description",
                "Use alpha prompt content.",
            ),
            Some(SkillTrust::Trusted),
            Some(SkillVisibility::Visible),
        ),
    ]));
    let context_port = Arc::new(
        ThreadBackedLoopContextPort::new(
            Arc::clone(&fixture.thread_service),
            fixture.thread_scope.clone(),
            fixture.run_context.clone(),
            16,
        )
        .with_skill_context_source(source.clone()),
    );
    let milestones = Arc::new(InMemoryLoopHostMilestoneSink::default());
    let prompt_port =
        HostManagedLoopPromptPort::new(fixture.run_context.clone(), context_port, milestones);
    let prompt_bundle = prompt_port
        .build_prompt_bundle(ironclaw_loop_contracts::LoopPromptBundleRequest {
            mode: ironclaw_loop_contracts::PromptMode::TextOnly,
            context_cursor: None,
            surface_version: None,
            checkpoint_state_ref: None,
            max_messages: None,
            inline_messages: Vec::new(),
            capability_view: None,
        })
        .await
        .unwrap();
    assert_eq!(prompt_bundle.messages.len(), 2);
    assert_eq!(prompt_bundle.messages[0].role, "system");
    assert_eq!(
        prompt_bundle.messages[0].content_ref,
        LoopMessageRef::new("msg:snippet.skill.alpha.0.c6c47a3818b58f3d").unwrap()
    );

    let gateway = Arc::new(RecordingGateway::reply("model says hi"));
    let model_port = ThreadBackedLoopModelPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
        gateway.clone(),
        16,
    )
    .with_skill_context_source(source);

    model_port
        .stream_model(LoopModelRequest {
            inline_messages: Vec::new(),
            messages: prompt_bundle.messages,
            surface_version: None,
            model_preference: None,
            fallback_index: 0,
            iteration: 0,
            capability_view: None,
        })
        .await
        .unwrap();

    let calls = gateway.calls.lock().unwrap();
    assert_eq!(
        calls[0].messages[0].role,
        HostManagedModelMessageRole::System
    );
    assert!(
        calls[0].messages[0]
            .content
            .contains("safe alpha description")
    );
    assert!(
        calls[0].messages[0]
            .content
            .contains("Use alpha prompt content.")
    );
    assert_eq!(calls[0].messages[1].role, HostManagedModelMessageRole::User);
    assert_eq!(calls[0].messages[1].content, "hello reborn");
}

#[tokio::test]
async fn prompt_and_model_ports_resolve_skill_refs_after_prompt_sorting() {
    let fixture = ThreadFixture::new().await;
    let source = Arc::new(StaticSkillContextSource::new(vec![
        HostSkillContextCandidate::loaded(
            skill_md("zeta", "safe zeta description", "Use zeta prompt content."),
            Some(SkillTrust::Trusted),
            Some(SkillVisibility::Visible),
        )
        .with_ordering_key("0000000000000000"),
        HostSkillContextCandidate::loaded(
            skill_md(
                "alpha",
                "safe alpha description",
                "Use alpha prompt content.",
            ),
            Some(SkillTrust::Trusted),
            Some(SkillVisibility::Visible),
        )
        .with_ordering_key("0000000000000001"),
    ]));
    let context_port = Arc::new(
        ThreadBackedLoopContextPort::new(
            Arc::clone(&fixture.thread_service),
            fixture.thread_scope.clone(),
            fixture.run_context.clone(),
            16,
        )
        .with_skill_context_source(source.clone()),
    );
    let milestones = Arc::new(InMemoryLoopHostMilestoneSink::default());
    let prompt_port =
        HostManagedLoopPromptPort::new(fixture.run_context.clone(), context_port, milestones);
    let prompt_bundle = prompt_port
        .build_prompt_bundle(ironclaw_loop_contracts::LoopPromptBundleRequest {
            mode: ironclaw_loop_contracts::PromptMode::TextOnly,
            context_cursor: None,
            surface_version: None,
            checkpoint_state_ref: None,
            max_messages: None,
            inline_messages: Vec::new(),
            capability_view: None,
        })
        .await
        .unwrap();

    assert_eq!(prompt_bundle.messages.len(), 3);
    assert_eq!(prompt_bundle.messages[0].role, "system");
    assert!(
        prompt_bundle.messages[0]
            .content_ref
            .as_str()
            .contains("skill.alpha")
    );
    assert_eq!(prompt_bundle.messages[1].role, "system");
    assert!(
        prompt_bundle.messages[1]
            .content_ref
            .as_str()
            .contains("skill.zeta")
    );

    let gateway = Arc::new(RecordingGateway::reply("model says hi"));
    let model_port = ThreadBackedLoopModelPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
        gateway.clone(),
        16,
    )
    .with_skill_context_source(source);

    model_port
        .stream_model(LoopModelRequest {
            inline_messages: Vec::new(),
            messages: prompt_bundle.messages,
            surface_version: None,
            model_preference: None,
            fallback_index: 0,
            iteration: 0,
            capability_view: None,
        })
        .await
        .unwrap();

    let calls = gateway.calls.lock().unwrap();
    assert!(
        calls[0].messages[0]
            .content
            .contains("safe alpha description")
    );
    assert!(
        calls[0].messages[1]
            .content
            .contains("safe zeta description")
    );
    assert_eq!(calls[0].messages[2].role, HostManagedModelMessageRole::User);
}

#[tokio::test]
async fn prompt_and_model_ports_resolve_instruction_memory_and_identity_refs() {
    let fixture = ThreadFixture::new().await;
    let materialization_store = Arc::new(EphemeralInstructionMaterializationStore::default());
    let context_port = Arc::new(StaticLoopContextPort {
        bundle: LoopContextBundle {
            identity_messages: vec![LoopContextMessage {
                message_ref: Some(LoopMessageRef::new("msg:identity-policy").unwrap()),
                role: "system".to_string(),
                safe_summary: "identity policy summary".to_string(),
                compaction: None,
            }],
            messages: vec![LoopContextMessage {
                message_ref: Some(
                    LoopMessageRef::new(format!("msg:{}", fixture.user_message_id)).unwrap(),
                ),
                role: "user".to_string(),
                safe_summary: "user message available".to_string(),
                compaction: None,
            }],
            compaction_message_index: Vec::new(),
            instruction_snippets: vec![LoopContextSnippet {
                snippet_ref: "instruction:project".to_string(),
                model_content: "project instruction summary".to_string(),
                safe_summary: "project instruction summary".to_string(),
                metadata: None,
            }],
            memory_snippets: vec![LoopContextSnippet {
                snippet_ref: "memory:project-summary".to_string(),
                model_content: "project memory summary".to_string(),
                safe_summary: "project memory summary".to_string(),
                metadata: None,
            }],
        },
    });
    let milestones = Arc::new(InMemoryLoopHostMilestoneSink::default());
    let prompt_port =
        HostManagedLoopPromptPort::new(fixture.run_context.clone(), context_port, milestones)
            .with_instruction_materialization_store(materialization_store.clone());
    let prompt_bundle = prompt_port
        .build_prompt_bundle(ironclaw_loop_contracts::LoopPromptBundleRequest {
            mode: ironclaw_loop_contracts::PromptMode::TextOnly,
            context_cursor: None,
            surface_version: None,
            checkpoint_state_ref: None,
            max_messages: None,
            inline_messages: Vec::new(),
            capability_view: None,
        })
        .await
        .unwrap();
    assert_eq!(prompt_bundle.messages.len(), 4);

    let gateway = Arc::new(RecordingGateway::reply("model says hi"));
    let model_port = ThreadBackedLoopModelPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
        gateway.clone(),
        16,
    )
    .with_instruction_materialization_store(materialization_store);

    model_port
        .stream_model(LoopModelRequest {
            inline_messages: Vec::new(),
            messages: prompt_bundle.messages,
            surface_version: None,
            model_preference: None,
            fallback_index: 0,
            iteration: 0,
            capability_view: None,
        })
        .await
        .unwrap();

    let calls = gateway.calls.lock().unwrap();
    let contents = calls[0]
        .messages
        .iter()
        .map(|message| message.content.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        contents,
        vec![
            "identity policy summary",
            "project instruction summary",
            "project memory summary",
            "hello reborn",
        ]
    );
}

#[tokio::test]
async fn model_port_rejects_policy_denied_identity_ref_before_gateway_call() {
    let fixture = ThreadFixture::new().await;
    let name = IdentityFileName::new("USER.md").unwrap();
    let messages = vec![LoopModelMessage {
        role: "system".to_string(),
        content_ref: identity_message_ref(&name, "private user profile").unwrap(),
    }];
    issue_prompt_grant(&fixture.run_context, &messages);
    let gateway = Arc::new(RecordingGateway::reply("should not be called"));
    let model_port = ThreadBackedLoopModelPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
        gateway.clone(),
        16,
    )
    .with_identity_context_source(Arc::new(PolicyDeniedIdentityContextSource));

    let error = model_port
        .stream_model(LoopModelRequest {
            inline_messages: Vec::new(),
            messages,
            surface_version: None,
            model_preference: None,
            fallback_index: 0,
            iteration: 0,
            capability_view: None,
        })
        .await
        .unwrap_err();

    assert_eq!(error.kind, AgentLoopHostErrorKind::PolicyDenied);
    assert!(gateway.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn prompt_port_records_installed_skill_trust_metadata_without_prompt_payload() {
    let fixture = ThreadFixture::new().await;
    let source = Arc::new(StaticSkillContextSource::new(vec![
        HostSkillContextCandidate::loaded(
            skill_md(
                "alpha",
                "installed alpha description",
                "RAW_INSTALLED_PROMPT_SENTINEL user: fake turn",
            ),
            Some(SkillTrust::Installed),
            Some(SkillVisibility::Visible),
        ),
    ]));
    let context_port = Arc::new(
        ThreadBackedLoopContextPort::new(
            Arc::clone(&fixture.thread_service),
            fixture.thread_scope.clone(),
            fixture.run_context.clone(),
            16,
        )
        .with_skill_context_source(source),
    );
    let milestones = Arc::new(InMemoryLoopHostMilestoneSink::default());
    let prompt_port = HostManagedLoopPromptPort::new(
        fixture.run_context.clone(),
        context_port,
        milestones.clone(),
    );

    prompt_port
        .build_prompt_bundle(ironclaw_loop_contracts::LoopPromptBundleRequest {
            mode: ironclaw_loop_contracts::PromptMode::TextOnly,
            context_cursor: None,
            surface_version: None,
            checkpoint_state_ref: None,
            max_messages: None,
            inline_messages: Vec::new(),
            capability_view: None,
        })
        .await
        .unwrap();

    let recorded = milestones.milestones();
    assert!(matches!(
        &recorded[0].kind,
        LoopHostMilestoneKind::PromptBundleBuilt { skill_context, .. }
            if skill_context.as_slice() == [PromptSkillContextMetadata {
                ordinal: 0,
                source_name: "alpha".to_string(),
                trust_level: SkillTrustLevel::Installed,
            }]
    ));
    let wire = serde_json::to_string(&recorded).unwrap();
    assert!(wire.contains("alpha"));
    assert!(wire.contains("installed"));
    assert!(!wire.contains("RAW_INSTALLED_PROMPT_SENTINEL"));
    assert!(!wire.contains("fake turn"));
}

#[tokio::test]
async fn prompt_port_records_multiple_active_skill_metadata_in_prompt_order() {
    let fixture = ThreadFixture::new().await;
    let source = Arc::new(StaticSkillContextSource::new(vec![
        HostSkillContextCandidate::loaded(
            skill_md("bravo", "trusted bravo description", "trusted prompt"),
            Some(SkillTrust::Trusted),
            Some(SkillVisibility::Visible),
        ),
        HostSkillContextCandidate::loaded(
            skill_md("alpha", "installed alpha description", "installed prompt"),
            Some(SkillTrust::Installed),
            Some(SkillVisibility::Visible),
        ),
    ]));
    let context_port = Arc::new(
        ThreadBackedLoopContextPort::new(
            Arc::clone(&fixture.thread_service),
            fixture.thread_scope.clone(),
            fixture.run_context.clone(),
            16,
        )
        .with_skill_context_source(source),
    );
    let milestones = Arc::new(InMemoryLoopHostMilestoneSink::default());
    let prompt_port = HostManagedLoopPromptPort::new(
        fixture.run_context.clone(),
        context_port,
        milestones.clone(),
    );

    prompt_port
        .build_prompt_bundle(ironclaw_loop_contracts::LoopPromptBundleRequest {
            mode: ironclaw_loop_contracts::PromptMode::TextOnly,
            context_cursor: None,
            surface_version: None,
            checkpoint_state_ref: None,
            max_messages: None,
            inline_messages: Vec::new(),
            capability_view: None,
        })
        .await
        .unwrap();

    let recorded = milestones.milestones();
    let LoopHostMilestoneKind::PromptBundleBuilt { skill_context, .. } = &recorded[0].kind else {
        panic!("expected prompt_bundle_built milestone");
    };
    assert_eq!(
        skill_context,
        &vec![
            PromptSkillContextMetadata {
                ordinal: 0,
                source_name: "alpha".to_string(),
                trust_level: SkillTrustLevel::Installed,
            },
            PromptSkillContextMetadata {
                ordinal: 1,
                source_name: "bravo".to_string(),
                trust_level: SkillTrustLevel::Trusted,
            },
        ]
    );
}

#[tokio::test]
async fn prompt_and_model_ports_keep_duplicate_skill_names_distinct() {
    let fixture = ThreadFixture::new().await;
    let source = Arc::new(StaticSkillContextSource::new(vec![
        HostSkillContextCandidate::loaded(
            skill_md("alpha", "first description", "first prompt"),
            Some(SkillTrust::Trusted),
            Some(SkillVisibility::Visible),
        )
        .with_ordering_key("alpha-1"),
        HostSkillContextCandidate::loaded(
            skill_md("alpha", "second description", "second prompt"),
            Some(SkillTrust::Trusted),
            Some(SkillVisibility::Visible),
        )
        .with_ordering_key("alpha-2"),
    ]));
    let context_port = Arc::new(
        ThreadBackedLoopContextPort::new(
            Arc::clone(&fixture.thread_service),
            fixture.thread_scope.clone(),
            fixture.run_context.clone(),
            16,
        )
        .with_skill_context_source(source.clone()),
    );
    let prompt_port = HostManagedLoopPromptPort::new(
        fixture.run_context.clone(),
        context_port,
        Arc::new(InMemoryLoopHostMilestoneSink::default()),
    );
    let prompt_bundle = prompt_port
        .build_prompt_bundle(ironclaw_loop_contracts::LoopPromptBundleRequest {
            mode: ironclaw_loop_contracts::PromptMode::TextOnly,
            context_cursor: None,
            surface_version: None,
            checkpoint_state_ref: None,
            max_messages: None,
            inline_messages: Vec::new(),
            capability_view: None,
        })
        .await
        .unwrap();

    assert_eq!(prompt_bundle.messages.len(), 3);
    assert_ne!(
        prompt_bundle.messages[0].content_ref,
        prompt_bundle.messages[1].content_ref
    );

    let gateway = Arc::new(RecordingGateway::reply("model says hi"));
    let model_port = ThreadBackedLoopModelPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
        gateway.clone(),
        16,
    )
    .with_skill_context_source(source);

    model_port
        .stream_model(LoopModelRequest {
            inline_messages: Vec::new(),
            messages: prompt_bundle.messages,
            surface_version: None,
            model_preference: None,
            fallback_index: 0,
            iteration: 0,
            capability_view: None,
        })
        .await
        .unwrap();

    let calls = gateway.calls.lock().unwrap();
    assert!(calls[0].messages[0].content.contains("first prompt"));
    assert!(calls[0].messages[1].content.contains("second prompt"));
}

#[tokio::test]
async fn model_port_rejects_skill_context_refs_when_source_changes_after_prompt_build() {
    let fixture = ThreadFixture::new().await;
    let source = Arc::new(MutableSkillContextSource::new(vec![
        HostSkillContextCandidate::loaded(
            skill_md("alpha", "original description", "original prompt"),
            Some(SkillTrust::Trusted),
            Some(SkillVisibility::Visible),
        ),
    ]));
    let context_port = Arc::new(
        ThreadBackedLoopContextPort::new(
            Arc::clone(&fixture.thread_service),
            fixture.thread_scope.clone(),
            fixture.run_context.clone(),
            16,
        )
        .with_skill_context_source(source.clone()),
    );
    let prompt_port = HostManagedLoopPromptPort::new(
        fixture.run_context.clone(),
        context_port,
        Arc::new(InMemoryLoopHostMilestoneSink::default()),
    );
    let prompt_bundle = prompt_port
        .build_prompt_bundle(ironclaw_loop_contracts::LoopPromptBundleRequest {
            mode: ironclaw_loop_contracts::PromptMode::TextOnly,
            context_cursor: None,
            surface_version: None,
            checkpoint_state_ref: None,
            max_messages: None,
            inline_messages: Vec::new(),
            capability_view: None,
        })
        .await
        .unwrap();

    source.set(vec![HostSkillContextCandidate::loaded(
        skill_md("alpha", "changed description", "changed prompt"),
        Some(SkillTrust::Trusted),
        Some(SkillVisibility::Visible),
    )]);
    let gateway = Arc::new(RecordingGateway::reply("should not be called"));
    let model_port = ThreadBackedLoopModelPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
        gateway.clone(),
        16,
    )
    .with_skill_context_source(source);

    let error = model_port
        .stream_model(LoopModelRequest {
            inline_messages: Vec::new(),
            messages: prompt_bundle.messages,
            surface_version: None,
            model_preference: None,
            fallback_index: 0,
            iteration: 0,
            capability_view: None,
        })
        .await
        .unwrap_err();

    assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
    assert!(gateway.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn thread_context_port_rejects_non_origin_context_cursor() {
    let fixture = ThreadFixture::new().await;
    let adapter = ThreadBackedLoopContextPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
        16,
    );

    let error = adapter
        .load_loop_context(LoopContextRequest {
            after: Some(LoopInputCursor::from_host_token(
                &fixture.run_context,
                LoopInputCursorToken::new("input-cursor:after-first-input").unwrap(),
            )),
            limit: 16,
            mode: ironclaw_loop_contracts::PromptMode::TextOnly,
        })
        .await
        .unwrap_err();

    assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
}

#[tokio::test]
async fn thread_ports_reject_thread_scope_mismatch_before_thread_access() {
    let fixture = ThreadFixture::new().await;
    let mut wrong_scope = fixture.thread_scope.clone();
    wrong_scope.tenant_id = TenantId::new("different-tenant").unwrap();
    let adapter = ThreadBackedLoopContextPort::new(
        Arc::clone(&fixture.thread_service),
        wrong_scope,
        fixture.run_context.clone(),
        16,
    );

    let error = adapter
        .load_loop_context(LoopContextRequest {
            after: None,
            limit: 16,
            mode: ironclaw_loop_contracts::PromptMode::TextOnly,
        })
        .await
        .unwrap_err();

    assert_eq!(error.kind, AgentLoopHostErrorKind::ScopeMismatch);
}

#[tokio::test]
async fn context_port_rejects_cursor_from_another_run() {
    let fixture = ThreadFixture::new().await;
    let other_context = LoopRunContext::new(
        fixture.run_context.scope.clone(),
        fixture.run_context.turn_id,
        TurnRunId::new(),
        fixture.run_context.resolved_run_profile.clone(),
    );
    let adapter = ThreadBackedLoopContextPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
        16,
    );

    let error = adapter
        .load_loop_context(LoopContextRequest {
            after: Some(LoopInputCursor::from_host_token(
                &other_context,
                LoopInputCursorToken::new("input-cursor:foreign-run").unwrap(),
            )),
            limit: 16,
            mode: ironclaw_loop_contracts::PromptMode::TextOnly,
        })
        .await
        .unwrap_err();

    assert_eq!(error.kind, AgentLoopHostErrorKind::ScopeMismatch);
}

#[tokio::test]
async fn transcript_port_finalizes_assistant_reply_into_durable_thread_history() {
    let fixture = ThreadFixture::new().await;
    let adapter = ThreadBackedLoopTranscriptPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
    );

    let message_ref = adapter
        .finalize_assistant_message(FinalizeAssistantMessage {
            reply: AssistantReply {
                content: "hi from reborn".to_string(),
            },
        })
        .await
        .unwrap();

    assert!(message_ref.as_str().starts_with("msg:"));
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
        .expect("assistant reply must be persisted");
    assert_eq!(assistant.status, MessageStatus::Finalized);
    assert_eq!(assistant.content.as_deref(), Some("hi from reborn"));
    assert_eq!(
        message_ref.as_str(),
        format!("msg:{}", assistant.message_id)
    );
}

#[tokio::test]
async fn finalized_assistant_attachment_refs_are_sealed_in_registration_order() {
    let fixture = ThreadFixture::new().await;
    let store = Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
    register_reply_attachment(
        store.as_ref(),
        &fixture,
        "/workspace/report.csv",
        "report.csv",
        "text/csv",
        19,
    )
    .await;
    register_reply_attachment(
        store.as_ref(),
        &fixture,
        "/workspace/chart.gif",
        "chart.gif",
        "image/gif",
        23,
    )
    .await;
    register_reply_attachment(
        store.as_ref(),
        &fixture,
        "/workspace/voice.wav",
        "voice.wav",
        "audio/wav",
        11,
    )
    .await;
    register_reply_attachment(
        store.as_ref(),
        &fixture,
        "/workspace/clip.mp4",
        "clip.mp4",
        "video/mp4",
        13,
    )
    .await;
    register_reply_attachment(
        store.as_ref(),
        &fixture,
        "/workspace/scene.glb",
        "scene.glb",
        "model/gltf-binary",
        17,
    )
    .await;
    let intent_port: Arc<dyn ReplyAttachmentIntentPort> = store.clone();
    let adapter = ThreadBackedLoopTranscriptPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
    )
    .with_reply_attachment_intent_port(intent_port);

    adapter
        .finalize_assistant_message(FinalizeAssistantMessage {
            reply: AssistantReply {
                content: "Created [the report](/workspace/report.csv); unrelated [link](/workspace/missing.txt).".to_string(),
            },
        })
        .await
        .unwrap();

    let assistant = finalized_assistant_message(&fixture).await;
    assert_eq!(
        assistant.content.as_deref(),
        Some(
            "Created [the report](/workspace/report.csv); unrelated [link](/workspace/missing.txt)."
        )
    );
    assert_eq!(assistant.attachments.len(), 5);
    let run_id = reply_attachment_run_id(&fixture);
    assert_eq!(
        assistant
            .attachments
            .iter()
            .map(|attachment| attachment.id.clone())
            .collect::<Vec<_>>(),
        vec![
            ReplyAttachmentHandle::for_run_path(
                &run_id,
                &ScopedPath::new("/workspace/report.csv").unwrap(),
            )
            .to_string(),
            ReplyAttachmentHandle::for_run_path(
                &run_id,
                &ScopedPath::new("/workspace/chart.gif").unwrap(),
            )
            .to_string(),
            ReplyAttachmentHandle::for_run_path(
                &run_id,
                &ScopedPath::new("/workspace/voice.wav").unwrap(),
            )
            .to_string(),
            ReplyAttachmentHandle::for_run_path(
                &run_id,
                &ScopedPath::new("/workspace/clip.mp4").unwrap(),
            )
            .to_string(),
            ReplyAttachmentHandle::for_run_path(
                &run_id,
                &ScopedPath::new("/workspace/scene.glb").unwrap(),
            )
            .to_string(),
        ]
    );
    assert_eq!(
        assistant
            .attachments
            .iter()
            .map(|attachment| attachment.storage_key.as_deref())
            .collect::<Vec<_>>(),
        vec![
            Some("/workspace/report.csv"),
            Some("/workspace/chart.gif"),
            Some("/workspace/voice.wav"),
            Some("/workspace/clip.mp4"),
            Some("/workspace/scene.glb"),
        ]
    );
    assert_eq!(
        assistant.attachments[0].filename.as_deref(),
        Some("report.csv")
    );
    assert_eq!(assistant.attachments[0].mime_type, "text/csv");
    assert_eq!(assistant.attachments[0].size_bytes, Some(19));
    assert_eq!(assistant.attachments[0].kind, AttachmentKind::Document);
    assert_eq!(assistant.attachments[1].kind, AttachmentKind::Image);
    assert_eq!(assistant.attachments[2].kind, AttachmentKind::Audio);
    assert_eq!(assistant.attachments[3].kind, AttachmentKind::Video);
    assert_eq!(assistant.attachments[4].kind, AttachmentKind::Other);
    let error = store
        .register(
            &reply_attachment_scope(&fixture),
            &reply_attachment_run_id(&fixture),
            reply_attachment("/workspace/late.txt", "late.txt", "text/plain", 4),
        )
        .await
        .unwrap_err();
    assert!(matches!(error, OutboundError::ReplyAttachmentIntentsSealed));
}

#[tokio::test]
async fn finalized_assistant_deprojects_host_context_for_the_same_typed_attachment() {
    let fixture = ThreadFixture::new().await;
    let store = Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
    register_reply_attachment(
        store.as_ref(),
        &fixture,
        "/workspace/attachments/photo.jpg",
        "photo.jpg",
        "image/jpeg",
        3714,
    )
    .await;
    let intent_port: Arc<dyn ReplyAttachmentIntentPort> = store;
    let adapter = ThreadBackedLoopTranscriptPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
    )
    .with_reply_attachment_intent_port(intent_port);
    let copied_context = "Reattached the image.\n\n<attachments>\n\
<attachment index=\"1\" type=\"document\" filename=\"photo.jpg\" mime=\"image/jpeg\" project_path=\"/workspace/attachments/photo.jpg\" size=\"3KB\">\n\
Saved to project file: /workspace/attachments/photo.jpg\n\
[Document attached — text extraction unavailable]\n\
</attachment>\n\
</attachments>";

    adapter
        .finalize_assistant_message(FinalizeAssistantMessage {
            reply: AssistantReply {
                content: copied_context.to_string(),
            },
        })
        .await
        .unwrap();

    let assistant = finalized_assistant_message(&fixture).await;
    assert_eq!(assistant.content.as_deref(), Some("Reattached the image."));
    assert_eq!(assistant.attachments.len(), 1);
    assert_eq!(assistant.attachments[0].kind, AttachmentKind::Image);
    assert_eq!(
        assistant.attachments[0].storage_key.as_deref(),
        Some("/workspace/attachments/photo.jpg")
    );
}

#[tokio::test]
async fn finalized_assistant_attachment_port_with_no_intents_stays_text_only() {
    let fixture = ThreadFixture::new().await;
    let store = Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
    let intent_port: Arc<dyn ReplyAttachmentIntentPort> = store;
    let adapter = ThreadBackedLoopTranscriptPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
    )
    .with_reply_attachment_intent_port(intent_port);

    adapter
        .finalize_assistant_message(FinalizeAssistantMessage {
            reply: AssistantReply {
                content: "plain reply".to_string(),
            },
        })
        .await
        .unwrap();

    let assistant = finalized_assistant_message(&fixture).await;
    assert_eq!(assistant.content.as_deref(), Some("plain reply"));
    assert!(assistant.attachments.is_empty());
}

#[tokio::test]
async fn finalized_assistant_attachment_seal_failure_prevents_transcript_write() {
    let fixture = ThreadFixture::new().await;
    let adapter = ThreadBackedLoopTranscriptPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
    )
    .with_reply_attachment_intent_port(Arc::new(FailingSealReplyAttachmentIntentPort));

    let error = adapter
        .finalize_assistant_message(FinalizeAssistantMessage {
            reply: AssistantReply {
                content: "must not persist".to_string(),
            },
        })
        .await
        .unwrap_err();

    assert_eq!(error.kind, AgentLoopHostErrorKind::TranscriptWriteFailed);
    let history = fixture
        .thread_service
        .list_thread_history(ThreadHistoryRequest {
            scope: fixture.thread_scope.clone(),
            thread_id: fixture.thread_id.clone(),
        })
        .await
        .unwrap();
    assert!(
        history
            .messages
            .iter()
            .all(|message| message.kind != MessageKind::Assistant)
    );
}

#[tokio::test]
async fn finalized_assistant_attachment_duplicate_retry_preserves_complete_content() {
    let fixture = ThreadFixture::new().await;
    let store = Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
    register_reply_attachment(
        store.as_ref(),
        &fixture,
        "/workspace/result.json",
        "result.json",
        "application/json",
        7,
    )
    .await;
    let intent_port: Arc<dyn ReplyAttachmentIntentPort> = store;
    let adapter = ThreadBackedLoopTranscriptPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
    )
    .with_reply_attachment_intent_port(intent_port);
    let request = FinalizeAssistantMessage {
        reply: AssistantReply {
            content: "retry reply".to_string(),
        },
    };

    let first = adapter
        .finalize_assistant_message(request.clone())
        .await
        .unwrap();
    let second = adapter.finalize_assistant_message(request).await.unwrap();

    assert_eq!(first, second);
    let assistant = finalized_assistant_message(&fixture).await;
    assert_eq!(assistant.attachments.len(), 1);
    assert_eq!(
        assistant.attachments[0].storage_key.as_deref(),
        Some("/workspace/result.json")
    );
}

#[tokio::test]
async fn finalized_assistant_attachment_retry_rejects_mismatched_refs() {
    let fixture = ThreadFixture::new().await;
    let plain_adapter = ThreadBackedLoopTranscriptPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
    );
    let request = FinalizeAssistantMessage {
        reply: AssistantReply {
            content: "same text".to_string(),
        },
    };
    plain_adapter
        .finalize_assistant_message(request.clone())
        .await
        .unwrap();

    let store = Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
    register_reply_attachment(
        store.as_ref(),
        &fixture,
        "/workspace/different.txt",
        "different.txt",
        "text/plain",
        9,
    )
    .await;
    let intent_port: Arc<dyn ReplyAttachmentIntentPort> = store;
    let attachment_adapter = ThreadBackedLoopTranscriptPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
    )
    .with_reply_attachment_intent_port(intent_port);

    let error = attachment_adapter
        .finalize_assistant_message(request)
        .await
        .unwrap_err();

    assert_eq!(error.kind, AgentLoopHostErrorKind::TranscriptWriteFailed);
    assert!(
        finalized_assistant_message(&fixture)
            .await
            .attachments
            .is_empty()
    );
}

#[tokio::test]
async fn transcript_port_retries_transient_finalized_assistant_backend_failure() {
    let fixture = ThreadFixture::new().await;
    let service = Arc::new(ScriptedTranscriptWriteThreadService::new(
        Arc::clone(&fixture.thread_service),
        TranscriptWriteOperation::FinalizedAssistant,
        TranscriptWriteFailure::Backend,
        1,
    ));
    let adapter = ThreadBackedLoopTranscriptPort::new(
        Arc::clone(&service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
    );

    adapter
        .finalize_assistant_message(FinalizeAssistantMessage {
            reply: AssistantReply {
                content: "persist after transient failure".to_string(),
            },
        })
        .await
        .expect("the exact finalized assistant write is retried");

    assert_eq!(service.attempts(), 2);
    let history = fixture
        .thread_service
        .list_thread_history(ThreadHistoryRequest {
            scope: fixture.thread_scope,
            thread_id: fixture.thread_id,
        })
        .await
        .unwrap();
    assert_eq!(
        history
            .messages
            .iter()
            .filter(|message| message.kind == MessageKind::Assistant)
            .count(),
        1,
        "retry must converge on one durable assistant message"
    );
}

#[tokio::test]
async fn transcript_port_retries_transient_tool_result_backend_failure() {
    let fixture = ThreadFixture::new().await;
    let service = Arc::new(ScriptedTranscriptWriteThreadService::new(
        Arc::clone(&fixture.thread_service),
        TranscriptWriteOperation::ToolResultReference,
        TranscriptWriteFailure::Backend,
        1,
    ));
    let adapter = ThreadBackedLoopTranscriptPort::new(
        Arc::clone(&service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
    );

    adapter
        .append_capability_result_ref(AppendCapabilityResultRef {
            result_ref: LoopResultRef::new("result:transient-tool-write").unwrap(),
            safe_summary: "tool completed once".to_string(),
            provider_call: None,
            model_observation: None,
        })
        .await
        .expect("the exact tool-result reference write is retried");

    assert_eq!(service.attempts(), 2);
    let history = fixture
        .thread_service
        .list_thread_history(ThreadHistoryRequest {
            scope: fixture.thread_scope,
            thread_id: fixture.thread_id,
        })
        .await
        .unwrap();
    assert_eq!(
        history
            .messages
            .iter()
            .filter(|message| message.kind == MessageKind::ToolResultReference)
            .count(),
        1,
        "retry must not duplicate the tool-result reference"
    );
}

#[tokio::test]
async fn transcript_port_does_not_retry_non_backend_write_failure() {
    let fixture = ThreadFixture::new().await;
    let service = Arc::new(ScriptedTranscriptWriteThreadService::new(
        Arc::clone(&fixture.thread_service),
        TranscriptWriteOperation::FinalizedAssistant,
        TranscriptWriteFailure::Serialization,
        1,
    ));
    let adapter = ThreadBackedLoopTranscriptPort::new(
        Arc::clone(&service),
        fixture.thread_scope,
        fixture.run_context,
    );

    let error = adapter
        .finalize_assistant_message(FinalizeAssistantMessage {
            reply: AssistantReply {
                content: "invalid write".to_string(),
            },
        })
        .await
        .expect_err("serialization failures are terminal without retry");

    assert_eq!(error.kind, AgentLoopHostErrorKind::TranscriptWriteFailed);
    assert_eq!(service.attempts(), 1);
}

#[tokio::test]
async fn transcript_port_stops_after_bounded_backend_write_attempts() {
    let fixture = ThreadFixture::new().await;
    let service = Arc::new(ScriptedTranscriptWriteThreadService::new(
        Arc::clone(&fixture.thread_service),
        TranscriptWriteOperation::ToolResultReference,
        TranscriptWriteFailure::Backend,
        usize::MAX,
    ));
    let adapter = ThreadBackedLoopTranscriptPort::new(
        Arc::clone(&service),
        fixture.thread_scope,
        fixture.run_context,
    );

    let error = adapter
        .append_capability_result_ref(AppendCapabilityResultRef {
            result_ref: LoopResultRef::new("result:permanent-tool-write").unwrap(),
            safe_summary: "tool completed once".to_string(),
            provider_call: None,
            model_observation: None,
        })
        .await
        .expect_err("persistent backend failure remains terminal");

    assert_eq!(error.kind, AgentLoopHostErrorKind::TranscriptWriteFailed);
    assert_eq!(service.attempts(), 3);
}

#[tokio::test]
async fn transcript_port_appends_tool_result_reference_envelope_idempotently() {
    let fixture = ThreadFixture::new().await;
    let adapter = ThreadBackedLoopTranscriptPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
    );
    let result_ref = LoopResultRef::new("result:demo-tool").unwrap();
    // Provider validation has accepted metadata above 4 KiB since #5001.
    // This exercises the transcript caller that used to retain the stale
    // 4 KiB bound and terminate otherwise successful reasoning-heavy runs.
    let response_reasoning = "response reasoning ".repeat(256);
    let call_reasoning = "call reasoning ".repeat(320);
    assert!(response_reasoning.len() > 4096);
    assert!(call_reasoning.len() > 4096);

    let first_ref = adapter
        .append_capability_result_ref(AppendCapabilityResultRef {
            result_ref: result_ref.clone(),
            safe_summary: "tool completed".to_string(),
            provider_call: Some(ProviderToolCallReference {
                replay: ProviderToolCallReplay {
                    provider_id: "test-provider".to_string(),
                    provider_model_id: "test-model".to_string(),
                    provider_turn_id: "turn_1".to_string(),
                    provider_call_id: "call_1".to_string(),
                    provider_tool_name: ProviderToolName::new("demo__echo")
                        .expect("provider tool name"),
                    arguments: serde_json::json!({"message":"hello"}),
                    response_reasoning: Some(response_reasoning.clone()),
                    reasoning: Some(call_reasoning.clone()),
                    signature: Some("sig-1".to_string()),
                },
                capability_id: CapabilityId::new("demo.echo").unwrap(),
            }),
            model_observation: None,
        })
        .await
        .unwrap();
    let second_ref = adapter
        .append_capability_result_ref(AppendCapabilityResultRef {
            result_ref: result_ref.clone(),
            safe_summary: "retry summary ignored".to_string(),
            provider_call: None,
            model_observation: None,
        })
        .await
        .unwrap();

    assert_eq!(first_ref, second_ref);
    let history = fixture
        .thread_service
        .list_thread_history(ThreadHistoryRequest {
            scope: fixture.thread_scope.clone(),
            thread_id: fixture.thread_id.clone(),
        })
        .await
        .unwrap();
    let records = history
        .messages
        .iter()
        .filter(|message| message.kind == MessageKind::ToolResultReference)
        .collect::<Vec<_>>();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].status, MessageStatus::Finalized);
    assert_eq!(
        records[0].tool_result_ref.as_deref(),
        Some(result_ref.as_str())
    );
    let envelope: ToolResultReferenceEnvelope =
        serde_json::from_str(records[0].content.as_deref().unwrap()).unwrap();
    assert_eq!(envelope.version, 1);
    assert_eq!(envelope.result_ref, result_ref.as_str());
    assert_eq!(envelope.safe_summary.as_str(), "tool completed");
    assert!(records[0].tool_result_provider_call.is_none());
    let context = fixture
        .thread_service
        .load_context_window(ironclaw_threads::LoadContextWindowRequest {
            scope: fixture.thread_scope.clone(),
            thread_id: fixture.thread_id.clone(),
            max_messages: 16,
        })
        .await
        .unwrap();
    let context_record = context
        .messages
        .iter()
        .find(|message| message.kind == MessageKind::ToolResultReference)
        .expect("tool result reference context");
    let provider_call = context_record
        .tool_result_provider_call
        .as_ref()
        .expect("provider call metadata");
    assert_eq!(provider_call.provider_turn_id, "turn_1");
    assert_eq!(provider_call.provider_call_id, "call_1");
    assert_eq!(provider_call.provider_tool_name.as_str(), "demo__echo");
    assert_eq!(provider_call.capability_id.as_str(), "demo.echo");
    assert_eq!(
        provider_call.arguments,
        serde_json::json!({"message":"hello"})
    );
    assert_eq!(
        provider_call.response_reasoning.as_deref(),
        Some(response_reasoning.as_str())
    );
    assert_eq!(
        provider_call.reasoning.as_deref(),
        Some(call_reasoning.as_str())
    );
    assert_eq!(provider_call.signature.as_deref(), Some("sig-1"));
}

#[tokio::test]
async fn transcript_port_appends_model_observation_in_tool_result_reference_envelope() {
    let fixture = ThreadFixture::new().await;
    let adapter = ThreadBackedLoopTranscriptPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
    );
    let result_ref = LoopResultRef::new("result:model-observation-tool").unwrap();
    let observation = ModelVisibleToolObservation {
        schema_version: 1,
        status: ToolObservationStatus::Error,
        summary: "Tool input failed schema validation.".to_string(),
        detail: ToolObservationDetail::InvalidInput {
            issues: vec![CapabilityInputIssue {
                path: "file_path".to_string(),
                code: DispatchInputIssueCode::MissingRequired,
                expected: Some("required field".to_string()),
                received: None,
                schema_path: Some("required".to_string()),
            }],
        },
        artifacts: Vec::new(),
        recovery: None,
        trust: ObservationTrust::UntrustedToolOutput,
    };

    adapter
        .append_capability_result_ref(AppendCapabilityResultRef {
            result_ref: result_ref.clone(),
            safe_summary: "tool failed".to_string(),
            provider_call: None,
            model_observation: Some(observation.clone()),
        })
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
    let record = history
        .messages
        .iter()
        .find(|message| message.kind == MessageKind::ToolResultReference)
        .expect("tool result reference");
    let envelope = ToolResultReferenceEnvelope::from_json_str(record.content.as_deref().unwrap())
        .expect("valid tool result reference envelope");

    assert_eq!(envelope.result_ref, result_ref.as_str());
    assert_eq!(
        envelope.model_observation,
        Some(serde_json::to_value(observation).unwrap())
    );
}

#[tokio::test]
async fn transcript_port_drops_invalid_model_observation_without_failing_append() {
    let fixture = ThreadFixture::new().await;
    let adapter = ThreadBackedLoopTranscriptPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
    );
    let result_ref = LoopResultRef::new("result:invalid-model-observation-tool").unwrap();
    let observation = ModelVisibleToolObservation {
        schema_version: 999,
        status: ToolObservationStatus::Error,
        summary: "Tool input failed schema validation.".to_string(),
        detail: ToolObservationDetail::InvalidInput { issues: Vec::new() },
        artifacts: Vec::new(),
        recovery: None,
        trust: ObservationTrust::UntrustedToolOutput,
    };

    adapter
        .append_capability_result_ref(AppendCapabilityResultRef {
            result_ref: result_ref.clone(),
            safe_summary: "tool failed".to_string(),
            provider_call: None,
            model_observation: Some(observation),
        })
        .await
        .expect("invalid model observation should not fail append");

    let history = fixture
        .thread_service
        .list_thread_history(ThreadHistoryRequest {
            scope: fixture.thread_scope.clone(),
            thread_id: fixture.thread_id.clone(),
        })
        .await
        .unwrap();
    let record = history
        .messages
        .iter()
        .find(|message| message.tool_result_ref.as_deref() == Some(result_ref.as_str()))
        .expect("tool result reference message");
    let envelope = ToolResultReferenceEnvelope::from_json_str(record.content.as_deref().unwrap())
        .expect("valid tool result reference envelope");

    assert_eq!(envelope.safe_summary.as_str(), "tool failed");
    assert!(envelope.model_observation.is_none());
}

/// Issue #5838: a control character in a `ResultReference` preview must
/// degrade to ref-only (drop `preview`, keep `result_ref`) through the real
/// `append_capability_result_ref` gate, not lose the whole observation. This
/// pins the boundary between this crate's neutral pre-check (shape only,
/// `model_observation.rs`) and `ironclaw_threads`'s canonical content scan
/// (secret markers/control chars, with graceful degrade).
#[tokio::test]
async fn transcript_port_degrades_control_char_result_reference_preview_without_dropping_ref() {
    let fixture = ThreadFixture::new().await;
    let adapter = ThreadBackedLoopTranscriptPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
    );
    let result_ref = LoopResultRef::new("result:control-char-preview-tool").unwrap();
    let observation = ModelVisibleToolObservation {
        schema_version: 1,
        status: ToolObservationStatus::Success,
        summary: "Tool completed; preview available.".to_string(),
        detail: ToolObservationDetail::ResultReference {
            result_ref: result_ref.as_str().to_string(),
            byte_len: 16,
            preview: Some("bad\u{0}null and \u{7}bell".to_string()),
            total_bytes: Some(16),
            next_offset: None,
            item_count: None,
        },
        artifacts: Vec::new(),
        recovery: None,
        trust: ObservationTrust::UntrustedToolOutput,
    };

    adapter
        .append_capability_result_ref(AppendCapabilityResultRef {
            result_ref: result_ref.clone(),
            safe_summary: "tool completed".to_string(),
            provider_call: None,
            model_observation: Some(observation),
        })
        .await
        .expect("control-char preview should not fail append");

    let history = fixture
        .thread_service
        .list_thread_history(ThreadHistoryRequest {
            scope: fixture.thread_scope.clone(),
            thread_id: fixture.thread_id.clone(),
        })
        .await
        .unwrap();
    let record = history
        .messages
        .iter()
        .find(|message| message.tool_result_ref.as_deref() == Some(result_ref.as_str()))
        .expect("tool result reference message");
    let envelope = ToolResultReferenceEnvelope::from_json_str(record.content.as_deref().unwrap())
        .expect("valid tool result reference envelope");

    let observation = envelope
        .model_observation
        .expect("result reference observation is retained, not fully dropped");
    assert_eq!(
        observation["detail"]["result_ref"],
        result_ref.as_str(),
        "result_ref must survive so the model can still call result_read"
    );
    assert!(
        observation["detail"].get("preview").is_none(),
        "the unsafe preview must be stripped, not merely truncated"
    );
}

/// A `ResultReference` observation carrying `item_count` (truncated
/// top-level-array preview) must persist intact through the real
/// `append_capability_result_ref` gate — a persistence-side allowlist that
/// rejects the field silently drops the WHOLE observation, not just the
/// count.
#[tokio::test]
async fn transcript_port_persists_result_reference_item_count() {
    let fixture = ThreadFixture::new().await;
    let adapter = ThreadBackedLoopTranscriptPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
    );
    let result_ref = LoopResultRef::new("result:array-item-count-tool").unwrap();
    let observation = ModelVisibleToolObservation {
        schema_version: 1,
        status: ToolObservationStatus::Success,
        summary: "Tool completed; preview truncated. Full result is a JSON array of 600 items."
            .to_string(),
        detail: ToolObservationDetail::ResultReference {
            result_ref: result_ref.as_str().to_string(),
            byte_len: 8_192,
            preview: Some("[\"item-0000\",\"item-0001\"".to_string()),
            total_bytes: Some(8_192),
            next_offset: Some(2_048),
            item_count: Some(600),
        },
        artifacts: Vec::new(),
        recovery: None,
        trust: ObservationTrust::UntrustedToolOutput,
    };

    adapter
        .append_capability_result_ref(AppendCapabilityResultRef {
            result_ref: result_ref.clone(),
            safe_summary: "tool completed".to_string(),
            provider_call: None,
            model_observation: Some(observation),
        })
        .await
        .expect("item_count observation should not fail append");

    let history = fixture
        .thread_service
        .list_thread_history(ThreadHistoryRequest {
            scope: fixture.thread_scope.clone(),
            thread_id: fixture.thread_id.clone(),
        })
        .await
        .unwrap();
    let record = history
        .messages
        .iter()
        .find(|message| message.tool_result_ref.as_deref() == Some(result_ref.as_str()))
        .expect("tool result reference message");
    let envelope = ToolResultReferenceEnvelope::from_json_str(record.content.as_deref().unwrap())
        .expect("valid tool result reference envelope");

    let observation = envelope
        .model_observation
        .expect("item_count observation is retained, not silently dropped");
    assert_eq!(
        observation["detail"]["item_count"], 600,
        "the structured array count must survive persistence"
    );
    assert_eq!(
        observation["detail"]["next_offset"], 2_048,
        "continuation metadata must survive alongside item_count"
    );
}

#[tokio::test]
async fn transcript_port_degrades_unsafe_tool_result_summary_without_borking() {
    let fixture = ThreadFixture::new().await;
    let adapter = ThreadBackedLoopTranscriptPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
    );

    // A summary that trips the strict validator (credential marker here; a
    // path delimiter behaves the same) must NOT end the run: the result
    // reference is still written with a fixed redaction marker as its label,
    // and the raw rejected text never reaches the transcript.
    adapter
        .append_capability_result_ref(AppendCapabilityResultRef {
            result_ref: LoopResultRef::new("result:unsafe-tool").unwrap(),
            safe_summary: "raw tool input includes secret".to_string(),
            provider_call: None,
            model_observation: None,
        })
        .await
        .expect("unsafe summary must degrade to a fixed label, not end the run");

    let history = fixture
        .thread_service
        .list_thread_history(ThreadHistoryRequest {
            scope: fixture.thread_scope.clone(),
            thread_id: fixture.thread_id.clone(),
        })
        .await
        .unwrap();
    let reference = history
        .messages
        .iter()
        .find(|message| message.kind == MessageKind::ToolResultReference)
        .expect("result reference must be written despite the unsafe summary");
    let wire = serde_json::to_string(&reference).unwrap();
    assert!(
        !wire.contains("raw tool input includes secret"),
        "raw rejected summary must not reach the transcript: {wire}"
    );
    assert!(
        wire.contains("the tool result summary was redacted"),
        "degraded label must be the fixed redaction marker: {wire}"
    );
}

#[tokio::test]
async fn transcript_port_emits_assistant_reply_finalized_milestone_without_reply_content() {
    let fixture = ThreadFixture::new().await;
    let milestone_sink = Arc::new(InMemoryLoopHostMilestoneSink::default());
    let adapter = ThreadBackedLoopTranscriptPort::with_milestone_sink(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
        milestone_sink.clone(),
    );

    let message_ref = adapter
        .finalize_assistant_message(FinalizeAssistantMessage {
            reply: AssistantReply {
                content: "RAW_ASSISTANT_CONTENT_SENTINEL sk-reply-secret /host/path tool_input"
                    .to_string(),
            },
        })
        .await
        .unwrap();

    let milestones = milestone_sink.milestones();
    assert_eq!(milestones.len(), 1);
    assert!(matches!(
        &milestones[0].kind,
        LoopHostMilestoneKind::AssistantReplyFinalized { message_ref: finalized_ref }
            if finalized_ref == &message_ref
    ));
    let wire = serde_json::to_string(&milestones).unwrap();
    assert!(!wire.contains("RAW_ASSISTANT_CONTENT_SENTINEL"));
    assert!(!wire.contains("sk-reply-secret"));
    assert!(!wire.contains("/host/path"));
    assert!(!wire.contains("tool_input"));
}

#[traced_test]
#[tokio::test]
async fn transcript_port_keeps_finalized_reply_successful_after_milestone_sink_failure() {
    let fixture = ThreadFixture::new().await;
    let milestone_sink = Arc::new(FailOnceMilestoneSink::default());
    let adapter = ThreadBackedLoopTranscriptPort::with_milestone_sink(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
        milestone_sink.clone(),
    );
    let request = FinalizeAssistantMessage {
        reply: AssistantReply {
            content: "retryable milestone failure".to_string(),
        },
    };

    let first_ref = adapter
        .finalize_assistant_message(request.clone())
        .await
        .unwrap();
    assert!(milestone_sink.milestones().is_empty());
    assert!(logs_contain(
        "loop assistant_reply_finalized milestone failed after finalized transcript write"
    ));

    let message_ref = adapter.finalize_assistant_message(request).await.unwrap();
    assert_eq!(first_ref, message_ref);

    let milestones = milestone_sink.milestones();
    assert_eq!(milestones.len(), 1);
    assert!(matches!(
        &milestones[0].kind,
        LoopHostMilestoneKind::AssistantReplyFinalized { message_ref: finalized_ref }
            if finalized_ref == &message_ref
    ));
    let history = fixture
        .thread_service
        .list_thread_history(ThreadHistoryRequest {
            scope: fixture.thread_scope.clone(),
            thread_id: fixture.thread_id.clone(),
        })
        .await
        .unwrap();
    let finalized = history
        .messages
        .iter()
        .filter(|message| message.kind == MessageKind::Assistant)
        .collect::<Vec<_>>();
    assert_eq!(finalized.len(), 1);
}

#[tokio::test]
async fn transcript_port_finalize_is_idempotent_for_matching_reply() {
    let fixture = ThreadFixture::new().await;
    let milestone_sink = Arc::new(InMemoryLoopHostMilestoneSink::default());
    let adapter = ThreadBackedLoopTranscriptPort::with_milestone_sink(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
        milestone_sink.clone(),
    );
    let request = FinalizeAssistantMessage {
        reply: AssistantReply {
            content: "idempotent reply RAW_IDEMPOTENT_REPLY_SENTINEL".to_string(),
        },
    };

    let first_ref = adapter
        .finalize_assistant_message(request.clone())
        .await
        .unwrap();
    let second_ref = adapter.finalize_assistant_message(request).await.unwrap();

    assert_eq!(first_ref, second_ref);
    let milestones = milestone_sink.milestones();
    assert_eq!(milestones.len(), 1);
    assert!(matches!(
        &milestones[0].kind,
        LoopHostMilestoneKind::AssistantReplyFinalized { message_ref }
            if message_ref == &first_ref
    ));
    assert!(
        !serde_json::to_string(&milestones)
            .unwrap()
            .contains("RAW_IDEMPOTENT_REPLY_SENTINEL")
    );
    let history = fixture
        .thread_service
        .list_thread_history(ThreadHistoryRequest {
            scope: fixture.thread_scope.clone(),
            thread_id: fixture.thread_id.clone(),
        })
        .await
        .unwrap();
    let finalized = history
        .messages
        .iter()
        .filter(|message| message.kind == MessageKind::Assistant)
        .collect::<Vec<_>>();
    assert_eq!(finalized.len(), 1);
    assert_eq!(finalized[0].status, MessageStatus::Finalized);
    assert_eq!(
        finalized[0].content.as_deref(),
        Some("idempotent reply RAW_IDEMPOTENT_REPLY_SENTINEL")
    );
}

#[tokio::test]
async fn transcript_port_finalize_is_idempotent_under_concurrent_duplicate_calls() {
    let fixture = GatedThreadFixture::new().await;
    let milestone_sink = Arc::new(InMemoryLoopHostMilestoneSink::default());
    let adapter = ThreadBackedLoopTranscriptPort::with_milestone_sink(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
        milestone_sink.clone(),
    );
    let request = FinalizeAssistantMessage {
        reply: AssistantReply {
            content: "concurrent reply".to_string(),
        },
    };

    let (first, second) = tokio::join!(
        adapter.finalize_assistant_message(request.clone()),
        adapter.finalize_assistant_message(request),
    );
    let first_ref = first.unwrap();
    let second_ref = second.unwrap();

    assert_eq!(first_ref, second_ref);
    let milestones = milestone_sink.milestones();
    assert_eq!(milestones.len(), 1);
    assert!(matches!(
        &milestones[0].kind,
        LoopHostMilestoneKind::AssistantReplyFinalized { message_ref }
            if message_ref == &first_ref
    ));
    let history = fixture
        .thread_service
        .list_thread_history(ThreadHistoryRequest {
            scope: fixture.thread_scope.clone(),
            thread_id: fixture.thread_id.clone(),
        })
        .await
        .unwrap();
    let finalized = history
        .messages
        .iter()
        .filter(|message| message.kind == MessageKind::Assistant)
        .collect::<Vec<_>>();
    assert_eq!(finalized.len(), 1);
    assert_eq!(finalized[0].status, MessageStatus::Finalized);
    assert_eq!(finalized[0].content.as_deref(), Some("concurrent reply"));
}

#[tokio::test]
async fn transcript_port_rejects_draft_updates_from_other_runs() {
    let fixture = ThreadFixture::new().await;
    let run_a = ThreadBackedLoopTranscriptPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
    );
    let draft_ref = run_a
        .begin_assistant_draft(BeginAssistantDraft {
            reply: AssistantReply {
                content: "run A draft".to_string(),
            },
        })
        .await
        .unwrap();
    let mut run_b_context = fixture.run_context.clone();
    run_b_context.run_id = TurnRunId::new();
    let run_b = ThreadBackedLoopTranscriptPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        run_b_context,
    );

    let error = run_b
        .update_assistant_draft(UpdateAssistantDraft {
            message_ref: draft_ref,
            reply: AssistantReply {
                content: "run B overwrite".to_string(),
            },
        })
        .await
        .unwrap_err();

    assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
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
        .expect("assistant draft must exist");
    assert_eq!(assistant.content.as_deref(), Some("run A draft"));
}

#[tokio::test]
async fn empty_capability_port_exposes_empty_surface_and_rejects_invocations() {
    let port = EmptyLoopCapabilityPort;

    let surface = port
        .visible_capabilities(VisibleCapabilityRequest)
        .await
        .unwrap();
    assert_eq!(surface.version.as_str(), "empty:v1");
    assert!(surface.descriptors.is_empty());

    let error = port
        .invoke_capability(LoopRequest {
            activity_id: ironclaw_turns::CapabilityActivityId::new(),
            surface_version: CapabilitySurfaceVersion::new("empty:v1").unwrap(),
            capability_id: CapabilityId::new("demo.echo").unwrap(),
            input_ref: CapabilityInputRef::new("input:opaque").unwrap(),
            approval_resume: None,
            auth_resume: None,
        })
        .await
        .unwrap_err();

    assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
    assert!(!serde_json::to_string(&error).unwrap().contains("opaque"));
}

#[tokio::test]
async fn empty_capability_batch_returns_typed_denial_reason() {
    let port = EmptyLoopCapabilityPort;

    let outcome = port
        .invoke_capability_batch(ironclaw_loop_contracts::LoopRequestBatch {
            invocations: vec![LoopRequest {
                activity_id: ironclaw_turns::CapabilityActivityId::new(),
                surface_version: CapabilitySurfaceVersion::new("empty:v1").unwrap(),
                capability_id: CapabilityId::new("demo.echo").unwrap(),
                input_ref: CapabilityInputRef::new("input:opaque").unwrap(),
                approval_resume: None,
                auth_resume: None,
            }],
            stop_on_first_suspension: true,
        })
        .await
        .unwrap();

    // The loop-side `EmptySurface` reason has no host_api `DenyReason` tag, so the
    // §5.3 collapse buckets it into `PolicyDenied`; the empty-surface specificity
    // survives on the denial summary instead of the typed reason.
    assert!(matches!(
        outcome.resolutions.as_slice(),
        [Resolution::Denied(denied)]
            if denied.reason_kind == Some(DenyReason::PolicyDenied)
                && denied
                    .summary
                    .as_ref()
                    .is_some_and(|summary| summary.as_str().contains("capabilities"))
    ));
}

#[tokio::test]
async fn empty_capability_batch_rejects_stale_surface() {
    let port = EmptyLoopCapabilityPort;

    let error = port
        .invoke_capability_batch(ironclaw_loop_contracts::LoopRequestBatch {
            invocations: vec![LoopRequest {
                activity_id: ironclaw_turns::CapabilityActivityId::new(),
                surface_version: CapabilitySurfaceVersion::new("nonempty:v1").unwrap(),
                capability_id: CapabilityId::new("demo.echo").unwrap(),
                input_ref: CapabilityInputRef::new("input:opaque").unwrap(),
                approval_resume: None,
                auth_resume: None,
            }],
            stop_on_first_suspension: true,
        })
        .await
        .unwrap_err();

    assert_eq!(error.kind, AgentLoopHostErrorKind::StaleSurface);
}

#[tokio::test]
async fn model_port_resolves_thread_message_refs_and_delegates_to_gateway() {
    let fixture = ThreadFixture::new().await;
    let gateway = Arc::new(RecordingGateway::reply("model says hi"));
    let port = ThreadBackedLoopModelPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
        gateway.clone(),
        16,
    );
    let messages = user_model_messages(&fixture);
    issue_prompt_grant(&fixture.run_context, &messages);

    let response = port
        .stream_model(LoopModelRequest {
            inline_messages: Vec::new(),
            messages,
            surface_version: None,
            model_preference: None,
            fallback_index: 0,
            iteration: 0,
            capability_view: None,
        })
        .await
        .unwrap();

    assert_eq!(response.chunks[0].safe_text_delta, "model says hi");
    assert_eq!(
        response.effective_model_profile_id.as_str(),
        "interactive_model"
    );
    assert!(matches!(
        response.output,
        ParentLoopOutput::AssistantReply(AssistantReply { ref content }) if content == "model says hi"
    ));
    let calls = gateway.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].model_profile_id.as_str(), "interactive_model");
    assert_eq!(calls[0].run_id, fixture.run_context.run_id);
    assert_eq!(calls[0].turn_id, fixture.run_context.turn_id);
    assert_eq!(calls[0].messages[0].role, HostManagedModelMessageRole::User);
    assert_eq!(calls[0].messages[0].content, "hello reborn");
}

#[tokio::test]
async fn model_port_rejects_mismatched_fallback_route_evidence() {
    let fixture = ThreadFixture::new().await;
    let milestone_sink = Arc::new(InMemoryLoopHostMilestoneSink::default());
    let gateway = Arc::new(RecordingGateway::reply_with_fallback("model says hi", 1));
    let port = ThreadBackedLoopModelPort::with_milestone_sink(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
        gateway.clone(),
        16,
        milestone_sink.clone(),
    );
    let messages = user_model_messages(&fixture);
    issue_prompt_grant(&fixture.run_context, &messages);

    let error = port
        .stream_model(LoopModelRequest {
            inline_messages: Vec::new(),
            messages,
            surface_version: None,
            model_preference: None,
            fallback_index: 2,
            iteration: 0,
            capability_view: None,
        })
        .await
        .unwrap_err();

    assert_eq!(error.kind, AgentLoopHostErrorKind::Internal);
    assert_eq!(
        error.safe_summary,
        "model gateway returned mismatched fallback route evidence"
    );
    let calls = gateway.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].fallback_index, 2);
    let milestones = milestone_sink.milestones();
    assert_eq!(milestones.len(), 2);
    assert!(matches!(
        &milestones[1].kind,
        LoopHostMilestoneKind::ModelFailed {
            reason_kind: AgentLoopHostErrorKind::Internal
        }
    ));
}

#[tokio::test]
async fn model_port_rejects_missing_fallback_route_evidence() {
    let fixture = ThreadFixture::new().await;
    let gateway = Arc::new(RecordingGateway::reply_without_fallback_evidence(
        "model says hi",
    ));
    let port = ThreadBackedLoopModelPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
        gateway,
        16,
    );
    let messages = user_model_messages(&fixture);
    issue_prompt_grant(&fixture.run_context, &messages);

    let error = port
        .stream_model(LoopModelRequest {
            inline_messages: Vec::new(),
            messages,
            surface_version: None,
            model_preference: None,
            fallback_index: 0,
            iteration: 0,
            capability_view: None,
        })
        .await
        .unwrap_err();

    assert_eq!(error.kind, AgentLoopHostErrorKind::Internal);
    assert_eq!(
        error.safe_summary,
        "model gateway returned mismatched fallback route evidence"
    );
}

#[tokio::test]
async fn model_port_accepts_matching_fallback_route_evidence() {
    let fixture = ThreadFixture::new().await;
    let gateway = Arc::new(RecordingGateway::reply_with_fallback(
        "fallback model says hi",
        2,
    ));
    let port = ThreadBackedLoopModelPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
        gateway.clone(),
        16,
    );
    let messages = user_model_messages(&fixture);
    issue_prompt_grant(&fixture.run_context, &messages);

    let response = port
        .stream_model(LoopModelRequest {
            inline_messages: Vec::new(),
            messages,
            surface_version: None,
            model_preference: None,
            fallback_index: 2,
            iteration: 0,
            capability_view: None,
        })
        .await
        .unwrap();

    assert_eq!(response.chunks[0].safe_text_delta, "fallback model says hi");
    let calls = gateway.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].fallback_index, 2);
}

/// Records every storage key the model port asks it to read and returns a fixed
/// byte payload, so a test can assert the producer (`read_image_parts`) both
/// consulted the read port and threaded the raw bytes it returned.
struct StubImageReader {
    bytes: Vec<u8>,
    reads: Mutex<Vec<String>>,
}

#[async_trait]
impl LoopAttachmentReadPort for StubImageReader {
    async fn read_attachment_bytes(
        &self,
        _scope: &ResourceScope,
        storage_key: &str,
    ) -> Result<Vec<u8>, LoopAttachmentReadError> {
        self.reads.lock().unwrap().push(storage_key.to_string());
        Ok(self.bytes.clone())
    }
}

/// Producer-side coverage for the image-vision path: a landed image attachment
/// on a resolved user message must be read back through the
/// [`LoopAttachmentReadPort`] and threaded to the gateway as a base64 image
/// part. The consumer side (`convert_messages` -> `ContentPart::ImageUrl`) is
/// unit-tested in `ironclaw_turn_runner`; this closes the loop on the read side per
/// the "test through the caller" rule (the read port gates a side effect with
/// the model port wrapper between).
#[tokio::test]
async fn model_port_reads_image_attachment_bytes_into_model_image_parts() {
    let fixture = ThreadFixture::new().await;
    let image = AttachmentRef {
        id: "att-img-0".to_string(),
        kind: AttachmentKind::Image,
        mime_type: "image/png".to_string(),
        filename: Some("diagram.png".to_string()),
        size_bytes: Some(4),
        storage_key: Some("/workspace/attachments/2026-06-14/m1-0-diagram.png".to_string()),
        extracted_text: None,
    };
    let accepted = fixture
        .thread_service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: fixture.thread_scope.clone(),
            thread_id: fixture.thread_id.clone(),
            actor_id: "user-loop-support".to_string(),
            source_binding_id: Some("source-web".to_string()),
            reply_target_binding_id: Some("reply-web".to_string()),
            external_event_id: Some("event-image".to_string()),
            content: MessageContent::with_attachments("look at this", vec![image]),
        })
        .await
        .unwrap();

    let reader = Arc::new(StubImageReader {
        bytes: vec![1, 2, 3, 4],
        reads: Mutex::new(Vec::new()),
    });
    let gateway = Arc::new(RecordingGateway::reply("looks like a diagram"));
    let port = ThreadBackedLoopModelPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
        gateway.clone(),
        16,
    )
    .with_attachment_read_port(reader.clone());

    let messages = vec![LoopModelMessage {
        role: "user".to_string(),
        content_ref: LoopMessageRef::new(format!("msg:{}", accepted.message_id)).unwrap(),
    }];
    issue_prompt_grant(&fixture.run_context, &messages);

    port.stream_model(LoopModelRequest {
        inline_messages: Vec::new(),
        messages,
        surface_version: None,
        model_preference: None,
        fallback_index: 0,
        iteration: 0,
        capability_view: None,
    })
    .await
    .unwrap();

    // The read port was consulted exactly once, for the landed storage key.
    assert_eq!(
        reader.reads.lock().unwrap().as_slice(),
        &["/workspace/attachments/2026-06-14/m1-0-diagram.png".to_string()]
    );

    // The producer threaded the raw bytes the reader returned to the gateway as
    // a typed image part on the resolved user message (base64 encoding happens
    // later, in the gateway, and only for a vision model).
    let calls = gateway.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    let image_parts = &calls[0].messages[0].image_parts;
    assert_eq!(image_parts.len(), 1);
    assert_eq!(image_parts[0].mime_type, "image/png");
    assert_eq!(image_parts[0].bytes, vec![1, 2, 3, 4]);
}

#[tokio::test]
async fn model_port_merges_consecutive_text_user_messages_for_prompt() {
    let fixture = ThreadFixture::new_with_user_content("first follow-up").await;
    fixture
        .accept_user_message("event-2", "second follow-up")
        .await;
    fixture
        .accept_user_message("event-3", "third follow-up")
        .await;

    let gateway = Arc::new(RecordingGateway::reply("merged"));
    let port = ThreadBackedLoopModelPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
        gateway.clone(),
        16,
    );
    issue_prompt_grant(&fixture.run_context, &[]);

    port.stream_model(LoopModelRequest {
        messages: Vec::new(),
        inline_messages: Vec::new(),
        surface_version: None,
        model_preference: None,
        capability_view: None,
        fallback_index: 0,
        iteration: 0,
    })
    .await
    .unwrap();

    let messages = {
        let calls = gateway.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        calls[0].messages.clone()
    };
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].role, HostManagedModelMessageRole::User);
    assert_eq!(
        messages[0].content,
        "first follow-up\nsecond follow-up\nthird follow-up"
    );

    let history = fixture
        .thread_service
        .list_thread_history(ThreadHistoryRequest {
            scope: fixture.thread_scope.clone(),
            thread_id: fixture.thread_id.clone(),
        })
        .await
        .unwrap();
    let user_rows = history
        .messages
        .iter()
        .filter(|message| message.kind == MessageKind::User)
        .count();
    assert_eq!(user_rows, 3, "durable transcript rows stay separate");
}

#[tokio::test]
async fn model_port_threads_resolved_model_route_snapshot_to_gateway() {
    let fixture = ThreadFixture::new().await;
    let snapshot = LoopModelRouteSnapshot::new("anthropic", "claude-opus-4", "cfg-1", "auth-1");
    let run_context = fixture
        .run_context
        .clone()
        .with_resolved_model_route(snapshot.clone());
    let gateway = Arc::new(RecordingGateway::reply("model says hi"));
    let port = ThreadBackedLoopModelPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        run_context,
        gateway.clone(),
        16,
    );
    let messages = user_model_messages(&fixture);
    issue_prompt_grant(&fixture.run_context, &messages);

    port.stream_model(LoopModelRequest {
        inline_messages: Vec::new(),
        messages,
        surface_version: None,
        model_preference: None,
        fallback_index: 0,
        iteration: 0,
        capability_view: None,
    })
    .await
    .unwrap();

    let calls = gateway.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].resolved_model_route, Some(snapshot));
}

#[tokio::test]
async fn model_port_resolves_explicit_refs_that_fall_outside_context_window() {
    let fixture = ThreadFixture::new().await;
    for index in 0..3 {
        fixture
            .thread_service
            .accept_inbound_message(AcceptInboundMessageRequest {
                scope: fixture.thread_scope.clone(),
                thread_id: fixture.thread_id.clone(),
                actor_id: "user-loop-support".to_string(),
                source_binding_id: Some("source-web".to_string()),
                reply_target_binding_id: Some("reply-web".to_string()),
                external_event_id: Some(format!("event-extra-{index}")),
                content: MessageContent::text(format!("newer message {index}")),
            })
            .await
            .unwrap();
    }
    let gateway = Arc::new(RecordingGateway::reply("model says hi"));
    let port = ThreadBackedLoopModelPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
        gateway.clone(),
        1,
    );
    let messages = user_model_messages(&fixture);
    issue_prompt_grant(&fixture.run_context, &messages);

    port.stream_model(LoopModelRequest {
        inline_messages: Vec::new(),
        messages,
        surface_version: None,
        model_preference: None,
        fallback_index: 0,
        iteration: 0,
        capability_view: None,
    })
    .await
    .unwrap();

    let calls = gateway.calls.lock().unwrap();
    assert_eq!(calls[0].messages[0].content, "hello reborn");
}

#[tokio::test]
async fn model_port_preserves_provider_metadata_for_explicit_refs_outside_context_window() {
    let fixture = ThreadFixture::new().await;
    let tool_result = fixture
        .thread_service
        .append_tool_result_reference(AppendToolResultReferenceRequest {
            scope: fixture.thread_scope.clone(),
            thread_id: fixture.thread_id.clone(),
            turn_run_id: fixture.run_context.run_id.to_string(),
            result_ref: "result:old-provider-tool".to_string(),
            safe_summary: ToolResultSafeSummary::new("old provider tool completed").unwrap(),
            provider_call: Some(ProviderToolCallReferenceEnvelope {
                provider_id: "test-provider".to_string(),
                provider_model_id: "test-model".to_string(),
                provider_turn_id: "turn_1".to_string(),
                provider_call_id: "call_1".to_string(),
                provider_tool_name: ProviderToolName::new("demo__echo")
                    .expect("provider tool name"),
                capability_id: CapabilityId::new("demo.echo").unwrap(),
                arguments: serde_json::json!({"message":"hello"}),
                response_reasoning: Some("provider response reasoning".to_string()),
                reasoning: Some("provider call reasoning".to_string()),
                signature: Some("sig-1".to_string()),
            }),
            model_observation: None,
        })
        .await
        .unwrap();
    for index in 0..3 {
        fixture
            .thread_service
            .accept_inbound_message(AcceptInboundMessageRequest {
                scope: fixture.thread_scope.clone(),
                thread_id: fixture.thread_id.clone(),
                actor_id: "user-loop-support".to_string(),
                source_binding_id: Some("source-web".to_string()),
                reply_target_binding_id: Some("reply-web".to_string()),
                external_event_id: Some(format!("event-after-tool-{index}")),
                content: MessageContent::text(format!("newer message {index}")),
            })
            .await
            .unwrap();
    }
    let gateway = Arc::new(RecordingGateway::reply("model says hi"));
    let port = ThreadBackedLoopModelPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
        gateway.clone(),
        1,
    );
    let messages = vec![LoopModelMessage {
        role: "tool_result_reference".to_string(),
        content_ref: LoopMessageRef::new(format!("msg:{}", tool_result.message_id)).unwrap(),
    }];
    issue_prompt_grant(&fixture.run_context, &messages);

    port.stream_model(LoopModelRequest {
        inline_messages: Vec::new(),
        messages,
        surface_version: None,
        model_preference: None,
        fallback_index: 0,
        iteration: 0,
        capability_view: None,
    })
    .await
    .unwrap();

    let calls = gateway.calls.lock().unwrap();
    let provider_call = calls[0].messages[0]
        .tool_result_provider_call
        .as_ref()
        .expect("model fallback preserves provider metadata");
    assert_eq!(provider_call.provider_id, "test-provider");
    assert_eq!(provider_call.provider_model_id, "test-model");
    assert_eq!(provider_call.provider_call_id, "call_1");
    assert_eq!(provider_call.provider_tool_name.as_str(), "demo__echo");
}

#[tokio::test]
async fn prompt_port_builds_bundle_with_tool_result_reference_context() {
    let fixture = ThreadFixture::new().await;
    let tool_result_ref = LoopMessageRef::new("msg:11111111-1111-1111-1111-111111111111").unwrap();
    let thread_service = Arc::new(StaticContextThreadService::new(ContextMessage {
        message_id: Some(ThreadMessageId::parse("11111111-1111-1111-1111-111111111111").unwrap()),
        summary_id: None,
        sequence: 1,
        kind: MessageKind::ToolResultReference,
        tool_result_provider_call: None,
        content: "tool result content".to_string(),
        image_attachments: Vec::new(),
    }));
    let context_port = Arc::new(ThreadBackedLoopContextPort::new(
        thread_service,
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
        16,
    ));
    let prompt_port = HostManagedLoopPromptPort::new(
        fixture.run_context.clone(),
        context_port,
        Arc::new(InMemoryLoopHostMilestoneSink::default()),
    );

    let prompt_bundle = prompt_port
        .build_prompt_bundle(ironclaw_loop_contracts::LoopPromptBundleRequest {
            mode: ironclaw_loop_contracts::PromptMode::TextOnly,
            context_cursor: None,
            surface_version: None,
            checkpoint_state_ref: None,
            max_messages: None,
            inline_messages: Vec::new(),
            capability_view: None,
        })
        .await
        .unwrap();

    assert_eq!(prompt_bundle.messages.len(), 1);
    assert_eq!(prompt_bundle.messages[0].role, "tool_result_reference");
    assert_eq!(prompt_bundle.messages[0].content_ref, tool_result_ref);
}

#[tokio::test]
async fn model_port_round_trips_tool_result_reference_context_as_typed_model_input() {
    let fixture = ThreadFixture::new().await;
    let tool_result_ref = LoopMessageRef::new("msg:11111111-1111-1111-1111-111111111111").unwrap();
    let envelope = ToolResultReferenceEnvelope {
        version: 1,
        result_ref: "result:round-trip".to_string(),
        safe_summary: ToolResultSafeSummary::new("tool result content").unwrap(),
        model_observation: None,
    };
    let envelope_content = serde_json::to_string(&envelope).unwrap();
    let thread_service = Arc::new(StaticContextThreadService::new(ContextMessage {
        message_id: Some(ThreadMessageId::parse("11111111-1111-1111-1111-111111111111").unwrap()),
        summary_id: None,
        sequence: 1,
        kind: MessageKind::ToolResultReference,
        tool_result_provider_call: None,
        content: envelope_content.clone(),
        image_attachments: Vec::new(),
    }));
    let context_port = ThreadBackedLoopContextPort::new(
        thread_service.clone(),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
        16,
    );
    let context = context_port
        .load_loop_context(LoopContextRequest {
            after: None,
            limit: 16,
            mode: ironclaw_loop_contracts::PromptMode::TextOnly,
        })
        .await
        .unwrap();
    assert_eq!(context.messages[0].role, "tool_result_reference");
    assert_eq!(context.messages[0].message_ref, Some(tool_result_ref));

    let gateway = Arc::new(RecordingGateway::reply("model says hi"));
    let model_port = ThreadBackedLoopModelPort::new(
        thread_service,
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
        gateway.clone(),
        16,
    );
    let messages = context
        .messages
        .into_iter()
        .filter_map(|message| {
            message.message_ref.map(|content_ref| LoopModelMessage {
                role: message.role,
                content_ref,
            })
        })
        .collect::<Vec<_>>();
    issue_prompt_grant(&fixture.run_context, &messages);

    model_port
        .stream_model(LoopModelRequest {
            inline_messages: Vec::new(),
            messages,
            surface_version: None,
            model_preference: None,
            fallback_index: 0,
            iteration: 0,
            capability_view: None,
        })
        .await
        .unwrap();

    let calls = gateway.calls.lock().unwrap();
    assert_eq!(
        calls[0].messages[0].role,
        HostManagedModelMessageRole::ToolResult
    );
    assert_eq!(calls[0].messages[0].content, envelope_content);
    assert_eq!(
        calls[0].messages[0].tool_result_content,
        Some(HostManagedToolResultContent::Reference { envelope })
    );
}

#[tokio::test]
async fn model_port_rejects_malformed_tool_result_reference_content() {
    let fixture = ThreadFixture::new().await;
    let tool_result_ref = LoopMessageRef::new("msg:22222222-2222-2222-2222-222222222222").unwrap();
    let thread_service = Arc::new(StaticContextThreadService::new(ContextMessage {
        message_id: Some(ThreadMessageId::parse("22222222-2222-2222-2222-222222222222").unwrap()),
        summary_id: None,
        sequence: 1,
        kind: MessageKind::ToolResultReference,
        tool_result_provider_call: None,
        content: "not a tool-result reference envelope".to_string(),
        image_attachments: Vec::new(),
    }));
    let gateway = Arc::new(RecordingGateway::reply("model says hi"));
    let model_port = ThreadBackedLoopModelPort::new(
        thread_service,
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
        gateway.clone(),
        16,
    );
    let messages = vec![LoopModelMessage {
        role: "tool_result_reference".to_string(),
        content_ref: tool_result_ref,
    }];
    issue_prompt_grant(&fixture.run_context, &messages);

    let error = model_port
        .stream_model(LoopModelRequest {
            inline_messages: Vec::new(),
            messages,
            surface_version: None,
            model_preference: None,
            fallback_index: 0,
            iteration: 0,
            capability_view: None,
        })
        .await
        .expect_err("malformed tool result reference content should fail");

    assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
    assert!(gateway.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn model_port_rejects_missing_explicit_tool_result_reference_before_gateway_call() {
    let fixture = ThreadFixture::new().await;
    let missing_tool_result_ref =
        LoopMessageRef::new("msg:33333333-3333-3333-3333-333333333333").unwrap();
    let thread_service = Arc::new(StaticContextThreadService::new(ContextMessage {
        message_id: Some(ThreadMessageId::parse("44444444-4444-4444-4444-444444444444").unwrap()),
        summary_id: None,
        sequence: 1,
        kind: MessageKind::User,
        tool_result_provider_call: None,
        content: "newer user message still exists".to_string(),
        image_attachments: Vec::new(),
    }));
    let gateway = Arc::new(RecordingGateway::reply("model says hi"));
    let model_port = ThreadBackedLoopModelPort::new(
        thread_service,
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
        gateway.clone(),
        16,
    );
    let messages = vec![LoopModelMessage {
        role: "tool_result_reference".to_string(),
        content_ref: missing_tool_result_ref,
    }];
    issue_prompt_grant(&fixture.run_context, &messages);

    let error = model_port
        .stream_model(LoopModelRequest {
            messages,
            inline_messages: Vec::new(),
            surface_version: None,
            model_preference: None,
            fallback_index: 0,
            iteration: 0,
            capability_view: None,
        })
        .await
        .expect_err("missing tool result reference should fail before model call");

    assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
    assert_eq!(error.safe_summary, "model message reference is unavailable");
    assert!(gateway.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn model_port_emits_model_milestones_without_prompt_or_output_payloads() {
    let fixture = ThreadFixture::new_with_user_content(
        "RAW_PROMPT_TEXT_SENTINEL sk-prompt-secret /host/path tool_input",
    )
    .await;
    let milestone_sink = Arc::new(InMemoryLoopHostMilestoneSink::default());
    let gateway = Arc::new(RecordingGateway::reply(
        "RAW_ASSISTANT_CONTENT_SENTINEL sk-output-secret",
    ));
    let port = ThreadBackedLoopModelPort::with_milestone_sink(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
        gateway,
        16,
        milestone_sink.clone(),
    );
    let messages = user_model_messages(&fixture);
    issue_prompt_grant(&fixture.run_context, &messages);

    let response = port
        .stream_model(LoopModelRequest {
            inline_messages: Vec::new(),
            messages,
            surface_version: None,
            model_preference: Some(
                fixture
                    .run_context
                    .resolved_run_profile
                    .model_profile_id
                    .clone(),
            ),
            fallback_index: 0,
            iteration: 0,
            capability_view: None,
        })
        .await
        .unwrap();

    assert_eq!(
        response.effective_model_profile_id,
        fixture.run_context.resolved_run_profile.model_profile_id
    );
    let milestones = milestone_sink.milestones();
    assert_eq!(milestones.len(), 2);
    assert!(matches!(
        &milestones[0].kind,
        LoopHostMilestoneKind::ModelStarted { requested_model_profile_id: Some(model_profile_id) }
            if model_profile_id == &fixture.run_context.resolved_run_profile.model_profile_id
    ));
    assert!(matches!(
        &milestones[1].kind,
        LoopHostMilestoneKind::ModelCompleted { effective_model_profile_id }
            if effective_model_profile_id == &fixture.run_context.resolved_run_profile.model_profile_id
    ));
    let wire = serde_json::to_string(&milestones).unwrap();
    for forbidden in [
        "RAW_PROMPT_TEXT_SENTINEL",
        "RAW_ASSISTANT_CONTENT_SENTINEL",
        "sk-prompt-secret",
        "sk-output-secret",
        "/host/path",
        "tool_input",
    ] {
        assert!(!wire.contains(forbidden), "milestone leaked {forbidden}");
    }
}

#[tokio::test]
async fn model_port_emits_started_and_failed_milestones_when_gateway_fails() {
    let fixture = ThreadFixture::new_with_user_content("RAW_PROMPT_TEXT_SENTINEL").await;
    let milestone_sink = Arc::new(InMemoryLoopHostMilestoneSink::default());
    let gateway = Arc::new(RecordingGateway::deny(
        "RAW_PROVIDER_ERROR invalid api key sk-provider-secret /host/path tool_input",
    ));
    let port = ThreadBackedLoopModelPort::with_milestone_sink(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
        gateway,
        16,
        milestone_sink.clone(),
    );
    let messages = user_model_messages(&fixture);
    issue_prompt_grant(&fixture.run_context, &messages);

    let error = port
        .stream_model(LoopModelRequest {
            inline_messages: Vec::new(),
            messages,
            surface_version: None,
            model_preference: None,
            fallback_index: 0,
            iteration: 0,
            capability_view: None,
        })
        .await
        .unwrap_err();

    assert_eq!(error.kind, AgentLoopHostErrorKind::PolicyDenied);
    let milestones = milestone_sink.milestones();
    assert_eq!(milestones.len(), 2);
    assert!(matches!(
        &milestones[0].kind,
        LoopHostMilestoneKind::ModelStarted {
            requested_model_profile_id: None
        }
    ));
    assert!(matches!(
        &milestones[1].kind,
        LoopHostMilestoneKind::ModelFailed {
            reason_kind: AgentLoopHostErrorKind::PolicyDenied
        }
    ));
    let wire = serde_json::to_string(&milestones).unwrap();
    for forbidden in [
        "RAW_PROMPT_TEXT_SENTINEL",
        "RAW_PROVIDER_ERROR",
        "invalid api key",
        "sk-provider-secret",
        "/host/path",
        "tool_input",
    ] {
        assert!(!wire.contains(forbidden), "milestone leaked {forbidden}");
    }
}

#[traced_test]
#[tokio::test]
async fn model_port_logs_model_started_milestone_failure_without_losing_response() {
    let fixture = ThreadFixture::new().await;
    let milestone_sink = Arc::new(FailOnModelStartedMilestoneSink::default());
    let gateway = Arc::new(RecordingGateway::reply(
        "model response survives start milestone failure",
    ));
    let port = ThreadBackedLoopModelPort::with_milestone_sink(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
        gateway,
        16,
        milestone_sink.clone(),
    );
    let messages = user_model_messages(&fixture);
    issue_prompt_grant(&fixture.run_context, &messages);

    let response = port
        .stream_model(LoopModelRequest {
            inline_messages: Vec::new(),
            messages,
            surface_version: None,
            model_preference: None,
            fallback_index: 0,
            iteration: 0,
            capability_view: None,
        })
        .await
        .unwrap();

    assert!(matches!(
        response.output,
        ParentLoopOutput::AssistantReply(AssistantReply { ref content })
            if content == "model response survives start milestone failure"
    ));
    assert_eq!(milestone_sink.kind_names(), vec!["model_completed"]);
    assert!(logs_contain(
        "loop model_started milestone failed before model request"
    ));
}

#[traced_test]
#[tokio::test]
async fn model_port_logs_model_completed_milestone_failure_without_losing_response() {
    let fixture = ThreadFixture::new().await;
    let milestone_sink = Arc::new(FailOnModelCompletedMilestoneSink::default());
    let gateway = Arc::new(RecordingGateway::reply(
        "model response survives milestone failure",
    ));
    let port = ThreadBackedLoopModelPort::with_milestone_sink(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
        gateway,
        16,
        milestone_sink.clone(),
    );
    let messages = user_model_messages(&fixture);
    issue_prompt_grant(&fixture.run_context, &messages);

    let response = port
        .stream_model(LoopModelRequest {
            inline_messages: Vec::new(),
            messages,
            surface_version: None,
            model_preference: None,
            fallback_index: 0,
            iteration: 0,
            capability_view: None,
        })
        .await
        .unwrap();

    assert!(matches!(
        response.output,
        ParentLoopOutput::AssistantReply(AssistantReply { ref content })
            if content == "model response survives milestone failure"
    ));
    assert_eq!(milestone_sink.kind_names(), vec!["model_started"]);
    assert!(logs_contain(
        "loop model_completed milestone failed after successful model response"
    ));
}

#[tokio::test]
async fn model_port_rejects_message_role_that_disagrees_with_thread_record() {
    let fixture = ThreadFixture::new().await;
    let gateway = Arc::new(RecordingGateway::reply("should not be called"));
    let port = ThreadBackedLoopModelPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
        gateway.clone(),
        16,
    );
    let messages = vec![LoopModelMessage {
        role: "system".to_string(),
        content_ref: LoopMessageRef::new(format!("msg:{}", fixture.user_message_id)).unwrap(),
    }];
    issue_prompt_grant(&fixture.run_context, &messages);

    let error = port
        .stream_model(LoopModelRequest {
            inline_messages: Vec::new(),
            messages,
            surface_version: None,
            model_preference: None,
            fallback_index: 0,
            iteration: 0,
            capability_view: None,
        })
        .await
        .unwrap_err();

    assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
    assert!(gateway.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn model_port_surfaces_fail_closed_gateway_policy_errors_without_raw_details() {
    let fixture = ThreadFixture::new().await;
    let gateway = Arc::new(RecordingGateway::deny("RAW_PROVIDER_SECRET"));
    let port = ThreadBackedLoopModelPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
        gateway,
        16,
    );
    let messages = user_model_messages(&fixture);
    issue_prompt_grant(&fixture.run_context, &messages);

    let error = port
        .stream_model(LoopModelRequest {
            inline_messages: Vec::new(),
            messages,
            surface_version: None,
            model_preference: None,
            fallback_index: 0,
            iteration: 0,
            capability_view: None,
        })
        .await
        .unwrap_err();

    assert_eq!(error.kind, AgentLoopHostErrorKind::PolicyDenied);
    let wire = serde_json::to_string(&error).unwrap();
    assert!(!wire.contains("RAW_PROVIDER_SECRET"));
}

#[tokio::test]
async fn model_port_replaces_invalid_gateway_safe_summary_with_stable_summary() {
    let fixture = ThreadFixture::new().await;
    let gateway = Arc::new(RecordingGateway::deny_with_safe_summary(concat!(
        "RAW_PROVIDER_SECRET invalid api key sk-provider-secret \
         ghp",
        "_012345678901234567890123456789012345",
        " /host/path tool_input"
    )));
    let port = ThreadBackedLoopModelPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
        gateway,
        16,
    );
    let messages = user_model_messages(&fixture);
    issue_prompt_grant(&fixture.run_context, &messages);

    let error = port
        .stream_model(LoopModelRequest {
            inline_messages: Vec::new(),
            messages,
            surface_version: None,
            model_preference: None,
            fallback_index: 0,
            iteration: 0,
            capability_view: None,
        })
        .await
        .unwrap_err();

    // The card summary still degrades to the fixed category sentence.
    assert_eq!(error.kind, AgentLoopHostErrorKind::PolicyDenied);
    assert_eq!(error.safe_summary, "model profile is not permitted");
    let wire = format!("{}{:?}", serde_json::to_string(&error).unwrap(), error);
    // Phase 2 (item 4): the rejected provider summary now rides the
    // model-visible `detail` channel through the hardened scrubber. Secret
    // VALUES, credential tokens, and sentinel markers must NEVER appear.
    for forbidden in [
        "RAW_PROVIDER_SECRET",
        "sk-provider-secret",
        concat!("ghp", "_012345678901234567890123456789012345", ""),
    ] {
        assert!(!wire.contains(forbidden), "model error leaked {forbidden}");
    }
    // The descriptive cause DOES survive on the model-visible detail so the
    // failure explainer can describe the fault (stripped only at the public
    // projection boundary).
    let detail = error
        .detail
        .expect("rejected provider cause should ride detail");
    assert!(
        detail.contains("/host/path"),
        "path cause must survive: {detail}"
    );
    assert!(
        detail.contains("tool_input"),
        "descriptive cause must survive: {detail}"
    );
}

#[tokio::test]
async fn model_port_preserves_gateway_safe_reason_kind() {
    let fixture = ThreadFixture::new().await;
    let gateway = Arc::new(RecordingGateway::model_error_with_reason_kind(
        HostManagedModelErrorKind::CredentialUnavailable,
        "display summary can change without changing the reason",
        AgentLoopHostErrorReasonKind::ModelCreditsExhausted,
    ));
    let port = ThreadBackedLoopModelPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
        gateway,
        16,
    );
    let messages = user_model_messages(&fixture);
    issue_prompt_grant(&fixture.run_context, &messages);

    let error = port
        .stream_model(LoopModelRequest {
            inline_messages: Vec::new(),
            messages,
            surface_version: None,
            model_preference: None,
            fallback_index: 0,
            iteration: 0,
            capability_view: None,
        })
        .await
        .unwrap_err();

    assert_eq!(error.kind, AgentLoopHostErrorKind::CredentialUnavailable);
    assert_eq!(
        error.reason_kind,
        Some(AgentLoopHostErrorReasonKind::ModelCreditsExhausted)
    );
}

#[derive(Clone)]
struct StaticLoopContextPort {
    bundle: LoopContextBundle,
}

#[async_trait]
impl LoopContextPort for StaticLoopContextPort {
    async fn load_loop_context(
        &self,
        _request: LoopContextRequest,
    ) -> Result<LoopContextBundle, ironclaw_loop_contracts::AgentLoopHostError> {
        Ok(self.bundle.clone())
    }
}

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

struct DelayedFailingSkillContextSource {
    delay: Duration,
}

#[async_trait]
impl HostSkillContextSource for DelayedFailingSkillContextSource {
    async fn load_skill_context_candidates(
        &self,
        _run_context: &LoopRunContext,
    ) -> Result<Vec<HostSkillContextCandidate>, HostSkillContextBuildError> {
        tokio::time::sleep(self.delay).await;
        Err(HostSkillContextBuildError::SourceUnavailable)
    }
}

struct StaticSkillBundleSource {
    descriptors: Vec<SkillBundleDescriptor>,
    reads: Mutex<Vec<String>>,
}

impl StaticSkillBundleSource {
    fn new(descriptors: Vec<SkillBundleDescriptor>) -> Self {
        Self {
            descriptors,
            reads: Mutex::new(Vec::new()),
        }
    }

    fn reads(&self) -> Vec<String> {
        self.reads.lock().unwrap().clone()
    }
}

#[async_trait]
impl SkillBundleSource for StaticSkillBundleSource {
    async fn list_skill_bundles(
        &self,
        _run_context: &LoopRunContext,
    ) -> Result<Vec<SkillBundleDescriptor>, SkillBundleSourceError> {
        Ok(self.descriptors.clone())
    }

    async fn read_skill_bundle_file(
        &self,
        _run_context: &LoopRunContext,
        bundle_id: &SkillBundleId,
        path: &SkillFilePath,
    ) -> Result<Vec<u8>, SkillBundleSourceError> {
        let key = format!("{bundle_id}:{path}");
        self.reads.lock().unwrap().push(key);
        Err(SkillBundleSourceError::FileNotFound)
    }
}

#[derive(Clone)]
struct StaticIdentityContextSource {
    candidates: Vec<HostIdentityContextCandidate>,
    content_by_ref: std::collections::HashMap<String, HostIdentityMessageContent>,
    calls: Arc<AtomicUsize>,
}

impl StaticIdentityContextSource {
    fn new(candidates: Vec<(HostIdentityContextCandidate, String)>) -> Self {
        let mut context_candidates = Vec::with_capacity(candidates.len());
        let mut content_by_ref = std::collections::HashMap::new();
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
            calls: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn load_calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

struct ModeAwareIdentityContextSource {
    text_only: Vec<HostIdentityContextCandidate>,
    codeact: Vec<HostIdentityContextCandidate>,
    calls: Arc<AtomicUsize>,
}

impl ModeAwareIdentityContextSource {
    fn new(
        text_only: Vec<HostIdentityContextCandidate>,
        codeact: Vec<HostIdentityContextCandidate>,
    ) -> Self {
        Self {
            text_only,
            codeact,
            calls: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn load_calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl HostIdentityContextSource for ModeAwareIdentityContextSource {
    async fn load_identity_candidates(
        &self,
        _run_context: &LoopRunContext,
        mode: PromptMode,
    ) -> Result<Vec<HostIdentityContextCandidate>, HostIdentityContextBuildError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        match mode {
            PromptMode::TextOnly => Ok(self.text_only.clone()),
            PromptMode::CodeAct => Ok(self.codeact.clone()),
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
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.candidates.clone())
    }

    async fn resolve_identity_message_content(
        &self,
        _run_context: &LoopRunContext,
        message_ref: &LoopMessageRef,
    ) -> Result<Option<HostIdentityMessageContent>, HostIdentityContextBuildError> {
        Ok(self.content_by_ref.get(message_ref.as_str()).cloned())
    }
}

struct PolicyDeniedIdentityContextSource;

#[async_trait]
impl HostIdentityContextSource for PolicyDeniedIdentityContextSource {
    async fn load_identity_candidates(
        &self,
        _run_context: &LoopRunContext,
        _mode: PromptMode,
    ) -> Result<Vec<HostIdentityContextCandidate>, HostIdentityContextBuildError> {
        Ok(Vec::new())
    }

    async fn resolve_identity_message_content(
        &self,
        _run_context: &LoopRunContext,
        _message_ref: &LoopMessageRef,
    ) -> Result<Option<HostIdentityMessageContent>, HostIdentityContextBuildError> {
        Err(HostIdentityContextBuildError::PolicyDenied)
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

fn personal_identity(name: &str, content: &str) -> (HostIdentityContextCandidate, String) {
    trusted_identity(
        name,
        content,
        IdentityApplicability::OnPersonalContextAllowed,
    )
}

fn skill_bundle_descriptor(
    source_kind: SkillSourceKind,
    name: &str,
    trust: Option<SkillTrust>,
    visibility: Option<SkillVisibility>,
) -> SkillBundleDescriptor {
    SkillBundleDescriptor::new(
        SkillBundleId::new(source_kind, name).unwrap(),
        trust,
        visibility,
        format!("{name} description"),
    )
}

struct MutableSkillContextSource {
    candidates: Mutex<Vec<HostSkillContextCandidate>>,
}

impl MutableSkillContextSource {
    fn new(candidates: Vec<HostSkillContextCandidate>) -> Self {
        Self {
            candidates: Mutex::new(candidates),
        }
    }

    fn set(&self, candidates: Vec<HostSkillContextCandidate>) {
        *self.candidates.lock().unwrap() = candidates;
    }
}

#[async_trait]
impl HostSkillContextSource for MutableSkillContextSource {
    async fn load_skill_context_candidates(
        &self,
        _run_context: &LoopRunContext,
    ) -> Result<Vec<HostSkillContextCandidate>, HostSkillContextBuildError> {
        Ok(self.candidates.lock().unwrap().clone())
    }
}

fn skill_md(name: &str, description: &str, prompt: &str) -> String {
    format!(
        "---\nname: {name}\ndescription: {description}\nactivation:\n  keywords: [{name}]\n---\n\n{prompt}\n"
    )
}

fn user_model_messages(fixture: &ThreadFixture) -> Vec<LoopModelMessage> {
    vec![LoopModelMessage {
        role: "user".to_string(),
        content_ref: LoopMessageRef::new(format!("msg:{}", fixture.user_message_id)).unwrap(),
    }]
}

fn provider_tool_definition(capability_id: CapabilityId, name: &str) -> ProviderToolDefinition {
    ProviderToolDefinition {
        capability_id,
        name: ProviderToolName::new(name).expect("provider tool name"),
        description: "test provider tool".to_string(),
        description_trust: Default::default(),
        parameters: serde_json::json!({"type": "object"}),
    }
}

fn issue_prompt_grant(context: &LoopRunContext, messages: &[LoopModelMessage]) {
    let bundle = LoopPromptBundle {
        bundle_ref: LoopPromptBundleRef::for_run(context, "test-bundle").unwrap(),
        messages: messages.to_vec(),
        surface_version: None,
        compaction_message_index: Vec::new(),
        instruction_fingerprint: None,
        identity_message_count: 0,
        instruction_snippet_count: 0,
    };
    LoopPromptBundleAuthority::shared()
        .issue_bundle(context, &bundle)
        .unwrap();
}

struct ThreadFixture {
    thread_service: Arc<InMemorySessionThreadService>,
    thread_scope: ThreadScope,
    thread_id: ThreadId,
    user_message_id: ironclaw_threads::ThreadMessageId,
    run_context: LoopRunContext,
}

impl ThreadFixture {
    async fn new() -> Self {
        Self::new_with_user_content("hello reborn").await
    }

    async fn new_with_user_content(user_content: &str) -> Self {
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        let tenant_id = TenantId::new("tenant-loop-support").unwrap();
        let agent_id = AgentId::new("agent-loop-support").unwrap();
        let project_id = ProjectId::new("project-loop-support").unwrap();
        let user_id = UserId::new("user-loop-support").unwrap();
        let thread_id = ThreadId::new("thread-loop-support").unwrap();
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
                created_by_actor_id: user_id.as_str().to_string(),
                title: None,
                metadata_json: None,
            })
            .await
            .unwrap();
        let accepted = thread_service
            .accept_inbound_message(AcceptInboundMessageRequest {
                scope: thread_scope.clone(),
                thread_id: thread_id.clone(),
                actor_id: user_id.as_str().to_string(),
                source_binding_id: Some("source-web".to_string()),
                reply_target_binding_id: Some("reply-web".to_string()),
                external_event_id: Some("event-1".to_string()),
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
        let run_context =
            LoopRunContext::new(turn_scope, TurnId::new(), TurnRunId::new(), resolved);
        let _actor = TurnActor::new(user_id);
        Self {
            thread_service,
            thread_scope,
            thread_id,
            user_message_id: accepted.message_id,
            run_context,
        }
    }

    async fn accept_user_message(
        &self,
        external_event_id: &str,
        content: &str,
    ) -> AcceptedInboundMessage {
        self.thread_service
            .accept_inbound_message(AcceptInboundMessageRequest {
                scope: self.thread_scope.clone(),
                thread_id: self.thread_id.clone(),
                actor_id: "user-loop-support".to_string(),
                source_binding_id: Some("source-web".to_string()),
                reply_target_binding_id: Some("reply-web".to_string()),
                external_event_id: Some(external_event_id.to_string()),
                content: MessageContent::text(content),
            })
            .await
            .unwrap()
    }
}

fn reply_attachment_scope(fixture: &ThreadFixture) -> ResourceScope {
    let mut scope = fixture.thread_scope.to_resource_scope();
    scope.thread_id = Some(fixture.thread_id.clone());
    scope
}

fn reply_attachment_run_id(fixture: &ThreadFixture) -> RunId {
    RunId::from_uuid(fixture.run_context.run_id.as_uuid())
}

fn reply_attachment(
    path: &str,
    filename: &str,
    mime_type: &str,
    size_bytes: u64,
) -> ReplyAttachmentIntent {
    ReplyAttachmentIntent {
        path: ScopedPath::new(path).expect("reply attachment path"),
        filename: filename.to_string(),
        mime_type: mime_type.to_string(),
        size_bytes,
    }
}

async fn register_reply_attachment(
    store: &OutboundStateStore<ironclaw_filesystem::InMemoryBackend>,
    fixture: &ThreadFixture,
    path: &str,
    filename: &str,
    mime_type: &str,
    size_bytes: u64,
) {
    store
        .register(
            &reply_attachment_scope(fixture),
            &reply_attachment_run_id(fixture),
            reply_attachment(path, filename, mime_type, size_bytes),
        )
        .await
        .expect("reply attachment registration");
}

async fn finalized_assistant_message(fixture: &ThreadFixture) -> ThreadMessageRecord {
    fixture
        .thread_service
        .list_thread_history(ThreadHistoryRequest {
            scope: fixture.thread_scope.clone(),
            thread_id: fixture.thread_id.clone(),
        })
        .await
        .expect("thread history")
        .messages
        .into_iter()
        .find(|message| message.kind == MessageKind::Assistant)
        .expect("finalized assistant message")
}

struct FailingSealReplyAttachmentIntentPort;

#[async_trait]
impl ReplyAttachmentIntentPort for FailingSealReplyAttachmentIntentPort {
    async fn register(
        &self,
        _scope: &ResourceScope,
        _run_id: &RunId,
        _intent: ReplyAttachmentIntent,
    ) -> Result<(), OutboundError> {
        Ok(())
    }

    async fn seal(
        &self,
        _scope: &ResourceScope,
        _run_id: &RunId,
    ) -> Result<Vec<ReplyAttachmentIntent>, OutboundError> {
        Err(OutboundError::Backend)
    }
}

struct GatedThreadFixture {
    thread_service: Arc<GatedFinalizeThreadService>,
    thread_scope: ThreadScope,
    thread_id: ThreadId,
    run_context: LoopRunContext,
}

impl GatedThreadFixture {
    async fn new() -> Self {
        let base = ThreadFixture::new().await;
        let gated = Arc::new(GatedFinalizeThreadService {
            inner: Arc::clone(&base.thread_service),
            finalize_entries: AtomicUsize::new(0),
            context_window_loads: AtomicUsize::new(0),
        });
        Self {
            thread_service: gated,
            thread_scope: base.thread_scope,
            thread_id: base.thread_id,
            run_context: base.run_context,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TranscriptWriteOperation {
    FinalizedAssistant,
    ToolResultReference,
}

#[derive(Clone, Copy)]
enum TranscriptWriteFailure {
    Backend,
    Serialization,
}

struct ScriptedTranscriptWriteThreadService {
    inner: Arc<InMemorySessionThreadService>,
    operation: TranscriptWriteOperation,
    failure: TranscriptWriteFailure,
    failures_remaining: AtomicUsize,
    attempts: AtomicUsize,
}

impl ScriptedTranscriptWriteThreadService {
    fn new(
        inner: Arc<InMemorySessionThreadService>,
        operation: TranscriptWriteOperation,
        failure: TranscriptWriteFailure,
        failures: usize,
    ) -> Self {
        Self {
            inner,
            operation,
            failure,
            failures_remaining: AtomicUsize::new(failures),
            attempts: AtomicUsize::new(0),
        }
    }

    fn attempts(&self) -> usize {
        self.attempts.load(Ordering::SeqCst)
    }

    fn scripted_failure(&self, operation: TranscriptWriteOperation) -> Option<SessionThreadError> {
        if self.operation != operation {
            return None;
        }
        self.attempts.fetch_add(1, Ordering::SeqCst);
        if self
            .failures_remaining
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                remaining.checked_sub(1)
            })
            .is_err()
        {
            return None;
        }
        Some(match self.failure {
            TranscriptWriteFailure::Backend => {
                SessionThreadError::Backend("transient transcript backend failure".to_string())
            }
            TranscriptWriteFailure::Serialization => {
                SessionThreadError::Serialization("invalid transcript request".to_string())
            }
        })
    }
}

#[async_trait]
impl SessionThreadService for ScriptedTranscriptWriteThreadService {
    async fn ensure_thread(
        &self,
        _request: EnsureThreadRequest,
    ) -> Result<SessionThreadRecord, SessionThreadError> {
        panic!("scripted transcript service does not create threads")
    }

    async fn accept_inbound_message(
        &self,
        _request: AcceptInboundMessageRequest,
    ) -> Result<AcceptedInboundMessage, SessionThreadError> {
        panic!("scripted transcript service does not accept inbound messages")
    }

    async fn replay_accepted_inbound_message(
        &self,
        _request: ReplayAcceptedInboundMessageRequest,
    ) -> Result<Option<AcceptedInboundMessageReplay>, SessionThreadError> {
        panic!("scripted transcript service does not replay inbound messages")
    }

    async fn mark_message_submitted(
        &self,
        _scope: &ThreadScope,
        _thread_id: &ThreadId,
        _message_id: ThreadMessageId,
        _turn_id: String,
        _turn_run_id: String,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        panic!("scripted transcript service does not mark submitted")
    }

    async fn mark_message_rejected_busy(
        &self,
        _scope: &ThreadScope,
        _thread_id: &ThreadId,
        _message_id: ThreadMessageId,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        panic!("scripted transcript service does not reject messages")
    }

    async fn append_assistant_draft(
        &self,
        _request: AppendAssistantDraftRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        panic!("scripted transcript service does not append assistant drafts")
    }

    async fn append_finalized_assistant_message(
        &self,
        request: AppendFinalizedAssistantMessageRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        if let Some(error) = self.scripted_failure(TranscriptWriteOperation::FinalizedAssistant) {
            return Err(error);
        }
        self.inner.append_finalized_assistant_message(request).await
    }

    async fn append_tool_result_reference(
        &self,
        request: AppendToolResultReferenceRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        if let Some(error) = self.scripted_failure(TranscriptWriteOperation::ToolResultReference) {
            return Err(error);
        }
        self.inner.append_tool_result_reference(request).await
    }

    async fn append_capability_display_preview(
        &self,
        _request: AppendCapabilityDisplayPreviewRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        panic!("scripted transcript service does not append capability display previews")
    }

    async fn update_tool_result_reference(
        &self,
        _request: UpdateToolResultReferenceRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        panic!("scripted transcript service does not update tool result references")
    }

    async fn update_assistant_draft(
        &self,
        _request: UpdateAssistantDraftRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        panic!("scripted transcript service does not update assistant drafts")
    }

    async fn finalize_assistant_message(
        &self,
        _scope: &ThreadScope,
        _thread_id: &ThreadId,
        _message_id: ThreadMessageId,
        _content: MessageContent,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        panic!("scripted transcript service does not finalize draft messages")
    }

    async fn redact_message(
        &self,
        _request: RedactMessageRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        panic!("scripted transcript service does not redact messages")
    }

    async fn load_context_window(
        &self,
        _request: ironclaw_threads::LoadContextWindowRequest,
    ) -> Result<ContextWindow, SessionThreadError> {
        panic!("scripted transcript service does not load context windows")
    }

    async fn load_context_messages(
        &self,
        _request: LoadContextMessagesRequest,
    ) -> Result<ContextMessages, SessionThreadError> {
        panic!("scripted transcript service does not load context messages")
    }

    async fn list_thread_history(
        &self,
        request: ThreadHistoryRequest,
    ) -> Result<ThreadHistory, SessionThreadError> {
        self.inner.list_thread_history(request).await
    }

    async fn create_summary_artifact(
        &self,
        _request: CreateSummaryArtifactRequest,
    ) -> Result<SummaryArtifact, SessionThreadError> {
        panic!("scripted transcript service does not create summaries")
    }
}

struct GatedFinalizeThreadService {
    inner: Arc<InMemorySessionThreadService>,
    finalize_entries: AtomicUsize,
    context_window_loads: AtomicUsize,
}

impl GatedFinalizeThreadService {
    fn context_window_loads(&self) -> usize {
        self.context_window_loads.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl SessionThreadService for GatedFinalizeThreadService {
    async fn ensure_thread(
        &self,
        request: EnsureThreadRequest,
    ) -> Result<SessionThreadRecord, SessionThreadError> {
        self.inner.ensure_thread(request).await
    }

    async fn accept_inbound_message(
        &self,
        request: AcceptInboundMessageRequest,
    ) -> Result<AcceptedInboundMessage, SessionThreadError> {
        self.inner.accept_inbound_message(request).await
    }

    async fn replay_accepted_inbound_message(
        &self,
        request: ReplayAcceptedInboundMessageRequest,
    ) -> Result<Option<AcceptedInboundMessageReplay>, SessionThreadError> {
        self.inner.replay_accepted_inbound_message(request).await
    }

    async fn mark_message_submitted(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        message_id: ThreadMessageId,
        turn_id: String,
        turn_run_id: String,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        self.inner
            .mark_message_submitted(scope, thread_id, message_id, turn_id, turn_run_id)
            .await
    }

    async fn mark_message_rejected_busy(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        message_id: ThreadMessageId,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        self.inner
            .mark_message_rejected_busy(scope, thread_id, message_id)
            .await
    }

    async fn append_assistant_draft(
        &self,
        request: AppendAssistantDraftRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        self.inner.append_assistant_draft(request).await
    }

    async fn append_tool_result_reference(
        &self,
        request: AppendToolResultReferenceRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        self.inner.append_tool_result_reference(request).await
    }

    async fn append_capability_display_preview(
        &self,
        request: AppendCapabilityDisplayPreviewRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        self.inner.append_capability_display_preview(request).await
    }

    async fn update_tool_result_reference(
        &self,
        request: UpdateToolResultReferenceRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        self.inner.update_tool_result_reference(request).await
    }

    async fn update_assistant_draft(
        &self,
        request: UpdateAssistantDraftRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        self.inner.update_assistant_draft(request).await
    }

    async fn finalize_assistant_message(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        message_id: ThreadMessageId,
        content: MessageContent,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        self.finalize_entries.fetch_add(1, Ordering::SeqCst);
        while self.finalize_entries.load(Ordering::SeqCst) < 2 {
            tokio::task::yield_now().await;
        }
        self.inner
            .finalize_assistant_message(scope, thread_id, message_id, content)
            .await
    }

    async fn redact_message(
        &self,
        request: RedactMessageRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        self.inner.redact_message(request).await
    }

    async fn load_context_window(
        &self,
        request: ironclaw_threads::LoadContextWindowRequest,
    ) -> Result<ContextWindow, SessionThreadError> {
        self.context_window_loads.fetch_add(1, Ordering::SeqCst);
        self.inner.load_context_window(request).await
    }

    async fn load_context_messages(
        &self,
        request: LoadContextMessagesRequest,
    ) -> Result<ContextMessages, SessionThreadError> {
        self.inner.load_context_messages(request).await
    }

    async fn list_thread_history(
        &self,
        request: ThreadHistoryRequest,
    ) -> Result<ThreadHistory, SessionThreadError> {
        self.inner.list_thread_history(request).await
    }

    async fn create_summary_artifact(
        &self,
        request: CreateSummaryArtifactRequest,
    ) -> Result<SummaryArtifact, SessionThreadError> {
        self.inner.create_summary_artifact(request).await
    }
}

struct StaticContextThreadService {
    context_message: ContextMessage,
    scoped_context_messages: HashMap<Option<MissionId>, ContextMessage>,
    context_window_loads: AtomicUsize,
}

impl StaticContextThreadService {
    fn new(context_message: ContextMessage) -> Self {
        Self {
            context_message,
            scoped_context_messages: HashMap::new(),
            context_window_loads: AtomicUsize::new(0),
        }
    }

    fn with_scoped_context_messages(
        scoped_context_messages: Vec<(Option<MissionId>, ContextMessage)>,
    ) -> Self {
        let context_message = scoped_context_messages
            .first()
            .map(|(_, message)| message.clone())
            .unwrap_or_else(|| ContextMessage {
                message_id: Some(ThreadMessageId::new()),
                summary_id: None,
                sequence: 1,
                kind: MessageKind::User,
                tool_result_provider_call: None,
                content: String::new(),
                image_attachments: Vec::new(),
            });
        Self {
            context_message,
            scoped_context_messages: scoped_context_messages.into_iter().collect(),
            context_window_loads: AtomicUsize::new(0),
        }
    }

    fn context_window_loads(&self) -> usize {
        self.context_window_loads.load(Ordering::SeqCst)
    }

    fn context_message_for_scope(&self, scope: &ThreadScope) -> ContextMessage {
        self.scoped_context_messages
            .get(&scope.mission_id)
            .unwrap_or(&self.context_message)
            .clone()
    }
}

#[async_trait]
impl SessionThreadService for StaticContextThreadService {
    async fn ensure_thread(
        &self,
        _request: EnsureThreadRequest,
    ) -> Result<SessionThreadRecord, SessionThreadError> {
        panic!("static context service does not create threads")
    }

    async fn accept_inbound_message(
        &self,
        _request: AcceptInboundMessageRequest,
    ) -> Result<AcceptedInboundMessage, SessionThreadError> {
        panic!("static context service does not accept inbound messages")
    }

    async fn replay_accepted_inbound_message(
        &self,
        _request: ReplayAcceptedInboundMessageRequest,
    ) -> Result<Option<AcceptedInboundMessageReplay>, SessionThreadError> {
        panic!("static context service does not replay inbound messages")
    }

    async fn mark_message_submitted(
        &self,
        _scope: &ThreadScope,
        _thread_id: &ThreadId,
        _message_id: ThreadMessageId,
        _turn_id: String,
        _turn_run_id: String,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        panic!("static context service does not mark submitted")
    }

    async fn mark_message_rejected_busy(
        &self,
        _scope: &ThreadScope,
        _thread_id: &ThreadId,
        _message_id: ThreadMessageId,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        panic!("static context service does not reject messages")
    }

    async fn append_assistant_draft(
        &self,
        _request: AppendAssistantDraftRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        panic!("static context service does not append assistant drafts")
    }

    async fn append_tool_result_reference(
        &self,
        _request: AppendToolResultReferenceRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        panic!("static context service does not append tool result references")
    }

    async fn append_capability_display_preview(
        &self,
        _request: AppendCapabilityDisplayPreviewRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        panic!("static context service does not append capability display previews")
    }

    async fn update_tool_result_reference(
        &self,
        _request: UpdateToolResultReferenceRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        panic!("static context service does not update tool result references")
    }

    async fn update_assistant_draft(
        &self,
        _request: UpdateAssistantDraftRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        panic!("static context service does not update assistant drafts")
    }

    async fn finalize_assistant_message(
        &self,
        _scope: &ThreadScope,
        _thread_id: &ThreadId,
        _message_id: ThreadMessageId,
        _content: MessageContent,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        panic!("static context service does not finalize assistant messages")
    }

    async fn redact_message(
        &self,
        _request: RedactMessageRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        panic!("static context service does not redact messages")
    }

    async fn load_context_window(
        &self,
        request: ironclaw_threads::LoadContextWindowRequest,
    ) -> Result<ContextWindow, SessionThreadError> {
        self.context_window_loads.fetch_add(1, Ordering::SeqCst);
        let context_message = self.context_message_for_scope(&request.scope);
        Ok(ContextWindow {
            thread_id: request.thread_id,
            messages: vec![context_message],
        })
    }

    async fn load_context_messages(
        &self,
        request: LoadContextMessagesRequest,
    ) -> Result<ContextMessages, SessionThreadError> {
        let context_message = self.context_message_for_scope(&request.scope);
        Ok(ContextMessages {
            thread_id: request.thread_id,
            messages: vec![context_message],
        })
    }

    async fn list_thread_history(
        &self,
        _request: ThreadHistoryRequest,
    ) -> Result<ThreadHistory, SessionThreadError> {
        panic!("static context service does not list history")
    }

    async fn create_summary_artifact(
        &self,
        _request: CreateSummaryArtifactRequest,
    ) -> Result<SummaryArtifact, SessionThreadError> {
        panic!("static context service does not create summaries")
    }
}

#[derive(Default)]
struct FailOnceMilestoneSink {
    attempts: Mutex<Vec<ironclaw_loop_contracts::LoopHostMilestone>>,
}

impl FailOnceMilestoneSink {
    fn milestones(&self) -> Vec<ironclaw_loop_contracts::LoopHostMilestone> {
        self.attempts
            .lock()
            .unwrap()
            .iter()
            .skip(1)
            .cloned()
            .collect()
    }

    fn attempts_len(&self) -> usize {
        self.attempts.lock().unwrap().len()
    }
}

async fn wait_for_in_memory_milestones(
    sink: &InMemoryLoopHostMilestoneSink,
    expected: usize,
) -> Vec<ironclaw_loop_contracts::LoopHostMilestone> {
    for _ in 0..20 {
        let milestones = sink.milestones();
        if milestones.len() == expected {
            return milestones;
        }
        tokio::task::yield_now().await;
    }
    sink.milestones()
}

async fn wait_for_fail_once_attempts(sink: &FailOnceMilestoneSink, expected: usize) {
    for _ in 0..20 {
        if sink.attempts_len() == expected {
            return;
        }
        tokio::task::yield_now().await;
    }
}

#[async_trait]
impl ironclaw_loop_contracts::LoopHostMilestoneSink for FailOnceMilestoneSink {
    async fn publish_loop_milestone(
        &self,
        milestone: ironclaw_loop_contracts::LoopHostMilestone,
    ) -> Result<(), ironclaw_loop_contracts::AgentLoopHostError> {
        let mut attempts = self.attempts.lock().unwrap();
        if attempts.is_empty() {
            attempts.push(milestone);
            return Err(ironclaw_loop_contracts::AgentLoopHostError::new(
                AgentLoopHostErrorKind::Unavailable,
                "loop milestone sink unavailable",
            ));
        }
        attempts.push(milestone);
        Ok(())
    }
}

#[derive(Default)]
struct FailOnModelStartedMilestoneSink {
    published: Mutex<Vec<ironclaw_loop_contracts::LoopHostMilestone>>,
}

impl FailOnModelStartedMilestoneSink {
    fn kind_names(&self) -> Vec<&'static str> {
        self.published
            .lock()
            .unwrap()
            .iter()
            .map(|milestone| milestone.kind.kind_name())
            .collect()
    }
}

#[async_trait]
impl ironclaw_loop_contracts::LoopHostMilestoneSink for FailOnModelStartedMilestoneSink {
    async fn publish_loop_milestone(
        &self,
        milestone: ironclaw_loop_contracts::LoopHostMilestone,
    ) -> Result<(), ironclaw_loop_contracts::AgentLoopHostError> {
        if matches!(milestone.kind, LoopHostMilestoneKind::ModelStarted { .. }) {
            return Err(ironclaw_loop_contracts::AgentLoopHostError::new(
                AgentLoopHostErrorKind::Unavailable,
                "loop milestone sink unavailable",
            ));
        }
        self.published.lock().unwrap().push(milestone);
        Ok(())
    }
}

#[derive(Default)]
struct FailOnModelCompletedMilestoneSink {
    published: Mutex<Vec<ironclaw_loop_contracts::LoopHostMilestone>>,
}

impl FailOnModelCompletedMilestoneSink {
    fn kind_names(&self) -> Vec<&'static str> {
        self.published
            .lock()
            .unwrap()
            .iter()
            .map(|milestone| milestone.kind.kind_name())
            .collect()
    }
}

#[async_trait]
impl ironclaw_loop_contracts::LoopHostMilestoneSink for FailOnModelCompletedMilestoneSink {
    async fn publish_loop_milestone(
        &self,
        milestone: ironclaw_loop_contracts::LoopHostMilestone,
    ) -> Result<(), ironclaw_loop_contracts::AgentLoopHostError> {
        if matches!(milestone.kind, LoopHostMilestoneKind::ModelCompleted { .. }) {
            return Err(ironclaw_loop_contracts::AgentLoopHostError::new(
                AgentLoopHostErrorKind::Unavailable,
                "loop milestone sink unavailable",
            ));
        }
        self.published.lock().unwrap().push(milestone);
        Ok(())
    }
}

struct RecordingGateway {
    calls: Mutex<Vec<HostManagedModelRequest>>,
    tool_definition_calls: Mutex<Vec<Vec<ProviderToolDefinition>>>,
    response: Result<HostManagedModelResponse, HostManagedModelError>,
}

struct MissingDiagnosticModelGateway;

#[async_trait]
impl HostManagedModelGateway for MissingDiagnosticModelGateway {
    async fn stream_model(
        &self,
        request: HostManagedModelRequest,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        Ok(
            HostManagedModelResponse::assistant_reply("model says hi".to_string())
                .with_effective_fallback_index(request.fallback_index),
        )
    }
}

#[derive(Default)]
struct RecordingPromptDiagnosticSink {
    captures: Mutex<Vec<HostManagedPromptDiagnosticCapture>>,
    model_calls: Mutex<Vec<HostManagedModelCallDiagnosticCapture>>,
}

fn model_call_diagnostic(
    capture: &HostManagedModelCallDiagnosticCapture,
) -> &HostManagedModelCallDiagnostic {
    match capture {
        HostManagedModelCallDiagnosticCapture::Started(diagnostic)
        | HostManagedModelCallDiagnosticCapture::Completed { diagnostic, .. } => diagnostic,
    }
}

fn model_call_outcome(
    capture: &HostManagedModelCallDiagnosticCapture,
) -> Option<&HostManagedModelCallDiagnosticOutcome> {
    match capture {
        HostManagedModelCallDiagnosticCapture::Started(_) => None,
        HostManagedModelCallDiagnosticCapture::Completed { outcome, .. } => Some(outcome),
    }
}

impl HostManagedPromptDiagnosticSink for RecordingPromptDiagnosticSink {
    fn record_prompt(&self, capture: HostManagedPromptDiagnosticCapture) {
        self.captures.lock().expect("captures").push(capture);
    }

    fn record_model_call(&self, capture: HostManagedModelCallDiagnosticCapture) {
        self.model_calls.lock().expect("model calls").push(capture);
    }
}

impl RecordingGateway {
    fn reply(content: &str) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            tool_definition_calls: Mutex::new(Vec::new()),
            response: Ok(HostManagedModelResponse::assistant_reply(
                content.to_string(),
            )),
        }
    }

    fn reply_with_usage_and_fallback(
        content: &str,
        usage: LoopModelUsage,
        fallback_index: u32,
    ) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            tool_definition_calls: Mutex::new(Vec::new()),
            response: Ok(
                HostManagedModelResponse::assistant_reply(content.to_string())
                    .with_usage(usage)
                    .with_effective_fallback_index(fallback_index)
                    .with_diagnostic_effective_model("provider-model-from-response"),
            ),
        }
    }

    fn reply_with_fallback(content: &str, fallback_index: u32) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            tool_definition_calls: Mutex::new(Vec::new()),
            response: Ok(
                HostManagedModelResponse::assistant_reply(content.to_string())
                    .with_effective_fallback_index(fallback_index),
            ),
        }
    }

    fn reply_without_fallback_evidence(content: &str) -> Self {
        let mut response = HostManagedModelResponse::assistant_reply(content.to_string());
        response.effective_fallback_index = None;
        Self {
            calls: Mutex::new(Vec::new()),
            tool_definition_calls: Mutex::new(Vec::new()),
            response: Ok(response),
        }
    }

    fn deny(raw_detail: &str) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            tool_definition_calls: Mutex::new(Vec::new()),
            response: Err(HostManagedModelError::new(
                HostManagedModelErrorKind::PolicyDenied,
                raw_detail,
            )),
        }
    }

    fn deny_with_safe_summary(safe_summary: &str) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            tool_definition_calls: Mutex::new(Vec::new()),
            response: Err(HostManagedModelError::safe(
                HostManagedModelErrorKind::PolicyDenied,
                safe_summary,
            )),
        }
    }

    fn model_error(kind: HostManagedModelErrorKind, safe_summary: &str) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            tool_definition_calls: Mutex::new(Vec::new()),
            response: Err(HostManagedModelError::safe(kind, safe_summary)),
        }
    }

    fn model_error_with_usage(
        kind: HostManagedModelErrorKind,
        safe_summary: &str,
        usage: LoopModelUsage,
    ) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            tool_definition_calls: Mutex::new(Vec::new()),
            response: Err(HostManagedModelError::safe(kind, safe_summary)
                .with_usage(usage)
                .with_diagnostic_effective_model("provider-model-from-error")),
        }
    }

    fn model_error_with_reason_kind(
        kind: HostManagedModelErrorKind,
        safe_summary: &str,
        reason_kind: AgentLoopHostErrorReasonKind,
    ) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            tool_definition_calls: Mutex::new(Vec::new()),
            response: Err(
                HostManagedModelError::safe(kind, safe_summary).with_reason_kind(reason_kind)
            ),
        }
    }

    fn tool_definition_calls(&self) -> Vec<Vec<ProviderToolDefinition>> {
        self.tool_definition_calls
            .lock()
            .unwrap()
            .iter()
            .cloned()
            .collect()
    }
}

#[async_trait]
impl HostManagedModelGateway for RecordingGateway {
    fn diagnostic_effective_model(
        &self,
        _model_profile_id: &ModelProfileId,
        fallback_index: u32,
        _resolved_model_route: Option<&ironclaw_loop_host::HostManagedModelRouteSnapshot>,
    ) -> Option<ProviderModelId> {
        ProviderModelId::new(if fallback_index == 0 {
            "provider-model"
        } else {
            "fallback-provider-model"
        })
        .ok()
    }

    async fn stream_model(
        &self,
        request: HostManagedModelRequest,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        self.calls.lock().unwrap().push(request);
        self.response.clone()
    }

    async fn stream_model_with_capabilities(
        &self,
        request: HostManagedModelRequest,
        capabilities: Arc<dyn LoopCapabilityPort>,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        self.calls.lock().unwrap().push(request);
        self.tool_definition_calls
            .lock()
            .unwrap()
            .push(capabilities.tool_definitions().expect("tool definitions"));
        self.response.clone()
    }
}

struct StaticToolDefinitionPort {
    definitions: Vec<ProviderToolDefinition>,
    tool_definition_calls: AtomicUsize,
    fail_first_lookup: bool,
}

impl StaticToolDefinitionPort {
    fn new(definitions: Vec<ProviderToolDefinition>) -> Self {
        Self {
            definitions,
            tool_definition_calls: AtomicUsize::new(0),
            fail_first_lookup: false,
        }
    }

    fn failing_first_lookup(definitions: Vec<ProviderToolDefinition>) -> Self {
        Self {
            definitions,
            tool_definition_calls: AtomicUsize::new(0),
            fail_first_lookup: true,
        }
    }

    fn tool_definition_calls(&self) -> usize {
        self.tool_definition_calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl LoopCapabilityPort for StaticToolDefinitionPort {
    fn tool_definitions(&self) -> Result<Vec<ProviderToolDefinition>, AgentLoopHostError> {
        let call_index = self.tool_definition_calls.fetch_add(1, Ordering::SeqCst);
        if self.fail_first_lookup && call_index == 0 {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::Unavailable,
                "transient capability surface failure",
            ));
        }
        Ok(self.definitions.clone())
    }

    async fn visible_capabilities(
        &self,
        _request: VisibleCapabilityRequest,
    ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
        Ok(VisibleCapabilitySurface {
            callable_capability_ids: None,
            version: CapabilitySurfaceVersion::new("surface:test").unwrap(),
            descriptors: Vec::new(),
        })
    }

    async fn invoke_capability(
        &self,
        _request: LoopRequest,
    ) -> Result<Resolution, AgentLoopHostError> {
        Ok(resolution::denied(
            CapabilityDeniedReasonKind::EmptySurface,
            "test capability port does not execute tools".to_string(),
        )
        .resolution)
    }

    async fn invoke_capability_batch(
        &self,
        request: LoopRequestBatch,
    ) -> Result<ResolutionBatch, AgentLoopHostError> {
        let resolutions = request
            .invocations
            .into_iter()
            .map(|_| {
                resolution::denied(
                    CapabilityDeniedReasonKind::EmptySurface,
                    "test capability port does not execute tools".to_string(),
                )
                .resolution
            })
            .collect();
        Ok(ResolutionBatch {
            resolutions,
            stopped_on_suspension: false,
        })
    }
}
// arch-exempt: large_file, thread loop host contract remains one integration suite, plan #6175

/// Records every memory request and returns one fixed snippet, so the
/// caller-level test can assert both the captured request and the once-per-run
/// fetch guarantee.
struct RecordingMemoryPromptContextService {
    calls: Mutex<Vec<ironclaw_loop_contracts::MemoryPromptContextRequest>>,
}

#[async_trait]
impl ironclaw_loop_contracts::MemoryPromptContextService for RecordingMemoryPromptContextService {
    async fn load_memory_snippets(
        &self,
        request: ironclaw_loop_contracts::MemoryPromptContextRequest,
    ) -> Result<Vec<LoopContextSnippet>, AgentLoopHostError> {
        self.calls.lock().expect("memory calls lock").push(request);
        Ok(vec![LoopContextSnippet {
            snippet_ref: "memory-snippet:test".to_string(),
            model_content: "remembered fact".to_string(),
            safe_summary: "remembered fact".to_string(),
            metadata: None,
        }])
    }
}

/// Caller-level proof of the proactive-memory lane through the REAL
/// `LoopContextPort::load_loop_context` path (not the message-selection helper
/// alone): the wired service is queried with the latest user message as the
/// query, its snippets surface on `LoopContextBundle.memory_snippets`, and the
/// fetch happens ONCE per run — the second prompt build reuses the cache.
#[tokio::test]
async fn thread_context_port_loads_memory_snippets_through_wired_service_once_per_run() {
    let fixture = ThreadFixture::new().await;
    let service = Arc::new(RecordingMemoryPromptContextService {
        calls: Mutex::new(Vec::new()),
    });
    // Memory retrieval is keyed to the acting human user; a context without an
    // actor deliberately skips the fetch, so stamp one like production does —
    // the actor must be the thread-scope owner or scope validation fails.
    let owner = fixture
        .thread_scope
        .owner_user_id
        .clone()
        .expect("fixture thread scope carries an owner");
    let run_context = fixture
        .run_context
        .clone()
        .with_actor(TurnActor::new(owner.clone()));
    let adapter = ThreadBackedLoopContextPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        run_context.clone(),
        16,
    )
    .with_memory_context_service(Arc::clone(&service) as Arc<_>);

    let request = || LoopContextRequest {
        after: None,
        limit: 16,
        mode: ironclaw_loop_contracts::PromptMode::TextOnly,
    };
    let first = adapter.load_loop_context(request()).await.unwrap();
    let second = adapter.load_loop_context(request()).await.unwrap();

    for bundle in [&first, &second] {
        assert_eq!(bundle.memory_snippets.len(), 1);
        assert_eq!(bundle.memory_snippets[0].snippet_ref, "memory-snippet:test");
        assert_eq!(bundle.memory_snippets[0].model_content, "remembered fact");
    }

    let calls = service.calls.lock().expect("memory calls lock");
    assert_eq!(
        calls.len(),
        1,
        "memory is fetched once per run and cached; the second prompt build must reuse it"
    );
    assert_eq!(
        calls[0].query, "hello reborn",
        "the retrieval query is the latest user message"
    );
    assert_eq!(calls[0].max_snippets, 8);
    assert_eq!(calls[0].scope, run_context.scope);
    assert_eq!(calls[0].actor.user_id, owner);
}
