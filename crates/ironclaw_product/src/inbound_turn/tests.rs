use std::{
    collections::VecDeque,
    future::pending,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use crate::{
    AdapterInstallationId, DeliveryReport, ExternalActorRef, ExternalConversationRef,
    InboundOutcome, OutboundEnvelope, ProductAdapterId, ProductAttachmentDescriptor,
    ProductAttachmentKind, ProductRejectionKind, ProductTriggerReason, UserMessagePayload,
    VerifiedInbound,
};
use async_trait::async_trait;
use chrono::TimeZone;
use ironclaw_host_api::{
    ids::{AgentId, TenantId, ThreadId, UserId},
    tool_adapter::{RestrictedEgressError, RestrictedEgressRequest, RestrictedEgressResponse},
};
use ironclaw_threads::{
    AcceptInboundMessageRequest, AcceptedInboundMessage, AcceptedInboundMessageReplay,
    AppendAssistantDraftRequest, AppendCapabilityDisplayPreviewRequest,
    AppendToolResultReferenceRequest, ContextMessages, ContextWindow, CreateSummaryArtifactRequest,
    EnsureThreadRequest, ListThreadsForScopeRequest, ListThreadsForScopeResponse,
    LoadContextMessagesRequest, LoadContextWindowRequest, MessageContent, RedactMessageRequest,
    ReplayAcceptedInboundMessageRequest, SessionThreadError, SessionThreadRecord, SummaryArtifact,
    ThreadHistory, ThreadHistoryRequest, ThreadMessageId, ThreadMessageRecord, ThreadScope,
    UpdateAssistantDraftRequest, UpdateToolResultReferenceRequest,
};
use ironclaw_turns::{
    CancelRunRequest, CancelRunResponse, EventCursor, GetRunStateRequest, ResumeTurnRequest,
    ResumeTurnResponse, RetryTurnRequest, RetryTurnResponse, RunProfileId, RunProfileVersion,
    SubmitTurnRequest, SubmitTurnResponse, TurnCoordinator, TurnError, TurnId, TurnOriginKind,
    TurnRunId, TurnRunState, TurnScope, TurnStatus, TurnSurfaceType,
};

use crate::action::SourceBindingKey;

use super::*;

// --- Minimal stubs for submit path tests ---

#[derive(Default)]
struct CapturingTurnCoordinator {
    submissions: Mutex<Vec<SubmitTurnRequest>>,
}

impl CapturingTurnCoordinator {
    fn submissions(&self) -> Vec<SubmitTurnRequest> {
        self.submissions.lock().unwrap().clone()
    }
}

#[async_trait]
impl TurnCoordinator for CapturingTurnCoordinator {
    async fn prepare_turn(&self, _scope: TurnScope) -> Result<TurnRunId, TurnError> {
        Ok(TurnRunId::new())
    }

    async fn submit_turn(
        &self,
        request: SubmitTurnRequest,
    ) -> Result<SubmitTurnResponse, TurnError> {
        let run_id = TurnRunId::new();
        let message_ref = request.accepted_message_ref.clone();
        let reply_ref = request.reply_target_binding_ref.clone();
        self.submissions.lock().unwrap().push(request);
        Ok(SubmitTurnResponse::Accepted {
            turn_id: TurnId::new(),
            run_id,
            status: TurnStatus::Completed,
            resolved_run_profile_id: RunProfileId::default_profile(),
            resolved_run_profile_version: RunProfileVersion::new(1),
            event_cursor: EventCursor(0),
            accepted_message_ref: message_ref,
            reply_target_binding_ref: reply_ref,
        })
    }

    async fn resume_turn(
        &self,
        _request: ResumeTurnRequest,
    ) -> Result<ResumeTurnResponse, TurnError> {
        unimplemented!("not used in submit path tests")
    }

    async fn retry_turn(&self, _request: RetryTurnRequest) -> Result<RetryTurnResponse, TurnError> {
        unimplemented!("not used in submit path tests")
    }

    async fn cancel_run(&self, _request: CancelRunRequest) -> Result<CancelRunResponse, TurnError> {
        unimplemented!("not used in submit path tests")
    }

    async fn get_run_state(&self, _request: GetRunStateRequest) -> Result<TurnRunState, TurnError> {
        unimplemented!("not used in submit path tests")
    }
}

struct StubSessionThreadService;

#[async_trait]
impl ironclaw_threads::SessionThreadService for StubSessionThreadService {
    async fn ensure_thread(
        &self,
        _request: EnsureThreadRequest,
    ) -> Result<SessionThreadRecord, SessionThreadError> {
        unimplemented!("not used in submit path tests")
    }

    async fn accept_inbound_message(
        &self,
        _request: AcceptInboundMessageRequest,
    ) -> Result<AcceptedInboundMessage, SessionThreadError> {
        unimplemented!("not used in submit path tests")
    }

    async fn replay_accepted_inbound_message(
        &self,
        _request: ReplayAcceptedInboundMessageRequest,
    ) -> Result<Option<AcceptedInboundMessageReplay>, SessionThreadError> {
        Ok(None)
    }

    async fn mark_message_submitted(
        &self,
        _scope: &ThreadScope,
        _thread_id: &ThreadId,
        _message_id: ThreadMessageId,
        _turn_id: String,
        _turn_run_id: String,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        Ok(stub_message_record(_message_id))
    }

    async fn mark_message_rejected_busy(
        &self,
        _scope: &ThreadScope,
        _thread_id: &ThreadId,
        _message_id: ThreadMessageId,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        unimplemented!("not used in submit path tests")
    }

    async fn append_assistant_draft(
        &self,
        _request: AppendAssistantDraftRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        unimplemented!("not used in submit path tests")
    }

    async fn append_tool_result_reference(
        &self,
        _request: AppendToolResultReferenceRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        unimplemented!("not used in submit path tests")
    }

    async fn append_capability_display_preview(
        &self,
        _request: AppendCapabilityDisplayPreviewRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        unimplemented!("not used in submit path tests")
    }

    async fn update_tool_result_reference(
        &self,
        _request: UpdateToolResultReferenceRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        unimplemented!("not used in submit path tests")
    }

    async fn update_assistant_draft(
        &self,
        _request: UpdateAssistantDraftRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        unimplemented!("not used in submit path tests")
    }

    async fn finalize_assistant_message(
        &self,
        _scope: &ThreadScope,
        _thread_id: &ThreadId,
        _message_id: ThreadMessageId,
        _content: MessageContent,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        unimplemented!("not used in submit path tests")
    }

    async fn redact_message(
        &self,
        _request: RedactMessageRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        unimplemented!("not used in submit path tests")
    }

    async fn load_context_window(
        &self,
        _request: LoadContextWindowRequest,
    ) -> Result<ContextWindow, SessionThreadError> {
        unimplemented!("not used in submit path tests")
    }

    async fn load_context_messages(
        &self,
        _request: LoadContextMessagesRequest,
    ) -> Result<ContextMessages, SessionThreadError> {
        unimplemented!("not used in submit path tests")
    }

    async fn list_thread_history(
        &self,
        _request: ThreadHistoryRequest,
    ) -> Result<ThreadHistory, SessionThreadError> {
        unimplemented!("not used in submit path tests")
    }

    async fn create_summary_artifact(
        &self,
        _request: CreateSummaryArtifactRequest,
    ) -> Result<SummaryArtifact, SessionThreadError> {
        unimplemented!("not used in submit path tests")
    }

    async fn list_threads_for_scope(
        &self,
        _request: ListThreadsForScopeRequest,
    ) -> Result<ListThreadsForScopeResponse, SessionThreadError> {
        unimplemented!("not used in submit path tests")
    }
}

fn stub_message_record(message_id: ThreadMessageId) -> ThreadMessageRecord {
    ThreadMessageRecord {
        message_id,
        thread_id: thread_id(),
        sequence: 1,
        kind: ironclaw_threads::MessageKind::User,
        status: ironclaw_threads::MessageStatus::Submitted,
        created_at: None,
        updated_at: None,
        actor_id: None,
        source_binding_id: None,
        reply_target_binding_id: None,
        turn_id: None,
        turn_run_id: None,
        tool_result_ref: None,
        tool_result_provider_call: None,
        content: None,
        attachments: Vec::new(),
        redaction_ref: None,
    }
}

/// The legacy `from_replay` path hard-codes `TurnSurfaceType::Direct` and injects the
/// adapter id. This test drives the handoff through `submit_or_replay` and asserts
/// that the submitted `SubmitTurnRequest.product_context` carries `Direct` surface and
/// the adapter from the replay call.
#[tokio::test]
async fn replay_submit_carries_direct_surface_type_and_adapter_id() {
    let adapter_id = ProductAdapterId::new("telegram").unwrap();
    let message_id = ThreadMessageId::new();
    let handoff = ProductInboundTurnHandoff::from_replay(
        replay(
            message_id,
            MessageStatus::DeferredBusy,
            Some("src:replay"),
            Some("reply:replay"),
            None,
        ),
        "turn-key-replay".to_string(),
        received_at(),
        adapter_id.clone(),
    )
    .expect("replay handoff");

    let coordinator = CapturingTurnCoordinator::default();
    let thread_service = StubSessionThreadService;

    handoff
        .submit_or_replay(&thread_service, &coordinator)
        .await
        .expect("submit_or_replay succeeds");

    let submissions = coordinator.submissions();
    assert_eq!(submissions.len(), 1, "one turn must be submitted");
    let ctx = submissions[0]
        .product_context
        .as_ref()
        .expect("product_context must be set");
    assert_eq!(
        ctx.surface_type,
        Some(TurnSurfaceType::Direct),
        "replay path must carry Direct surface type"
    );
    assert_eq!(
        ctx.adapter.as_ref().map(|a| a.as_str()),
        Some(adapter_id.as_str()),
        "replay path must carry the adapter id"
    );
    assert_eq!(
        ctx.origin,
        TurnOriginKind::Inbound,
        "replay path must record Inbound origin (Untrusted classification)"
    );
}

struct PendingBeforeInboundPolicy;

#[async_trait]
impl BeforeInboundPolicy for PendingBeforeInboundPolicy {
    async fn check_user_message(
        &self,
        _request: BeforeInboundPolicyRequest,
    ) -> Result<BeforeInboundPolicyOutcome, ProductSurfaceFailure> {
        pending().await
    }
}

#[tokio::test]
async fn check_before_inbound_policy_times_out_as_retryable_failure() {
    let err = check_before_inbound_policy(&PendingBeforeInboundPolicy, policy_request())
        .await
        .expect_err("pending policy should time out");

    assert!(matches!(
        err,
        ProductSurfaceFailure::BeforeInboundPolicyFailed {
            permanent: false,
            ..
        }
    ));
}

#[tokio::test]
async fn noop_before_inbound_policy_allows_user_messages() {
    let outcome = NoopBeforeInboundPolicy
        .check_user_message(policy_request())
        .await
        .expect("noop policy should not fail");

    assert_eq!(outcome, BeforeInboundPolicyOutcome::Allow);
}

#[test]
fn submitted_replay_becomes_already_submitted_handoff() {
    let submitted_run_id = TurnRunId::new();
    let message_id = ThreadMessageId::new();
    let handoff = ProductInboundTurnHandoff::from_replay(
        replay(
            message_id,
            MessageStatus::Submitted,
            Some("src:alpha"),
            Some("reply:alpha"),
            Some(submitted_run_id.to_string()),
        ),
        "turn-key".to_string(),
        received_at(),
        ProductAdapterId::new("test_adapter").unwrap(),
    )
    .expect("submitted replay handoff");

    let ProductInboundTurnHandoff::AlreadySubmitted {
        accepted_message_ref: actual_message_ref,
        submitted_run_id: actual_run_id,
        binding,
    } = handoff
    else {
        panic!("expected submitted replay to short-circuit turn submission")
    };

    assert_eq!(actual_run_id, submitted_run_id);
    assert_eq!(
        actual_message_ref,
        accepted_message_ref(message_id).unwrap()
    );
    assert_eq!(binding.thread_id, thread_id());
}

#[test]
fn rejected_busy_replay_becomes_already_rejected_handoff() {
    let message_id = ThreadMessageId::new();
    let handoff = ProductInboundTurnHandoff::from_replay(
        replay(
            message_id,
            MessageStatus::RejectedBusy,
            Some("src:alpha"),
            Some("reply:alpha"),
            None,
        ),
        "turn-key".to_string(),
        received_at(),
        ProductAdapterId::new("test_adapter").unwrap(),
    )
    .expect("rejected busy replay handoff");

    let ProductInboundTurnHandoff::AlreadyRejected {
        accepted_message_ref: actual_message_ref,
        active_run_id,
        ..
    } = handoff
    else {
        panic!("expected rejected busy replay to be terminal, not resubmitted")
    };

    assert_eq!(
        actual_message_ref,
        accepted_message_ref(message_id).unwrap()
    );
    assert!(active_run_id.is_none());
}

#[test]
fn legacy_replay_without_actor_id_uses_owner_as_actor() {
    let message_id = ThreadMessageId::new();
    let mut replay = replay(
        message_id,
        MessageStatus::DeferredBusy,
        Some("src:alpha"),
        Some("reply:alpha"),
        None,
    );
    replay.actor_id = None;

    let handoff = ProductInboundTurnHandoff::from_replay(
        replay,
        "turn-key".to_string(),
        received_at(),
        ProductAdapterId::new("test_adapter").unwrap(),
    )
    .expect("legacy replay handoff");

    let ProductInboundTurnHandoff::NeedsSubmission(submission) = handoff else {
        panic!("expected legacy replay to require a new turn submission")
    };

    assert_eq!(submission.binding.actor_user_id, user_id());
    assert_eq!(submission.binding.subject_user_id, Some(user_id()));
    assert_eq!(submission.message_id, message_id);
}

#[test]
fn prepared_replay_uses_fresh_binding_scope_over_persisted_scope() {
    let message_id = ThreadMessageId::new();
    let mut replay = replay(
        message_id,
        MessageStatus::DeferredBusy,
        Some("src:alpha"),
        Some("reply:alpha"),
        None,
    );
    replay.scope.owner_user_id = None;
    let subject_user_id = UserId::new("user:team-subject").unwrap();
    let prepared = PreparedUserMessage {
        binding: ResolvedBinding {
            tenant_id: tenant_id(),
            actor_user_id: user_id(),
            subject_user_id: Some(subject_user_id.clone()),
            thread_id: thread_id(),
            agent_id: Some(AgentId::new("agent:alpha").unwrap()),
            project_id: None,
        },
        thread_scope: ThreadScope {
            tenant_id: tenant_id(),
            agent_id: AgentId::new("agent:alpha").unwrap(),
            project_id: None,
            owner_user_id: Some(subject_user_id.clone()),
            mission_id: None,
        },
        source_binding_id: "src:alpha".to_string(),
        submit_idempotency_key: "turn-key".to_string(),
        adapter_id: ProductAdapterId::new("test_adapter").unwrap(),
        source_channel: ProductSourceChannel::new("test_adapter").unwrap(),
        surface_type: TurnSurfaceType::Direct,
    };

    let handoff = ProductInboundTurnHandoff::from_replay_with_prepared(
        replay,
        "turn-key".to_string(),
        received_at(),
        &prepared,
    )
    .expect("prepared replay handoff");

    let ProductInboundTurnHandoff::NeedsSubmission(submission) = handoff else {
        panic!("expected prepared replay to require a new turn submission")
    };

    assert_eq!(
        submission.binding.subject_user_id,
        Some(subject_user_id.clone())
    );
    assert_eq!(submission.thread_scope.owner_user_id, Some(subject_user_id));
    assert_eq!(submission.message_id, message_id);
}

/// A BotMention shared route must produce `TurnSurfaceType::Channel` in the
/// submitted `SubmitTurnRequest.product_context`. This exercises the
/// `ProductConversationRouteKind::Shared => TurnSurfaceType::Channel` branch
/// in `prepare_user_message` through the replay-with-prepared handoff path,
/// which is the same submission seam the full inbound-turn pipeline uses.
#[tokio::test]
async fn shared_user_message_records_channel_surface_type() {
    let message_id = ThreadMessageId::new();
    let prepared = PreparedUserMessage {
        binding: ResolvedBinding {
            tenant_id: tenant_id(),
            actor_user_id: user_id(),
            subject_user_id: Some(user_id()),
            thread_id: thread_id(),
            agent_id: Some(AgentId::new("agent:alpha").unwrap()),
            project_id: None,
        },
        thread_scope: ThreadScope {
            tenant_id: tenant_id(),
            agent_id: AgentId::new("agent:alpha").unwrap(),
            project_id: None,
            owner_user_id: Some(user_id()),
            mission_id: None,
        },
        source_binding_id: "src:shared".to_string(),
        submit_idempotency_key: "turn-key-shared".to_string(),
        adapter_id: ProductAdapterId::new("slack").unwrap(),
        source_channel: ProductSourceChannel::new("slack").unwrap(),
        // BotMention shared route maps to Channel surface type.
        surface_type: TurnSurfaceType::Channel,
    };

    let handoff = ProductInboundTurnHandoff::from_replay_with_prepared(
        replay(
            message_id,
            MessageStatus::DeferredBusy,
            Some("src:shared"),
            Some("reply:shared"),
            None,
        ),
        "turn-key-shared".to_string(),
        received_at(),
        &prepared,
    )
    .expect("shared route replay handoff");

    let coordinator = CapturingTurnCoordinator::default();
    let thread_service = StubSessionThreadService;

    handoff
        .submit_or_replay(&thread_service, &coordinator)
        .await
        .expect("submit_or_replay succeeds");

    let submissions = coordinator.submissions();
    assert_eq!(submissions.len(), 1, "one turn must be submitted");
    let ctx = submissions[0]
        .product_context
        .as_ref()
        .expect("product_context must be set");
    assert_eq!(
        ctx.surface_type,
        Some(TurnSurfaceType::Channel),
        "BotMention shared route must carry Channel surface type"
    );
    assert_eq!(
        ctx.source_channel
            .as_ref()
            .map(ironclaw_turns::RunOriginAdapter::as_str),
        Some("slack"),
        "shared route must preserve source channel"
    );
}

fn policy_request() -> BeforeInboundPolicyRequest {
    BeforeInboundPolicyRequest {
        adapter_id: ProductAdapterId::new("test_adapter").expect("adapter"),
        installation_id: AdapterInstallationId::new("install_alpha").expect("installation"),
        external_actor_ref: ExternalActorRef::new("test", "user1", Option::<String>::None)
            .expect("actor"),
        external_conversation_ref: ExternalConversationRef::new(None, "conv1", None, None)
            .expect("conversation"),
        source_binding_key: SourceBindingKey::new("space:0:;conversation:5:conv1;topic:0:;")
            .expect("source binding key"),
        rate_limit_key: SourceBindingKey::new("space:0:;conversation:5:conv1;topic:0:;")
            .expect("rate limit key"),
        user_message: UserMessagePayload::new("hello", vec![], ProductTriggerReason::DirectChat)
            .expect("message"),
    }
}

fn replay(
    message_id: ThreadMessageId,
    status: MessageStatus,
    source_binding_id: Option<&str>,
    reply_target_binding_id: Option<&str>,
    turn_run_id: Option<String>,
) -> AcceptedInboundMessageReplay {
    AcceptedInboundMessageReplay {
        scope: ThreadScope {
            tenant_id: tenant_id(),
            agent_id: AgentId::new("agent:alpha").unwrap(),
            project_id: None,
            owner_user_id: Some(user_id()),
            mission_id: None,
        },
        thread_id: thread_id(),
        message_id,
        sequence: 1,
        status,
        actor_id: Some(user_id().as_str().to_string()),
        source_binding_id: source_binding_id.map(str::to_string),
        reply_target_binding_id: reply_target_binding_id.map(str::to_string),
        turn_run_id,
    }
}

fn received_at() -> DateTime<Utc> {
    Utc.timestamp_opt(0, 0).single().unwrap()
}

fn tenant_id() -> TenantId {
    TenantId::new("tenant:alpha").unwrap()
}

fn user_id() -> UserId {
    UserId::new("user:alpha").unwrap()
}

fn thread_id() -> ThreadId {
    ThreadId::new("thread:alpha").unwrap()
}

mod attachments;

#[test]
fn rejected_busy_replay_with_invalid_turn_run_id_fails_loudly() {
    let message_id = ThreadMessageId::new();
    let result = ProductInboundTurnHandoff::from_replay(
        replay(
            message_id,
            MessageStatus::RejectedBusy,
            Some("src:alpha"),
            Some("reply:alpha"),
            Some("not-a-uuid".to_string()),
        ),
        "turn-key".to_string(),
        received_at(),
        ProductAdapterId::new("test_adapter").unwrap(),
    );
    let err = match result {
        Err(e) => e,
        Ok(_) => panic!("expected Err for malformed turn_run_id, got Ok"),
    };

    match err {
        ProductSurfaceFailure::TurnSubmissionRejected { reason } => {
            assert!(
                reason.contains("invalid rejected busy turn_run_id"),
                "expected reason to contain 'invalid rejected busy turn_run_id', got: {reason}"
            );
        }
        other => panic!("expected TurnSubmissionRejected, got: {other:?}"),
    }
}
