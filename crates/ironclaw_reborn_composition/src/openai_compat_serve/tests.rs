use super::*;

use std::collections::VecDeque;
use std::sync::Mutex;

use ironclaw_host_api::ThreadId;
use ironclaw_product_adapters::{
    AdapterInstallationId, ExternalConversationRef, ProductAdapterError, ProductAdapterId,
    ProductOutboundTarget, ProjectionCursor,
};
use ironclaw_reborn_openai_compat::{
    OpenAiCompatActorScope, OpenAiCompatInternalRefs, OpenAiCompatProductActionRef,
    OpenAiCompatProjectionRef, OpenAiCompatPublicId, OpenAiCompatRequestFingerprint,
    OpenAiCompatRouteSurface, OpenAiCompatTurnRunRef, OpenAiResponseId,
};
use ironclaw_threads::{
    AppendAssistantDraftRequest, EnsureThreadRequest, InMemorySessionThreadService, MessageContent,
};
use ironclaw_turns::{ReplyTargetBindingRef, TurnActor, TurnRunId, TurnScope};

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
    let mut reader =
        OpenAiResponsesThreadProjectionReader::new(fixture.threads.clone(), stream.clone());
    reader.poll_interval = Duration::from_millis(1);

    let projection = reader
        .wait_for_response_completion(fixture.wait_request(run_id))
        .await
        .expect("wait response");

    assert_eq!(projection.response.status, OpenAiResponseStatus::Failed);
    assert_eq!(stream.after_cursors(), vec![None, Some(first_cursor)]);
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
            accepted_ack: ProductInboundAck::Accepted {
                accepted_message_ref: ironclaw_turns::AcceptedMessageRef::new("accepted:test")
                    .expect("accepted ref"),
                submitted_run_id: run_id,
            },
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
                }],
            )
            .expect("projection state"),
        },
    )
}
