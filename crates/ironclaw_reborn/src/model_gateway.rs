//! LLM provider-backed Reborn model gateway wiring.
//!
//! The loop-support crate owns the host-facing model gateway contract. This
//! adapter lives in the standalone Reborn composition crate because it bridges
//! that contract to the shared `ironclaw_llm` provider abstraction.

use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use ironclaw_llm::{
    ChatMessage, CompletionRequest, CompletionResponse, FinishReason, LlmError, LlmProvider,
};
use ironclaw_loop_support::{
    HostManagedModelError, HostManagedModelErrorKind, HostManagedModelGateway,
    HostManagedModelMessage, HostManagedModelMessageRole, HostManagedModelRequest,
    HostManagedModelResponse, ThreadBackedLoopModelPort,
};
use ironclaw_threads::{SessionThreadService, ThreadScope};
use ironclaw_turns::run_profile::{
    AgentLoopHostError, LoopModelGateway, LoopModelGatewayError, LoopModelGatewayRequest,
    LoopModelPort, LoopModelResponse, LoopSafeSummary, ModelProfileId,
};

/// Fail-closed routing policy from resolved Reborn model profile ids to the
/// host-selected provider/model envelope.
#[derive(Debug, Clone, Default)]
pub struct LlmModelProfilePolicy {
    routes: HashMap<ModelProfileId, LlmModelProfileRoute>,
}

impl LlmModelProfilePolicy {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn allow_model_profile(
        mut self,
        model_profile_id: ModelProfileId,
        model_override: Option<String>,
    ) -> Self {
        self.routes
            .insert(model_profile_id, LlmModelProfileRoute { model_override });
        self
    }

    fn route_for(&self, model_profile_id: &ModelProfileId) -> Option<&LlmModelProfileRoute> {
        self.routes.get(model_profile_id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LlmModelProfileRoute {
    model_override: Option<String>,
}

/// Production Reborn model gateway backed by durable session-thread context.
///
/// This is the concrete adapter intended to sit behind
/// [`HostManagedLoopModelPort`](ironclaw_turns::run_profile::HostManagedLoopModelPort):
/// it resolves loop message refs from the durable thread service, then delegates
/// provider routing and sanitization to the host-managed model gateway.
#[derive(Clone)]
pub struct ThreadBackedLoopModelGateway<S, G>
where
    S: SessionThreadService + ?Sized,
    G: HostManagedModelGateway + ?Sized,
{
    thread_service: Arc<S>,
    thread_scope: ThreadScope,
    host_gateway: Arc<G>,
    max_messages: usize,
}

impl<S, G> ThreadBackedLoopModelGateway<S, G>
where
    S: SessionThreadService + ?Sized,
    G: HostManagedModelGateway + ?Sized,
{
    pub fn new(
        thread_service: Arc<S>,
        thread_scope: ThreadScope,
        host_gateway: Arc<G>,
        max_messages: usize,
    ) -> Self {
        Self {
            thread_service,
            thread_scope,
            host_gateway,
            max_messages,
        }
    }
}

#[async_trait]
impl<S, G> LoopModelGateway for ThreadBackedLoopModelGateway<S, G>
where
    S: SessionThreadService + ?Sized + Send + Sync,
    G: HostManagedModelGateway + ?Sized + Send + Sync,
{
    async fn stream_model(
        &self,
        request: LoopModelGatewayRequest,
    ) -> Result<LoopModelResponse, LoopModelGatewayError> {
        ThreadBackedLoopModelPort::new(
            Arc::clone(&self.thread_service),
            self.thread_scope.clone(),
            request.context,
            Arc::clone(&self.host_gateway),
            self.max_messages,
        )
        .stream_model(request.request)
        .await
        .map_err(host_error_to_model_gateway_error)
    }
}

/// Host-managed model gateway backed by the shared `ironclaw_llm::LlmProvider` abstraction.
#[derive(Clone)]
pub struct LlmProviderModelGateway<P>
where
    P: LlmProvider + ?Sized,
{
    provider: Arc<P>,
    policy: LlmModelProfilePolicy,
}

impl<P> LlmProviderModelGateway<P>
where
    P: LlmProvider + ?Sized,
{
    pub fn new(provider: Arc<P>, policy: LlmModelProfilePolicy) -> Self {
        Self { provider, policy }
    }
}

#[async_trait]
impl<P> HostManagedModelGateway for LlmProviderModelGateway<P>
where
    P: LlmProvider + ?Sized + Send + Sync,
{
    async fn stream_model(
        &self,
        request: HostManagedModelRequest,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        let route = self
            .policy
            .route_for(&request.model_profile_id)
            .ok_or_else(|| {
                HostManagedModelError::safe(
                    HostManagedModelErrorKind::PolicyDenied,
                    "model profile is not permitted",
                )
            })?;
        let model_override = pinned_model_override(route)?;
        let mut completion = CompletionRequest::new(convert_messages(request.messages)?);
        completion.model = Some(model_override.to_string());
        completion.metadata.insert(
            "model_profile_id".to_string(),
            request.model_profile_id.as_str().to_string(),
        );
        completion
            .metadata
            .insert("turn_id".to_string(), request.turn_id.to_string());
        completion
            .metadata
            .insert("run_id".to_string(), request.run_id.to_string());

        let response = self
            .provider
            .complete(completion)
            .await
            .map_err(map_provider_error)?;
        response_to_host_reply(response)
    }
}

fn host_error_to_model_gateway_error(error: AgentLoopHostError) -> LoopModelGatewayError {
    let diagnostic_ref = error.diagnostic_ref;
    let mut converted = match LoopModelGatewayError::new(error.kind, error.safe_summary) {
        Ok(error) => error,
        Err(_) => LoopModelGatewayError {
            kind: error.kind,
            safe_summary: LoopSafeSummary::model_gateway_failed(),
            diagnostic_ref: None,
        },
    };
    if let Some(diagnostic_ref) = diagnostic_ref {
        converted = converted.with_diagnostic_ref(diagnostic_ref);
    }
    converted
}

fn pinned_model_override(route: &LlmModelProfileRoute) -> Result<&str, HostManagedModelError> {
    let Some(model_override) = route.model_override.as_deref() else {
        return Err(HostManagedModelError::safe(
            HostManagedModelErrorKind::PolicyDenied,
            "model profile route must pin a concrete provider model",
        ));
    };
    let trimmed = model_override.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("default") {
        return Err(HostManagedModelError::safe(
            HostManagedModelErrorKind::PolicyDenied,
            "model profile route must pin a concrete provider model",
        ));
    }
    Ok(trimmed)
}

fn response_to_host_reply(
    response: CompletionResponse,
) -> Result<HostManagedModelResponse, HostManagedModelError> {
    match response.finish_reason {
        FinishReason::Stop => Ok(HostManagedModelResponse::assistant_reply(response.content)),
        FinishReason::Length => Err(HostManagedModelError::safe(
            HostManagedModelErrorKind::BudgetExceeded,
            "model response was truncated before completion",
        )),
        FinishReason::ContentFilter => Err(HostManagedModelError::safe(
            HostManagedModelErrorKind::PolicyDenied,
            "model response was blocked by provider policy",
        )),
        FinishReason::ToolUse => Err(HostManagedModelError::safe(
            HostManagedModelErrorKind::InvalidRequest,
            "model returned unsupported tool calls for a text-only loop",
        )),
        FinishReason::Unknown => Err(HostManagedModelError::safe(
            HostManagedModelErrorKind::Unavailable,
            "model response did not complete cleanly",
        )),
    }
}

fn convert_messages(
    messages: Vec<HostManagedModelMessage>,
) -> Result<Vec<ChatMessage>, HostManagedModelError> {
    messages
        .into_iter()
        .map(|message| match message.role {
            HostManagedModelMessageRole::System => Ok(ChatMessage::system(message.content)),
            HostManagedModelMessageRole::User => Ok(ChatMessage::user(message.content)),
            HostManagedModelMessageRole::Assistant => Ok(ChatMessage::assistant(message.content)),
        })
        .collect()
}

fn map_provider_error(error: LlmError) -> HostManagedModelError {
    match error {
        LlmError::ContextLengthExceeded { .. } => HostManagedModelError::safe(
            HostManagedModelErrorKind::BudgetExceeded,
            "model request exceeded its context budget",
        ),
        LlmError::ModelNotAvailable { .. } => HostManagedModelError::safe(
            HostManagedModelErrorKind::PolicyDenied,
            "requested model is not available through this profile",
        ),
        LlmError::AuthFailed { .. } | LlmError::SessionExpired { .. } => {
            HostManagedModelError::safe(
                HostManagedModelErrorKind::Unavailable,
                "model credentials are unavailable",
            )
        }
        _ => HostManagedModelError::safe(
            HostManagedModelErrorKind::Unavailable,
            "model service is unavailable",
        ),
    }
}
