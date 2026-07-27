use super::*;

use std::collections::VecDeque;
use std::sync::Mutex;

use chrono::Utc;
use ironclaw_host_api::{CapabilityId, ProviderToolName, ThreadId, UserId};
use ironclaw_product::{
    AdapterInstallationId, ExternalConversationRef, ProductAdapterError, ProductAdapterId,
    ProductOutboundTarget, ProjectionCursor,
};
use ironclaw_reborn_openai_compat::{
    OPENAI_COMPAT_ADAPTER_ID, OPENAI_COMPAT_INSTALLATION_ID, OpenAiCompatActorScope,
    OpenAiCompatInternalRefs, OpenAiCompatProductActionRef, OpenAiCompatProjectionRef,
    OpenAiCompatPublicId, OpenAiCompatRequestFingerprint, OpenAiCompatRouteSurface,
    OpenAiCompatTurnRunRef, OpenAiResponseId,
};
use ironclaw_threads::{
    AppendAssistantDraftRequest, AppendToolResultReferenceRequest, EnsureThreadRequest,
    InMemorySessionThreadService, MessageContent, ToolResultSafeSummary,
};
use ironclaw_turns::{
    AcceptedMessageRef, EventCursor, ExternalToolCatalog, InMemoryExternalToolCatalog,
    PendingExternalCall, ReplyTargetBindingRef, ResumeTurnResponse, RunProfileId,
    RunProfileVersion, SourceBindingRef, TurnActor, TurnId, TurnRunId, TurnRunState, TurnScope,
};

#[tokio::test]
async fn openai_responses_retrieve_returns_failed_projection_status() {
    let fixture = ResponseReaderFixture::new("failed").await;
    let run_id = TurnRunId::new();
    let reader = OpenAiResponsesThreadProjectionReader::new(
        fixture.threads.clone(),
        Arc::new(StaticProjectionStream::new(vec![run_status_envelope(
            fixture.thread_id.as_str(),
            run_id,
            "failed",
        )])),
        no_external_tools(),
    );

    let response = reader
        .read_response(fixture.read_request(run_id))
        .await
        .expect("read response");

    assert_eq!(response.status, OpenAiResponseStatus::Failed);
    assert!(response.output.is_empty());
    assert!(response.error.is_some());
}

#[tokio::test]
async fn openai_responses_retrieve_returns_cancelled_projection_status() {
    let fixture = ResponseReaderFixture::new("cancelled").await;
    let run_id = TurnRunId::new();
    let reader = OpenAiResponsesThreadProjectionReader::new(
        fixture.threads.clone(),
        Arc::new(StaticProjectionStream::new(vec![run_status_envelope(
            fixture.thread_id.as_str(),
            run_id,
            "cancelled",
        )])),
        no_external_tools(),
    );

    let response = reader
        .read_response(fixture.read_request(run_id))
        .await
        .expect("read response");

    assert_eq!(response.status, OpenAiResponseStatus::Cancelled);
    assert!(response.output.is_empty());
    assert!(response.error.is_none());
}

#[tokio::test]
async fn openai_responses_retrieve_ignores_other_run_statuses() {
    let fixture = ResponseReaderFixture::new("other-run").await;
    let requested_run_id = TurnRunId::new();
    let reader = OpenAiResponsesThreadProjectionReader::new(
        fixture.threads.clone(),
        Arc::new(StaticProjectionStream::new(vec![run_status_envelope(
            fixture.thread_id.as_str(),
            TurnRunId::new(),
            "failed",
        )])),
        no_external_tools(),
    );

    let response = reader
        .read_response(fixture.read_request(requested_run_id))
        .await
        .expect("read response");

    assert_eq!(response.status, OpenAiResponseStatus::InProgress);
    assert!(response.output.is_empty());
    assert!(response.error.is_none());
}

#[tokio::test]
async fn openai_responses_retrieve_keeps_finalized_message_completion() {
    let fixture = ResponseReaderFixture::new("completed").await;
    let run_id = TurnRunId::new();
    fixture
        .append_final_assistant_message(run_id, "done")
        .await
        .expect("append final message");
    let reader = OpenAiResponsesThreadProjectionReader::new(
        fixture.threads.clone(),
        Arc::new(StaticProjectionStream::new(vec![run_status_envelope(
            fixture.thread_id.as_str(),
            run_id,
            "running",
        )])),
        no_external_tools(),
    );

    let response = reader
        .read_response(fixture.read_request(run_id))
        .await
        .expect("read response");

    assert_eq!(response.status, OpenAiResponseStatus::Completed);
    assert_eq!(response.output.len(), 1);
    assert!(response.error.is_none());
}

#[tokio::test]
async fn openai_responses_retrieve_keeps_completed_projection_in_progress_until_message() {
    let fixture = ResponseReaderFixture::new("completed-lag").await;
    let run_id = TurnRunId::new();
    let reader = OpenAiResponsesThreadProjectionReader::new(
        fixture.threads.clone(),
        Arc::new(StaticProjectionStream::new(vec![run_status_envelope(
            fixture.thread_id.as_str(),
            run_id,
            "completed",
        )])),
        no_external_tools(),
    );

    let response = reader
        .read_response(fixture.read_request(run_id))
        .await
        .expect("read response");

    assert_eq!(response.status, OpenAiResponseStatus::InProgress);
    assert!(response.output.is_empty());
    assert!(response.error.is_none());
}

#[tokio::test]
async fn openai_responses_wait_returns_terminal_projection_status_without_message() {
    let fixture = ResponseReaderFixture::new("wait-failed").await;
    let run_id = TurnRunId::new();
    let reader = OpenAiResponsesThreadProjectionReader::new(
        fixture.threads.clone(),
        Arc::new(StaticProjectionStream::new(vec![run_status_envelope(
            fixture.thread_id.as_str(),
            run_id,
            "failed",
        )])),
        no_external_tools(),
    );

    let projection = reader
        .wait_for_response_completion(fixture.wait_request(run_id))
        .await
        .expect("wait response");

    assert_eq!(projection.response.status, OpenAiResponseStatus::Failed);
    assert!(projection.response.output.is_empty());
    assert!(projection.response.error.is_some());
}

#[tokio::test]
async fn openai_responses_wait_advances_projection_cursor_between_polls() {
    let fixture = ResponseReaderFixture::new("wait-cursor").await;
    let run_id = TurnRunId::new();
    let first = run_status_envelope(fixture.thread_id.as_str(), TurnRunId::new(), "running");
    let first_cursor = first.projection_cursor().clone();
    let stream = Arc::new(SequencedProjectionStream::new(vec![
        vec![first],
        vec![run_status_envelope(
            fixture.thread_id.as_str(),
            run_id,
            "failed",
        )],
    ]));
    let mut reader = OpenAiResponsesThreadProjectionReader::new(
        fixture.threads.clone(),
        stream.clone(),
        no_external_tools(),
    );
    reader.poll_interval = Duration::from_millis(1);

    let projection = reader
        .wait_for_response_completion(fixture.wait_request(run_id))
        .await
        .expect("wait response");

    assert_eq!(projection.response.status, OpenAiResponseStatus::Failed);
    assert_eq!(stream.after_cursors(), vec![None, Some(first_cursor)]);
}

#[tokio::test]
async fn openai_responses_retrieve_surfaces_parked_external_tool_call() {
    let fixture = ResponseReaderFixture::new("blocked-ext").await;
    let run_id = TurnRunId::new();
    let catalog = Arc::new(InMemoryExternalToolCatalog::new());
    catalog
        .record_pending_call(
            run_id,
            PendingExternalCall::new("call_abc", "get_weather", serde_json::json!({"city": "SF"})),
        )
        .await
        .expect("record pending call");
    let reader = OpenAiResponsesThreadProjectionReader::new(
        fixture.threads.clone(),
        Arc::new(StaticProjectionStream::new(vec![run_status_envelope(
            fixture.thread_id.as_str(),
            run_id,
            "blocked_external_tool",
        )])),
        catalog,
    );

    let response = reader
        .read_response(fixture.read_request(run_id))
        .await
        .expect("read response");

    // A run parked on a client tool reads as a completed turn whose output is
    // the pending `function_call` the client must fulfil.
    assert_eq!(response.status, OpenAiResponseStatus::Completed);
    assert_eq!(response.output.len(), 1);
    match &response.output[0] {
        OpenAiResponseOutputItem::FunctionCall {
            call_id,
            name,
            arguments,
            ..
        } => {
            assert_eq!(call_id, "call_abc");
            assert_eq!(name, "get_weather");
            assert!(arguments.contains("SF"), "arguments: {arguments}");
        }
        other => panic!("expected a function_call item, got {other:?}"),
    }
}

#[tokio::test]
async fn openai_responses_retrieve_prefers_coordinator_state_over_stale_blocked_event() {
    let fixture = ResponseReaderFixture::new("blocked-ext-stale-state").await;
    let run_id = TurnRunId::new();
    let catalog = Arc::new(InMemoryExternalToolCatalog::new());
    catalog
        .record_pending_call(
            run_id,
            PendingExternalCall::new("call_stale", "lookup", serde_json::json!({"q": "rust"})),
        )
        .await
        .expect("record pending call");
    let reader = OpenAiResponsesThreadProjectionReader::new(
        fixture.threads.clone(),
        Arc::new(StaticProjectionStream::new(vec![run_status_envelope(
            fixture.thread_id.as_str(),
            run_id,
            "blocked_external_tool",
        )])),
        catalog,
    )
    .with_turn_coordinator(Arc::new(StaticTurnCoordinator::new(turn_run_state(
        &fixture.projection_read,
        run_id,
        TurnStatus::Running,
    ))));

    let response = reader
        .read_response(fixture.read_request(run_id))
        .await
        .expect("read response");

    assert_eq!(response.status, OpenAiResponseStatus::InProgress);
    assert!(
        response.output.is_empty(),
        "stale blocked events must not surface pending calls while coordinator says running"
    );
}

#[tokio::test]
async fn openai_responses_wait_surfaces_parked_external_tool_call() {
    let fixture = ResponseReaderFixture::new("blocked-ext-wait").await;
    let run_id = TurnRunId::new();
    let catalog = Arc::new(InMemoryExternalToolCatalog::new());
    catalog
        .record_pending_call(
            run_id,
            PendingExternalCall::new("call_xyz", "lookup", serde_json::json!({"q": "rust"})),
        )
        .await
        .expect("record pending call");
    let reader = OpenAiResponsesThreadProjectionReader::new(
        fixture.threads.clone(),
        Arc::new(StaticProjectionStream::new(vec![run_status_envelope(
            fixture.thread_id.as_str(),
            run_id,
            "blocked_external_tool",
        )])),
        catalog,
    );

    let projection = reader
        .wait_for_response_completion(fixture.wait_request(run_id))
        .await
        .expect("wait response");

    assert_eq!(projection.response.status, OpenAiResponseStatus::Completed);
    assert_eq!(projection.response.output.len(), 1);
    match &projection.response.output[0] {
        OpenAiResponseOutputItem::FunctionCall { call_id, name, .. } => {
            assert_eq!(call_id, "call_xyz");
            assert_eq!(name, "lookup");
        }
        other => panic!("expected a function_call item, got {other:?}"),
    }
}

#[tokio::test]
async fn openai_responses_wait_surfaces_parked_external_tool_call_from_run_state() {
    let fixture = ResponseReaderFixture::new("blocked-ext-state").await;
    let run_id = TurnRunId::new();
    let catalog = Arc::new(InMemoryExternalToolCatalog::new());
    catalog
        .record_pending_call(
            run_id,
            PendingExternalCall::new("call_state", "lookup", serde_json::json!({"q": "rust"})),
        )
        .await
        .expect("record pending call");
    let mut reader = OpenAiResponsesThreadProjectionReader::new(
        fixture.threads.clone(),
        Arc::new(StaticProjectionStream::new(vec![])),
        catalog,
    )
    .with_turn_coordinator(Arc::new(StaticTurnCoordinator::new(turn_run_state(
        &fixture.projection_read,
        run_id,
        TurnStatus::BlockedExternalTool,
    ))));
    reader.poll_interval = Duration::from_millis(1);

    let projection = reader
        .wait_for_response_completion(fixture.wait_request(run_id))
        .await
        .expect("wait response");

    assert_eq!(projection.response.status, OpenAiResponseStatus::Completed);
    assert_eq!(projection.response.output.len(), 1);
    match &projection.response.output[0] {
        OpenAiResponseOutputItem::FunctionCall { call_id, name, .. } => {
            assert_eq!(call_id, "call_state");
            assert_eq!(name, "lookup");
        }
        other => panic!("expected a function_call item, got {other:?}"),
    }
}

#[tokio::test]
async fn openai_responses_wait_surfaces_pending_external_call_with_sibling_tool_output() {
    let fixture = ResponseReaderFixture::new("blocked-ext-sibling-output").await;
    let run_id = TurnRunId::new();
    append_run_output_tool_result_for_run(
        &fixture.threads,
        &fixture.thread_scope,
        &fixture.thread_id,
        &run_id.to_string(),
    )
    .await;
    let catalog = Arc::new(InMemoryExternalToolCatalog::new());
    catalog
        .record_pending_call(
            run_id,
            PendingExternalCall::new(
                "call_pending",
                "lookup_weather",
                serde_json::json!({"city": "Boston"}),
            ),
        )
        .await
        .expect("record pending call");
    let mut reader = OpenAiResponsesThreadProjectionReader::new(
        fixture.threads.clone(),
        Arc::new(StaticProjectionStream::new(vec![])),
        catalog,
    )
    .with_turn_coordinator(Arc::new(StaticTurnCoordinator::new(turn_run_state(
        &fixture.projection_read,
        run_id,
        TurnStatus::BlockedExternalTool,
    ))));
    reader.poll_interval = Duration::from_millis(1);

    let projection = reader
        .wait_for_response_completion(fixture.wait_request(run_id))
        .await
        .expect("wait response");

    assert_eq!(projection.response.status, OpenAiResponseStatus::Completed);
    assert_eq!(projection.response.output.len(), 3);
    assert!(
        projection.response.output.iter().any(|item| matches!(
            item,
            OpenAiResponseOutputItem::FunctionCallOutput { call_id, .. }
                if call_id == "call_abc"
        )),
        "sibling internal tool output must be preserved"
    );
    assert!(
        projection.response.output.iter().any(|item| matches!(
            item,
            OpenAiResponseOutputItem::FunctionCall { call_id, name, .. }
                if call_id == "call_pending" && name == "lookup_weather"
        )),
        "pending external tool call must be surfaced"
    );
}

#[tokio::test]
async fn external_tool_store_rejects_output_for_non_pending_call_id() {
    let catalog = Arc::new(InMemoryExternalToolCatalog::new());
    let run_id = TurnRunId::new();
    catalog
        .record_pending_call(
            run_id,
            PendingExternalCall::new(
                "call_expected",
                "lookup_weather",
                serde_json::json!({"city": "Boston"}),
            ),
        )
        .await
        .expect("record pending call");
    let store = OpenAiCompatRuntimeExternalToolStore {
        catalog: catalog.clone(),
    };

    let error = store
        .submit_tool_output(
            OpenAiCompatTurnRunRef::new(run_id.to_string()).expect("run ref"),
            "call_wrong".to_string(),
            serde_json::json!("weather:sunny"),
        )
        .await
        .expect_err("wrong call id must be rejected");

    assert_eq!(error.status_code(), 400);
    assert_eq!(
        catalog.pending_calls(run_id).await.expect("pending").len(),
        1
    );
    assert_eq!(
        catalog
            .take_output(run_id, "call_wrong")
            .await
            .expect("take wrong output"),
        None
    );
}

#[tokio::test]
async fn external_tool_store_accepts_output_for_pending_call_id() {
    let catalog = Arc::new(InMemoryExternalToolCatalog::new());
    let run_id = TurnRunId::new();
    catalog
        .record_pending_call(
            run_id,
            PendingExternalCall::new(
                "call_expected",
                "lookup_weather",
                serde_json::json!({"city": "Boston"}),
            ),
        )
        .await
        .expect("record pending call");
    let store = OpenAiCompatRuntimeExternalToolStore {
        catalog: catalog.clone(),
    };

    store
        .submit_tool_output(
            OpenAiCompatTurnRunRef::new(run_id.to_string()).expect("run ref"),
            "call_expected".to_string(),
            serde_json::json!("weather:sunny"),
        )
        .await
        .expect("pending call id is accepted");

    assert_eq!(
        catalog.pending_calls(run_id).await.expect("pending").len(),
        1,
        "pending call is cleared only after the resumed capability result is written"
    );
    assert_eq!(
        catalog
            .take_output(run_id, "call_expected")
            .await
            .expect("take output"),
        Some(serde_json::json!("weather:sunny"))
    );
}

#[test]
fn openai_compat_resume_scope_preserves_actor_owner_boundary() {
    let tenant_id = TenantId::new("tenant-resume").expect("tenant");
    let user_id = UserId::new("user-resume").expect("user");
    let agent_id = AgentId::new("agent-resume").expect("agent");
    let project_id = ProjectId::new("project-resume").expect("project");
    let thread_id = ThreadId::new("thread-resume").expect("thread");
    let actor_scope = OpenAiCompatActorScope::new(
        tenant_id.clone(),
        user_id.clone(),
        Some(agent_id.clone()),
        Some(project_id.clone()),
    );

    let scope = openai_compat_resume_turn_scope(&actor_scope, thread_id.clone());

    assert_eq!(scope.tenant_id, tenant_id);
    assert_eq!(scope.agent_id, Some(agent_id));
    assert_eq!(scope.project_id, Some(project_id));
    assert_eq!(scope.thread_id, thread_id);
    assert_eq!(scope.explicit_owner_user_id(), Some(&user_id));
}

#[test]
fn external_tool_resume_idempotency_key_is_stable_and_gate_scoped() {
    let gate_ref = ironclaw_turns::GateRef::new("gate:external_tool-call-a").expect("gate ref");
    let same_gate_ref =
        ironclaw_turns::GateRef::new("gate:external_tool-call-a").expect("gate ref");
    let other_gate_ref =
        ironclaw_turns::GateRef::new("gate:external_tool-call-b").expect("gate ref");

    let first =
        openai_compat_external_tool_resume_idempotency_key(&gate_ref).expect("idempotency key");
    let second = openai_compat_external_tool_resume_idempotency_key(&same_gate_ref)
        .expect("idempotency key");
    let other = openai_compat_external_tool_resume_idempotency_key(&other_gate_ref)
        .expect("idempotency key");

    assert_eq!(first.as_str(), second.as_str());
    assert_ne!(first.as_str(), other.as_str());
    assert!(first.as_str().starts_with("openai-compat-ext-resume-v1-"));
    assert!(!first.as_str().contains(gate_ref.as_str()));
}

#[tokio::test]
async fn external_tool_resume_rejects_non_blocked_running_run() {
    let fixture = ResponseReaderFixture::new("resume-running").await;
    let run_id = TurnRunId::new();
    let resume = OpenAiCompatRuntimeExternalToolResume {
        coordinator: Arc::new(StaticTurnCoordinator::new(turn_run_state(
            &fixture.projection_read,
            run_id,
            TurnStatus::Running,
        ))),
    };

    let error = resume
        .resume_external_tool_run(OpenAiCompatExternalToolResumeRequest {
            actor_scope: fixture.actor_scope.clone(),
            run_ref: OpenAiCompatTurnRunRef::new(run_id.to_string()).expect("run ref"),
            thread_id: fixture.thread_id.as_str().to_string(),
        })
        .await
        .expect_err("running run must not accept external tool resume");

    assert_eq!(error.status_code(), 409);
}

struct ResponseReaderFixture {
    threads: Arc<InMemorySessionThreadService>,
    actor_scope: OpenAiCompatActorScope,
    projection_read: ProjectionReadRequest,
    thread_scope: ThreadScope,
    thread_id: ThreadId,
}

impl ResponseReaderFixture {
    async fn new(suffix: &str) -> Self {
        let tenant_id = TenantId::new(format!("tenant-{suffix}")).expect("tenant");
        let user_id = UserId::new(format!("user-{suffix}")).expect("user");
        let agent_id = AgentId::new(format!("agent-{suffix}")).expect("agent");
        let thread_id = ThreadId::new(format!("thread-{suffix}")).expect("thread");
        let actor_scope = OpenAiCompatActorScope::new(
            tenant_id.clone(),
            user_id.clone(),
            Some(agent_id.clone()),
            None,
        );
        let projection_read = ProjectionReadRequest {
            actor: TurnActor::new(user_id.clone()),
            scope: TurnScope::new_with_owner(
                tenant_id.clone(),
                Some(agent_id.clone()),
                None,
                thread_id.clone(),
                Some(user_id.clone()),
            ),
            after_cursor: None,
            limit: None,
        };
        let thread_scope = ThreadScope {
            tenant_id,
            agent_id,
            project_id: None,
            owner_user_id: Some(user_id),
            mission_id: None,
        };
        let threads = Arc::new(InMemorySessionThreadService::default());
        threads
            .ensure_thread(EnsureThreadRequest {
                scope: thread_scope.clone(),
                thread_id: Some(thread_id.clone()),
                created_by_actor_id: "actor:test".to_string(),
                title: None,
                metadata_json: None,
            })
            .await
            .expect("ensure thread");

        Self {
            threads,
            actor_scope,
            projection_read,
            thread_scope,
            thread_id,
        }
    }

    fn read_request(&self, run_id: TurnRunId) -> OpenAiResponseReadRequest {
        OpenAiResponseReadRequest {
            public_id: OpenAiResponseId::new("resp_test").expect("response id"),
            actor_scope: self.actor_scope.clone(),
            requested_model: Some("reborn-test".to_string()),
            projection_read: self.projection_read.clone(),
            mapping: self.mapping(run_id),
        }
    }

    fn wait_request(&self, run_id: TurnRunId) -> OpenAiResponseWaitRequest {
        OpenAiResponseWaitRequest {
            public_id: OpenAiResponseId::new("resp_test").expect("response id"),
            actor_scope: self.actor_scope.clone(),
            accepted_ack: Some(ProductInboundAck::Accepted {
                accepted_message_ref: ironclaw_turns::AcceptedMessageRef::new("accepted:test")
                    .expect("accepted ref"),
                submitted_run_id: run_id,
            }),
            requested_model: "reborn-test".to_string(),
            projection_read: self.projection_read.clone(),
            mapping: self.mapping(run_id),
        }
    }

    fn mapping(
        &self,
        run_id: TurnRunId,
    ) -> ironclaw_reborn_openai_compat::OpenAiCompatResourceMapping {
        ironclaw_reborn_openai_compat::OpenAiCompatResourceMapping {
            public_id: OpenAiCompatPublicId::Response(
                OpenAiResponseId::new("resp_test").expect("response id"),
            ),
            owner: self.actor_scope.clone(),
            surface: OpenAiCompatRouteSurface::ResponsesApi,
            request_fingerprint: OpenAiCompatRequestFingerprint::from_body_bytes(b"{}"),
            created_at: 123,
            idempotency_key: None,
            accepted_ack: None,
            external_tool_resume_completed: false,
            binding: OpenAiCompatResourceBinding::Bound {
                internal_refs: OpenAiCompatInternalRefs::new(
                    OpenAiCompatProductActionRef::new("product-action:test").expect("action ref"),
                )
                .with_turn_run_ref(
                    OpenAiCompatTurnRunRef::new(run_id.to_string()).expect("run ref"),
                )
                .with_projection_ref(
                    OpenAiCompatProjectionRef::new("projection:test").expect("projection ref"),
                ),
            },
        }
    }

    async fn append_final_assistant_message(
        &self,
        run_id: TurnRunId,
        text: &str,
    ) -> Result<(), SessionThreadError> {
        let message = self
            .threads
            .append_assistant_draft(AppendAssistantDraftRequest {
                scope: self.thread_scope.clone(),
                thread_id: self.thread_id.clone(),
                turn_run_id: run_id.to_string(),
                content: MessageContent::text(text),
            })
            .await?;
        self.threads
            .finalize_assistant_message(
                &self.thread_scope,
                &self.thread_id,
                message.message_id,
                MessageContent::text(text),
            )
            .await?;
        Ok(())
    }
}

struct StaticProjectionStream {
    envelopes: Vec<ProductOutboundEnvelope>,
}

impl StaticProjectionStream {
    fn new(envelopes: Vec<ProductOutboundEnvelope>) -> Self {
        Self { envelopes }
    }
}

#[async_trait]
impl ProjectionStream for StaticProjectionStream {
    async fn drain(
        &self,
        _request: ProjectionSubscriptionRequest,
    ) -> Result<Vec<ProductOutboundEnvelope>, ProductAdapterError> {
        Ok(self.envelopes.clone())
    }
}

struct StaticTurnCoordinator {
    state: TurnRunState,
}

impl StaticTurnCoordinator {
    fn new(state: TurnRunState) -> Self {
        Self { state }
    }
}

#[async_trait]
impl TurnCoordinator for StaticTurnCoordinator {
    async fn prepare_turn(&self, _scope: TurnScope) -> Result<TurnRunId, TurnError> {
        Err(TurnError::Unavailable {
            reason: "test coordinator only supports get_run_state".to_string(),
        })
    }

    async fn submit_turn(
        &self,
        _request: ironclaw_turns::SubmitTurnRequest,
    ) -> Result<ironclaw_turns::SubmitTurnResponse, TurnError> {
        Err(TurnError::Unavailable {
            reason: "test coordinator only supports get_run_state".to_string(),
        })
    }

    async fn resume_turn(
        &self,
        _request: ResumeTurnRequest,
    ) -> Result<ResumeTurnResponse, TurnError> {
        Err(TurnError::Unavailable {
            reason: "test coordinator only supports get_run_state".to_string(),
        })
    }

    async fn cancel_run(
        &self,
        _request: ironclaw_turns::CancelRunRequest,
    ) -> Result<ironclaw_turns::CancelRunResponse, TurnError> {
        Err(TurnError::Unavailable {
            reason: "test coordinator only supports get_run_state".to_string(),
        })
    }

    async fn retry_turn(
        &self,
        _request: ironclaw_turns::RetryTurnRequest,
    ) -> Result<ironclaw_turns::RetryTurnResponse, TurnError> {
        Err(TurnError::Unavailable {
            reason: "test coordinator only supports get_run_state".to_string(),
        })
    }

    async fn get_run_state(&self, request: GetRunStateRequest) -> Result<TurnRunState, TurnError> {
        if request.run_id == self.state.run_id && request.scope == self.state.scope {
            Ok(self.state.clone())
        } else {
            Err(TurnError::ScopeNotFound)
        }
    }
}

struct SequencedProjectionStream {
    batches: Mutex<VecDeque<Vec<ProductOutboundEnvelope>>>,
    after_cursors: Mutex<Vec<Option<ProjectionCursor>>>,
}

impl SequencedProjectionStream {
    fn new(batches: Vec<Vec<ProductOutboundEnvelope>>) -> Self {
        Self {
            batches: Mutex::new(batches.into()),
            after_cursors: Mutex::new(Vec::new()),
        }
    }

    fn after_cursors(&self) -> Vec<Option<ProjectionCursor>> {
        self.after_cursors.lock().expect("after cursor log").clone()
    }
}

#[async_trait]
impl ProjectionStream for SequencedProjectionStream {
    async fn drain(
        &self,
        request: ProjectionSubscriptionRequest,
    ) -> Result<Vec<ProductOutboundEnvelope>, ProductAdapterError> {
        self.after_cursors
            .lock()
            .expect("after cursor log")
            .push(request.after_cursor);
        Ok(self
            .batches
            .lock()
            .expect("projection batches")
            .pop_front()
            .unwrap_or_default())
    }
}

fn run_status_envelope(
    thread_id: &str,
    run_id: TurnRunId,
    status: &str,
) -> ProductOutboundEnvelope {
    ProductOutboundEnvelope::new(
        ProductAdapterId::new(OPENAI_COMPAT_ADAPTER_ID).expect("adapter id"),
        AdapterInstallationId::new(OPENAI_COMPAT_INSTALLATION_ID).expect("installation id"),
        ProductOutboundTarget::new(
            ReplyTargetBindingRef::new("reply:test").expect("reply target"),
            ExternalConversationRef::new(None, "conversation:test", None, None)
                .expect("conversation ref"),
            None,
        ),
        ProjectionCursor::new(format!("cursor:{}", run_id.as_uuid())).expect("cursor"),
        ProductOutboundPayload::ProjectionUpdate {
            state: ProductProjectionState::new(
                thread_id,
                vec![ProductProjectionItem::RunStatus {
                    run_id,
                    status: status.to_string(),
                    failure_category: None,
                    failure_summary: None,
                    retryable: None,
                }],
            )
            .expect("projection state"),
        },
    )
}

fn turn_run_state(
    projection_read: &ProjectionReadRequest,
    run_id: TurnRunId,
    status: TurnStatus,
) -> TurnRunState {
    TurnRunState {
        scope: projection_read.scope.clone(),
        actor: Some(projection_read.actor.clone()),
        turn_id: TurnId::new(),
        run_id,
        status,
        accepted_message_ref: AcceptedMessageRef::new("message:openai-compat-test")
            .expect("accepted ref"),
        source_binding_ref: SourceBindingRef::new("source:openai-compat-test").expect("source ref"),
        reply_target_binding_ref: ReplyTargetBindingRef::new("reply:openai-compat-test")
            .expect("reply target"),
        resolved_run_profile_id: RunProfileId::default_profile(),
        resolved_run_profile_version: RunProfileVersion::new(1),
        resolved_model_route: None,
        model_usage: None,
        received_at: Utc::now(),
        checkpoint_id: None,
        gate_ref: None,
        blocked_activity_id: None,
        credential_requirements: Vec::new(),
        failure: None,
        event_cursor: EventCursor::default(),
        product_context: None,
        resume_disposition: None,
    }
}

const RUN_ID: &str = "turn-run-1";

fn run_output_scope() -> ThreadScope {
    ThreadScope {
        tenant_id: TenantId::new("tenant-1").expect("tenant"),
        agent_id: AgentId::new("agent-1").expect("agent"),
        project_id: Some(ProjectId::new("project-1").expect("project")),
        owner_user_id: Some(UserId::new("user-1").expect("user")),
        mission_id: None,
    }
}

/// A schema-valid `model_observation` so the thread service preserves it as
/// the raw, model-visible tool output rather than dropping it.
fn model_observation() -> serde_json::Value {
    serde_json::json!({
        "schema_version": 1,
        "status": "error",
        "summary": "search failed",
        "detail": {
            "kind": "invalid_input",
            "issues": [{ "path": "query", "code": "invalid_value" }]
        },
        "trust": "untrusted_tool_output"
    })
}

fn run_output_provider_call() -> ProviderToolCallReferenceEnvelope {
    ProviderToolCallReferenceEnvelope {
        provider_id: "openai".to_string(),
        provider_model_id: "gpt-test".to_string(),
        provider_turn_id: "turn-1".to_string(),
        provider_call_id: "call_abc".to_string(),
        provider_tool_name: ProviderToolName::new("web_search").expect("provider tool name"),
        capability_id: CapabilityId::new("web.search").expect("capability id"),
        arguments: serde_json::json!({ "query": "rust" }),
        response_reasoning: None,
        reasoning: None,
        signature: None,
    }
}

fn run_output_projection_read(scope: &ThreadScope, thread_id: &ThreadId) -> ProjectionReadRequest {
    ProjectionReadRequest {
        actor: TurnActor::new(scope.owner_user_id.clone().expect("owner")),
        scope: TurnScope::new_with_owner(
            scope.tenant_id.clone(),
            Some(scope.agent_id.clone()),
            scope.project_id.clone(),
            thread_id.clone(),
            scope.owner_user_id.clone(),
        ),
        after_cursor: None,
        limit: None,
    }
}

async fn ensure_run_output_thread(
    service: &InMemorySessionThreadService,
    scope: &ThreadScope,
    thread_id: &ThreadId,
) {
    service
        .ensure_thread(EnsureThreadRequest {
            scope: scope.clone(),
            thread_id: Some(thread_id.clone()),
            created_by_actor_id: "actor".to_string(),
            title: None,
            metadata_json: None,
        })
        .await
        .expect("ensure thread");
}

async fn append_run_output_tool_result(
    service: &InMemorySessionThreadService,
    scope: &ThreadScope,
    thread_id: &ThreadId,
) {
    append_run_output_tool_result_for_run(service, scope, thread_id, RUN_ID).await;
}

async fn append_run_output_tool_result_for_run(
    service: &InMemorySessionThreadService,
    scope: &ThreadScope,
    thread_id: &ThreadId,
    run_id: &str,
) {
    service
        .append_tool_result_reference(AppendToolResultReferenceRequest {
            scope: scope.clone(),
            thread_id: thread_id.clone(),
            turn_run_id: run_id.to_string(),
            result_ref: "result:tool-1".to_string(),
            safe_summary: ToolResultSafeSummary::new("search failed").expect("summary"),
            provider_call: Some(run_output_provider_call()),
            model_observation: Some(model_observation()),
        })
        .await
        .expect("append tool result");
}

fn run_output_reader(
    service: Arc<InMemorySessionThreadService>,
) -> OpenAiResponsesThreadProjectionReader {
    OpenAiResponsesThreadProjectionReader::new(
        service,
        Arc::new(StaticProjectionStream::new(vec![])),
        no_external_tools(),
    )
}

fn no_external_tools() -> Arc<dyn ExternalToolCatalog> {
    Arc::new(InMemoryExternalToolCatalog::new())
}

#[tokio::test]
async fn read_run_output_emits_paired_function_call_and_raw_output() {
    let service = Arc::new(InMemorySessionThreadService::default());
    let scope = run_output_scope();
    let thread_id = ThreadId::new("thread-1").expect("thread");
    ensure_run_output_thread(&service, &scope, &thread_id).await;
    append_run_output_tool_result(&service, &scope, &thread_id).await;

    let reader = run_output_reader(service);
    let public_id = OpenAiResponseId::new("resp_test").expect("response id");
    let projection = reader
        .read_run_output(
            &run_output_projection_read(&scope, &thread_id),
            RUN_ID.to_string(),
            &public_id,
        )
        .await
        .expect("read run output");

    assert!(!projection.assistant_finalized);
    match &projection.items[0] {
        OpenAiResponseOutputItem::FunctionCall {
            call_id,
            name,
            arguments,
            ..
        } => {
            assert_eq!(call_id, "call_abc");
            assert_eq!(name, "web_search");
            let parsed: serde_json::Value =
                serde_json::from_str(arguments).expect("arguments json");
            assert_eq!(parsed, serde_json::json!({ "query": "rust" }));
        }
        other => panic!("expected function_call, got {other:?}"),
    }
    match &projection.items[1] {
        OpenAiResponseOutputItem::FunctionCallOutput {
            call_id, output, ..
        } => {
            assert_eq!(call_id, "call_abc");
            // The raw model_observation flows through verbatim, not the summary.
            assert_eq!(output, &model_observation());
        }
        other => panic!("expected function_call_output, got {other:?}"),
    }
}

#[tokio::test]
async fn read_run_output_orders_tool_items_before_assistant_message() {
    // The assistant draft is created (sequence reserved) BEFORE the tool runs,
    // so a naive sequence sort would place the final message first. Tool items
    // must still come before the assistant message.
    let service = Arc::new(InMemorySessionThreadService::default());
    let scope = run_output_scope();
    let thread_id = ThreadId::new("thread-3").expect("thread");
    ensure_run_output_thread(&service, &scope, &thread_id).await;

    let draft = service
        .append_assistant_draft(AppendAssistantDraftRequest {
            scope: scope.clone(),
            thread_id: thread_id.clone(),
            turn_run_id: RUN_ID.to_string(),
            content: MessageContent::text("here is the answer"),
        })
        .await
        .expect("append assistant draft");
    append_run_output_tool_result(&service, &scope, &thread_id).await;
    service
        .finalize_assistant_message(
            &scope,
            &thread_id,
            draft.message_id,
            MessageContent::text("here is the answer"),
        )
        .await
        .expect("finalize assistant message");

    let reader = run_output_reader(service);
    let public_id = OpenAiResponseId::new("resp_test").expect("response id");
    let projection = reader
        .read_run_output(
            &run_output_projection_read(&scope, &thread_id),
            RUN_ID.to_string(),
            &public_id,
        )
        .await
        .expect("read run output");

    assert!(projection.assistant_finalized);
    assert!(matches!(
        projection.items[0],
        OpenAiResponseOutputItem::FunctionCall { .. }
    ));
    assert!(matches!(
        projection.items[1],
        OpenAiResponseOutputItem::FunctionCallOutput { .. }
    ));
    match &projection.items[2] {
        OpenAiResponseOutputItem::Message {
            id, role, content, ..
        } => {
            assert!(matches!(role, OpenAiResponsesMessageRole::Assistant));
            // Message ids are response-id-keyed (`msg_{response_id}`).
            assert!(id.starts_with("msg_"));
            assert_eq!(
                content,
                &serde_json::json!([{ "type": "output_text", "text": "here is the answer" }])
            );
        }
        other => panic!("expected message, got {other:?}"),
    }
}

#[tokio::test]
async fn read_run_output_in_progress_surfaces_tool_output_without_final_message() {
    let service = Arc::new(InMemorySessionThreadService::default());
    let scope = run_output_scope();
    let thread_id = ThreadId::new("thread-2").expect("thread");
    ensure_run_output_thread(&service, &scope, &thread_id).await;
    append_run_output_tool_result(&service, &scope, &thread_id).await;

    let reader = run_output_reader(service);
    let public_id = OpenAiResponseId::new("resp_test").expect("response id");
    let projection = reader
        .read_run_output(
            &run_output_projection_read(&scope, &thread_id),
            RUN_ID.to_string(),
            &public_id,
        )
        .await
        .expect("read run output");

    assert!(!projection.assistant_finalized);
    assert_eq!(projection.items.len(), 2);
    assert!(matches!(
        projection.items[0],
        OpenAiResponseOutputItem::FunctionCall { .. }
    ));
    assert!(matches!(
        projection.items[1],
        OpenAiResponseOutputItem::FunctionCallOutput { .. }
    ));
}

fn provider_view(
    id: &str,
    default_model: &str,
    active: bool,
    active_model: Option<&str>,
) -> ironclaw_product::LlmProviderView {
    ironclaw_product::LlmProviderView {
        id: id.to_string(),
        description: String::new(),
        adapter: "open_ai_completions".to_string(),
        default_model: default_model.to_string(),
        base_url: None,
        builtin: true,
        active,
        active_model: active_model.map(str::to_string),
        api_key_required: false,
        accepts_api_key: true,
        api_key_set: false,
        can_list_models: true,
    }
}

#[test]
fn model_entries_list_active_first_then_providers_deduped() {
    let snapshot = ironclaw_product::LlmConfigSnapshot {
        providers: vec![
            provider_view("openai", "gpt-4o", true, Some("gpt-4o")),
            provider_view("anthropic", "claude-opus-4", false, None),
            // Duplicate model id (same default) must not be listed twice.
            provider_view("openai-mirror", "gpt-4o", false, None),
        ],
        active: Some(ironclaw_product::LlmActiveSelection {
            provider_id: "openai".to_string(),
            model: Some("gpt-4o".to_string()),
        }),
    };

    let entries = model_entries_from_snapshot(&snapshot);

    assert_eq!(entries.len(), 2, "duplicate model id must be de-duplicated");
    assert_eq!(entries[0].id, "gpt-4o", "active selection listed first");
    assert_eq!(entries[0].owned_by.as_deref(), Some("openai"));
    assert_eq!(entries[1].id, "claude-opus-4");
    assert_eq!(entries[1].owned_by.as_deref(), Some("anthropic"));
}

#[test]
fn model_entries_fall_back_to_default_model_when_no_active_selection() {
    let snapshot = ironclaw_product::LlmConfigSnapshot {
        providers: vec![provider_view("anthropic", "claude-opus-4", false, None)],
        active: None,
    };

    let entries = model_entries_from_snapshot(&snapshot);

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].id, "claude-opus-4");
    assert_eq!(entries[0].owned_by.as_deref(), Some("anthropic"));
}

#[test]
fn response_usage_reports_total_input_including_cache_and_breaks_out_cached_tokens() {
    // `cache_read_input_tokens` is a subset of `input_tokens` (here 2000 of the
    // 3000 total were cache hits), so it must NOT be added on top of the total.
    let usage = LoopModelUsage {
        input_tokens: 3_000,
        output_tokens: 500,
        cache_read_input_tokens: 2_000,
        cache_creation_input_tokens: 0,
    };
    let built = response_usage_from_model_usage(&usage, "gpt-4o");
    // OpenAI `input_tokens` is the total input including the cached subset.
    assert_eq!(built.input_tokens, 3_000);
    assert_eq!(built.output_tokens, 500);
    assert_eq!(built.total_tokens, 3_500);
    assert_eq!(
        built
            .input_tokens_details
            .expect("cached detail")
            .cached_tokens,
        2_000
    );
}

#[test]
fn response_usage_adds_cache_creation_on_top_of_input() {
    // `cache_creation_input_tokens` is a separate write-side count that is NOT
    // part of `input_tokens`, so it is added on top of the total. It is not a
    // cache *read*, so it never populates `input_tokens_details`.
    let usage = LoopModelUsage {
        input_tokens: 1_000,
        output_tokens: 500,
        cache_read_input_tokens: 0,
        cache_creation_input_tokens: 3_000,
    };
    let built = response_usage_from_model_usage(&usage, "gpt-4o");
    assert_eq!(built.input_tokens, 4_000); // 1000 + 3000
    assert_eq!(built.total_tokens, 4_500);
    assert!(built.input_tokens_details.is_none());
}

#[test]
fn response_cost_prices_input_output_and_discounts_cached_tokens() {
    // gpt-4o rates: input 0.0000025/tok, output 0.00001/tok; OpenAI cache-read
    // discount is 2x (50% off). `input_tokens` is the total (3000), of which
    // 2000 were cache reads, leaving 1000 fresh at the full rate.
    let usage = LoopModelUsage {
        input_tokens: 3_000,
        output_tokens: 500,
        cache_read_input_tokens: 2_000,
        cache_creation_input_tokens: 0,
    };
    let cost = response_cost_from_model_usage(&usage, "gpt-4o").expect("cost under llm feature");
    assert_eq!(cost.currency, "USD");
    // fresh input = (3000 - 2000) * 0.0000025 = 0.0025 (cache-read is NOT
    // charged again at the full rate here)
    assert_eq!(cost.input_cost_usd, "0.0025");
    // cached = 2000 * 0.0000025 / 2 = 0.0025
    assert_eq!(cost.cached_input_cost_usd, "0.0025");
    // output = 500 * 0.00001 = 0.005
    assert_eq!(cost.output_cost_usd, "0.005");
    // total = 0.0025 + 0.0025 + 0.005 = 0.01
    assert_eq!(cost.total_cost_usd, "0.01");
}

#[test]
fn response_cost_bills_cache_creation_at_full_input_rate() {
    // gpt-4o input rate 0.0000025/tok. cache_creation is a write-side count
    // billed at the full input rate on top of the fresh input; with no
    // cache_read there is no discounted portion.
    let usage = LoopModelUsage {
        input_tokens: 1_000,
        output_tokens: 0,
        cache_read_input_tokens: 0,
        cache_creation_input_tokens: 3_000,
    };
    let cost = response_cost_from_model_usage(&usage, "gpt-4o").expect("cost");
    // billable input = (1000 - 0) + 3000 = 4000 → 4000 * 0.0000025 = 0.01
    assert_eq!(cost.input_cost_usd, "0.01");
    assert_eq!(cost.cached_input_cost_usd, "0");
}

#[test]
fn response_cost_applies_claude_cache_read_discount() {
    // claude-opus-4-6 input rate 0.000015/tok; the Claude cache-read discount is
    // 10x (90% off). `input_tokens` is the total (3000), of which 2000 were
    // cache reads, leaving 1000 fresh.
    let usage = LoopModelUsage {
        input_tokens: 3_000,
        output_tokens: 0,
        cache_read_input_tokens: 2_000,
        cache_creation_input_tokens: 0,
    };
    let cost = response_cost_from_model_usage(&usage, "claude-opus-4-6").expect("cost");
    // fresh input = (3000 - 2000) * 0.000015 = 0.015
    assert_eq!(cost.input_cost_usd, "0.015");
    // cached = 2000 * 0.000015 / 10 = 0.003
    assert_eq!(cost.cached_input_cost_usd, "0.003");
}

#[test]
fn response_cost_falls_back_to_default_rate_for_unknown_model() {
    // Unknown models must not price at zero — the cost table default (~GPT-4o)
    // applies so a new paid model still bills.
    let usage = LoopModelUsage {
        input_tokens: 1_000,
        output_tokens: 0,
        cache_read_input_tokens: 0,
        cache_creation_input_tokens: 0,
    };
    let cost =
        response_cost_from_model_usage(&usage, "some-brand-new-model-9000").expect("cost present");
    // default input rate is 0.0000025 → 1000 * 0.0000025 = 0.0025
    assert_eq!(cost.input_cost_usd, "0.0025");
    assert_eq!(cost.cached_input_cost_usd, "0");
}
