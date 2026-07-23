#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_filesystem::{InMemoryBackend, RootFilesystem};
use ironclaw_host_api::ThreadId;
use ironclaw_product::{
    CANCEL_RUN_COMMAND, CREATE_THREAD_COMMAND, ProductCancelRunRequest, ProductCreateThreadRequest,
    ProductSubmitTurnRequest, ProductSurface, ProductSurfaceCaller, ProductSurfaceCallerExt,
    ProductSurfaceError, ProductSurfaceErrorCode, ProductSurfaceErrorKind, RebornCancelRunResponse,
    RebornCreateThreadResponse, RebornStreamEventsRequest, RebornStreamEventsResponse,
    RebornSubmitTurnResponse, SUBMIT_TURN_COMMAND,
};
use ironclaw_product::{
    ExternalEventId, ProductAdapterId, ProductAttachmentDescriptor, ProductAttachmentKind,
    ProductInboundAck, ProductInboundPayload, ProductRejection, ProductRejectionKind,
    ProductTriggerReason, ProjectionReadRequest, UserMessagePayload,
};
use ironclaw_reborn_openai_compat::{FilesystemOpenAiCompatRefStore, OPENAI_COMPAT_ADAPTER_ID};
use ironclaw_threads::{SessionThreadRecord, ThreadScope};
use ironclaw_turns::{AcceptedMessageRef, EventCursor, TurnRunId, TurnStatus};

pub(crate) fn in_memory_openai_compat_ref_store() -> Arc<FilesystemOpenAiCompatRefStore> {
    let filesystem: Arc<dyn RootFilesystem> = Arc::new(InMemoryBackend::new());
    Arc::new(FilesystemOpenAiCompatRefStore::new(filesystem))
}

pub(crate) struct FakeProductSurface {
    state: Mutex<FakeProductSurfaceState>,
}

#[derive(Default)]
struct FakeProductSurfaceState {
    programmed: HashMap<String, ProductInboundAck>,
    fixed_outcome: Option<ProductInboundAck>,
    submitted: Vec<RecordedProductSurfaceSubmit>,
    cancelled: Vec<ProductCancelRunRequest>,
    read_inputs: Vec<ProjectionReadRequest>,
    stream_events: Vec<RebornStreamEventsRequest>,
    fail_with: Option<ProductSurfaceError>,
}

#[derive(Debug, Clone)]
pub(crate) struct RecordedProductSurfaceSubmit {
    request: ProductSubmitTurnRequest,
}

impl RecordedProductSurfaceSubmit {
    pub(crate) fn adapter_id(&self) -> ProductAdapterId {
        ProductAdapterId::new(OPENAI_COMPAT_ADAPTER_ID).expect("adapter id")
    }

    pub(crate) fn external_event_id(&self) -> ExternalEventId {
        ExternalEventId::new(
            self.request
                .client_action_id
                .as_deref()
                .unwrap_or("missing-client-action-id"),
        )
        .expect("external event id")
    }

    pub(crate) fn payload(&self) -> ProductInboundPayload {
        ProductInboundPayload::UserMessage(
            UserMessagePayload::new(
                self.request.content.clone().unwrap_or_default(),
                self.request
                    .attachments
                    .iter()
                    .map(|attachment| {
                        ProductAttachmentDescriptor::new(
                            format!(
                                "attachment-{}",
                                attachment.filename.as_deref().unwrap_or("inline")
                            ),
                            attachment.mime_type.clone(),
                            attachment.filename.clone(),
                            None,
                            ProductAttachmentKind::Image,
                        )
                        .expect("attachment descriptor")
                    })
                    .collect(),
                ProductTriggerReason::DirectChat,
            )
            .expect("user message payload")
            .with_requested_model(self.request.model.clone()),
        )
    }
}

impl FakeProductSurface {
    pub(crate) fn new() -> Self {
        Self {
            state: Mutex::new(FakeProductSurfaceState::default()),
        }
    }

    pub(crate) fn with_outcome(outcome: ProductInboundAck) -> Self {
        Self {
            state: Mutex::new(FakeProductSurfaceState {
                fixed_outcome: Some(outcome),
                ..FakeProductSurfaceState::default()
            }),
        }
    }

    pub(crate) fn with_error(error: ProductSurfaceError) -> Self {
        Self {
            state: Mutex::new(FakeProductSurfaceState {
                fail_with: Some(error),
                ..FakeProductSurfaceState::default()
            }),
        }
    }

    pub(crate) fn program_outcome(&self, event_id: ExternalEventId, outcome: ProductInboundAck) {
        self.state
            .lock()
            .expect("surface state lock")
            .programmed
            .insert(event_id.as_str().to_string(), outcome);
    }

    pub(crate) fn force_failure(&self, error: ProductSurfaceError) {
        self.state.lock().expect("surface state lock").fail_with = Some(error);
    }

    pub(crate) fn program_projection_read_resolution(&self, _request: ProjectionReadRequest) {}

    pub(crate) fn program_projection_resolution(
        &self,
        _request: ironclaw_product::ProjectionSubscriptionRequest,
    ) {
    }

    pub(crate) fn accepted_envelopes(&self) -> Vec<RecordedProductSurfaceSubmit> {
        self.state
            .lock()
            .expect("surface state lock")
            .submitted
            .clone()
    }

    pub(crate) fn seen_envelopes(&self) -> Vec<RecordedProductSurfaceSubmit> {
        self.accepted_envelopes()
    }

    pub(crate) fn accepted_count(&self) -> usize {
        self.accepted_envelopes().len()
    }

    pub(crate) fn seen_count(&self) -> usize {
        self.accepted_count()
    }

    pub(crate) fn cancel_count(&self) -> usize {
        self.state
            .lock()
            .expect("surface state lock")
            .cancelled
            .len()
    }

    pub(crate) fn cancel_requests(&self) -> Vec<ProductCancelRunRequest> {
        self.state
            .lock()
            .expect("surface state lock")
            .cancelled
            .clone()
    }

    pub(crate) fn read_count(&self) -> usize {
        self.state
            .lock()
            .expect("surface state lock")
            .read_inputs
            .len()
    }

    pub(crate) fn read_inputs(&self) -> Vec<ProjectionReadRequest> {
        self.state
            .lock()
            .expect("surface state lock")
            .read_inputs
            .clone()
    }
}

impl Default for FakeProductSurface {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeProductSurface {
    async fn create_thread(
        &self,
        caller: ProductSurfaceCaller,
        request: ProductCreateThreadRequest,
    ) -> Result<RebornCreateThreadResponse, ProductSurfaceError> {
        if let Some(error) = self
            .state
            .lock()
            .expect("surface state lock")
            .fail_with
            .clone()
        {
            return Err(error);
        }
        let raw_thread_id = request
            .requested_thread_id
            .or(request.client_action_id)
            .ok_or_else(invalid_request)?;
        let thread_id = ThreadId::new(raw_thread_id).map_err(|_| invalid_request())?;
        Ok(RebornCreateThreadResponse {
            thread: thread_record(&caller, thread_id),
        })
    }

    async fn submit_turn(
        &self,
        caller: ProductSurfaceCaller,
        request: ProductSubmitTurnRequest,
    ) -> Result<RebornSubmitTurnResponse, ProductSurfaceError> {
        if let Some(error) = self
            .state
            .lock()
            .expect("surface state lock")
            .fail_with
            .clone()
        {
            return Err(error);
        }
        let thread_id = request
            .thread_id
            .as_deref()
            .ok_or_else(invalid_request)
            .and_then(|value| ThreadId::new(value).map_err(|_| invalid_request()))?;
        let _decoded_attachments = request.decode_attachments()?;
        let event_id = request
            .client_action_id
            .clone()
            .ok_or_else(invalid_request)?;
        let rejection_param = rejection_param_for_content(request.content.as_deref());
        let mut state = self.state.lock().expect("surface state lock");
        let outcome = state
            .programmed
            .remove(&event_id)
            .or_else(|| state.fixed_outcome.clone())
            .unwrap_or_else(|| default_ack(&event_id));
        state.submitted.push(RecordedProductSurfaceSubmit {
            request: request.clone(),
        });
        let response = reborn_submit_from_ack(thread_id.clone(), outcome, rejection_param);
        if matches!(response, Ok(RebornSubmitTurnResponse::Submitted { .. })) {
            state.read_inputs.push(ProjectionReadRequest {
                actor: caller.actor(),
                scope: caller.turn_scope(thread_id),
                after_cursor: None,
                limit: None,
            });
        }
        drop(state);
        response
    }

    async fn cancel_run(
        &self,
        caller: ProductSurfaceCaller,
        request: ProductCancelRunRequest,
    ) -> Result<RebornCancelRunResponse, ProductSurfaceError> {
        if let Some(error) = self
            .state
            .lock()
            .expect("surface state lock")
            .fail_with
            .clone()
        {
            return Err(error);
        }
        self.state
            .lock()
            .expect("surface state lock")
            .cancelled
            .push(request.clone());
        let run_id = request
            .run_id
            .as_deref()
            .and_then(|value| TurnRunId::parse(value).ok())
            .unwrap_or_default();
        let _ = caller;
        Ok(RebornCancelRunResponse {
            run_id,
            status: TurnStatus::CancelRequested,
            event_cursor: EventCursor(0),
            already_terminal: false,
        })
    }

    async fn stream_events(
        &self,
        _caller: ProductSurfaceCaller,
        request: RebornStreamEventsRequest,
    ) -> Result<RebornStreamEventsResponse, ProductSurfaceError> {
        self.state
            .lock()
            .expect("surface state lock")
            .stream_events
            .push(request);
        Ok(RebornStreamEventsResponse { events: Vec::new() })
    }
}

#[async_trait]
impl ProductSurface for FakeProductSurface {
    async fn invoke(
        &self,
        caller: ProductSurfaceCaller,
        request: ironclaw_host_api::ProductSurfaceInvokeRequest,
    ) -> Result<ironclaw_host_api::ProductSurfaceInvokeResponse, ProductSurfaceError> {
        let output = if request.operation_id.as_str() == CREATE_THREAD_COMMAND.id {
            let input = serde_json::from_value::<ProductCreateThreadRequest>(request.input)
                .map_err(ProductSurfaceError::internal_from)?;
            serde_json::to_value(self.create_thread(caller, input).await?)
                .map_err(ProductSurfaceError::internal_from)?
        } else if request.operation_id.as_str() == SUBMIT_TURN_COMMAND.id {
            let input = serde_json::from_value::<ProductSubmitTurnRequest>(request.input)
                .map_err(ProductSurfaceError::internal_from)?;
            serde_json::to_value(self.submit_turn(caller, input).await?)
                .map_err(ProductSurfaceError::internal_from)?
        } else if request.operation_id.as_str() == CANCEL_RUN_COMMAND.id {
            let input = serde_json::from_value::<ProductCancelRunRequest>(request.input)
                .map_err(ProductSurfaceError::internal_from)?;
            serde_json::to_value(self.cancel_run(caller, input).await?)
                .map_err(ProductSurfaceError::internal_from)?
        } else {
            return Err(invalid_request());
        };
        Ok(ironclaw_host_api::ProductSurfaceInvokeResponse { output })
    }

    async fn query(
        &self,
        _caller: ProductSurfaceCaller,
        _request: ironclaw_host_api::ProductSurfaceQueryRequest,
    ) -> Result<ironclaw_host_api::ProductSurfaceQueryPage, ProductSurfaceError> {
        Err(invalid_request())
    }

    async fn stream_events(
        &self,
        caller: ProductSurfaceCaller,
        request: ironclaw_host_api::ProductSurfaceStreamRequest,
    ) -> Result<ironclaw_host_api::ProductSurfaceStreamResponse, ProductSurfaceError> {
        let thread_id = request.stream_id.ok_or_else(invalid_request)?;
        let after_cursor = request
            .after_cursor
            .map(ironclaw_product::ProjectionCursor::new)
            .transpose()
            .map_err(|_| invalid_request())?;
        let response = self
            .stream_events(
                caller,
                RebornStreamEventsRequest {
                    thread_id,
                    after_cursor,
                },
            )
            .await?;
        let events = response
            .events
            .into_iter()
            .map(serde_json::to_value)
            .collect::<Result<Vec<_>, _>>()
            .map_err(ProductSurfaceError::internal_from)?;
        Ok(ironclaw_host_api::ProductSurfaceStreamResponse {
            events,
            next_cursor: None,
        })
    }
}

fn default_ack(event_id: &str) -> ProductInboundAck {
    ProductInboundAck::Accepted {
        accepted_message_ref: AcceptedMessageRef::new(format!("msg:{event_id}"))
            .expect("accepted message ref"),
        submitted_run_id: TurnRunId::new(),
    }
}

fn reborn_submit_from_ack(
    thread_id: ThreadId,
    mut ack: ProductInboundAck,
    rejection_param: &'static str,
) -> Result<RebornSubmitTurnResponse, ProductSurfaceError> {
    loop {
        match ack {
            ProductInboundAck::Accepted {
                accepted_message_ref,
                submitted_run_id,
            } => {
                return Ok(RebornSubmitTurnResponse::Submitted {
                    thread_id,
                    accepted_message_ref,
                    turn_id: "turn-openai-test".to_string(),
                    run_id: submitted_run_id,
                    status: TurnStatus::Queued,
                    resolved_run_profile_id: "test-profile".to_string(),
                    resolved_run_profile_version: 1,
                    event_cursor: EventCursor(0),
                });
            }
            ProductInboundAck::Duplicate { prior } => ack = *prior,
            ProductInboundAck::DeferredBusy {
                accepted_message_ref,
                active_run_id,
            } => {
                return Ok(RebornSubmitTurnResponse::RejectedBusy {
                    thread_id,
                    accepted_message_ref,
                    active_run_id: Some(active_run_id),
                    status: None,
                    event_cursor: None,
                    notice: "busy".to_string(),
                });
            }
            ProductInboundAck::RejectedBusy {
                accepted_message_ref,
                active_run_id,
            } => {
                return Ok(RebornSubmitTurnResponse::RejectedBusy {
                    thread_id,
                    accepted_message_ref,
                    active_run_id,
                    status: None,
                    event_cursor: None,
                    notice: "busy".to_string(),
                });
            }
            ProductInboundAck::Rejected(rejection) => {
                return Err(service_error_from_rejection(&rejection, rejection_param));
            }
            ProductInboundAck::CommandResult { .. } | ProductInboundAck::NoOp => {
                return Err(internal_error());
            }
        }
    }
}

fn thread_record(caller: &ProductSurfaceCaller, thread_id: ThreadId) -> SessionThreadRecord {
    SessionThreadRecord {
        scope: ThreadScope {
            tenant_id: caller.tenant_id.clone(),
            agent_id: caller
                .agent_id
                .clone()
                .unwrap_or_else(|| ironclaw_host_api::AgentId::new("agent-a").expect("agent")),
            project_id: caller.project_id.clone(),
            owner_user_id: Some(caller.user_id.clone()),
            mission_id: None,
        },
        thread_id,
        created_by_actor_id: caller.user_id.as_str().to_string(),
        title: None,
        metadata_json: None,
        goal: None,
        created_at: None,
        updated_at: None,
    }
}

pub(crate) fn service_unavailable() -> ProductSurfaceError {
    ProductSurfaceError {
        code: ProductSurfaceErrorCode::Unavailable,
        kind: ProductSurfaceErrorKind::ServiceUnavailable,
        status_code: 503,
        retryable: true,
        field: None,
        validation_code: None,
    }
}

pub(crate) fn rate_limited() -> ProductSurfaceError {
    ProductSurfaceError {
        code: ProductSurfaceErrorCode::RateLimited,
        kind: ProductSurfaceErrorKind::Busy,
        status_code: 429,
        retryable: false,
        field: None,
        validation_code: None,
    }
}

pub(crate) fn internal_error() -> ProductSurfaceError {
    ProductSurfaceError {
        code: ProductSurfaceErrorCode::Internal,
        kind: ProductSurfaceErrorKind::Internal,
        status_code: 500,
        retryable: false,
        field: None,
        validation_code: None,
    }
}

fn invalid_request() -> ProductSurfaceError {
    ProductSurfaceError {
        code: ProductSurfaceErrorCode::InvalidRequest,
        kind: ProductSurfaceErrorKind::Validation,
        status_code: 400,
        retryable: false,
        field: None,
        validation_code: None,
    }
}

fn rejection_param_for_content(content: Option<&str>) -> &'static str {
    if content.is_some_and(|content| content.contains("openai_compat.responses_input.v1")) {
        "input"
    } else {
        "messages"
    }
}

fn service_error_from_rejection(
    rejection: &ProductRejection,
    param: &'static str,
) -> ProductSurfaceError {
    match rejection.kind {
        ProductRejectionKind::BindingRequired => ProductSurfaceError {
            code: ProductSurfaceErrorCode::NotFound,
            kind: ProductSurfaceErrorKind::NotFound,
            status_code: 404,
            retryable: false,
            field: Some(param.to_string()),
            validation_code: None,
        },
        ProductRejectionKind::AccessDenied | ProductRejectionKind::PolicyDenied => {
            ProductSurfaceError {
                code: ProductSurfaceErrorCode::Forbidden,
                kind: ProductSurfaceErrorKind::ParticipantDenied,
                status_code: 403,
                retryable: false,
                field: None,
                validation_code: None,
            }
        }
        ProductRejectionKind::UnknownInstallation => service_unavailable(),
        ProductRejectionKind::InvalidRequest => ProductSurfaceError {
            code: ProductSurfaceErrorCode::InvalidRequest,
            kind: ProductSurfaceErrorKind::Validation,
            status_code: 400,
            retryable: false,
            field: Some(param.to_string()),
            validation_code: None,
        },
        ProductRejectionKind::AmbiguousResolution | ProductRejectionKind::StaleGate => {
            ProductSurfaceError {
                code: ProductSurfaceErrorCode::Conflict,
                kind: ProductSurfaceErrorKind::Conflict,
                status_code: 409,
                retryable: false,
                field: None,
                validation_code: None,
            }
        }
    }
}
