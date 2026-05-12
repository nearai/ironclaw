use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use async_trait::async_trait;
use ironclaw_host_api::{AgentId, CapabilityId, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_loop_support::{
    EmptyLoopCapabilityPort, HostManagedModelError, HostManagedModelErrorKind,
    HostManagedModelGateway, HostManagedModelMessageRole, HostManagedModelRequest,
    HostManagedModelResponse, HostSkillContextBuildError, HostSkillContextCandidate,
    HostSkillContextSource, ThreadBackedLoopContextPort, ThreadBackedLoopModelPort,
    ThreadBackedLoopTranscriptPort, build_skill_run_snapshot,
};
use ironclaw_skills::SkillTrust;
use ironclaw_threads::{
    AcceptInboundMessageRequest, AcceptedInboundMessage, AcceptedInboundMessageReplay,
    AppendAssistantDraftRequest, ContextMessage, ContextWindow, CreateSummaryArtifactRequest,
    EnsureThreadRequest, InMemorySessionThreadService, MessageContent, MessageKind, MessageStatus,
    RedactMessageRequest, ReplayAcceptedInboundMessageRequest, SessionThreadError,
    SessionThreadRecord, SessionThreadService, SummaryArtifact, ThreadHistory,
    ThreadHistoryRequest, ThreadMessageId, ThreadMessageRecord, ThreadScope,
    UpdateAssistantDraftRequest,
};
use ironclaw_turns::{
    LoopMessageRef, RunProfileResolutionRequest, RunProfileResolver, TurnActor, TurnId, TurnRunId,
    TurnScope,
    run_profile::{
        AgentLoopHostErrorKind, AssistantReply, BeginAssistantDraft, CapabilityDeniedReasonKind,
        CapabilityInputRef, CapabilityInvocation, CapabilityOutcome, CapabilitySurfaceVersion,
        FinalizeAssistantMessage, HostManagedLoopPromptPort, InMemoryLoopHostMilestoneSink,
        InMemoryRunProfileResolver, LoopCapabilityPort, LoopContextPort, LoopContextRequest,
        LoopHostMilestoneKind, LoopInputCursor, LoopInputCursorToken, LoopModelMessage,
        LoopModelPort, LoopModelRequest, LoopModelRouteSnapshot, LoopPromptPort, LoopRunContext,
        LoopTranscriptPort, ParentLoopOutput, PromptSkillContextMetadata, SkillVisibility,
        UpdateAssistantDraft, VisibleCapabilityRequest,
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
        })
        .await
        .unwrap();

    assert_eq!(bundle.messages.len(), 1);
    assert_eq!(bundle.messages[0].role, "user");
    assert_eq!(bundle.messages[0].safe_summary, "user message available");
    assert!(!bundle.messages[0].safe_summary.contains("hello reborn"));
    assert_eq!(
        bundle.messages[0].message_ref.as_str(),
        format!("msg:{}", fixture.user_message_id).as_str()
    );
    assert!(bundle.memory_snippets.is_empty());
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
            summary_kind: "model_context".to_string(),
            content: MessageContent::text("summarized hello"),
            model_context_policy: Some("replace_range_when_selected".to_string()),
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
            .as_str()
            .starts_with("msg:summary-")
    );
    assert!(bundle.instruction_snippets.is_empty());
}

#[tokio::test]
async fn thread_context_port_builds_skill_instruction_snippets_from_real_skill_md() {
    let fixture = ThreadFixture::new().await;
    let source = Arc::new(StaticSkillContextSource::new(vec![
        HostSkillContextCandidate::new(
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
        })
        .await
        .unwrap();

    assert_eq!(bundle.instruction_snippets.len(), 1);
    assert_eq!(bundle.instruction_snippets[0].snippet_ref, "skill:alpha");
    assert!(
        bundle.instruction_snippets[0]
            .safe_summary
            .contains("safe alpha description")
    );
    assert!(
        bundle.instruction_snippets[0]
            .safe_summary
            .contains("Use alpha prompt content.")
    );
    assert!(!bundle.instruction_snippets[0].safe_summary.contains("/tmp"));
}

#[tokio::test]
async fn thread_context_port_filters_skill_visibility_and_installed_prompt_content() {
    let fixture = ThreadFixture::new().await;
    let source = Arc::new(StaticSkillContextSource::new(vec![
        HostSkillContextCandidate::new(
            skill_md("alpha", "installed description", "installed prompt secret"),
            Some(SkillTrust::Installed),
            Some(SkillVisibility::Visible),
        ),
        HostSkillContextCandidate::new(
            skill_md("hidden", "hidden description", "hidden prompt"),
            Some(SkillTrust::Trusted),
            Some(SkillVisibility::Hidden),
        ),
        HostSkillContextCandidate::new(
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
    let serialized = serde_json::to_string(&bundle).unwrap();
    assert!(!serialized.contains("hidden"));
    assert!(!serialized.contains("denied"));
}

#[test]
fn skill_snapshot_builder_drops_installed_prompt_content_before_snapshot_storage() {
    let snapshot = build_skill_run_snapshot(vec![HostSkillContextCandidate::new(
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
        HostSkillContextCandidate::new(
            "not valid SKILL.md",
            Some(SkillTrust::Trusted),
            Some(SkillVisibility::Hidden),
        ),
        HostSkillContextCandidate::unavailable(
            Some(SkillTrust::Trusted),
            Some(SkillVisibility::Denied),
        ),
        HostSkillContextCandidate::new(
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
        })
        .await
        .unwrap();

    assert_eq!(bundle.instruction_snippets.len(), 1);
    assert_eq!(bundle.instruction_snippets[0].snippet_ref, "skill:alpha");
    assert!(
        bundle.instruction_snippets[0]
            .safe_summary
            .contains("visible prompt")
    );
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
        })
        .await
        .unwrap_err();

    assert_eq!(error.kind, AgentLoopHostErrorKind::Unavailable);
}

#[tokio::test]
async fn thread_context_port_fails_closed_when_skill_policy_data_is_missing() {
    let fixture = ThreadFixture::new().await;
    let source = Arc::new(StaticSkillContextSource::new(vec![
        HostSkillContextCandidate::new(
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
        HostSkillContextCandidate::new(
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
        })
        .await
        .unwrap();
    assert_eq!(prompt_bundle.messages.len(), 2);
    assert_eq!(prompt_bundle.messages[0].role, "system");
    assert!(
        prompt_bundle.messages[0]
            .content_ref
            .as_str()
            .starts_with("msg:snippet.skill.alpha.")
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
            messages: prompt_bundle.messages,
            surface_version: None,
            model_preference: None,
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
async fn prompt_port_records_installed_skill_trust_metadata_without_prompt_payload() {
    let fixture = ThreadFixture::new().await;
    let source = Arc::new(StaticSkillContextSource::new(vec![
        HostSkillContextCandidate::new(
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
async fn prompt_and_model_ports_keep_duplicate_skill_names_distinct() {
    let fixture = ThreadFixture::new().await;
    let source = Arc::new(StaticSkillContextSource::new(vec![
        HostSkillContextCandidate::new(
            skill_md("alpha", "first description", "first prompt"),
            Some(SkillTrust::Trusted),
            Some(SkillVisibility::Visible),
        )
        .with_ordering_key("alpha-1"),
        HostSkillContextCandidate::new(
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
            messages: prompt_bundle.messages,
            surface_version: None,
            model_preference: None,
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
        HostSkillContextCandidate::new(
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
        })
        .await
        .unwrap();

    source.set(vec![HostSkillContextCandidate::new(
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
            messages: prompt_bundle.messages,
            surface_version: None,
            model_preference: None,
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

#[tokio::test]
async fn transcript_port_retries_assistant_reply_finalized_milestone_after_transient_sink_failure()
{
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

    let first_error = adapter
        .finalize_assistant_message(request.clone())
        .await
        .unwrap_err();
    assert_eq!(first_error.kind, AgentLoopHostErrorKind::Unavailable);

    let message_ref = adapter.finalize_assistant_message(request).await.unwrap();

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
            surface_version: CapabilitySurfaceVersion::new("empty:v1").unwrap(),
            capability_id: CapabilityId::new("demo.echo").unwrap(),
            input_ref: CapabilityInputRef::new("input:opaque").unwrap(),
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
                surface_version: CapabilitySurfaceVersion::new("empty:v1").unwrap(),
                capability_id: CapabilityId::new("demo.echo").unwrap(),
                input_ref: CapabilityInputRef::new("input:opaque").unwrap(),
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
                surface_version: CapabilitySurfaceVersion::new("nonempty:v1").unwrap(),
                capability_id: CapabilityId::new("demo.echo").unwrap(),
                input_ref: CapabilityInputRef::new("input:opaque").unwrap(),
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

    let response = port
        .stream_model(LoopModelRequest {
            messages: vec![LoopModelMessage {
                role: "user".to_string(),
                content_ref: LoopMessageRef::new(format!("msg:{}", fixture.user_message_id))
                    .unwrap(),
            }],
            surface_version: None,
            model_preference: None,
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

    port.stream_model(LoopModelRequest {
        messages: vec![LoopModelMessage {
            role: "user".to_string(),
            content_ref: LoopMessageRef::new(format!("msg:{}", fixture.user_message_id)).unwrap(),
        }],
        surface_version: None,
        model_preference: None,
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

    port.stream_model(LoopModelRequest {
        messages: vec![LoopModelMessage {
            role: "user".to_string(),
            content_ref: LoopMessageRef::new(format!("msg:{}", fixture.user_message_id)).unwrap(),
        }],
        surface_version: None,
        model_preference: None,
    })
    .await
    .unwrap();

    let calls = gateway.calls.lock().unwrap();
    assert_eq!(calls[0].messages[0].content, "hello reborn");
}

#[tokio::test]
async fn model_port_round_trips_tool_result_reference_context_as_system_model_input() {
    let fixture = ThreadFixture::new().await;
    let tool_result_ref = LoopMessageRef::new("msg:11111111-1111-1111-1111-111111111111").unwrap();
    let thread_service = Arc::new(StaticContextThreadService::new(ContextMessage {
        message_id: Some(ThreadMessageId::parse("11111111-1111-1111-1111-111111111111").unwrap()),
        summary_id: None,
        sequence: 1,
        kind: MessageKind::ToolResultReference,
        content: "tool result content".to_string(),
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
        })
        .await
        .unwrap();
    assert_eq!(context.messages[0].role, "tool_result_reference");
    assert_eq!(context.messages[0].message_ref, tool_result_ref);

    let gateway = Arc::new(RecordingGateway::reply("model says hi"));
    let model_port = ThreadBackedLoopModelPort::new(
        thread_service,
        fixture.thread_scope.clone(),
        fixture.run_context.clone(),
        gateway.clone(),
        16,
    );

    model_port
        .stream_model(LoopModelRequest {
            messages: context
                .messages
                .into_iter()
                .map(|message| LoopModelMessage {
                    role: message.role,
                    content_ref: message.message_ref,
                })
                .collect(),
            surface_version: None,
            model_preference: None,
        })
        .await
        .unwrap();

    let calls = gateway.calls.lock().unwrap();
    assert_eq!(
        calls[0].messages[0].role,
        HostManagedModelMessageRole::System
    );
    assert_eq!(calls[0].messages[0].content, "tool result content");
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

    let response = port
        .stream_model(LoopModelRequest {
            messages: vec![LoopModelMessage {
                role: "user".to_string(),
                content_ref: LoopMessageRef::new(format!("msg:{}", fixture.user_message_id))
                    .unwrap(),
            }],
            surface_version: None,
            model_preference: Some(
                fixture
                    .run_context
                    .resolved_run_profile
                    .model_profile_id
                    .clone(),
            ),
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
async fn model_port_emits_only_started_milestone_when_gateway_fails() {
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

    let error = port
        .stream_model(LoopModelRequest {
            messages: vec![LoopModelMessage {
                role: "user".to_string(),
                content_ref: LoopMessageRef::new(format!("msg:{}", fixture.user_message_id))
                    .unwrap(),
            }],
            surface_version: None,
            model_preference: None,
        })
        .await
        .unwrap_err();

    assert_eq!(error.kind, AgentLoopHostErrorKind::PolicyDenied);
    let milestones = milestone_sink.milestones();
    assert_eq!(milestones.len(), 1);
    assert!(matches!(
        &milestones[0].kind,
        LoopHostMilestoneKind::ModelStarted {
            requested_model_profile_id: None
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

    let response = port
        .stream_model(LoopModelRequest {
            messages: vec![LoopModelMessage {
                role: "user".to_string(),
                content_ref: LoopMessageRef::new(format!("msg:{}", fixture.user_message_id))
                    .unwrap(),
            }],
            surface_version: None,
            model_preference: None,
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

    let error = port
        .stream_model(LoopModelRequest {
            messages: vec![LoopModelMessage {
                role: "system".to_string(),
                content_ref: LoopMessageRef::new(format!("msg:{}", fixture.user_message_id))
                    .unwrap(),
            }],
            surface_version: None,
            model_preference: None,
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

    let error = port
        .stream_model(LoopModelRequest {
            messages: vec![LoopModelMessage {
                role: "user".to_string(),
                content_ref: LoopMessageRef::new(format!("msg:{}", fixture.user_message_id))
                    .unwrap(),
            }],
            surface_version: None,
            model_preference: None,
        })
        .await
        .unwrap_err();

    assert_eq!(error.kind, AgentLoopHostErrorKind::PolicyDenied);
    let wire = serde_json::to_string(&error).unwrap();
    assert!(!wire.contains("RAW_PROVIDER_SECRET"));
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

    async fn mark_message_deferred_busy(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        message_id: ThreadMessageId,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        self.inner
            .mark_message_deferred_busy(scope, thread_id, message_id)
            .await
    }

    async fn append_assistant_draft(
        &self,
        request: AppendAssistantDraftRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        self.inner.append_assistant_draft(request).await
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
        self.inner.load_context_window(request).await
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
}

impl StaticContextThreadService {
    fn new(context_message: ContextMessage) -> Self {
        Self { context_message }
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

    async fn mark_message_deferred_busy(
        &self,
        _scope: &ThreadScope,
        _thread_id: &ThreadId,
        _message_id: ThreadMessageId,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        panic!("static context service does not defer messages")
    }

    async fn append_assistant_draft(
        &self,
        _request: AppendAssistantDraftRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        panic!("static context service does not append assistant drafts")
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
        Ok(ContextWindow {
            thread_id: request.thread_id,
            messages: vec![self.context_message.clone()],
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
    response: Result<HostManagedModelResponse, HostManagedModelError>,
}

impl RecordingGateway {
    fn reply(content: &str) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            response: Ok(HostManagedModelResponse::assistant_reply(
                content.to_string(),
            )),
        }
    }

    fn deny(raw_detail: &str) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            response: Err(HostManagedModelError::new(
                HostManagedModelErrorKind::PolicyDenied,
                raw_detail,
            )),
        }
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
}
