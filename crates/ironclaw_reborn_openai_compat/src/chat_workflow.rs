//! ProductWorkflow-backed Chat Completions route service.
//!
//! This module is the first non-streaming OpenAI-compatible Chat slice. It
//! translates the HTTP DTO into a product inbound user-message envelope, routes
//! the mutating action through `ProductWorkflow`, and waits on a projection
//! waiter port supplied by host composition. It deliberately does not call v1
//! gateway handlers, LLM providers, `TurnCoordinator`, or projection internals.

use std::sync::Arc;
use std::time::Duration;

use crate::{
    OpenAiChatChoice, OpenAiChatCompletionId, OpenAiChatCompletionRequest,
    OpenAiChatCompletionResponse, OpenAiChatFinishReason, OpenAiChatMessage, OpenAiChatMessageRole,
    OpenAiChatTool, OpenAiChatToolCall, OpenAiCompatActorScope, OpenAiCompatBindInternalRefs,
    OpenAiCompatHttpError, OpenAiCompatIdempotencyKey, OpenAiCompatInternalRefs,
    OpenAiCompatPublicId, OpenAiCompatRecordAcceptedAck, OpenAiCompatRefReservation,
    OpenAiCompatRefReservationOutcome, OpenAiCompatRefStore, OpenAiCompatRequestFingerprint,
    OpenAiCompatRouteSurface, OpenAiUsage,
};
use async_trait::async_trait;
use chrono::Utc;
use ironclaw_product_adapters::{
    AdapterInstallationId, ExternalActorRef, ExternalConversationRef, ExternalEventId,
    ParsedProductInbound, ProductAdapterId, ProductInboundAck, ProductInboundEnvelope,
    ProductInboundPayload, ProductRejection, ProductRejectionKind, ProductTriggerReason,
    ProductWorkflow, ProductWorkflowRejectionKind, ProtocolAuthEvidence, TrustedInboundContext,
    UserMessagePayload,
};

const DEFAULT_CHAT_WAIT_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_BIND_INTERNAL_REFS_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_CHAT_COMPLETION_MESSAGES: usize = 1_000;
const OPENAI_COMPAT_ADAPTER_ID: &str = "openai_compat";
const OPENAI_COMPAT_INSTALLATION_ID: &str = "openai_compat_default";
const OPENAI_COMPAT_ACTOR_KIND: &str = "openai_compat_user";
const OPENAI_COMPAT_CONVERSATION_PREFIX: &str = "chat_completion";

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
                crate::OpenAiCompatErrorKind::Authentication,
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
    completion_waiter: Arc<dyn OpenAiChatCompletionWaiter>,
    wait_timeout: Duration,
    adapter_id: ProductAdapterId,
    installation_id: AdapterInstallationId,
}

impl OpenAiChatCompletionsWorkflow {
    pub fn new(
        product_workflow: Arc<dyn ProductWorkflow>,
        ref_store: Arc<dyn OpenAiCompatRefStore>,
        completion_waiter: Arc<dyn OpenAiChatCompletionWaiter>,
    ) -> Self {
        Self {
            product_workflow,
            ref_store,
            completion_waiter,
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

    pub async fn complete_chat(
        &self,
        caller: OpenAiCompatAuthenticatedCaller,
        raw_body: &[u8],
        idempotency_key: Option<OpenAiCompatIdempotencyKey>,
    ) -> Result<OpenAiChatCompletionResponse, OpenAiCompatHttpError> {
        let request = parse_chat_request(raw_body)?;
        if request.stream.unwrap_or(false) {
            return Err(OpenAiCompatHttpError::invalid_request(Some(
                "stream".to_string(),
            )));
        }

        let user_message_payload = chat_user_message_payload(&request)?;
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
                let envelope =
                    self.chat_product_envelope(&caller, &public_id, user_message_payload)?;
                let ack = self.product_workflow.submit_inbound(envelope).await?;
                let accepted_ack = accepted_ack_from_ack(ack)?;
                let _ = self
                    .ref_store
                    .record_accepted_ack(OpenAiCompatRecordAcceptedAck::new(
                        caller.scope().clone(),
                        OpenAiCompatPublicId::ChatCompletion(public_id.clone()),
                        accepted_ack.clone(),
                    ))
                    .await?;
                (public_id, accepted_ack, created_at)
            }
            OpenAiCompatRefReservationOutcome::Replayed(mapping) => {
                let created_at = mapping.created_at;
                let accepted_ack = mapping
                    .accepted_ack
                    .ok_or_else(OpenAiCompatHttpError::internal)?;
                let OpenAiCompatPublicId::ChatCompletion(public_id) = mapping.public_id else {
                    return Err(OpenAiCompatHttpError::internal());
                };
                (public_id, accepted_ack, created_at)
            }
            OpenAiCompatRefReservationOutcome::Conflict(_) => {
                return Err(OpenAiCompatHttpError::conflict(Some(
                    "idempotency_key".to_string(),
                )));
            }
        };
        let wait_request = OpenAiChatCompletionWaitRequest {
            public_id: public_id.clone(),
            actor_scope: caller.scope().clone(),
            accepted_ack,
            requested_model: request.model.clone(),
            model_only_tools,
        };

        let wait_result = tokio::time::timeout(
            self.wait_timeout,
            self.completion_waiter
                .wait_for_chat_completion(wait_request),
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
}

#[derive(Debug, Clone, PartialEq)]
pub struct OpenAiChatCompletionWaitRequest {
    pub public_id: OpenAiChatCompletionId,
    pub actor_scope: OpenAiCompatActorScope,
    pub accepted_ack: ProductInboundAck,
    /// Public model string requested by the OpenAI-compatible client.
    ///
    /// This is a composition/policy hint for the completion waiter and must not
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
pub trait OpenAiChatCompletionWaiter: Send + Sync {
    async fn wait_for_chat_completion(
        &self,
        request: OpenAiChatCompletionWaitRequest,
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
            ProductInboundAck::Rejected(rejection) => return Err(error_from_rejection(rejection)),
            ProductInboundAck::CommandResult { .. } | ProductInboundAck::NoOp => {
                return Err(OpenAiCompatHttpError::internal());
            }
        }
    }
}

fn error_from_rejection(rejection: ProductRejection) -> OpenAiCompatHttpError {
    match rejection.kind {
        ProductRejectionKind::BindingRequired => {
            OpenAiCompatHttpError::not_found(Some("messages".to_string()))
        }
        ProductRejectionKind::AccessDenied => OpenAiCompatHttpError::from_workflow_rejection(
            ProductWorkflowRejectionKind::Unauthorized,
            403,
            false,
            None,
        ),
        ProductRejectionKind::UnknownInstallation => OpenAiCompatHttpError::from_kind(
            503,
            true,
            crate::OpenAiCompatErrorKind::ServiceUnavailable,
            None,
        ),
        ProductRejectionKind::InvalidRequest => {
            OpenAiCompatHttpError::invalid_request(Some("messages".to_string()))
        }
        ProductRejectionKind::PolicyDenied => OpenAiCompatHttpError::from_workflow_rejection(
            ProductWorkflowRejectionKind::Unauthorized,
            403,
            false,
            None,
        ),
    }
}

fn parse_chat_request(
    raw_body: &[u8],
) -> Result<OpenAiChatCompletionRequest, OpenAiCompatHttpError> {
    serde_json::from_slice(raw_body)
        .map_err(|_| OpenAiCompatHttpError::invalid_request(Some("body".to_string())))
}

fn chat_messages_to_product_text(
    request: &OpenAiChatCompletionRequest,
) -> Result<String, OpenAiCompatHttpError> {
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
    let mut lines = Vec::with_capacity(request.messages.len() + 1);
    for message in &request.messages {
        let role = match message.role {
            OpenAiChatMessageRole::Developer => "developer",
            OpenAiChatMessageRole::System => "system",
            OpenAiChatMessageRole::User => "user",
            OpenAiChatMessageRole::Assistant => "assistant",
            OpenAiChatMessageRole::Tool => "tool",
        };
        lines.push(format!(
            "{role}: {}",
            content_value_to_text(message.content.as_ref())
        ));
        if let Some(tool_calls) = &message.tool_calls {
            lines.push(format!("assistant_tool_calls: {}", tool_calls.len()));
        }
        if let Some(tool_call_id) = &message.tool_call_id {
            lines.push(format!(
                "tool_call_id: {}",
                sanitize_product_text_fragment(tool_call_id)
            ));
        }
    }
    Ok(lines.join("\n"))
}

fn chat_user_message_payload(
    request: &OpenAiChatCompletionRequest,
) -> Result<UserMessagePayload, OpenAiCompatHttpError> {
    Ok(UserMessagePayload::new(
        chat_messages_to_product_text(request)?,
        vec![],
        ProductTriggerReason::DirectChat,
    )?)
}

fn content_value_to_text(content: Option<&serde_json::Value>) -> String {
    match content {
        Some(serde_json::Value::String(text)) => sanitize_product_text_fragment(text),
        Some(serde_json::Value::Array(items)) => items
            .iter()
            .filter_map(content_array_item_text)
            .collect::<Vec<_>>()
            .join(" "),
        Some(value) if !value.is_null() => "[non_text_content]".to_string(),
        _ => String::new(),
    }
}

fn content_array_item_text(value: &serde_json::Value) -> Option<String> {
    let object = value.as_object()?;
    match object.get("type").and_then(serde_json::Value::as_str) {
        Some("text" | "input_text" | "output_text") => object
            .get("text")
            .and_then(serde_json::Value::as_str)
            .map(sanitize_product_text_fragment),
        _ => Some("[non_text_content]".to_string()),
    }
}

fn sanitize_product_text_fragment(value: &str) -> String {
    value.replace(['\n', '\r'], " ")
}
