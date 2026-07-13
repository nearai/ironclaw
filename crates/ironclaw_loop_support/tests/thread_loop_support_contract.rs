use std::collections::HashMap;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

use async_trait::async_trait;
use ironclaw_host_api::{
    AgentId, CapabilityId, DispatchInputIssueCode, MissionId, ProjectId, ProviderToolName,
    ResourceScope, TenantId, ThreadId, UserId,
};
use ironclaw_loop_support::{
    EmptyLoopCapabilityPort, HostIdentityContextBuildError, HostIdentityContextCandidate,
    HostIdentityContextSource, HostIdentityMessageContent, HostManagedModelError,
    HostManagedModelErrorKind, HostManagedModelGateway, HostManagedModelMessageRole,
    HostManagedModelRequest, HostManagedModelResponse, HostManagedToolResultContent,
    HostSkillContextBuildError, HostSkillContextCandidate, HostSkillContextSource,
    IdentityApplicability, IdentityBudget, IdentityFileName, LoopAttachmentReadError,
    LoopAttachmentReadPort, PromptContextTokenBudget, SkillBundleContextSource,
    SkillBundleDescriptor, SkillBundleId, SkillBundleSource, SkillBundleSourceError, SkillFilePath,
    SkillSourceKind, ThreadBackedLoopContextPort, ThreadBackedLoopModelPort,
    ThreadBackedLoopTranscriptPort, ThreadContextWindowCache, build_skill_run_snapshot,
    identity_message_ref,
};
use ironclaw_skills::SkillTrust;
use ironclaw_threads::{
    AcceptInboundMessageRequest, AcceptedInboundMessage, AcceptedInboundMessageReplay,
    AppendAssistantDraftRequest, AppendCapabilityDisplayPreviewRequest,
    AppendToolResultReferenceRequest, AttachmentKind, AttachmentRef, ContextMessage,
    ContextMessages, ContextWindow, CreateSummaryArtifactRequest, EnsureThreadRequest,
    InMemorySessionThreadService, LoadContextMessagesRequest, MessageContent, MessageKind,
    MessageStatus, ProviderToolCallReferenceEnvelope, RedactMessageRequest,
    ReplayAcceptedInboundMessageRequest, SessionThreadError, SessionThreadRecord,
    SessionThreadService, SummaryArtifact, SummaryModelContextPolicy, ThreadHistory,
    ThreadHistoryRequest, ThreadMessageId, ThreadMessageRecord, ThreadScope,
    ToolResultReferenceEnvelope, ToolResultSafeSummary, UpdateAssistantDraftRequest,
    UpdateToolResultReferenceRequest,
};
use ironclaw_turns::{
    LoopMessageRef, LoopResultRef, RunProfileResolutionRequest, RunProfileResolver, TurnActor,
    TurnId, TurnRunId, TurnScope,
    run_profile::{
        AgentLoopHostError, AgentLoopHostErrorKind, AgentLoopHostErrorReasonKind,
        AppendCapabilityResultRef, AssistantReply, BeginAssistantDraft, CapabilityBatchInvocation,
        CapabilityBatchOutcome, CapabilityDenied, CapabilityDeniedReasonKind, CapabilityInputIssue,
        CapabilityInputRef, CapabilityInvocation, CapabilityOutcome, CapabilitySurfaceVersion,
        FinalizeAssistantMessage, HostManagedLoopPromptPort,
        InMemoryInstructionMaterializationStore, InMemoryLoopHostMilestoneSink,
        InMemoryRunProfileResolver, LoopCapabilityPort, LoopContextBundle,
        LoopContextCompactionKind, LoopContextMessage, LoopContextPort, LoopContextRequest,
        LoopContextSnippet, LoopDriverNoteKind, LoopHostMilestoneKind, LoopHostMilestoneSink,
        LoopInputCursor, LoopInputCursorToken, LoopModelCapabilityView, LoopModelMessage,
        LoopModelPort, LoopModelRequest, LoopModelRouteSnapshot, LoopPromptBundle,
        LoopPromptBundleAuthority, LoopPromptBundleRef, LoopPromptBundleRequest, LoopPromptPort,
        LoopRunContext, LoopTranscriptPort, ModelVisibleToolObservation, ObservationTrust,
        ParentLoopOutput, PersonalContextPolicy, PromptMode, PromptSkillContextMetadata,
        ProviderToolCallReference, ProviderToolDefinition, SkillVisibility, ToolObservationDetail,
        ToolObservationStatus, UpdateAssistantDraft, VisibleCapabilityRequest,
        VisibleCapabilitySurface,
    },
};
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
            mode: ironclaw_turns::run_profile::PromptMode::TextOnly,
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
            mode: ironclaw_turns::run_profile::PromptMode::TextOnly,
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
        .build_prompt_bundle(ironclaw_turns::run_profile::LoopPromptBundleRequest {
            mode: ironclaw_turns::run_profile::PromptMode::TextOnly,
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
            mode: ironclaw_turns::run_profile::PromptMode::TextOnly,
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
            mode: ironclaw_turns::run_profile::PromptMode::TextOnly,
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
            mode: ironclaw_turns::run_profile::PromptMode::TextOnly,
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
            mode: ironclaw_turns::run_profile::PromptMode::TextOnly,
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
            mode: ironclaw_turns::run_profile::PromptMode::TextOnly,
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
            mode: ironclaw_turns::run_profile::PromptMode::TextOnly,
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
            mode: ironclaw_turns::run_profile::PromptMode::TextOnly,
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
            mode: ironclaw_turns::run_profile::PromptMode::TextOnly,
        })
        .await
        .unwrap();
    let second = adapter
        .load_loop_context(LoopContextRequest {
            after: None,
            limit: 16,
            mode: ironclaw_turns::run_profile::PromptMode::TextOnly,
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
    let materialization_store = Arc::new(InMemoryInstructionMaterializationStore::default());
    let prompt_port =
        HostManagedLoopPromptPort::new(fixture.run_context.clone(), context_port, milestones)
            .with_instruction_materialization_store(materialization_store);
    let prompt_bundle = prompt_port
        .build_prompt_bundle(ironclaw_turns::run_profile::LoopPromptBundleRequest {
            mode: ironclaw_turns::run_profile::PromptMode::TextOnly,
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
            mode: ironclaw_turns::run_profile::PromptMode::TextOnly,
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
            mode: ironclaw_turns::run_profile::PromptMode::TextOnly,
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
            mode: ironclaw_turns::run_profile::PromptMode::TextOnly,
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
            mode: ironclaw_turns::run_profile::PromptMode::TextOnly,
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
        .build_prompt_bundle(ironclaw_turns::run_profile::LoopPromptBundleRequest {
            mode: ironclaw_turns::run_profile::PromptMode::TextOnly,
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
        LoopMessageRef::new("msg:snippet.skill.alpha.0.48cf250f887a9389").unwrap()
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
        .build_prompt_bundle(ironclaw_turns::run_profile::LoopPromptBundleRequest {
            mode: ironclaw_turns::run_profile::PromptMode::TextOnly,
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
    let materialization_store = Arc::new(InMemoryInstructionMaterializationStore::default());
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
        .build_prompt_bundle(ironclaw_turns::run_profile::LoopPromptBundleRequest {
            mode: ironclaw_turns::run_profile::PromptMode::TextOnly,
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
        .build_prompt_bundle(ironclaw_turns::run_profile::LoopPromptBundleRequest {
            mode: ironclaw_turns::run_profile::PromptMode::TextOnly,
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
                trust_level: "installed".to_string(),
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
        .build_prompt_bundle(ironclaw_turns::run_profile::LoopPromptBundleRequest {
            mode: ironclaw_turns::run_profile::PromptMode::TextOnly,
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
                trust_level: "installed".to_string(),
            },
            PromptSkillContextMetadata {
                ordinal: 1,
                source_name: "bravo".to_string(),
                trust_level: "trusted".to_string(),
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
        .build_prompt_bundle(ironclaw_turns::run_profile::LoopPromptBundleRequest {
            mode: ironclaw_turns::run_profile::PromptMode::TextOnly,
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
        .build_prompt_bundle(ironclaw_turns::run_profile::LoopPromptBundleRequest {
            mode: ironclaw_turns::run_profile::PromptMode::TextOnly,
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
            mode: ironclaw_turns::run_profile::PromptMode::TextOnly,
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
            mode: ironclaw_turns::run_profile::PromptMode::TextOnly,
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
            mode: ironclaw_turns::run_profile::PromptMode::TextOnly,
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
async fn transcript_port_appends_tool_result_reference_envelope_idempotently() {
    let fixture = ThreadFixture::new().await;
    let adapter = ThreadBackedLoopTranscriptPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
    );
    let result_ref = LoopResultRef::new("result:demo-tool").unwrap();

    let first_ref = adapter
        .append_capability_result_ref(AppendCapabilityResultRef {
            result_ref: result_ref.clone(),
            safe_summary: "tool completed".to_string(),
            provider_call: Some(ProviderToolCallReference {
                provider_id: "test-provider".to_string(),
                provider_model_id: "test-model".to_string(),
                provider_turn_id: "turn_1".to_string(),
                provider_call_id: "call_1".to_string(),
                provider_tool_name: ProviderToolName::new("demo__echo")
                    .expect("provider tool name"),
                capability_id: CapabilityId::new("demo.echo").unwrap(),
                arguments: serde_json::json!({"message":"hello"}),
                response_reasoning: Some("provider reasoning".to_string()),
                reasoning: Some("provider reasoning".to_string()),
                signature: Some("sig-1".to_string()),
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
        Some("provider reasoning")
    );
    assert_eq!(
        provider_call.reasoning.as_deref(),
        Some("provider reasoning")
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

#[tokio::test]
async fn transcript_port_rejects_unsafe_tool_result_summary() {
    let fixture = ThreadFixture::new().await;
    let adapter = ThreadBackedLoopTranscriptPort::new(
        Arc::clone(&fixture.thread_service),
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
    );

    let error = adapter
        .append_capability_result_ref(AppendCapabilityResultRef {
            result_ref: LoopResultRef::new("result:unsafe-tool").unwrap(),
            safe_summary: "raw tool input includes secret".to_string(),
            provider_call: None,
            model_observation: None,
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
    assert!(
        !history
            .messages
            .iter()
            .any(|message| message.kind == MessageKind::ToolResultReference)
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
        .invoke_capability(CapabilityInvocation {
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
        .invoke_capability_batch(ironclaw_turns::run_profile::CapabilityBatchInvocation {
            invocations: vec![CapabilityInvocation {
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

    assert!(matches!(
        outcome.outcomes.as_slice(),
        [CapabilityOutcome::Denied(denied)]
            if denied.reason_kind == CapabilityDeniedReasonKind::EmptySurface
    ));
}

#[tokio::test]
async fn empty_capability_batch_rejects_stale_surface() {
    let port = EmptyLoopCapabilityPort;

    let error = port
        .invoke_capability_batch(ironclaw_turns::run_profile::CapabilityBatchInvocation {
            invocations: vec![CapabilityInvocation {
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
/// unit-tested in `ironclaw_runner`; this closes the loop on the read side per
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
        .build_prompt_bundle(ironclaw_turns::run_profile::LoopPromptBundleRequest {
            mode: ironclaw_turns::run_profile::PromptMode::TextOnly,
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
            mode: ironclaw_turns::run_profile::PromptMode::TextOnly,
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
    let gateway = Arc::new(RecordingGateway::deny_with_safe_summary(
        "RAW_PROVIDER_SECRET invalid api key sk-provider-secret /host/path tool_input",
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
            capability_view: None,
        })
        .await
        .unwrap_err();

    assert_eq!(error.kind, AgentLoopHostErrorKind::PolicyDenied);
    assert_eq!(error.safe_summary, "model profile is not permitted");
    let wire = format!("{}{:?}", serde_json::to_string(&error).unwrap(), error);
    for forbidden in [
        "RAW_PROVIDER_SECRET",
        "invalid api key",
        "sk-provider-secret",
        "/host/path",
        "tool_input",
    ] {
        assert!(!wire.contains(forbidden), "model error leaked {forbidden}");
    }
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
    ) -> Result<LoopContextBundle, ironclaw_turns::run_profile::AgentLoopHostError> {
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
    attempts: Mutex<Vec<ironclaw_turns::run_profile::LoopHostMilestone>>,
}

impl FailOnceMilestoneSink {
    fn milestones(&self) -> Vec<ironclaw_turns::run_profile::LoopHostMilestone> {
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
) -> Vec<ironclaw_turns::run_profile::LoopHostMilestone> {
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
impl ironclaw_turns::run_profile::LoopHostMilestoneSink for FailOnceMilestoneSink {
    async fn publish_loop_milestone(
        &self,
        milestone: ironclaw_turns::run_profile::LoopHostMilestone,
    ) -> Result<(), ironclaw_turns::run_profile::AgentLoopHostError> {
        let mut attempts = self.attempts.lock().unwrap();
        if attempts.is_empty() {
            attempts.push(milestone);
            return Err(ironclaw_turns::run_profile::AgentLoopHostError::new(
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
    published: Mutex<Vec<ironclaw_turns::run_profile::LoopHostMilestone>>,
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
impl ironclaw_turns::run_profile::LoopHostMilestoneSink for FailOnModelStartedMilestoneSink {
    async fn publish_loop_milestone(
        &self,
        milestone: ironclaw_turns::run_profile::LoopHostMilestone,
    ) -> Result<(), ironclaw_turns::run_profile::AgentLoopHostError> {
        if matches!(milestone.kind, LoopHostMilestoneKind::ModelStarted { .. }) {
            return Err(ironclaw_turns::run_profile::AgentLoopHostError::new(
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
    published: Mutex<Vec<ironclaw_turns::run_profile::LoopHostMilestone>>,
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
impl ironclaw_turns::run_profile::LoopHostMilestoneSink for FailOnModelCompletedMilestoneSink {
    async fn publish_loop_milestone(
        &self,
        milestone: ironclaw_turns::run_profile::LoopHostMilestone,
    ) -> Result<(), ironclaw_turns::run_profile::AgentLoopHostError> {
        if matches!(milestone.kind, LoopHostMilestoneKind::ModelCompleted { .. }) {
            return Err(ironclaw_turns::run_profile::AgentLoopHostError::new(
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
}

impl StaticToolDefinitionPort {
    fn new(definitions: Vec<ProviderToolDefinition>) -> Self {
        Self { definitions }
    }
}

#[async_trait]
impl LoopCapabilityPort for StaticToolDefinitionPort {
    fn tool_definitions(&self) -> Result<Vec<ProviderToolDefinition>, AgentLoopHostError> {
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
        _request: CapabilityInvocation,
    ) -> Result<CapabilityOutcome, AgentLoopHostError> {
        Ok(CapabilityOutcome::Denied(CapabilityDenied {
            reason_kind: CapabilityDeniedReasonKind::EmptySurface,
            safe_summary: "test capability port does not execute tools".to_string(),
        }))
    }

    async fn invoke_capability_batch(
        &self,
        request: CapabilityBatchInvocation,
    ) -> Result<CapabilityBatchOutcome, AgentLoopHostError> {
        Ok(CapabilityBatchOutcome {
            outcomes: request
                .invocations
                .into_iter()
                .map(|_| {
                    CapabilityOutcome::Denied(CapabilityDenied {
                        reason_kind: CapabilityDeniedReasonKind::EmptySurface,
                        safe_summary: "test capability port does not execute tools".to_string(),
                    })
                })
                .collect(),
            stopped_on_suspension: false,
        })
    }
}
