//! ProductWorkflow-backed Chat Completions route service.
//!
//! This module translates OpenAI-compatible Chat requests into product inbound
//! user-message envelopes. Non-streaming requests wait on a projection waiter;
//! streaming requests consume a composition-supplied projection stream and emit
//! OpenAI-compatible SSE through the route-owned streaming translator.

use std::sync::Arc;
use std::time::Duration;

use crate::ack_helpers::internal_refs_from_ack;
use crate::content_parts::{
    DecodedInlineImage, content_value_to_text_and_images, image_mime_extension,
    sanitize_product_text_fragment,
};
use crate::descriptors::MAX_CHAT_BODY_BYTES;
use crate::error::product_rejection_to_openai_error;
use crate::identity::{
    OPENAI_COMPAT_ACTOR_KIND, OPENAI_COMPAT_ADAPTER_ID, OPENAI_COMPAT_INSTALLATION_ID,
};
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
use chrono::Utc;
use ironclaw_attachments::InboundAttachment;
use ironclaw_product_adapters::{
    AdapterInstallationId, ExternalActorRef, ExternalConversationRef, ExternalEventId,
    ParsedProductInbound, ProductAdapterError, ProductAdapterId, ProductInboundAck,
    ProductInboundEnvelope, ProductInboundPayload, ProductProjectionReadInput,
    ProductProjectionSubject, ProductProjectionSubscribeInput, ProductRejection,
    ProductTriggerReason, ProductWorkflow, ProjectionReadRequest, ProtocolAuthEvidence,
    TrustedInboundContext, UserMessagePayload,
};

/// Host-supplied native door for submitting a user message together with
/// host-staged inline attachment bytes (vision, #4644).
///
/// Defined in this route crate (mirroring [`OpenAiCompatProjectionStreamer`])
/// and implemented by host composition, because the route surface may only talk
/// to `ironclaw_product_adapters` traits — never the `ironclaw_product_workflow`
/// crate (enforced by `reborn_dependency_boundaries`). The decoded bytes are
/// landed at message acceptance by the host and never enter the bytes-free
/// product inbound envelope.
#[async_trait]
pub trait OpenAiCompatInboundAttachmentSubmit: Send + Sync {
    async fn submit_inbound_with_attachments(
        &self,
        envelope: ProductInboundEnvelope,
        attachments: Vec<InboundAttachment>,
    ) -> Result<ProductInboundAck, ProductAdapterError>;
}

const DEFAULT_CHAT_WAIT_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_BIND_INTERNAL_REFS_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_CHAT_COMPLETION_MESSAGES: usize = 1_000;
pub const OPENAI_COMPAT_CONVERSATION_PREFIX: &str = "chat_completion";

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
}

#[derive(Clone)]
pub struct OpenAiChatCompletionsWorkflow {
    product_workflow: Arc<dyn ProductWorkflow>,
    ref_store: Arc<dyn OpenAiCompatRefStore>,
    projection_reader: Arc<dyn OpenAiChatCompletionProjectionReader>,
    /// Wired by host composition when OpenAI-compatible streaming is enabled.
    /// When `None`, `stream: true` requests fail closed.
    /// arch-exempt: optional Arc, streaming is a staged #4446 capability layered
    /// onto the non-streaming #4444 workflow.
    projection_streamer: Option<Arc<dyn OpenAiCompatProjectionStreamer>>,
    /// Native door for submitting a user message with host-staged inline image
    /// bytes. Wired by host composition; when `None`, inline images fall back to
    /// the bytes-free submit path (and the model sees only the `[image omitted]`
    /// transcript marker).
    /// arch-exempt: optional Arc, vision is a staged #4644 capability layered
    /// onto the text-only chat workflow; a deployment may run without it.
    attachment_submit: Option<Arc<dyn OpenAiCompatInboundAttachmentSubmit>>,
    wait_timeout: Duration,
    adapter_id: ProductAdapterId,
    installation_id: AdapterInstallationId,
}

impl OpenAiChatCompletionsWorkflow {
    pub fn new(
        product_workflow: Arc<dyn ProductWorkflow>,
        ref_store: Arc<dyn OpenAiCompatRefStore>,
        projection_reader: Arc<dyn OpenAiChatCompletionProjectionReader>,
    ) -> Self {
        Self {
            product_workflow,
            ref_store,
            projection_reader,
            projection_streamer: None,
            attachment_submit: None,
            wait_timeout: DEFAULT_CHAT_WAIT_TIMEOUT,
            adapter_id: ProductAdapterId::new(OPENAI_COMPAT_ADAPTER_ID)
                .expect("OPENAI_COMPAT_ADAPTER_ID is valid"), // safety: hard-coded non-empty product adapter id literal.
            installation_id: AdapterInstallationId::new(OPENAI_COMPAT_INSTALLATION_ID)
                .expect("OPENAI_COMPAT_INSTALLATION_ID is valid"), // safety: hard-coded non-empty installation id literal.
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

    /// Wire the native door that lands inline image bytes into project storage
    /// before submission, enabling vision for `image_url` content parts.
    pub fn with_attachment_submit(
        mut self,
        attachment_submit: Arc<dyn OpenAiCompatInboundAttachmentSubmit>,
    ) -> Self {
        self.attachment_submit = Some(attachment_submit);
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
            chat_user_message_and_attachments(&request, self.attachment_submit.is_some())?;
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
        let projection_read = self
            .product_workflow
            .read_projection(self.chat_projection_read_input(&caller, &public_id)?)
            .await?;
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
            chat_user_message_and_attachments(&request, self.attachment_submit.is_some())?;
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
        let projection_subscription = self
            .product_workflow
            .subscribe_projection(self.chat_projection_subscribe_input(&caller, &public_id)?)
            .await?;
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
        let envelope = self.chat_product_envelope(caller, public_id, user_message_payload)?;
        let ack = match self.attachment_submit.as_ref() {
            // Inline images decoded from the request: land them via the native
            // door (bytes never enter the bytes-free product envelope).
            Some(attachment_submit) if !attachments.is_empty() => {
                attachment_submit
                    .submit_inbound_with_attachments(envelope, attachments)
                    .await?
            }
            // No images, or no native door wired. If images were decoded but no
            // door is wired (unwired/test path), they are not carried; the
            // transcript text already routed them through the bytes-free path.
            _ => {
                if !attachments.is_empty() {
                    tracing::debug!(
                        target = "ironclaw::reborn::openai_compat",
                        image_count = attachments.len(),
                        "inline images dropped: attachment-submit door not wired"
                    );
                }
                self.product_workflow.submit_inbound(envelope).await?
            }
        };
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

    fn chat_product_envelope(
        &self,
        caller: &OpenAiCompatAuthenticatedCaller,
        public_id: &OpenAiChatCompletionId,
        user_message_payload: UserMessagePayload,
    ) -> Result<ProductInboundEnvelope, OpenAiCompatHttpError> {
        let context = TrustedInboundContext::from_verified_evidence(
            self.adapter_id.clone(),
            self.installation_id.clone(),
            Utc::now(),
            caller.auth_evidence(),
        )?;
        let parsed = ParsedProductInbound::new(
            ExternalEventId::new(public_id.as_str())?,
            ExternalActorRef::new(
                OPENAI_COMPAT_ACTOR_KIND,
                caller.scope().user_id().as_str(),
                Option::<String>::None,
            )?,
            ExternalConversationRef::new(
                None,
                format!("{OPENAI_COMPAT_CONVERSATION_PREFIX}:{}", public_id.as_str()),
                None,
                None,
            )?,
            ProductInboundPayload::UserMessage(user_message_payload),
        )?;
        ProductInboundEnvelope::from_trusted_parse(context, parsed).map_err(Into::into)
    }

    fn chat_projection_read_input(
        &self,
        caller: &OpenAiCompatAuthenticatedCaller,
        public_id: &OpenAiChatCompletionId,
    ) -> Result<ProductProjectionReadInput, OpenAiCompatHttpError> {
        Ok(ProductProjectionReadInput::new(
            self.chat_projection_subject(caller, public_id)?,
            None,
            None,
            None,
        ))
    }

    fn chat_projection_subscribe_input(
        &self,
        caller: &OpenAiCompatAuthenticatedCaller,
        public_id: &OpenAiChatCompletionId,
    ) -> Result<ProductProjectionSubscribeInput, OpenAiCompatHttpError> {
        Ok(ProductProjectionSubscribeInput::new(
            self.chat_projection_subject(caller, public_id)?,
            None,
            None,
        ))
    }

    fn chat_projection_subject(
        &self,
        caller: &OpenAiCompatAuthenticatedCaller,
        public_id: &OpenAiChatCompletionId,
    ) -> Result<ProductProjectionSubject, OpenAiCompatHttpError> {
        let Some(auth_claim) = caller.auth_evidence().claim().cloned() else {
            return Err(OpenAiCompatHttpError::internal());
        };
        Ok(ProductProjectionSubject::AdapterExternalRefs {
            adapter_id: self.adapter_id.clone(),
            installation_id: self.installation_id.clone(),
            external_event_id: ExternalEventId::new(public_id.as_str())?,
            external_actor_ref: ExternalActorRef::new(
                OPENAI_COMPAT_ACTOR_KIND,
                caller.scope().user_id().as_str(),
                Option::<String>::None,
            )?,
            external_conversation_ref: ExternalConversationRef::new(
                None,
                format!("{OPENAI_COMPAT_CONVERSATION_PREFIX}:{}", public_id.as_str()),
                None,
                None,
            )?,
            auth_claim,
        })
    }
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
    serde_json::from_slice(raw_body)
        .map_err(|_| OpenAiCompatHttpError::invalid_request(Some("body".to_string())))
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
    let payload = UserMessagePayload::new(text, vec![], ProductTriggerReason::DirectChat)?;
    Ok((payload, attachments))
}

#[cfg(test)]
mod tests {
    use ironclaw_product_adapters::ProductInboundAck;
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
