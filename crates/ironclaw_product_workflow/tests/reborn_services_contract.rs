//! Contract tests for WebUI-facing RebornServices facade.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_product_adapters::{
    ProductAdapterError, ProductOutboundEnvelope, ProductWorkflowRejectionKind, ProjectionStream,
    ProjectionSubscriptionRequest, ProtocolAuthFailure, RedactedString,
};
use ironclaw_product_workflow::{
    RebornResolveGateResponse, RebornServices, RebornServicesApi, RebornServicesError,
    RebornServicesErrorCode, RebornServicesErrorKind, RebornStreamEventsRequest,
    RebornSubmitTurnResponse, RebornTimelineRequest, WebUiAuthenticatedCaller,
    WebUiCancelRunRequest, WebUiCreateThreadRequest, WebUiInboundValidationCode,
    WebUiResolveGateRequest, WebUiSendMessageRequest,
};
use ironclaw_threads::{
    AcceptInboundMessageRequest, AcceptedInboundMessage, AcceptedInboundMessageReplay,
    AppendAssistantDraftRequest, ContextWindow, CreateSummaryArtifactRequest, EnsureThreadRequest,
    InMemorySessionThreadService, LoadContextWindowRequest, MessageContent, MessageStatus,
    RedactMessageRequest, ReplayAcceptedInboundMessageRequest, SessionThreadError,
    SessionThreadRecord, SessionThreadService, SummaryArtifact, ThreadHistory,
    ThreadHistoryRequest, ThreadMessageId, ThreadMessageRecord, ThreadScope,
    UpdateAssistantDraftRequest,
};
use ironclaw_turns::{
    AcceptedMessageRef, AdmissionRejection, AdmissionRejectionReason, CancelRunRequest,
    CancelRunResponse, EventCursor, GateRef, GetRunStateRequest, ReplyTargetBindingRef,
    ResumeTurnRequest, ResumeTurnResponse, RunProfileId, RunProfileVersion, SourceBindingRef,
    SubmitTurnRequest, SubmitTurnResponse, TurnCoordinator, TurnError, TurnId, TurnRunId,
    TurnRunState, TurnStatus,
};
use serde_json::json;

fn caller() -> WebUiAuthenticatedCaller {
    WebUiAuthenticatedCaller::new(
        TenantId::new("tenant-alpha").expect("valid tenant"),
        UserId::new("user-alpha").expect("valid user"),
        Some(AgentId::new("agent-alpha").expect("valid agent")),
        Some(ProjectId::new("project-alpha").expect("valid project")),
    )
}

fn run_id_string() -> String {
    "3d54a1f0-0a7f-4b9c-a350-4258f2fa3e18".to_string()
}

/// Establish thread ownership for `caller` under `thread_id` so subsequent
/// `cancel_run` / `resolve_gate` calls pass the `assert_thread_owned_by`
/// check. Goes through `submit_turn` because that is the only public path
/// that calls `ensure_thread` with the actor pinned as `owner_user_id`.
async fn setup_owned_thread(
    services: &RebornServices,
    owner: WebUiAuthenticatedCaller,
    thread_id: &str,
) {
    services
        .submit_turn(
            owner,
            serde_json::from_value::<WebUiSendMessageRequest>(json!({
                "client_action_id": format!("setup-{thread_id}"),
                "thread_id": thread_id,
                "content": "seed",
            }))
            .expect("setup request"),
        )
        .await
        .expect("setup submit succeeds");
}

#[derive(Default)]
struct FakeTurnCoordinator {
    submissions: Mutex<Vec<SubmitTurnRequest>>,
    cancellations: Mutex<Vec<CancelRunRequest>>,
    resumptions: Mutex<Vec<ResumeTurnRequest>>,
    submit_error: Mutex<Option<TurnError>>,
    parked_gate_ref: Mutex<Option<GateRef>>,
}

impl FakeTurnCoordinator {
    fn with_submit_error(error: TurnError) -> Self {
        Self {
            submit_error: Mutex::new(Some(error)),
            ..Self::default()
        }
    }

    /// Programs `get_run_state` to return this gate as the run's currently
    /// parked gate. Needed by tests that exercise `resolve_gate` denied/
    /// cancelled paths now that `RebornServices` verifies the run is parked
    /// on the supplied gate before issuing cancellation.
    fn set_parked_gate(&self, gate_ref: GateRef) {
        *self.parked_gate_ref.lock().expect("lock") = Some(gate_ref);
    }

    fn submission_count(&self) -> usize {
        self.submissions.lock().expect("lock").len()
    }

    fn cancellation_count(&self) -> usize {
        self.cancellations.lock().expect("lock").len()
    }

    fn resumption_count(&self) -> usize {
        self.resumptions.lock().expect("lock").len()
    }

    fn last_resumption_source_binding_ref(&self) -> Option<String> {
        self.resumptions
            .lock()
            .expect("lock")
            .last()
            .map(|request| request.source_binding_ref.as_str().to_string())
    }
}

#[async_trait]
impl TurnCoordinator for FakeTurnCoordinator {
    async fn submit_turn(
        &self,
        request: SubmitTurnRequest,
    ) -> Result<SubmitTurnResponse, TurnError> {
        if let Some(error) = self.submit_error.lock().expect("lock").take() {
            return Err(error);
        }
        self.submissions.lock().expect("lock").push(request.clone());
        Ok(SubmitTurnResponse::Accepted {
            turn_id: TurnId::new(),
            run_id: TurnRunId::new(),
            status: TurnStatus::Queued,
            resolved_run_profile_id: RunProfileId::default_profile(),
            resolved_run_profile_version: RunProfileVersion::new(1),
            event_cursor: EventCursor(7),
            accepted_message_ref: request.accepted_message_ref,
            reply_target_binding_ref: request.reply_target_binding_ref,
        })
    }

    async fn resume_turn(
        &self,
        request: ResumeTurnRequest,
    ) -> Result<ResumeTurnResponse, TurnError> {
        self.resumptions.lock().expect("lock").push(request);
        Ok(ResumeTurnResponse {
            run_id: TurnRunId::new(),
            status: TurnStatus::Queued,
            event_cursor: EventCursor(11),
        })
    }

    async fn cancel_run(&self, request: CancelRunRequest) -> Result<CancelRunResponse, TurnError> {
        let run_id = request.run_id;
        self.cancellations.lock().expect("lock").push(request);
        Ok(CancelRunResponse {
            run_id,
            status: TurnStatus::Cancelled,
            event_cursor: EventCursor(13),
            already_terminal: false,
        })
    }

    async fn get_run_state(&self, request: GetRunStateRequest) -> Result<TurnRunState, TurnError> {
        let gate_ref = self.parked_gate_ref.lock().expect("lock").clone();
        Ok(TurnRunState {
            scope: request.scope,
            turn_id: TurnId::new(),
            run_id: request.run_id,
            status: TurnStatus::Queued,
            accepted_message_ref: AcceptedMessageRef::new("msg:replayed").expect("valid ref"),
            source_binding_ref: SourceBindingRef::new("webui-src:replayed").expect("valid ref"),
            reply_target_binding_ref: ReplyTargetBindingRef::new("webui-reply:replayed")
                .expect("valid ref"),
            resolved_run_profile_id: RunProfileId::default_profile(),
            resolved_run_profile_version: RunProfileVersion::new(1),
            resolved_model_route: None,
            received_at: Utc::now(),
            checkpoint_id: None,
            gate_ref,
            failure: None,
            event_cursor: EventCursor(17),
        })
    }
}

struct AuthFailureProjectionStream;

#[async_trait]
impl ProjectionStream for AuthFailureProjectionStream {
    async fn drain(
        &self,
        _request: ProjectionSubscriptionRequest,
    ) -> Result<Vec<ProductOutboundEnvelope>, ProductAdapterError> {
        Err(ProductAdapterError::Authentication(
            ProtocolAuthFailure::SignatureMismatch,
        ))
    }
}

struct StaticErrorProjectionStream {
    error: ProductAdapterError,
}

#[async_trait]
impl ProjectionStream for StaticErrorProjectionStream {
    async fn drain(
        &self,
        _request: ProjectionSubscriptionRequest,
    ) -> Result<Vec<ProductOutboundEnvelope>, ProductAdapterError> {
        Err(self.error.clone())
    }
}

/// Stub thread service whose `list_thread_history` always returns
/// the configured failure. Used to lock in facade error mapping without
/// reaching the rest of the thread service.
struct FailingThreadHistoryStub {
    failure: ThreadHistoryFailure,
}

enum ThreadHistoryFailure {
    ScopeMismatch,
    Backend,
}

impl FailingThreadHistoryStub {
    fn scope_mismatch() -> Self {
        Self {
            failure: ThreadHistoryFailure::ScopeMismatch,
        }
    }

    fn backend() -> Self {
        Self {
            failure: ThreadHistoryFailure::Backend,
        }
    }
}

#[async_trait]
impl SessionThreadService for FailingThreadHistoryStub {
    async fn list_thread_history(
        &self,
        request: ThreadHistoryRequest,
    ) -> Result<ThreadHistory, SessionThreadError> {
        match self.failure {
            ThreadHistoryFailure::ScopeMismatch => Err(SessionThreadError::ThreadScopeMismatch {
                thread_id: request.thread_id,
            }),
            ThreadHistoryFailure::Backend => Err(SessionThreadError::Backend(
                "backend detail /host/path secret-token".to_string(),
            )),
        }
    }

    async fn ensure_thread(
        &self,
        _request: EnsureThreadRequest,
    ) -> Result<SessionThreadRecord, SessionThreadError> {
        panic!("FailingThreadHistoryStub::ensure_thread should not be reached")
    }

    async fn accept_inbound_message(
        &self,
        _request: AcceptInboundMessageRequest,
    ) -> Result<AcceptedInboundMessage, SessionThreadError> {
        panic!("FailingThreadHistoryStub::accept_inbound_message should not be reached")
    }

    async fn replay_accepted_inbound_message(
        &self,
        _request: ReplayAcceptedInboundMessageRequest,
    ) -> Result<Option<AcceptedInboundMessageReplay>, SessionThreadError> {
        panic!("FailingThreadHistoryStub::replay_accepted_inbound_message should not be reached")
    }

    async fn mark_message_submitted(
        &self,
        _scope: &ThreadScope,
        _thread_id: &ThreadId,
        _message_id: ThreadMessageId,
        _turn_id: String,
        _turn_run_id: String,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        panic!("FailingThreadHistoryStub::mark_message_submitted should not be reached")
    }

    async fn mark_message_deferred_busy(
        &self,
        _scope: &ThreadScope,
        _thread_id: &ThreadId,
        _message_id: ThreadMessageId,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        panic!("FailingThreadHistoryStub::mark_message_deferred_busy should not be reached")
    }

    async fn append_assistant_draft(
        &self,
        _request: AppendAssistantDraftRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        panic!("FailingThreadHistoryStub::append_assistant_draft should not be reached")
    }

    async fn update_assistant_draft(
        &self,
        _request: UpdateAssistantDraftRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        panic!("FailingThreadHistoryStub::update_assistant_draft should not be reached")
    }

    async fn finalize_assistant_message(
        &self,
        _scope: &ThreadScope,
        _thread_id: &ThreadId,
        _message_id: ThreadMessageId,
        _content: MessageContent,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        panic!("FailingThreadHistoryStub::finalize_assistant_message should not be reached")
    }

    async fn redact_message(
        &self,
        _request: RedactMessageRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        panic!("FailingThreadHistoryStub::redact_message should not be reached")
    }

    async fn load_context_window(
        &self,
        _request: LoadContextWindowRequest,
    ) -> Result<ContextWindow, SessionThreadError> {
        panic!("FailingThreadHistoryStub::load_context_window should not be reached")
    }

    async fn create_summary_artifact(
        &self,
        _request: CreateSummaryArtifactRequest,
    ) -> Result<SummaryArtifact, SessionThreadError> {
        panic!("FailingThreadHistoryStub::create_summary_artifact should not be reached")
    }
}

#[tokio::test]
async fn duplicate_create_thread_replays_generated_thread_for_same_client_action() {
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    );
    let request = || {
        serde_json::from_value::<WebUiCreateThreadRequest>(json!({
            "client_action_id": "create-duplicate"
        }))
        .expect("request")
    };

    let first = services
        .create_thread(caller(), request())
        .await
        .expect("first create succeeds");
    let replayed = services
        .create_thread(caller(), request())
        .await
        .expect("duplicate create replays");

    assert_eq!(first.thread.thread_id, replayed.thread.thread_id);
    assert_eq!(first.thread.metadata_json, replayed.thread.metadata_json);
}

#[tokio::test]
async fn create_thread_metadata_is_serialized_json() {
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    );
    let client_action_id = "create-quote-\"-slash-\\-line-\u{2028}".to_string();

    let response = services
        .create_thread(
            caller(),
            serde_json::from_value::<WebUiCreateThreadRequest>(json!({
                "client_action_id": client_action_id
            }))
            .expect("request"),
        )
        .await
        .expect("create succeeds");

    let metadata = response.thread.metadata_json.expect("metadata");
    let metadata: serde_json::Value = serde_json::from_str(&metadata).expect("metadata json");
    assert_eq!(
        metadata["client_action_id"].as_str(),
        Some(client_action_id.as_str())
    );
}

#[test]
fn facade_error_taxonomy_serializes_stable_wire_names() {
    let error = RebornServicesError {
        code: RebornServicesErrorCode::Conflict,
        kind: RebornServicesErrorKind::Busy,
        status_code: 409,
        retryable: false,
        field: None,
        validation_code: None,
    };

    let json = serde_json::to_value(&error).expect("error json");

    assert_eq!(json["code"], "conflict");
    assert_eq!(json["kind"], "busy");
    assert_eq!(json["status_code"], 409);
    assert_eq!(json["retryable"], false);
}

#[tokio::test]
async fn submit_turn_uses_facade_and_thread_history_without_route_store_access() {
    let threads: Arc<dyn SessionThreadService> = Arc::new(InMemorySessionThreadService::default());
    let coordinator = Arc::new(FakeTurnCoordinator::default());
    let services = RebornServices::new(threads, coordinator.clone());

    let response = services
        .submit_turn(
            caller(),
            serde_json::from_value::<WebUiSendMessageRequest>(json!({
                "client_action_id": "send-1",
                "thread_id": "thread-alpha",
                "content": "hello from webui"
            }))
            .expect("request"),
        )
        .await
        .expect("submit succeeds");

    let RebornSubmitTurnResponse::Submitted {
        thread_id,
        status,
        event_cursor,
        ..
    } = response
    else {
        panic!("expected submitted response");
    };
    assert_eq!(thread_id.as_str(), "thread-alpha");
    assert_eq!(status, TurnStatus::Queued);
    assert_eq!(event_cursor, EventCursor(7));
    assert_eq!(coordinator.submission_count(), 1);

    let timeline = services
        .get_timeline(
            caller(),
            RebornTimelineRequest {
                thread_id: "thread-alpha".to_string(),
            },
        )
        .await
        .expect("timeline");
    assert_eq!(timeline.messages.len(), 1);
    assert_eq!(timeline.messages[0].status, MessageStatus::Submitted);
    assert_eq!(
        timeline.messages[0].content.as_deref(),
        Some("hello from webui")
    );
}

#[tokio::test]
async fn duplicate_submit_replays_prior_handoff_without_second_submission() {
    let threads: Arc<dyn SessionThreadService> = Arc::new(InMemorySessionThreadService::default());
    let coordinator = Arc::new(FakeTurnCoordinator::default());
    let services = RebornServices::new(threads, coordinator.clone());

    let request = || {
        serde_json::from_value::<WebUiSendMessageRequest>(json!({
            "client_action_id": "send-duplicate",
            "thread_id": "thread-alpha",
            "content": "hello once"
        }))
        .expect("request")
    };

    services
        .submit_turn(caller(), request())
        .await
        .expect("first submit succeeds");
    let replayed = services
        .submit_turn(caller(), request())
        .await
        .expect("duplicate submit replays");

    assert!(matches!(
        replayed,
        RebornSubmitTurnResponse::AlreadySubmitted { .. }
    ));
    assert_eq!(coordinator.submission_count(), 1);
}

#[tokio::test]
async fn validation_errors_are_stable_and_sanitized() {
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    );

    let err = services
        .submit_turn(
            caller(),
            serde_json::from_value::<WebUiSendMessageRequest>(json!({
                "client_action_id": "send-1",
                "thread_id": "thread-alpha"
            }))
            .expect("request"),
        )
        .await
        .expect_err("missing content rejected");

    assert_eq!(err.code, RebornServicesErrorCode::InvalidRequest);
    assert_eq!(err.kind, RebornServicesErrorKind::Validation);
    assert_eq!(err.status_code, 400);
    assert_eq!(err.field.as_deref(), Some("content"));
    assert_eq!(
        err.validation_code,
        Some(WebUiInboundValidationCode::MissingField)
    );
    let rendered = serde_json::to_string(&err).expect("json");
    assert!(!rendered.contains("backend"));
    assert!(!rendered.contains("TurnCoordinator"));
}

#[tokio::test]
async fn turn_admission_rejected_maps_to_busy_taxonomy() {
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::with_submit_error(
            TurnError::AdmissionRejected(AdmissionRejection::new(
                AdmissionRejectionReason::TenantLimit,
            )),
        )),
    );

    let err = services
        .submit_turn(
            caller(),
            serde_json::from_value::<WebUiSendMessageRequest>(json!({
                "client_action_id": "send-rate-limited",
                "thread_id": "thread-alpha",
                "content": "hello from webui"
            }))
            .expect("request"),
        )
        .await
        .expect_err("admission rejection is a stable busy/rate-limited error");

    assert_eq!(err.code, RebornServicesErrorCode::RateLimited);
    assert_eq!(err.kind, RebornServicesErrorKind::Busy);
    assert_eq!(err.status_code, 429);
    assert!(err.retryable);
}

#[tokio::test]
async fn turn_unauthorized_maps_to_forbidden() {
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::with_submit_error(
            TurnError::Unauthorized,
        )),
    );

    let err = services
        .submit_turn(
            caller(),
            serde_json::from_value::<WebUiSendMessageRequest>(json!({
                "client_action_id": "send-forbidden",
                "thread_id": "thread-alpha",
                "content": "hello from webui"
            }))
            .expect("request"),
        )
        .await
        .expect_err("turn unauthorized is forbidden");

    assert_eq!(err.code, RebornServicesErrorCode::Forbidden);
    assert_eq!(err.kind, RebornServicesErrorKind::ParticipantDenied);
    assert_eq!(err.status_code, 403);
}

#[tokio::test]
async fn adapter_authentication_maps_to_unauthenticated() {
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    )
    .with_event_stream(Arc::new(AuthFailureProjectionStream));

    let err = services
        .stream_events(
            caller(),
            RebornStreamEventsRequest {
                thread_id: "thread-alpha".to_string(),
                after_cursor: None,
            },
        )
        .await
        .expect_err("adapter auth failure is unauthenticated");

    assert_eq!(err.code, RebornServicesErrorCode::Unauthenticated);
    assert_eq!(err.kind, RebornServicesErrorKind::ParticipantDenied);
    assert_eq!(err.status_code, 401);
}

#[tokio::test]
async fn projection_transient_maps_to_replay_unavailable_taxonomy() {
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    )
    .with_event_stream(Arc::new(StaticErrorProjectionStream {
        error: ProductAdapterError::WorkflowTransient {
            reason: RedactedString::new("provider stack trace with /host/path and secret-token"),
        },
    }));

    let err = services
        .stream_events(
            caller(),
            RebornStreamEventsRequest {
                thread_id: "thread-alpha".to_string(),
                after_cursor: None,
            },
        )
        .await
        .expect_err("projection transient is replay unavailable");

    assert_eq!(err.code, RebornServicesErrorCode::Unavailable);
    assert_eq!(err.kind, RebornServicesErrorKind::ReplayUnavailable);
    assert_eq!(err.status_code, 503);
    assert!(err.retryable);
    let rendered = format!("{err:?} {}", serde_json::to_string(&err).expect("json"));
    assert!(!rendered.contains("secret-token"));
    assert!(!rendered.contains("/host/path"));
    assert!(!rendered.contains("provider stack trace"));
}

#[tokio::test]
async fn projection_egress_denied_maps_to_blocked_resource_taxonomy() {
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FakeTurnCoordinator::default()),
    )
    .with_event_stream(Arc::new(StaticErrorProjectionStream {
        error: ProductAdapterError::EgressDenied {
            reason: RedactedString::new("denied api key secret-token"),
        },
    }));

    let err = services
        .stream_events(
            caller(),
            RebornStreamEventsRequest {
                thread_id: "thread-alpha".to_string(),
                after_cursor: None,
            },
        )
        .await
        .expect_err("blocked resource is stable taxonomy");

    assert_eq!(err.code, RebornServicesErrorCode::Forbidden);
    assert_eq!(err.kind, RebornServicesErrorKind::BlockedResource);
    assert_eq!(err.status_code, 403);
    let rendered = format!("{err:?} {}", serde_json::to_string(&err).expect("json"));
    assert!(!rendered.contains("secret-token"));
}

#[tokio::test]
async fn workflow_rejection_kinds_map_to_facade_taxonomy() {
    let cases = [
        (
            ProductWorkflowRejectionKind::ThreadBusy,
            409,
            RebornServicesErrorCode::Conflict,
            RebornServicesErrorKind::Busy,
        ),
        (
            ProductWorkflowRejectionKind::AdmissionRejected,
            429,
            RebornServicesErrorCode::RateLimited,
            RebornServicesErrorKind::Busy,
        ),
        (
            ProductWorkflowRejectionKind::ScopeNotFound,
            404,
            RebornServicesErrorCode::NotFound,
            RebornServicesErrorKind::NotFound,
        ),
        (
            ProductWorkflowRejectionKind::Unauthorized,
            403,
            RebornServicesErrorCode::Forbidden,
            RebornServicesErrorKind::ParticipantDenied,
        ),
        (
            ProductWorkflowRejectionKind::InvalidRequest,
            400,
            RebornServicesErrorCode::InvalidRequest,
            RebornServicesErrorKind::Validation,
        ),
        (
            ProductWorkflowRejectionKind::Unavailable,
            503,
            RebornServicesErrorCode::Unavailable,
            RebornServicesErrorKind::ReplayUnavailable,
        ),
        (
            ProductWorkflowRejectionKind::Conflict,
            409,
            RebornServicesErrorCode::Conflict,
            RebornServicesErrorKind::Conflict,
        ),
    ];

    for (workflow_kind, status_code, expected_code, expected_kind) in cases {
        let services = RebornServices::new(
            Arc::new(InMemorySessionThreadService::default()),
            Arc::new(FakeTurnCoordinator::default()),
        )
        .with_event_stream(Arc::new(StaticErrorProjectionStream {
            error: ProductAdapterError::WorkflowRejected {
                kind: workflow_kind,
                status_code,
                retryable: false,
                reason: RedactedString::new("internal workflow detail secret-token"),
            },
        }));

        let err = services
            .stream_events(
                caller(),
                RebornStreamEventsRequest {
                    thread_id: "thread-alpha".to_string(),
                    after_cursor: None,
                },
            )
            .await
            .expect_err("workflow rejection maps to stable facade taxonomy");

        assert_eq!(err.code, expected_code);
        assert_eq!(err.kind, expected_kind);
        assert_eq!(err.status_code, status_code);
        assert!(
            !serde_json::to_string(&err)
                .expect("json")
                .contains("secret-token")
        );
    }
}

#[tokio::test]
async fn timeline_backend_failure_maps_to_timeline_unavailable_taxonomy() {
    let services = RebornServices::new(
        Arc::new(FailingThreadHistoryStub::backend()),
        Arc::new(FakeTurnCoordinator::default()),
    );

    let err = services
        .get_timeline(
            caller(),
            RebornTimelineRequest {
                thread_id: "thread-alpha".to_string(),
            },
        )
        .await
        .expect_err("timeline backend failure is stable unavailable taxonomy");

    assert_eq!(err.code, RebornServicesErrorCode::Unavailable);
    assert_eq!(err.kind, RebornServicesErrorKind::TimelineUnavailable);
    assert_eq!(err.status_code, 503);
    assert!(err.retryable);
    let rendered = format!("{err:?} {}", serde_json::to_string(&err).expect("json"));
    assert!(!rendered.contains("secret-token"));
    assert!(!rendered.contains("/host/path"));
    assert!(!rendered.contains("backend detail"));
}

#[tokio::test]
async fn cancel_run_uses_turn_facade_and_stable_response() {
    let coordinator = Arc::new(FakeTurnCoordinator::default());
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        coordinator.clone(),
    );
    setup_owned_thread(&services, caller(), "thread-alpha").await;

    let response = services
        .cancel_run(
            caller(),
            serde_json::from_value::<WebUiCancelRunRequest>(json!({
                "client_action_id": "cancel-1",
                "thread_id": "thread-alpha",
                "run_id": run_id_string(),
                "reason": "user_requested"
            }))
            .expect("request"),
        )
        .await
        .expect("cancel succeeds");

    assert_eq!(response.status, TurnStatus::Cancelled);
    assert_eq!(response.event_cursor, EventCursor(13));
    assert!(!response.already_terminal);
    assert_eq!(coordinator.cancellation_count(), 1);
}

#[tokio::test]
async fn approved_gate_resolution_resumes_turn() {
    let coordinator = Arc::new(FakeTurnCoordinator::default());
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        coordinator.clone(),
    );
    setup_owned_thread(&services, caller(), "thread-alpha").await;

    let response = services
        .resolve_gate(
            caller(),
            serde_json::from_value::<WebUiResolveGateRequest>(json!({
                "client_action_id": "gate-1",
                "thread_id": "thread-alpha",
                "run_id": run_id_string(),
                "gate_ref": "gate-alpha",
                "resolution": "approved"
            }))
            .expect("request"),
        )
        .await
        .expect("gate resolution succeeds");

    assert!(matches!(response, RebornResolveGateResponse::Resumed(_)));
    assert_eq!(coordinator.resumption_count(), 1);
    assert!(
        coordinator
            .last_resumption_source_binding_ref()
            .expect("resume source binding")
            .contains("gate-alpha")
    );
}

#[tokio::test]
async fn credential_gate_resolution_returns_sanitized_stable_error_until_gate_port_exists() {
    let coordinator = Arc::new(FakeTurnCoordinator::default());
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        coordinator.clone(),
    );

    let err = services
        .resolve_gate(
            caller(),
            serde_json::from_value::<WebUiResolveGateRequest>(json!({
                "client_action_id": "gate-credential",
                "thread_id": "thread-alpha",
                "run_id": run_id_string(),
                "gate_ref": "gate-alpha",
                "resolution": "credential_provided",
                "credential_ref": "credential-alpha"
            }))
            .expect("request"),
        )
        .await
        .expect_err("credential resolution is not wired yet");

    assert_eq!(err.code, RebornServicesErrorCode::Unavailable);
    assert_eq!(err.kind, RebornServicesErrorKind::BlockedAuthentication);
    assert_eq!(err.status_code, 503);
    assert_eq!(coordinator.resumption_count(), 0);
    let rendered = format!("{err:?} {}", serde_json::to_string(&err).expect("json"));
    assert!(!rendered.contains("credential-alpha"));
}

#[tokio::test]
async fn denied_gate_resolution_cancels_run() {
    let coordinator = Arc::new(FakeTurnCoordinator::default());
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        coordinator.clone(),
    );
    setup_owned_thread(&services, caller(), "thread-alpha").await;
    coordinator.set_parked_gate(GateRef::new("gate-alpha").expect("gate"));

    let response = services
        .resolve_gate(
            caller(),
            serde_json::from_value::<WebUiResolveGateRequest>(json!({
                "client_action_id": "gate-2",
                "thread_id": "thread-alpha",
                "run_id": run_id_string(),
                "gate_ref": "gate-alpha",
                "resolution": "denied"
            }))
            .expect("request"),
        )
        .await
        .expect("gate denial succeeds");

    assert!(matches!(response, RebornResolveGateResponse::Cancelled(_)));
    assert_eq!(coordinator.cancellation_count(), 1);
}

// Regression: cancel_run must reject when the authenticated user does not own
// the thread. TurnScope only carries (tenant, agent, project, thread_id), so
// without this check any caller sharing an agent scope could cancel another
// user's run by guessing the run_id.
#[tokio::test]
async fn cancel_run_rejects_cross_user_access() {
    let coordinator = Arc::new(FakeTurnCoordinator::default());
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        coordinator.clone(),
    );
    let alice = caller();
    setup_owned_thread(&services, alice.clone(), "thread-alice").await;

    // Bob shares Alice's (tenant, agent, project) scope and guesses her thread.
    let bob = WebUiAuthenticatedCaller::new(
        TenantId::new("tenant-alpha").expect("tenant"),
        UserId::new("user-bob").expect("user"),
        alice.agent_id.clone(),
        alice.project_id.clone(),
    );

    let err = services
        .cancel_run(
            bob,
            serde_json::from_value::<WebUiCancelRunRequest>(json!({
                "client_action_id": "cancel-cross",
                "thread_id": "thread-alice",
                "run_id": run_id_string(),
                "reason": "user_requested"
            }))
            .expect("request"),
        )
        .await
        .expect_err("cross-user cancel must be rejected");

    // 404 rather than 403 so the existence of Alice's thread is not leaked.
    assert_eq!(err.code, RebornServicesErrorCode::NotFound);
    assert_eq!(err.kind, RebornServicesErrorKind::NotFound);
    assert_eq!(err.status_code, 404);
    assert_eq!(
        coordinator.cancellation_count(),
        0,
        "turn coordinator must NOT be called for cross-user cancel"
    );
}

// Regression: the ownership probe must collapse `ThreadScopeMismatch` and
// `UnknownThread` into the same NotFound response. Current backends return
// `UnknownThread` for `list_thread_history` scope mismatches, but the contract
// also permits `ThreadScopeMismatch`; if a future backend change starts
// emitting it, the default `map_thread_error` path would surface 409 Conflict
// instead, signalling to attackers that the thread exists under a different
// owner. Lock in the explicit remap by driving cancel_run through a stub that
// always returns `ThreadScopeMismatch`.
#[tokio::test]
async fn cancel_run_remaps_thread_scope_mismatch_to_not_found() {
    let coordinator = Arc::new(FakeTurnCoordinator::default());
    let services = RebornServices::new(
        Arc::new(FailingThreadHistoryStub::scope_mismatch()),
        coordinator.clone(),
    );

    let err = services
        .cancel_run(
            caller(),
            serde_json::from_value::<WebUiCancelRunRequest>(json!({
                "client_action_id": "cancel-scope-mismatch",
                "thread_id": "thread-alpha",
                "run_id": run_id_string(),
                "reason": "user_requested"
            }))
            .expect("request"),
        )
        .await
        .expect_err("scope mismatch must surface as NotFound");

    assert_eq!(err.code, RebornServicesErrorCode::NotFound);
    assert_eq!(err.kind, RebornServicesErrorKind::NotFound);
    assert_eq!(err.status_code, 404);
    assert_eq!(
        coordinator.cancellation_count(),
        0,
        "turn coordinator must NOT be called when ownership probe fails"
    );
}

// Regression: resolve_gate must reject when the authenticated user does not
// own the thread, for both the approve→resume path and the deny/cancel path.
#[tokio::test]
async fn resolve_gate_rejects_cross_user_access() {
    let coordinator = Arc::new(FakeTurnCoordinator::default());
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        coordinator.clone(),
    );
    let alice = caller();
    setup_owned_thread(&services, alice.clone(), "thread-alice").await;
    coordinator.set_parked_gate(GateRef::new("gate-alpha").expect("gate"));

    let bob = WebUiAuthenticatedCaller::new(
        TenantId::new("tenant-alpha").expect("tenant"),
        UserId::new("user-bob").expect("user"),
        alice.agent_id.clone(),
        alice.project_id.clone(),
    );

    let err = services
        .resolve_gate(
            bob,
            serde_json::from_value::<WebUiResolveGateRequest>(json!({
                "client_action_id": "gate-cross",
                "thread_id": "thread-alice",
                "run_id": run_id_string(),
                "gate_ref": "gate-alpha",
                "resolution": "denied"
            }))
            .expect("request"),
        )
        .await
        .expect_err("cross-user gate resolution must be rejected");

    assert_eq!(err.code, RebornServicesErrorCode::NotFound);
    assert_eq!(err.kind, RebornServicesErrorKind::NotFound);
    assert_eq!(err.status_code, 404);
    assert_eq!(
        coordinator.cancellation_count(),
        0,
        "turn coordinator must NOT be called for cross-user resolve"
    );
}

// Regression: cancel_run is not gate-aware, so without a parked-on-gate check
// a denied/cancelled resolution carrying a stale or attacker-supplied gate_ref
// would cancel any non-terminal run with the matching run_id. Mismatched gate
// must produce Conflict and cancel_run must never be invoked.
#[tokio::test]
async fn denied_gate_resolution_with_stale_gate_ref_returns_conflict() {
    let coordinator = Arc::new(FakeTurnCoordinator::default());
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        coordinator.clone(),
    );
    setup_owned_thread(&services, caller(), "thread-alpha").await;
    // The run is parked on `gate-current`, but the browser supplies `gate-stale`.
    coordinator.set_parked_gate(GateRef::new("gate-current").expect("gate"));

    let err = services
        .resolve_gate(
            caller(),
            serde_json::from_value::<WebUiResolveGateRequest>(json!({
                "client_action_id": "gate-stale",
                "thread_id": "thread-alpha",
                "run_id": run_id_string(),
                "gate_ref": "gate-stale",
                "resolution": "denied"
            }))
            .expect("request"),
        )
        .await
        .expect_err("stale gate_ref must produce Conflict, not silent cancel");

    assert_eq!(err.code, RebornServicesErrorCode::Conflict);
    assert_eq!(err.kind, RebornServicesErrorKind::BlockedApproval);
    assert_eq!(err.status_code, 409);
    assert_eq!(
        coordinator.cancellation_count(),
        0,
        "cancel_run must NOT be called for stale gate_ref"
    );
}

// Regression: `Approved { always: true }` requests a persistent approval which
// this facade cannot honor (no approval-policy port). Reject as Unavailable
// instead of silently downgrading to one-shot.
#[tokio::test]
async fn approved_gate_resolution_with_persistent_flag_is_rejected() {
    let coordinator = Arc::new(FakeTurnCoordinator::default());
    let services = RebornServices::new(
        Arc::new(InMemorySessionThreadService::default()),
        coordinator.clone(),
    );
    setup_owned_thread(&services, caller(), "thread-alpha").await;

    let err = services
        .resolve_gate(
            caller(),
            serde_json::from_value::<WebUiResolveGateRequest>(json!({
                "client_action_id": "gate-always",
                "thread_id": "thread-alpha",
                "run_id": run_id_string(),
                "gate_ref": "gate-alpha",
                "resolution": "approved",
                "always": true,
            }))
            .expect("request"),
        )
        .await
        .expect_err("persistent approval must be rejected");

    assert_eq!(err.code, RebornServicesErrorCode::Unavailable);
    assert_eq!(err.kind, RebornServicesErrorKind::BlockedApproval);
    assert_eq!(err.status_code, 503);
    assert_eq!(
        coordinator.resumption_count(),
        0,
        "resume_turn must NOT be called for unsupported persistent approval"
    );
}

#[test]
fn facade_source_avoids_forbidden_runtime_dependencies() {
    let source = std::fs::read_to_string("src/reborn_services.rs").expect("facade source");
    for forbidden in [
        "CapabilityHost",
        "ironclaw_capabilities",
        "ironclaw_dispatcher",
        "ironclaw_host_runtime",
        "ironclaw_run_state",
        "ironclaw_storage",
        "RuntimeLane",
        "pub fn thread_service",
        "pub fn turn_coordinator",
    ] {
        assert!(
            !source.contains(forbidden),
            "RebornServices facade must not expose route handlers to {forbidden}"
        );
    }

    let _ = Utc::now();
}
