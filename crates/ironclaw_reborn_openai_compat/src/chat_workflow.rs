//! ProductSurface-backed Chat Completions route service.
//!
//! This module translates OpenAI-compatible Chat requests into ProductSurface
//! thread/turn requests. Non-streaming requests wait on a projection waiter;
//! streaming requests consume a composition-supplied ProductSurface event drain
//! and emit OpenAI-compatible SSE through the route-owned streaming translator.

use std::sync::Arc;
use std::time::Duration;

use crate::ack_helpers::{internal_refs_from_ack, product_ack_from_reborn_submit};
use crate::content_parts::{
    DecodedInlineImage, content_value_to_text_and_images, image_mime_extension,
    sanitize_product_text_fragment,
};
use crate::descriptors::MAX_CHAT_BODY_BYTES;
use crate::error::product_rejection_to_openai_error;
use crate::projection_helpers::{
    ensure_projection_read_matches_caller, ensure_projection_subscription_matches_caller,
};
use crate::{
    OpenAiChatChoice, OpenAiChatCompletionId, OpenAiChatCompletionRequest,
    OpenAiChatCompletionResponse, OpenAiChatFinishReason, OpenAiChatMessage, OpenAiChatMessageRole,
    OpenAiChatProjectionStreamRequest, OpenAiChatTool, OpenAiChatToolCall, OpenAiCompatActorScope,
    OpenAiCompatBindInternalRefs, OpenAiCompatHttpError, OpenAiCompatIdempotencyKey,
    OpenAiCompatInternalRefs, OpenAiCompatProjectionStreamer, OpenAiCompatPublicId,
    OpenAiCompatRecordAcceptedAck, OpenAiCompatRefReservation, OpenAiCompatRefReservationOutcome,
    OpenAiCompatRefStore, OpenAiCompatRequestFingerprint, OpenAiCompatResourceMapping,
    OpenAiCompatRouteSurface, OpenAiUsage,
};
use async_trait::async_trait;
use axum::Json;
use axum::response::{IntoResponse, Response};
use base64::Engine as _;
use ironclaw_attachments::InboundAttachment;
use ironclaw_host_api::{ActivityId, ThreadId};
use ironclaw_product::{
    BoundProductSurface, CREATE_THREAD_COMMAND, ProductCreateThreadRequest,
    ProductInboundAttachment, ProductSubmitTurnRequest, ProductSurface, ProductSurfaceCaller,
    ProductSurfaceCallerExt, SUBMIT_TURN_COMMAND,
};
use ironclaw_product::{
    ProductInboundAck, ProductRejection, ProductTriggerReason, ProjectionReadRequest,
    ProjectionSubscriptionRequest, ProtocolAuthEvidence, UserMessagePayload,
};
use uuid::Uuid;

const DEFAULT_CHAT_WAIT_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_BIND_INTERNAL_REFS_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_CHAT_COMPLETION_MESSAGES: usize = 1_000;
pub const OPENAI_COMPAT_CONVERSATION_PREFIX: &str = "chat_completion";

fn openai_product_activity_id(surface: &str, operation_id: &str, public_id: &str) -> ActivityId {
    let mut seed = Vec::new();
    for segment in ["openai-compat", surface, operation_id, public_id] {
        seed.extend_from_slice(&(segment.len() as u64).to_be_bytes());
        seed.extend_from_slice(segment.as_bytes());
    }
    ActivityId::from_uuid(Uuid::new_v5(&Uuid::NAMESPACE_OID, &seed))
}

#[derive(Debug, Clone)]
pub struct OpenAiCompatAuthenticatedCaller {
    scope: OpenAiCompatActorScope,
    auth_evidence: ProtocolAuthEvidence,
}

impl OpenAiCompatAuthenticatedCaller {
    pub fn new(
        scope: OpenAiCompatActorScope,
        auth_evidence: ProtocolAuthEvidence,
    ) -> Result<Self, OpenAiCompatHttpError> {
        let Some(claim) = auth_evidence.claim() else {
            return Err(OpenAiCompatHttpError::from_kind(
                401,
                false,
                crate::OpenAiCompatErrorKind::Authentication,
                None,
            ));
        };
        if claim.subject() != scope.user_id().as_str() {
            return Err(OpenAiCompatHttpError::from_kind(
                403,
                false,
                crate::OpenAiCompatErrorKind::PermissionDenied,
                None,
            ));
        }
        if claim.tenant_id() != Some(scope.tenant_id()) {
            return Err(OpenAiCompatHttpError::from_kind(
                403,
                false,
                crate::OpenAiCompatErrorKind::PermissionDenied,
                None,
            ));
        }
        Ok(Self {
            scope,
            auth_evidence,
        })
    }

    pub fn scope(&self) -> &OpenAiCompatActorScope {
        &self.scope
    }

    pub fn auth_evidence(&self) -> &ProtocolAuthEvidence {
        &self.auth_evidence
    }

    pub(crate) fn product_surface_caller(&self) -> ProductSurfaceCaller {
        ProductSurfaceCaller::new(
            self.scope.tenant_id().clone(),
            self.scope.user_id().clone(),
            self.scope.agent_id().cloned(),
            self.scope.project_id().cloned(),
        )
    }
}

#[derive(Clone)]
pub struct OpenAiChatCompletionsWorkflow {
    product_surface: Arc<dyn ProductSurface>,
    ref_store: Arc<dyn OpenAiCompatRefStore>,
    projection_reader: Arc<dyn OpenAiChatCompletionProjectionReader>,
    /// Wired by host composition when OpenAI-compatible streaming is enabled.
    /// When `None`, `stream: true` requests fail closed.
    /// arch-exempt: optional Arc, streaming is a staged #4446 capability layered
    /// onto the non-streaming #4444 workflow.
    projection_streamer: Option<Arc<dyn OpenAiCompatProjectionStreamer>>,
    wait_timeout: Duration,
}

impl OpenAiChatCompletionsWorkflow {
    pub fn new(
        product_surface: Arc<dyn ProductSurface>,
        ref_store: Arc<dyn OpenAiCompatRefStore>,
        projection_reader: Arc<dyn OpenAiChatCompletionProjectionReader>,
    ) -> Self {
        Self {
            product_surface,
            ref_store,
            projection_reader,
            projection_streamer: None,
            wait_timeout: DEFAULT_CHAT_WAIT_TIMEOUT,
        }
    }

    pub fn with_wait_timeout(mut self, wait_timeout: Duration) -> Self {
        self.wait_timeout = wait_timeout;
        self
    }

    pub fn with_projection_streamer(
        mut self,
        projection_streamer: Arc<dyn OpenAiCompatProjectionStreamer>,
    ) -> Self {
        self.projection_streamer = Some(projection_streamer);
        self
    }

    pub async fn complete_chat(
        &self,
        caller: OpenAiCompatAuthenticatedCaller,
        raw_body: &[u8],
        idempotency_key: Option<OpenAiCompatIdempotencyKey>,
    ) -> Result<OpenAiChatCompletionResponse, OpenAiCompatHttpError> {
        let request = parse_chat_request(raw_body)?;
        self.complete_chat_request(caller, request, raw_body, idempotency_key)
            .await
    }

    pub(crate) async fn handle_chat_request(
        &self,
        caller: OpenAiCompatAuthenticatedCaller,
        request: OpenAiChatCompletionRequest,
        raw_body: &[u8],
        idempotency_key: Option<OpenAiCompatIdempotencyKey>,
    ) -> Result<Response, OpenAiCompatHttpError> {
        if request.stream.unwrap_or(false) {
            return self
                .stream_chat_request(caller, request, raw_body, idempotency_key)
                .await;
        }
        self.complete_chat_request(caller, request, raw_body, idempotency_key)
            .await
            .map(|response| Json(response).into_response())
    }

    async fn complete_chat_request(
        &self,
        caller: OpenAiCompatAuthenticatedCaller,
        request: OpenAiChatCompletionRequest,
        raw_body: &[u8],
        idempotency_key: Option<OpenAiCompatIdempotencyKey>,
    ) -> Result<OpenAiChatCompletionResponse, OpenAiCompatHttpError> {
        if request.stream.unwrap_or(false) {
            return Err(OpenAiCompatHttpError::invalid_request(Some(
                "stream".to_string(),
            )));
        }

        let (user_message_payload, attachments) =
            chat_user_message_and_attachments(&request, true)?;
        let model_only_tools = OpenAiChatModelOnlyTools::from_request(&request);

        let request_fingerprint = OpenAiCompatRequestFingerprint::from_body_bytes(raw_body);
        let reservation = self
            .ref_store
            .reserve(OpenAiCompatRefReservation::new(
                caller.scope().clone(),
                OpenAiCompatRouteSurface::ChatCompletions,
                request_fingerprint,
                idempotency_key,
            ))
            .await?;
        let (public_id, accepted_ack, created_at) = match reservation {
            OpenAiCompatRefReservationOutcome::Created(mapping) => {
                let created_at = mapping.created_at;
                let OpenAiCompatPublicId::ChatCompletion(public_id) = mapping.public_id else {
                    return Err(OpenAiCompatHttpError::internal());
                };
                let accepted_ack = self
                    .submit_chat_and_record_ack(
                        &caller,
                        &public_id,
                        user_message_payload,
                        attachments,
                    )
                    .await?;
                (public_id, accepted_ack, created_at)
            }
            OpenAiCompatRefReservationOutcome::Replayed(mapping) => {
                let created_at = mapping.created_at;
                let OpenAiCompatPublicId::ChatCompletion(public_id) = mapping.public_id else {
                    return Err(OpenAiCompatHttpError::internal());
                };
                let accepted_ack = match mapping.accepted_ack {
                    Some(accepted_ack) => accepted_ack,
                    None => {
                        self.submit_chat_and_record_ack(
                            &caller,
                            &public_id,
                            user_message_payload,
                            attachments,
                        )
                        .await?
                    }
                };
                (public_id, accepted_ack, created_at)
            }
            OpenAiCompatRefReservationOutcome::Conflict(_) => {
                return Err(OpenAiCompatHttpError::conflict(Some(
                    "idempotency_key".to_string(),
                )));
            }
        };
        let projection_read = self.chat_projection_read(&caller, &public_id)?;
        ensure_projection_read_matches_caller(&caller, &projection_read)?;
        let projection_request = OpenAiChatCompletionProjectionRequest {
            public_id: public_id.clone(),
            actor_scope: caller.scope().clone(),
            accepted_ack,
            projection_read,
            requested_model: request.model.clone(),
            model_only_tools,
        };

        let wait_result = tokio::time::timeout(
            self.wait_timeout,
            self.projection_reader
                .read_chat_completion_projection(projection_request),
        )
        .await
        .map_err(|_| {
            OpenAiCompatHttpError::from_kind(
                503,
                true,
                crate::OpenAiCompatErrorKind::ServiceUnavailable,
                None,
            )
        })??;

        if let Some(internal_refs) = wait_result.internal_refs {
            match tokio::time::timeout(
                DEFAULT_BIND_INTERNAL_REFS_TIMEOUT,
                self.ref_store
                    .bind_internal_refs(OpenAiCompatBindInternalRefs::new(
                        caller.scope().clone(),
                        OpenAiCompatPublicId::ChatCompletion(public_id.clone()),
                        internal_refs,
                    )),
            )
            .await
            {
                Ok(result) => {
                    let _ = result?;
                }
                Err(_) => tracing::warn!(
                    public_id = public_id.as_str(),
                    "bind_internal_refs timed out; continuing without binding"
                ),
            }
        }

        Ok(OpenAiChatCompletionResponse {
            id: public_id,
            object: "chat.completion".to_string(),
            created: created_at,
            model: wait_result.effective_model.unwrap_or(request.model),
            choices: vec![OpenAiChatChoice {
                index: 0,
                message: OpenAiChatMessage {
                    role: OpenAiChatMessageRole::Assistant,
                    content: wait_result.assistant_content.map(serde_json::Value::String),
                    name: None,
                    tool_call_id: None,
                    tool_calls: wait_result.tool_calls,
                },
                finish_reason: Some(wait_result.finish_reason),
            }],
            usage: wait_result.usage,
        })
    }

    pub async fn stream_chat(
        &self,
        caller: OpenAiCompatAuthenticatedCaller,
        raw_body: &[u8],
        idempotency_key: Option<OpenAiCompatIdempotencyKey>,
    ) -> Result<Response, OpenAiCompatHttpError> {
        let request = parse_chat_request(raw_body)?;
        self.stream_chat_request(caller, request, raw_body, idempotency_key)
            .await
    }

    async fn stream_chat_request(
        &self,
        caller: OpenAiCompatAuthenticatedCaller,
        request: OpenAiChatCompletionRequest,
        raw_body: &[u8],
        idempotency_key: Option<OpenAiCompatIdempotencyKey>,
    ) -> Result<Response, OpenAiCompatHttpError> {
        let projection_streamer = self
            .projection_streamer
            .clone()
            .ok_or_else(OpenAiCompatHttpError::not_wired)?;
        if !request.stream.unwrap_or(false) {
            return Err(OpenAiCompatHttpError::invalid_request(Some(
                "stream".to_string(),
            )));
        }

        let (user_message_payload, attachments) =
            chat_user_message_and_attachments(&request, true)?;
        let model_only_tools = OpenAiChatModelOnlyTools::from_request(&request);
        let request_fingerprint = OpenAiCompatRequestFingerprint::from_body_bytes(raw_body);
        let reservation = self
            .ref_store
            .reserve(OpenAiCompatRefReservation::new(
                caller.scope().clone(),
                OpenAiCompatRouteSurface::ChatCompletions,
                request_fingerprint,
                idempotency_key,
            ))
            .await?;
        let (mapping, accepted_ack) = match reservation {
            OpenAiCompatRefReservationOutcome::Created(mapping) => {
                let OpenAiCompatPublicId::ChatCompletion(public_id) = &mapping.public_id else {
                    return Err(OpenAiCompatHttpError::internal());
                };
                let accepted_ack = self
                    .submit_chat_and_record_ack(
                        &caller,
                        public_id,
                        user_message_payload,
                        attachments,
                    )
                    .await?;
                (mapping, accepted_ack)
            }
            OpenAiCompatRefReservationOutcome::Replayed(mapping) => {
                let OpenAiCompatPublicId::ChatCompletion(public_id) = &mapping.public_id else {
                    return Err(OpenAiCompatHttpError::internal());
                };
                let accepted_ack = match mapping.accepted_ack.clone() {
                    Some(accepted_ack) => accepted_ack,
                    None => {
                        self.submit_chat_and_record_ack(
                            &caller,
                            public_id,
                            user_message_payload,
                            attachments,
                        )
                        .await?
                    }
                };
                (mapping, accepted_ack)
            }
            OpenAiCompatRefReservationOutcome::Conflict(_) => {
                return Err(OpenAiCompatHttpError::conflict(Some(
                    "idempotency_key".to_string(),
                )));
            }
        };
        let OpenAiCompatPublicId::ChatCompletion(public_id) = mapping.public_id.clone() else {
            return Err(OpenAiCompatHttpError::internal());
        };
        let mapping = self
            .bind_internal_refs_from_ack(caller.scope().clone(), public_id.clone(), &accepted_ack)
            .await?
            .unwrap_or(mapping);
        let projection_subscription = self.chat_projection_subscription(&caller, &public_id)?;
        ensure_projection_subscription_matches_caller(&caller, &projection_subscription)?;

        Ok(crate::streaming::chat_sse_response(
            projection_streamer,
            OpenAiChatProjectionStreamRequest {
                public_id,
                actor_scope: caller.scope().clone(),
                accepted_ack,
                requested_model: request.model,
                model_only_tools,
                projection_subscription,
                mapping,
                wait_timeout: self.wait_timeout,
                after_cursor: None,
            },
        ))
    }

    async fn submit_chat_and_record_ack(
        &self,
        caller: &OpenAiCompatAuthenticatedCaller,
        public_id: &OpenAiChatCompletionId,
        user_message_payload: UserMessagePayload,
        attachments: Vec<InboundAttachment>,
    ) -> Result<ProductInboundAck, OpenAiCompatHttpError> {
        self.ensure_chat_thread(caller, public_id).await?;
        let surface = BoundProductSurface::new(
            Arc::clone(&self.product_surface),
            caller.product_surface_caller(),
        );
        let ack = product_ack_from_reborn_submit(
            SUBMIT_TURN_COMMAND
                .invoke_on(
                    &surface,
                    chat_surface_submit_request(public_id, user_message_payload, attachments),
                    openai_product_activity_id("chat", SUBMIT_TURN_COMMAND.id, public_id.as_str()),
                )
                .await?,
        );
        let accepted_ack = accepted_ack_from_ack(ack)?;
        self.ref_store
            .record_accepted_ack(OpenAiCompatRecordAcceptedAck::new(
                caller.scope().clone(),
                OpenAiCompatPublicId::ChatCompletion(public_id.clone()),
                accepted_ack.clone(),
            ))
            .await?
            .ok_or_else(|| OpenAiCompatHttpError::not_found(None))?;
        Ok(accepted_ack)
    }

    async fn bind_internal_refs_from_ack(
        &self,
        owner: OpenAiCompatActorScope,
        public_id: OpenAiChatCompletionId,
        accepted_ack: &ProductInboundAck,
    ) -> Result<Option<OpenAiCompatResourceMapping>, OpenAiCompatHttpError> {
        let internal_refs = internal_refs_from_ack(accepted_ack)?;
        match tokio::time::timeout(
            DEFAULT_BIND_INTERNAL_REFS_TIMEOUT,
            self.ref_store
                .bind_internal_refs(OpenAiCompatBindInternalRefs::new(
                    owner,
                    OpenAiCompatPublicId::ChatCompletion(public_id.clone()),
                    internal_refs,
                )),
        )
        .await
        {
            Ok(result) => result.map_err(Into::into),
            Err(_) => {
                tracing::warn!(
                    public_id = public_id.as_str(),
                    "bind_internal_refs timed out; continuing without binding"
                );
                Ok(None)
            }
        }
    }

    async fn ensure_chat_thread(
        &self,
        caller: &OpenAiCompatAuthenticatedCaller,
        public_id: &OpenAiChatCompletionId,
    ) -> Result<(), OpenAiCompatHttpError> {
        let surface = BoundProductSurface::new(
            Arc::clone(&self.product_surface),
            caller.product_surface_caller(),
        );
        CREATE_THREAD_COMMAND
            .invoke_on(
                &surface,
                ProductCreateThreadRequest {
                    client_action_id: Some(public_id.as_str().to_string()),
                    requested_thread_id: Some(public_id.as_str().to_string()),
                    project_id: None,
                },
                openai_product_activity_id("chat", CREATE_THREAD_COMMAND.id, public_id.as_str()),
            )
            .await?;
        Ok(())
    }

    fn chat_projection_read(
        &self,
        caller: &OpenAiCompatAuthenticatedCaller,
        public_id: &OpenAiChatCompletionId,
    ) -> Result<ProjectionReadRequest, OpenAiCompatHttpError> {
        let surface_caller = caller.product_surface_caller();
        let thread_id = ThreadId::new(public_id.as_str().to_string())
            .map_err(|_| OpenAiCompatHttpError::internal())?;
        Ok(ProjectionReadRequest {
            actor: surface_caller.actor(),
            scope: surface_caller.turn_scope(thread_id),
            after_cursor: None,
            limit: None,
        })
    }

    fn chat_projection_subscription(
        &self,
        caller: &OpenAiCompatAuthenticatedCaller,
        public_id: &OpenAiChatCompletionId,
    ) -> Result<ProjectionSubscriptionRequest, OpenAiCompatHttpError> {
        let surface_caller = caller.product_surface_caller();
        let thread_id = ThreadId::new(public_id.as_str().to_string())
            .map_err(|_| OpenAiCompatHttpError::internal())?;
        Ok(ProjectionSubscriptionRequest {
            actor: surface_caller.actor(),
            scope: surface_caller.turn_scope(thread_id),
            after_cursor: None,
        })
    }
}

fn chat_surface_submit_request(
    public_id: &OpenAiChatCompletionId,
    user_message_payload: UserMessagePayload,
    attachments: Vec<InboundAttachment>,
) -> ProductSubmitTurnRequest {
    ProductSubmitTurnRequest {
        client_action_id: Some(public_id.as_str().to_string()),
        thread_id: Some(public_id.as_str().to_string()),
        content: Some(user_message_payload.text),
        attachments: product_attachments(attachments),
        model: user_message_payload.requested_model,
    }
}

fn product_attachments(attachments: Vec<InboundAttachment>) -> Vec<ProductInboundAttachment> {
    attachments
        .into_iter()
        .map(|attachment| ProductInboundAttachment {
            mime_type: attachment.mime_type,
            filename: attachment.filename,
            data_base64: base64::engine::general_purpose::STANDARD.encode(attachment.bytes),
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq)]
pub struct OpenAiChatCompletionProjectionRequest {
    pub public_id: OpenAiChatCompletionId,
    pub actor_scope: OpenAiCompatActorScope,
    pub accepted_ack: ProductInboundAck,
    pub projection_read: ProjectionReadRequest,
    /// Public model string requested by the OpenAI-compatible client.
    ///
    /// This is a composition/policy hint for the projection reader and must not
    /// be mixed into the user transcript text by this route crate.
    pub requested_model: String,
    /// Client-supplied OpenAI tool declarations for model planning only.
    ///
    /// These declarations must not execute as Reborn capabilities from this
    /// route crate. Composition may translate them into provider model hints.
    pub model_only_tools: Option<OpenAiChatModelOnlyTools>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OpenAiChatModelOnlyTools {
    pub tools: Vec<OpenAiChatTool>,
    pub tool_choice: Option<serde_json::Value>,
}

impl OpenAiChatModelOnlyTools {
    fn from_request(request: &OpenAiChatCompletionRequest) -> Option<Self> {
        let tools = request.tools.clone().unwrap_or_default();
        let tool_choice = request.tool_choice.clone();
        if tools.is_empty() && tool_choice.is_none() {
            return None;
        }
        Some(Self { tools, tool_choice })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct OpenAiChatCompletionProjection {
    pub assistant_content: Option<String>,
    pub tool_calls: Option<Vec<OpenAiChatToolCall>>,
    pub finish_reason: OpenAiChatFinishReason,
    pub usage: Option<OpenAiUsage>,
    pub effective_model: Option<String>,
    pub internal_refs: Option<OpenAiCompatInternalRefs>,
}

impl OpenAiChatCompletionProjection {
    pub fn text(content: impl Into<String>) -> Self {
        Self {
            assistant_content: Some(content.into()),
            tool_calls: None,
            finish_reason: OpenAiChatFinishReason::Stop,
            usage: None,
            effective_model: None,
            internal_refs: None,
        }
    }
}

#[async_trait]
pub trait OpenAiChatCompletionProjectionReader: Send + Sync {
    async fn read_chat_completion_projection(
        &self,
        request: OpenAiChatCompletionProjectionRequest,
    ) -> Result<OpenAiChatCompletionProjection, OpenAiCompatHttpError>;
}

fn accepted_ack_from_ack(
    mut ack: ProductInboundAck,
) -> Result<ProductInboundAck, OpenAiCompatHttpError> {
    loop {
        match ack {
            ProductInboundAck::Accepted { .. } => return Ok(ack),
            ProductInboundAck::Duplicate { prior } => ack = *prior,
            ProductInboundAck::DeferredBusy { .. } => {
                return Err(OpenAiCompatHttpError::from_kind(
                    429,
                    true,
                    crate::OpenAiCompatErrorKind::RateLimited,
                    None,
                ));
            }
            ProductInboundAck::RejectedBusy { .. } => {
                // terminal/settled, not retryable — client must issue a new request
                return Err(OpenAiCompatHttpError::from_kind(
                    429,
                    false,
                    crate::OpenAiCompatErrorKind::RateLimited,
                    None,
                ));
            }
            ProductInboundAck::Rejected(rejection) => return Err(error_from_rejection(rejection)),
            ProductInboundAck::CommandResult { .. } | ProductInboundAck::NoOp => {
                return Err(OpenAiCompatHttpError::internal());
            }
        }
    }
}

fn error_from_rejection(rejection: ProductRejection) -> OpenAiCompatHttpError {
    product_rejection_to_openai_error(&rejection, Some("messages"))
}

pub(crate) fn parse_chat_request(
    raw_body: &[u8],
) -> Result<OpenAiChatCompletionRequest, OpenAiCompatHttpError> {
    if raw_body.len() > MAX_CHAT_BODY_BYTES {
        return Err(OpenAiCompatHttpError::invalid_request(Some(
            "body".to_string(),
        )));
    }
    let request: OpenAiChatCompletionRequest = serde_json::from_slice(raw_body)
        .map_err(|_| OpenAiCompatHttpError::invalid_request(Some("body".to_string())))?;
    crate::model_validation::validate_model_name(&request.model)?;
    Ok(request)
}

fn chat_messages_to_product_text_and_images(
    request: &OpenAiChatCompletionRequest,
    enable_attachments: bool,
) -> Result<(String, Vec<DecodedInlineImage>), OpenAiCompatHttpError> {
    if request.messages.is_empty() {
        return Err(OpenAiCompatHttpError::invalid_request(Some(
            "messages".to_string(),
        )));
    }
    if request.messages.len() > MAX_CHAT_COMPLETION_MESSAGES {
        return Err(OpenAiCompatHttpError::invalid_request(Some(
            "messages".to_string(),
        )));
    }
    let mut rendered_messages = Vec::with_capacity(request.messages.len());
    let mut images = Vec::new();
    for message in &request.messages {
        let (content_text, mut message_images) =
            content_value_to_text_and_images(message.content.as_ref(), enable_attachments);
        images.append(&mut message_images);
        rendered_messages.push(serde_json::json!({
            "role": chat_role_label(&message.role),
            "content": content_text,
            "tool_call_id": message
                .tool_call_id
                .as_ref()
                .map(|value| sanitize_product_text_fragment(value)),
            "assistant_tool_call_count": message.tool_calls.as_ref().map(Vec::len),
        }));
    }
    let text = serde_json::to_string(&serde_json::json!({
        "format": "openai_compat.chat_messages.v1",
        "messages": rendered_messages,
    }))
    .map_err(|_| OpenAiCompatHttpError::internal())?;
    Ok((text, images))
}

fn chat_role_label(role: &OpenAiChatMessageRole) -> &'static str {
    match role {
        OpenAiChatMessageRole::Developer => "developer",
        OpenAiChatMessageRole::System => "system",
        OpenAiChatMessageRole::User => "user",
        OpenAiChatMessageRole::Assistant => "assistant",
        OpenAiChatMessageRole::Tool => "tool",
    }
}

fn chat_user_message_and_attachments(
    request: &OpenAiChatCompletionRequest,
    enable_attachments: bool,
) -> Result<(UserMessagePayload, Vec<InboundAttachment>), OpenAiCompatHttpError> {
    let (text, images) = chat_messages_to_product_text_and_images(request, enable_attachments)?;
    let attachments = images
        .into_iter()
        .enumerate()
        .map(|(index, image)| InboundAttachment {
            id: format!("openai-image-{index}"),
            filename: Some(format!(
                "image-{index}.{}",
                image_mime_extension(&image.mime_type)
            )),
            mime_type: image.mime_type,
            bytes: image.bytes,
        })
        .collect();
    let payload = UserMessagePayload::new(text, vec![], ProductTriggerReason::DirectChat)?
        .with_requested_model(ironclaw_common::model_selection::requested_model_hint(
            &request.model,
        ));
    // The builder attaches the model hint after `new`'s validation, so bound the
    // assembled payload before it is submitted.
    payload.validate()?;
    Ok((payload, attachments))
}

#[cfg(test)]
mod tests {
    use ironclaw_product::ProductInboundAck;
    use ironclaw_turns::{AcceptedMessageRef, TurnRunId};

    use super::accepted_ack_from_ack;

    #[test]
    fn deferred_busy_ack_is_retryable_429() {
        let ack = ProductInboundAck::DeferredBusy {
            accepted_message_ref: AcceptedMessageRef::new("msg:deferred-busy").expect("ref"),
            active_run_id: TurnRunId::new(),
        };
        let err = accepted_ack_from_ack(ack).unwrap_err();
        assert_eq!(err.status_code(), 429);
        assert!(err.retryable(), "DeferredBusy must be retryable");
    }

    #[test]
    fn rejected_busy_ack_is_non_retryable_429() {
        let ack = ProductInboundAck::RejectedBusy {
            accepted_message_ref: AcceptedMessageRef::new("msg:rejected-busy").expect("ref"),
            active_run_id: None,
        };
        let err = accepted_ack_from_ack(ack).unwrap_err();
        assert_eq!(err.status_code(), 429);
        assert!(
            !err.retryable(),
            "RejectedBusy is terminal — must not be retryable"
        );
    }
}
