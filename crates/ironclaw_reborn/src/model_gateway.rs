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
    HostManagedModelResponse, HostManagedModelRouteSnapshot, ThreadBackedLoopModelPort,
};
use ironclaw_threads::{SessionThreadService, ThreadScope};
use ironclaw_turns::{
    TurnId, TurnRunId,
    run_profile::{
        AgentLoopHostError, LoopModelGateway, LoopModelGatewayError, LoopModelGatewayRequest,
        LoopModelPort, LoopModelResponse, LoopSafeSummary, ModelProfileId,
    },
};

use crate::model_routes::{
    ModelRoute, ModelRouteError, ModelRouteErrorKind, ModelRouteProviderKey, ModelRouteResolver,
    ModelSelectionMode, ModelSlot, ResolvedModelRouteSnapshot,
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
        let model_profile_id = request.model_profile_id.clone();
        let run_id = request.run_id;
        let turn_id = request.turn_id;
        let mut completion = CompletionRequest::new(convert_messages(request.messages)?);
        completion.model = Some(model_override.to_string());
        add_request_metadata(&mut completion, &model_profile_id, run_id, turn_id);

        let response = self
            .provider
            .complete(completion)
            .await
            .map_err(map_provider_error)?;
        response_to_host_reply(response)
    }
}

#[async_trait]
pub trait ModelRouteProviderPool: Send + Sync {
    async fn provider_for_route(
        &self,
        snapshot: &ResolvedModelRouteSnapshot,
    ) -> Result<Arc<dyn LlmProvider>, HostManagedModelError>;
}

#[derive(Clone)]
struct RouteBoundProvider {
    provider_id: String,
    provider: Arc<dyn LlmProvider>,
}

#[derive(Clone, Default)]
pub struct StaticModelRouteProviderPool {
    providers: HashMap<ModelRouteProviderKey, RouteBoundProvider>,
}

impl StaticModelRouteProviderPool {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_provider<P>(
        self,
        route: ModelRoute,
        provider: Arc<P>,
    ) -> Result<Self, HostManagedModelError>
    where
        P: LlmProvider + 'static,
    {
        self.with_provider_key(ModelRouteProviderKey::for_route(route), provider)
    }

    pub fn with_provider_key<P>(
        self,
        key: ModelRouteProviderKey,
        provider: Arc<P>,
    ) -> Result<Self, HostManagedModelError>
    where
        P: LlmProvider + 'static,
    {
        self.with_provider_identity(key.route().provider_id().to_string(), key, provider)
    }

    pub fn with_provider_identity<P>(
        mut self,
        provider_id: impl Into<String>,
        key: ModelRouteProviderKey,
        provider: Arc<P>,
    ) -> Result<Self, HostManagedModelError>
    where
        P: LlmProvider + 'static,
    {
        let provider_id = provider_id.into();
        validate_provider_identity_matches_route(&provider_id, key.route())?;
        validate_provider_model_binding_matches_route(key.route(), provider.as_ref())?;
        let provider: Arc<dyn LlmProvider> = provider;
        self.providers.insert(
            key,
            RouteBoundProvider {
                provider_id,
                provider,
            },
        );
        Ok(self)
    }
}

#[async_trait]
impl ModelRouteProviderPool for StaticModelRouteProviderPool {
    async fn provider_for_route(
        &self,
        snapshot: &ResolvedModelRouteSnapshot,
    ) -> Result<Arc<dyn LlmProvider>, HostManagedModelError> {
        let bound = self
            .providers
            .get(snapshot.provider_key())
            .cloned()
            .ok_or_else(|| {
                HostManagedModelError::safe(
                    HostManagedModelErrorKind::ConfigurationError,
                    "model route provider is not configured",
                )
            })?;
        validate_provider_identity_matches_route(&bound.provider_id, snapshot.route())?;
        Ok(bound.provider)
    }
}

/// Routed gateway that consumes a route snapshot already attached to the run.
///
/// Route resolution is intentionally done by the host/run composition layer so
/// resumed runs keep using the same persisted provider/model route. This gateway
/// validates the carried snapshot and selects the matching provider.
///
/// No mid-run fallback is attempted: if a pinned route becomes unavailable
/// because config or auth versions rotated, operators must either restore the
/// provider-pool entry for the persisted key or cancel/retry the run so host
/// composition can attach a fresh route snapshot before driver side effects.
pub struct RoutedLlmProviderModelGateway<P>
where
    P: ModelRouteProviderPool + ?Sized,
{
    provider_pool: Arc<P>,
    route_resolver: Arc<dyn ModelRouteResolver>,
}

impl<P> RoutedLlmProviderModelGateway<P>
where
    P: ModelRouteProviderPool + ?Sized,
{
    pub fn new(provider_pool: Arc<P>, route_resolver: Arc<dyn ModelRouteResolver>) -> Self {
        Self {
            provider_pool,
            route_resolver,
        }
    }
}

#[async_trait]
impl<P> HostManagedModelGateway for RoutedLlmProviderModelGateway<P>
where
    P: ModelRouteProviderPool + ?Sized + Send + Sync,
{
    async fn stream_model(
        &self,
        request: HostManagedModelRequest,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        let slot = slot_for_model_profile(&request.model_profile_id)?;
        let request_snapshot = request
            .resolved_model_route
            .as_ref()
            .ok_or_else(missing_route_snapshot_error)?;
        let policy_mode = self.validate_route_snapshot(slot, request_snapshot)?;
        let snapshot = snapshot_from_host_request(slot, request_snapshot, policy_mode)?;
        let provider = self.provider_pool.provider_for_route(&snapshot).await?;
        let model_profile_id = request.model_profile_id.clone();
        let run_id = request.run_id;
        let turn_id = request.turn_id;
        let mut completion = CompletionRequest::new(convert_messages(request.messages)?);
        completion.model = Some(snapshot.route().model_id().to_string());
        validate_provider_model_binding_matches_route(snapshot.route(), provider.as_ref())?;
        add_request_metadata(&mut completion, &model_profile_id, run_id, turn_id);
        add_route_metadata(&mut completion, &snapshot);

        let response = provider
            .complete(completion)
            .await
            .map_err(map_provider_error)?;
        response_to_host_reply(response)
    }
}

impl<P> RoutedLlmProviderModelGateway<P>
where
    P: ModelRouteProviderPool + ?Sized,
{
    fn validate_route_snapshot(
        &self,
        slot: ModelSlot,
        snapshot: &HostManagedModelRouteSnapshot,
    ) -> Result<ModelSelectionMode, HostManagedModelError> {
        let route = ModelRoute::new(snapshot.provider_id.clone(), snapshot.model_id.clone())
            .map_err(map_model_route_error)?;
        self.route_resolver
            .validate_model_route(slot, &route)
            .map_err(map_model_route_error)
    }
}

fn add_request_metadata(
    completion: &mut CompletionRequest,
    model_profile_id: &ModelProfileId,
    run_id: TurnRunId,
    turn_id: TurnId,
) {
    completion.metadata.insert(
        "model_profile_id".to_string(),
        model_profile_id.as_str().to_string(),
    );
    completion
        .metadata
        .insert("turn_id".to_string(), turn_id.to_string());
    completion
        .metadata
        .insert("run_id".to_string(), run_id.to_string());
}

fn add_route_metadata(completion: &mut CompletionRequest, snapshot: &ResolvedModelRouteSnapshot) {
    completion.metadata.insert(
        "model_slot".to_string(),
        snapshot.slot().as_str().to_string(),
    );
    completion.metadata.insert(
        "model_route_provider_id".to_string(),
        snapshot.route().provider_id().to_string(),
    );
    completion.metadata.insert(
        "model_route_model_id".to_string(),
        snapshot.route().model_id().to_string(),
    );
}

fn missing_route_snapshot_error() -> HostManagedModelError {
    HostManagedModelError::safe(
        HostManagedModelErrorKind::PolicyDenied,
        "model route snapshot is required for routed model gateway",
    )
}

fn snapshot_from_host_request(
    slot: ModelSlot,
    snapshot: &HostManagedModelRouteSnapshot,
    policy_mode: ModelSelectionMode,
) -> Result<ResolvedModelRouteSnapshot, HostManagedModelError> {
    let route = ModelRoute::new(snapshot.provider_id.clone(), snapshot.model_id.clone())
        .map_err(map_model_route_error)?;
    let key = ModelRouteProviderKey::new(
        route,
        snapshot.config_version.clone(),
        snapshot.auth_version.clone(),
    )
    .map_err(map_model_route_error)?;
    Ok(ResolvedModelRouteSnapshot::with_provider_key(
        slot,
        key,
        policy_mode,
    ))
}

fn validate_provider_identity_matches_route(
    provider_id: &str,
    route: &ModelRoute,
) -> Result<(), HostManagedModelError> {
    if provider_id != route.provider_id() {
        return Err(HostManagedModelError::safe(
            HostManagedModelErrorKind::InvalidRequest,
            "model route provider identity does not match route",
        ));
    }
    Ok(())
}

fn validate_provider_model_binding_matches_route<P>(
    route: &ModelRoute,
    provider: &P,
) -> Result<(), HostManagedModelError>
where
    P: LlmProvider + ?Sized,
{
    if provider.effective_model_name(Some(route.model_id())) != route.model_id() {
        return Err(HostManagedModelError::safe(
            HostManagedModelErrorKind::InvalidRequest,
            "model route provider effective model does not match route",
        ));
    }
    Ok(())
}

fn slot_for_model_profile(
    model_profile_id: &ModelProfileId,
) -> Result<ModelSlot, HostManagedModelError> {
    ModelSlot::from_model_profile_id(model_profile_id).ok_or_else(|| {
        HostManagedModelError::safe(
            HostManagedModelErrorKind::PolicyDenied,
            "model profile is not supported by the default route resolver",
        )
    })
}

fn map_model_route_error(error: ModelRouteError) -> HostManagedModelError {
    match error.kind() {
        ModelRouteErrorKind::RouteUnavailable => HostManagedModelError::safe(
            HostManagedModelErrorKind::ConfigurationError,
            "model route is not configured",
        ),
        ModelRouteErrorKind::RouteNotApproved => HostManagedModelError::safe(
            HostManagedModelErrorKind::PolicyDenied,
            "model route is not permitted",
        ),
        ModelRouteErrorKind::InvalidRoute => HostManagedModelError::safe(
            HostManagedModelErrorKind::InvalidRequest,
            "model route is invalid",
        ),
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
