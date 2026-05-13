//! Contract tests for the InboundTurnService.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_host_api::{AgentId, TenantId, ThreadId, UserId};
use ironclaw_product_adapters::{
    AdapterInstallationId, AuthRequirement, ExternalActorRef, ExternalConversationRef,
    ExternalEventId, ParsedProductInbound, ProductAdapterId, ProductInboundEnvelope,
    ProductInboundPayload, ProductTriggerReason, ProtocolAuthEvidence, TrustedInboundContext,
    UserMessagePayload,
};
use ironclaw_product_workflow::{
    DefaultInboundTurnService, FakeBeforeInboundPolicy, FakeConversationBindingService,
    InboundTurnOutcome, InboundTurnService, ProductWorkflowError,
};
use ironclaw_threads::{
    InMemorySessionThreadService, MessageStatus, SessionThreadService, ThreadHistoryRequest,
    ThreadScope,
};
use ironclaw_turns::{
    CancelRunRequest, CancelRunResponse, DefaultTurnCoordinator, EventCursor, GetRunStateRequest,
    InMemoryTurnStateStore, ResumeTurnRequest, ResumeTurnResponse, RunProfileId, RunProfileVersion,
    SubmitTurnRequest, SubmitTurnResponse, ThreadBusy, TurnCoordinator, TurnError, TurnId,
    TurnRunId, TurnRunState, TurnStatus,
};

fn sample_user_message_envelope(event_suffix: &str) -> ProductInboundEnvelope {
    sample_user_message_envelope_with_install_and_text(event_suffix, "install_alpha", "hello world")
}

#[derive(Default)]
struct CapturingTurnCoordinator {
    last_submit: Arc<Mutex<Option<SubmitTurnRequest>>>,
}

#[async_trait]
impl TurnCoordinator for CapturingTurnCoordinator {
    async fn submit_turn(
        &self,
        request: SubmitTurnRequest,
    ) -> Result<SubmitTurnResponse, TurnError> {
        let response = SubmitTurnResponse::Accepted {
            turn_id: TurnId::new(),
            run_id: TurnRunId::new(),
            status: TurnStatus::Queued,
            resolved_run_profile_id: RunProfileId::default_profile(),
            resolved_run_profile_version: RunProfileVersion::new(1),
            event_cursor: EventCursor::default(),
            accepted_message_ref: request.accepted_message_ref.clone(),
            reply_target_binding_ref: request.reply_target_binding_ref.clone(),
        };
        *self
            .last_submit
            .lock()
            .expect("capturing coordinator lock poisoned") = Some(request);
        Ok(response)
    }

    async fn resume_turn(
        &self,
        _request: ResumeTurnRequest,
    ) -> Result<ResumeTurnResponse, TurnError> {
        panic!("resume_turn is not used by inbound turn contract tests")
    }

    async fn cancel_run(&self, _request: CancelRunRequest) -> Result<CancelRunResponse, TurnError> {
        panic!("cancel_run is not used by inbound turn contract tests")
    }

    async fn get_run_state(&self, _request: GetRunStateRequest) -> Result<TurnRunState, TurnError> {
        panic!("get_run_state is not used by inbound turn contract tests")
    }
}

#[derive(Clone, Default)]
struct ScriptedTurnCoordinator {
    results: Arc<Mutex<VecDeque<Result<SubmitTurnResponse, TurnError>>>>,
    submissions: Arc<Mutex<Vec<SubmitTurnRequest>>>,
}

impl ScriptedTurnCoordinator {
    fn push_result(&self, result: Result<SubmitTurnResponse, TurnError>) {
        self.results
            .lock()
            .expect("scripted coordinator lock poisoned")
            .push_back(result);
    }

    fn submissions(&self) -> Vec<SubmitTurnRequest> {
        self.submissions
            .lock()
            .expect("scripted coordinator submissions lock poisoned")
            .clone()
    }
}

#[async_trait]
impl TurnCoordinator for ScriptedTurnCoordinator {
    async fn submit_turn(
        &self,
        request: SubmitTurnRequest,
    ) -> Result<SubmitTurnResponse, TurnError> {
        self.submissions
            .lock()
            .expect("scripted coordinator submissions lock poisoned")
            .push(request.clone());
        self.results
            .lock()
            .expect("scripted coordinator lock poisoned")
            .pop_front()
            .unwrap_or_else(|| {
                Ok(SubmitTurnResponse::Accepted {
                    turn_id: TurnId::new(),
                    run_id: TurnRunId::new(),
                    status: TurnStatus::Queued,
                    resolved_run_profile_id: RunProfileId::default_profile(),
                    resolved_run_profile_version: RunProfileVersion::new(1),
                    event_cursor: EventCursor::default(),
                    accepted_message_ref: request.accepted_message_ref.clone(),
                    reply_target_binding_ref: request.reply_target_binding_ref.clone(),
                })
            })
    }

    async fn resume_turn(
        &self,
        _request: ResumeTurnRequest,
    ) -> Result<ResumeTurnResponse, TurnError> {
        panic!("resume_turn is not used by inbound turn contract tests")
    }

    async fn cancel_run(&self, _request: CancelRunRequest) -> Result<CancelRunResponse, TurnError> {
        panic!("cancel_run is not used by inbound turn contract tests")
    }

    async fn get_run_state(&self, _request: GetRunStateRequest) -> Result<TurnRunState, TurnError> {
        panic!("get_run_state is not used by inbound turn contract tests")
    }
}

fn binding_with_user(user: &str, thread: &str) -> ironclaw_product_workflow::ResolvedBinding {
    ironclaw_product_workflow::ResolvedBinding {
        tenant_id: TenantId::new("tenant:install_alpha").expect("valid tenant"),
        user_id: UserId::new(user).expect("valid user"),
        thread_id: ThreadId::new(thread).expect("valid thread"),
        agent_id: Some(AgentId::new("agent:fake").expect("valid agent")),
        project_id: None,
    }
}

fn sample_user_message_envelope_with_text(
    event_suffix: &str,
    text: &str,
) -> ProductInboundEnvelope {
    sample_user_message_envelope_with_install_and_text(event_suffix, "install_alpha", text)
}

fn sample_user_message_envelope_with_install_and_text(
    event_suffix: &str,
    installation_id: &str,
    text: &str,
) -> ProductInboundEnvelope {
    let evidence = ProtocolAuthEvidence::test_verified(
        AuthRequirement::SharedSecretHeader {
            header_name: "X-Secret".into(),
        },
        installation_id,
    );
    let context = TrustedInboundContext::from_verified_evidence(
        ProductAdapterId::new("test_adapter").expect("valid"),
        AdapterInstallationId::new(installation_id).expect("valid"),
        Utc::now(),
        &evidence,
    )
    .expect("verified");

    let parsed = ParsedProductInbound::new(
        ExternalEventId::new(format!("evt:{event_suffix}")).expect("valid"),
        ExternalActorRef::new("test", "user1", Option::<String>::None).expect("valid"),
        ExternalConversationRef::new(None, "conv1", None, None).expect("valid"),
        ProductInboundPayload::UserMessage(
            UserMessagePayload::new(text, vec![], ProductTriggerReason::DirectChat).expect("valid"),
        ),
    )
    .expect("parsed");

    ProductInboundEnvelope::from_trusted_parse(context, parsed).expect("envelope")
}

#[tokio::test]
async fn user_message_resolves_binding_persists_message_and_submits_turn() {
    let binding_service = FakeConversationBindingService::new();
    let thread_service = InMemorySessionThreadService::default();
    let store = Arc::new(InMemoryTurnStateStore::default());
    let coordinator = DefaultTurnCoordinator::new(store);
    let service =
        DefaultInboundTurnService::new(binding_service, thread_service.clone(), coordinator);

    let envelope = sample_user_message_envelope("turn1");
    let outcome: InboundTurnOutcome = service
        .accept_user_message(&envelope)
        .await
        .expect("submit");

    let binding = match &outcome {
        InboundTurnOutcome::Submitted { binding, .. } => binding,
        _ => panic!("expected Submitted, got {outcome:?}"),
    };

    let history = thread_service
        .list_thread_history(ThreadHistoryRequest {
            scope: ThreadScope {
                tenant_id: binding.tenant_id.clone(),
                agent_id: binding.agent_id.clone().expect("agent id"),
                project_id: binding.project_id.clone(),
                owner_user_id: Some(binding.user_id.clone()),
                mission_id: None,
            },
            thread_id: binding.thread_id.clone(),
        })
        .await
        .expect("history");
    assert_eq!(history.messages.len(), 1);
    assert_eq!(history.messages[0].content.as_deref(), Some("hello world"));
    assert_eq!(history.messages[0].status, MessageStatus::Submitted);
    assert!(history.messages[0].turn_run_id.is_some());
}

#[tokio::test]
async fn busy_thread_persists_second_message_as_deferred() {
    let binding_service = FakeConversationBindingService::new();
    let thread_service = InMemorySessionThreadService::default();
    let store = Arc::new(InMemoryTurnStateStore::default());
    let coordinator = DefaultTurnCoordinator::new(store);
    let service =
        DefaultInboundTurnService::new(binding_service, thread_service.clone(), coordinator);

    let first = sample_user_message_envelope("busy1");
    service.accept_user_message(&first).await.expect("first");
    let second = sample_user_message_envelope_with_text("busy2", "second");
    let outcome = service
        .accept_user_message(&second)
        .await
        .expect("second deferred");
    assert!(matches!(outcome, InboundTurnOutcome::DeferredBusy { .. }));

    let binding = match outcome {
        InboundTurnOutcome::DeferredBusy { binding, .. } => binding,
        _ => unreachable!(),
    };
    let history = thread_service
        .list_thread_history(ThreadHistoryRequest {
            scope: ThreadScope {
                tenant_id: binding.tenant_id.clone(),
                agent_id: binding.agent_id.clone().expect("agent id"),
                project_id: binding.project_id.clone(),
                owner_user_id: Some(binding.user_id.clone()),
                mission_id: None,
            },
            thread_id: binding.thread_id.clone(),
        })
        .await
        .expect("history");
    assert_eq!(history.messages.len(), 2);
    assert_eq!(history.messages[1].content.as_deref(), Some("second"));
    assert_eq!(history.messages[1].status, MessageStatus::DeferredBusy);
}

#[tokio::test]
async fn retry_replays_accepted_message_before_live_binding_resolution() {
    let binding_service = FakeConversationBindingService::new();
    let binding_handle = binding_service.clone();
    let thread_service = InMemorySessionThreadService::default();
    let coordinator = ScriptedTurnCoordinator::default();
    coordinator.push_result(Err(TurnError::Unavailable {
        reason: "transient submit failure".into(),
    }));
    let coordinator_handle = coordinator.clone();
    let service =
        DefaultInboundTurnService::new(binding_service, thread_service.clone(), coordinator);

    let envelope = sample_user_message_envelope("binding-churn");
    let first_err = service
        .accept_user_message(&envelope)
        .await
        .expect_err("first submit fails after message acceptance");
    assert!(matches!(
        first_err,
        ProductWorkflowError::TurnSubmissionFailed { .. }
    ));
    assert_eq!(binding_handle.resolve_count(), 1);

    binding_handle.program_binding(
        envelope.source_binding_key(),
        binding_with_user("user:churned", "thread:churned"),
    );

    let outcome = service
        .accept_user_message(&envelope)
        .await
        .expect("retry reuses accepted message");
    let InboundTurnOutcome::Submitted { binding, .. } = outcome else {
        panic!("expected submitted retry")
    };
    assert_eq!(binding.user_id.as_str(), "user:user1");
    assert_ne!(binding.thread_id.as_str(), "thread:churned");
    assert_eq!(
        binding_handle.resolve_count(),
        1,
        "retry must replay accepted message before live binding resolution"
    );

    let history = thread_service
        .list_thread_history(ThreadHistoryRequest {
            scope: ThreadScope {
                tenant_id: binding.tenant_id.clone(),
                agent_id: binding.agent_id.clone().expect("agent id"),
                project_id: binding.project_id.clone(),
                owner_user_id: Some(binding.user_id.clone()),
                mission_id: None,
            },
            thread_id: binding.thread_id.clone(),
        })
        .await
        .expect("history");
    assert_eq!(history.messages.len(), 1);
    assert_eq!(history.messages[0].status, MessageStatus::Submitted);
    let submissions = coordinator_handle.submissions();
    assert_eq!(submissions.len(), 2);
    assert_eq!(
        submissions[0].idempotency_key.as_str(),
        submissions[1].idempotency_key.as_str(),
        "retry after post-submit failure must reuse stable turn idempotency key"
    );
}

#[tokio::test]
async fn replay_lookup_is_namespaced_by_installation() {
    let binding_service = FakeConversationBindingService::new();
    let binding_handle = binding_service.clone();
    let thread_service = InMemorySessionThreadService::default();
    let coordinator = ScriptedTurnCoordinator::default();
    coordinator.push_result(Err(TurnError::Unavailable {
        reason: "transient submit failure".into(),
    }));
    let service =
        DefaultInboundTurnService::new(binding_service, thread_service.clone(), coordinator);

    let first = sample_user_message_envelope_with_install_and_text(
        "shared-event",
        "install_alpha",
        "alpha",
    );
    service
        .accept_user_message(&first)
        .await
        .expect_err("first submit fails after accepting alpha message");

    let second =
        sample_user_message_envelope_with_install_and_text("shared-event", "install_beta", "beta");
    let outcome = service
        .accept_user_message(&second)
        .await
        .expect("second install must not replay alpha message");
    let InboundTurnOutcome::Submitted { binding, .. } = outcome else {
        panic!("expected submitted beta message")
    };
    assert_eq!(binding.tenant_id.as_str(), "tenant:install_beta");
    assert_eq!(
        binding_handle.resolve_count(),
        2,
        "same conversation/event under another installation must resolve its own binding"
    );

    let history = thread_service
        .list_thread_history(ThreadHistoryRequest {
            scope: ThreadScope {
                tenant_id: binding.tenant_id.clone(),
                agent_id: binding.agent_id.clone().expect("agent id"),
                project_id: binding.project_id.clone(),
                owner_user_id: Some(binding.user_id.clone()),
                mission_id: None,
            },
            thread_id: binding.thread_id.clone(),
        })
        .await
        .expect("history");
    assert_eq!(history.messages.len(), 1);
    assert_eq!(history.messages[0].content.as_deref(), Some("beta"));
}

#[tokio::test]
async fn deferred_busy_retry_resubmits_existing_message() {
    let binding_service = FakeConversationBindingService::new();
    let thread_service = InMemorySessionThreadService::default();
    let coordinator = ScriptedTurnCoordinator::default();
    let active_run_id = TurnRunId::new();
    coordinator.push_result(Err(TurnError::ThreadBusy(ThreadBusy {
        active_run_id,
        status: TurnStatus::Running,
        event_cursor: EventCursor::default(),
    })));
    let service =
        DefaultInboundTurnService::new(binding_service, thread_service.clone(), coordinator);

    let envelope = sample_user_message_envelope("busy-retry-existing");
    let first = service
        .accept_user_message(&envelope)
        .await
        .expect("first busy");
    assert!(matches!(first, InboundTurnOutcome::DeferredBusy { .. }));

    let second = service
        .accept_user_message(&envelope)
        .await
        .expect("retry submits existing deferred message");
    let InboundTurnOutcome::Submitted { binding, .. } = second else {
        panic!("expected submitted retry")
    };
    let history = thread_service
        .list_thread_history(ThreadHistoryRequest {
            scope: ThreadScope {
                tenant_id: binding.tenant_id.clone(),
                agent_id: binding.agent_id.clone().expect("agent id"),
                project_id: binding.project_id.clone(),
                owner_user_id: Some(binding.user_id.clone()),
                mission_id: None,
            },
            thread_id: binding.thread_id.clone(),
        })
        .await
        .expect("history");
    assert_eq!(history.messages.len(), 1);
    assert_eq!(history.messages[0].status, MessageStatus::Submitted);
}

#[tokio::test]
async fn reply_target_binding_ref_has_single_reply_prefix() {
    let binding_service = FakeConversationBindingService::new();
    let thread_service = InMemorySessionThreadService::default();
    let coordinator = CapturingTurnCoordinator::default();
    let captured_submit = coordinator.last_submit.clone();
    let service = DefaultInboundTurnService::new(binding_service, thread_service, coordinator);

    let envelope = sample_user_message_envelope("reply-prefix");
    service
        .accept_user_message(&envelope)
        .await
        .expect("submit");

    let request = captured_submit
        .lock()
        .expect("captured submit lock poisoned")
        .clone()
        .expect("submit request captured");
    let reply_ref = request.reply_target_binding_ref.as_str();
    assert!(reply_ref.starts_with("reply:"));
    assert!(!reply_ref.starts_with("reply:reply:"));
    assert_eq!(reply_ref.matches("reply:").count(), 1);
}

#[tokio::test]
async fn max_valid_external_ids_do_not_overflow_turn_refs() {
    let binding_service = FakeConversationBindingService::new();
    let thread_service = InMemorySessionThreadService::default();
    let store = Arc::new(InMemoryTurnStateStore::default());
    let coordinator = DefaultTurnCoordinator::new(store);
    let service = DefaultInboundTurnService::new(binding_service, thread_service, coordinator);

    let long_event_id = "e".repeat(250);
    let envelope = sample_user_message_envelope(&long_event_id);
    service
        .accept_user_message(&envelope)
        .await
        .expect("long ids accepted");
}

#[tokio::test]
async fn overflowing_turn_ref_inputs_hash_deterministically() {
    let long_event_id = "e".repeat(250);
    let mut captured = Vec::new();

    for _ in 0..2 {
        let binding_service = FakeConversationBindingService::new();
        let thread_service = InMemorySessionThreadService::default();
        let coordinator = CapturingTurnCoordinator::default();
        let captured_submit = coordinator.last_submit.clone();
        let service = DefaultInboundTurnService::new(binding_service, thread_service, coordinator);

        let envelope = sample_user_message_envelope(&long_event_id);
        service
            .accept_user_message(&envelope)
            .await
            .expect("long id submit");
        let request = captured_submit
            .lock()
            .expect("captured submit lock poisoned")
            .clone()
            .expect("submit request captured");
        captured.push(request.idempotency_key.as_str().to_string());
    }

    assert_eq!(captured[0], captured[1]);
    assert!(captured[0].starts_with("turn:"));
    assert!(captured[0].len() < 64);
}

#[tokio::test]
async fn binding_failure_surfaces_workflow_error() {
    let binding_service = FakeConversationBindingService::new();
    binding_service.force_failure(ProductWorkflowError::BindingResolutionFailed {
        reason: "no tenant found".into(),
    });

    let thread_service = InMemorySessionThreadService::default();
    let store = Arc::new(InMemoryTurnStateStore::default());
    let coordinator = DefaultTurnCoordinator::new(store);
    let service = DefaultInboundTurnService::new(binding_service, thread_service, coordinator);

    let envelope = sample_user_message_envelope("fail1");
    let err = service
        .accept_user_message(&envelope)
        .await
        .expect_err("should fail");

    assert!(matches!(
        err,
        ProductWorkflowError::BindingResolutionFailed { .. }
    ));
}

#[tokio::test]
async fn before_inbound_rewrite_stages_rewritten_content_as_accepted_message() {
    let binding_service = FakeConversationBindingService::new();
    let thread_service = InMemorySessionThreadService::default();
    let store = Arc::new(InMemoryTurnStateStore::default());
    let coordinator = DefaultTurnCoordinator::new(store);
    let policy = Arc::new(FakeBeforeInboundPolicy::new());
    policy.program_continue(Some("rewritten text".to_string()));
    let policy_handle = policy.clone();
    let service =
        DefaultInboundTurnService::new(binding_service, thread_service.clone(), coordinator)
            .with_before_inbound_policy(policy);

    let envelope = sample_user_message_envelope_with_text("rewrite-evt", "original text");
    let outcome = service
        .accept_user_message(&envelope)
        .await
        .expect("submit");

    let binding = match &outcome {
        InboundTurnOutcome::Submitted { binding, .. } => binding,
        _ => panic!("expected Submitted, got {outcome:?}"),
    };

    let history = thread_service
        .list_thread_history(ThreadHistoryRequest {
            scope: ThreadScope {
                tenant_id: binding.tenant_id.clone(),
                agent_id: binding.agent_id.clone().expect("agent id"),
                project_id: binding.project_id.clone(),
                owner_user_id: Some(binding.user_id.clone()),
                mission_id: None,
            },
            thread_id: binding.thread_id.clone(),
        })
        .await
        .expect("history");
    assert_eq!(history.messages.len(), 1);
    assert_eq!(
        history.messages[0].content.as_deref(),
        Some("rewritten text"),
        "BeforeInbound rewrite must replace original text before staging"
    );
    assert_eq!(history.messages[0].status, MessageStatus::Submitted);
    assert!(
        history.messages[0].turn_run_id.is_some(),
        "rewritten message must still have been submitted to the coordinator"
    );
    assert_eq!(
        policy_handle.evaluate_count(),
        1,
        "BeforeInbound policy must be called exactly once on the new-message path"
    );
}

#[tokio::test]
async fn before_inbound_rejection_creates_no_accepted_message_and_no_run() {
    let binding_service = FakeConversationBindingService::new();
    let binding_handle = binding_service.clone();
    let thread_service = InMemorySessionThreadService::default();
    let coordinator = ScriptedTurnCoordinator::default();
    let coordinator_handle = coordinator.clone();
    let policy = Arc::new(FakeBeforeInboundPolicy::new());
    policy.program_reject("test policy denied");
    let policy_handle = policy.clone();
    let service =
        DefaultInboundTurnService::new(binding_service, thread_service.clone(), coordinator)
            .with_before_inbound_policy(policy);

    let envelope = sample_user_message_envelope_with_text("reject-evt", "blocked content");
    let err = service
        .accept_user_message(&envelope)
        .await
        .expect_err("rejection must surface as an error");

    let ProductWorkflowError::TurnSubmissionRejected { reason } = &err else {
        panic!("expected TurnSubmissionRejected, got {err:?}");
    };
    // `RedactedString` redacts via `Display`; the workflow boundary re-wraps
    // this string into a fresh `RedactedString` for the adapter-facing ack, so
    // the placeholder is what the workflow surfaces.
    assert_eq!(
        reason, "<redacted>",
        "rejection reason must be redacted by the time it reaches the workflow error"
    );

    assert_eq!(
        binding_handle.resolve_count(),
        0,
        "rejection must short-circuit before binding resolution"
    );
    assert_eq!(
        coordinator_handle.submissions().len(),
        0,
        "rejection must not call submit_turn"
    );
    assert_eq!(
        policy_handle.evaluate_count(),
        1,
        "BeforeInbound policy must be evaluated exactly once before rejection"
    );
}

#[tokio::test]
async fn before_inbound_continue_without_rewrite_passes_text_through() {
    let binding_service = FakeConversationBindingService::new();
    let thread_service = InMemorySessionThreadService::default();
    let store = Arc::new(InMemoryTurnStateStore::default());
    let coordinator = DefaultTurnCoordinator::new(store);
    // Default fake (no programming) → Continue { rewritten_text: None }.
    let policy = Arc::new(FakeBeforeInboundPolicy::new());
    let policy_handle = policy.clone();
    let service =
        DefaultInboundTurnService::new(binding_service, thread_service.clone(), coordinator)
            .with_before_inbound_policy(policy);

    let envelope = sample_user_message_envelope_with_text("passthrough-evt", "verbatim");
    let outcome = service
        .accept_user_message(&envelope)
        .await
        .expect("submit");

    let binding = match &outcome {
        InboundTurnOutcome::Submitted { binding, .. } => binding,
        _ => panic!("expected Submitted, got {outcome:?}"),
    };

    let history = thread_service
        .list_thread_history(ThreadHistoryRequest {
            scope: ThreadScope {
                tenant_id: binding.tenant_id.clone(),
                agent_id: binding.agent_id.clone().expect("agent id"),
                project_id: binding.project_id.clone(),
                owner_user_id: Some(binding.user_id.clone()),
                mission_id: None,
            },
            thread_id: binding.thread_id.clone(),
        })
        .await
        .expect("history");
    assert_eq!(history.messages.len(), 1);
    assert_eq!(
        history.messages[0].content.as_deref(),
        Some("verbatim"),
        "Continue with no rewrite must pass original text through unchanged"
    );
    assert_eq!(history.messages[0].status, MessageStatus::Submitted);
    assert_eq!(
        policy_handle.evaluate_count(),
        1,
        "BeforeInbound policy must still be evaluated when it returns Continue"
    );
}
