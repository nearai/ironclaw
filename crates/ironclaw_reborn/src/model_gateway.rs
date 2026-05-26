//! LLM provider-backed Reborn model gateway wiring.
//!
//! The loop-support crate owns the host-facing model gateway contract. This
//! adapter lives in the standalone Reborn composition crate because it bridges
//! that contract to the shared `ironclaw_llm` provider abstraction.

use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use async_trait::async_trait;
use ironclaw_host_api::sha256_digest_token;
use ironclaw_llm::{
    ChatMessage, CompletionRequest, CompletionResponse, FinishReason, LlmError, LlmProvider,
    ToolCall, ToolCompletionRequest, ToolCompletionResponse, ToolDefinition,
};
use ironclaw_loop_support::{
    HostManagedModelError, HostManagedModelErrorKind, HostManagedModelGateway,
    HostManagedModelMessage, HostManagedModelMessageRole, HostManagedModelRequest,
    HostManagedModelResponse, HostManagedModelRouteSnapshot, ThreadBackedLoopContextPort,
    ThreadBackedLoopModelPort,
};
use ironclaw_threads::{
    ProviderToolCallReferenceEnvelope, SessionThreadService, ThreadScope,
    ToolResultReferenceEnvelope,
};
use ironclaw_turns::{
    TurnId, TurnRunId,
    run_profile::{
        AgentLoopHostError, AgentLoopHostErrorKind, HostManagedLoopPromptPort,
        InMemoryLoopHostMilestoneSink, LoopModelGateway, LoopModelGatewayError,
        LoopModelGatewayRequest, LoopModelPort, LoopModelRequest, LoopModelResponse,
        LoopPromptBundleRequest, LoopPromptPort, LoopRunContext, LoopSafeSummary, ModelProfileId,
        PromptMode, ProviderToolCall, ProviderToolDefinition,
    },
};
use tracing::debug;

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
        self.issue_host_prompt_bundle(&request.context, &request.request)
            .await?;
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

impl<S, G> ThreadBackedLoopModelGateway<S, G>
where
    S: SessionThreadService + ?Sized + Send + Sync,
    G: HostManagedModelGateway + ?Sized + Send + Sync,
{
    async fn issue_host_prompt_bundle(
        &self,
        context: &LoopRunContext,
        request: &LoopModelRequest,
    ) -> Result<(), LoopModelGatewayError> {
        let context_port = Arc::new(ThreadBackedLoopContextPort::new(
            Arc::clone(&self.thread_service),
            self.thread_scope.clone(),
            context.clone(),
            self.max_messages,
        ));
        let prompt_port = HostManagedLoopPromptPort::new(
            context.clone(),
            context_port,
            Arc::new(InMemoryLoopHostMilestoneSink::default()),
        );
        let prompt_bundle = prompt_port
            .build_prompt_bundle(LoopPromptBundleRequest {
                mode: PromptMode::TextOnly,
                context_cursor: None,
                surface_version: request.surface_version.clone(),
                checkpoint_state_ref: None,
                max_messages: Some(self.max_messages.min(u32::MAX as usize) as u32),
                inline_messages: Vec::new(),
                capability_view: None,
            })
            .await
            .map_err(host_error_to_model_gateway_error)?;

        if prompt_bundle.messages != request.messages {
            return Err(host_error_to_model_gateway_error(AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "model request does not match the host-built prompt bundle",
            )));
        }
        if prompt_bundle.surface_version != request.surface_version {
            return Err(host_error_to_model_gateway_error(AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "model request surface version does not match the host-built prompt bundle",
            )));
        }

        Ok(())
    }
}

/// Host-managed model gateway backed by the shared `ironclaw_llm::LlmProvider` abstraction.
#[derive(Clone)]
pub struct LlmProviderModelGateway<P>
where
    P: LlmProvider + ?Sized,
{
    provider_id: String,
    provider: Arc<P>,
    policy: LlmModelProfilePolicy,
    provider_turn_sequence: Arc<AtomicU64>,
}

impl<P> LlmProviderModelGateway<P>
where
    P: LlmProvider + ?Sized,
{
    pub fn new(provider: Arc<P>, policy: LlmModelProfilePolicy) -> Self {
        let provider_id = provider.model_name().to_string();
        Self::with_provider_identity(provider_id, provider, policy)
    }

    pub fn with_provider_identity(
        provider_id: impl Into<String>,
        provider: Arc<P>,
        policy: LlmModelProfilePolicy,
    ) -> Self {
        Self {
            provider_id: provider_id.into(),
            provider,
            policy,
            provider_turn_sequence: Arc::new(AtomicU64::new(1)),
        }
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
        let replay_identity = ProviderReplayIdentity::new(&self.provider_id, model_override)?;
        let mut completion =
            CompletionRequest::new(convert_messages(request.messages, &replay_identity)?);
        completion.model = Some(model_override.to_string());
        add_request_metadata(&mut completion, &model_profile_id, run_id, turn_id);

        complete_model_request(
            self.provider.as_ref(),
            completion,
            None,
            None,
            replay_identity,
        )
        .await
    }

    async fn stream_model_with_capabilities(
        &self,
        request: HostManagedModelRequest,
        capabilities: Arc<dyn ironclaw_turns::run_profile::LoopCapabilityPort>,
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
        let replay_identity = ProviderReplayIdentity::new(&self.provider_id, model_override)?;
        let mut completion =
            CompletionRequest::new(convert_messages(request.messages, &replay_identity)?);
        completion.model = Some(model_override.to_string());
        add_request_metadata(&mut completion, &model_profile_id, run_id, turn_id);

        let provider_turn_scope = format!(
            "run={run_id}\nturn={turn_id}\nmodel_call={}",
            self.provider_turn_sequence.fetch_add(1, Ordering::Relaxed)
        );
        complete_model_request(
            self.provider.as_ref(),
            completion,
            Some(capabilities),
            Some(provider_turn_scope),
            replay_identity,
        )
        .await
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
    provider_turn_sequence: Arc<AtomicU64>,
}

impl<P> RoutedLlmProviderModelGateway<P>
where
    P: ModelRouteProviderPool + ?Sized,
{
    pub fn new(provider_pool: Arc<P>, route_resolver: Arc<dyn ModelRouteResolver>) -> Self {
        Self {
            provider_pool,
            route_resolver,
            provider_turn_sequence: Arc::new(AtomicU64::new(1)),
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
        let replay_identity = ProviderReplayIdentity::new(
            snapshot.route().provider_id(),
            snapshot.route().model_id(),
        )?;
        let mut completion =
            CompletionRequest::new(convert_messages(request.messages, &replay_identity)?);
        completion.model = Some(snapshot.route().model_id().to_string());
        validate_provider_model_binding_matches_route(snapshot.route(), provider.as_ref())?;
        add_request_metadata(&mut completion, &model_profile_id, run_id, turn_id);
        add_route_metadata(&mut completion, &snapshot);

        complete_model_request(provider.as_ref(), completion, None, None, replay_identity).await
    }

    async fn stream_model_with_capabilities(
        &self,
        request: HostManagedModelRequest,
        capabilities: Arc<dyn ironclaw_turns::run_profile::LoopCapabilityPort>,
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
        let replay_identity = ProviderReplayIdentity::new(
            snapshot.route().provider_id(),
            snapshot.route().model_id(),
        )?;
        let mut completion =
            CompletionRequest::new(convert_messages(request.messages, &replay_identity)?);
        completion.model = Some(snapshot.route().model_id().to_string());
        validate_provider_model_binding_matches_route(snapshot.route(), provider.as_ref())?;
        add_request_metadata(&mut completion, &model_profile_id, run_id, turn_id);
        add_route_metadata(&mut completion, &snapshot);

        let provider_turn_scope = format!(
            "run={run_id}\nturn={turn_id}\nmodel_call={}",
            self.provider_turn_sequence.fetch_add(1, Ordering::Relaxed)
        );
        complete_model_request(
            provider.as_ref(),
            completion,
            Some(capabilities),
            Some(provider_turn_scope),
            replay_identity,
        )
        .await
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProviderReplayIdentity {
    provider_id: String,
    provider_model_id: String,
}

impl ProviderReplayIdentity {
    fn new(
        provider_id: impl Into<String>,
        provider_model_id: impl Into<String>,
    ) -> Result<Self, HostManagedModelError> {
        let identity = Self {
            provider_id: provider_id.into(),
            provider_model_id: provider_model_id.into(),
        };
        validate_replay_identity_text(&identity.provider_id, "provider id")?;
        validate_replay_identity_text(&identity.provider_model_id, "provider model id")?;
        Ok(identity)
    }
}

fn validate_replay_identity_text(
    value: &str,
    label: &'static str,
) -> Result<(), HostManagedModelError> {
    if value.trim().is_empty() {
        return Err(HostManagedModelError::safe(
            HostManagedModelErrorKind::PolicyDenied,
            format!("{label} must not be empty"),
        ));
    }
    if value.len() > 512 {
        return Err(HostManagedModelError::safe(
            HostManagedModelErrorKind::PolicyDenied,
            format!("{label} exceeds 512 bytes"),
        ));
    }
    if value
        .chars()
        .any(|character| character == '\0' || character.is_control())
    {
        return Err(HostManagedModelError::safe(
            HostManagedModelErrorKind::PolicyDenied,
            format!("{label} must not contain NUL/control characters"),
        ));
    }
    Ok(())
}

#[tracing::instrument(
    level = "debug",
    skip(provider, completion, capabilities, replay_identity),
    fields(
        provider_id = %replay_identity.provider_id,
        provider_model_id = %replay_identity.provider_model_id,
        provider_turn_scope = provider_turn_scope.as_deref().unwrap_or("model_call=unknown"),
    )
)]
async fn complete_model_request<P>(
    provider: &P,
    completion: CompletionRequest,
    capabilities: Option<Arc<dyn ironclaw_turns::run_profile::LoopCapabilityPort>>,
    provider_turn_scope: Option<String>,
    replay_identity: ProviderReplayIdentity,
) -> Result<HostManagedModelResponse, HostManagedModelError>
where
    P: LlmProvider + ?Sized,
{
    if let Some(capabilities) = capabilities {
        let tool_definitions = capabilities
            .tool_definitions()
            .map_err(map_capability_host_error)?;
        if tracing::enabled!(tracing::Level::DEBUG) {
            let tool_name_sample = tool_definitions
                .iter()
                .take(20)
                .map(|definition| definition.name.as_str())
                .collect::<Vec<_>>();
            debug!(
                tool_definition_count = tool_definitions.len(),
                tool_name_sample = ?tool_name_sample,
                "reborn model gateway resolved provider tool definitions"
            );
        }
        if !tool_definitions.is_empty() {
            let tool_request = ToolCompletionRequest::from_completion_request(
                completion,
                tool_definitions
                    .into_iter()
                    .map(provider_tool_definition_to_llm)
                    .collect(),
            );
            debug!("reborn model gateway dispatching tool-capable provider request");
            let response = provider
                .complete_with_tools(tool_request)
                .await
                .map_err(map_provider_error)?;
            return tool_response_to_host(
                response,
                capabilities,
                provider_turn_scope
                    .as_deref()
                    .unwrap_or("model_call=unknown"),
                &replay_identity,
            )
            .await;
        }
        debug!(
            "reborn model gateway falling back to text-only provider request because no provider tool definitions were available"
        );
    } else {
        debug!(
            "reborn model gateway dispatching text-only provider request because no capability port was supplied"
        );
    }

    let response = provider
        .complete(completion)
        .await
        .map_err(map_provider_error)?;
    debug!(
        finish_reason = ?response.finish_reason,
        content_bytes = response.content.len(),
        "reborn model gateway received text-only provider response"
    );
    response_to_host_reply(response)
}

fn provider_tool_definition_to_llm(definition: ProviderToolDefinition) -> ToolDefinition {
    ToolDefinition {
        name: definition.name,
        description: definition.description,
        parameters: definition.parameters,
    }
}

#[tracing::instrument(
    level = "debug",
    skip(response, capabilities, replay_identity),
    fields(
        provider_id = %replay_identity.provider_id,
        provider_model_id = %replay_identity.provider_model_id,
        provider_turn_scope,
    )
)]
async fn tool_response_to_host(
    response: ToolCompletionResponse,
    capabilities: Arc<dyn ironclaw_turns::run_profile::LoopCapabilityPort>,
    provider_turn_scope: &str,
    replay_identity: &ProviderReplayIdentity,
) -> Result<HostManagedModelResponse, HostManagedModelError> {
    if tracing::enabled!(tracing::Level::DEBUG) {
        let tool_call_name_sample = response
            .tool_calls
            .iter()
            .take(20)
            .map(|tool_call| tool_call.name.as_str())
            .collect::<Vec<_>>();
        debug!(
            finish_reason = ?response.finish_reason,
            tool_call_count = response.tool_calls.len(),
            tool_call_name_sample = ?tool_call_name_sample,
            content_bytes = response.content.as_ref().map(|content| content.len()).unwrap_or(0),
            "reborn model gateway received tool-capable provider response"
        );
    }
    if !response.tool_calls.is_empty()
        && matches!(
            response.finish_reason,
            FinishReason::ToolUse | FinishReason::Stop
        )
    {
        let advertised_tool_names = capabilities
            .tool_definitions()
            .map_err(map_capability_host_error)?
            .into_iter()
            .map(|definition| definition.name)
            .collect::<HashSet<_>>();
        if response
            .tool_calls
            .iter()
            .any(|tool_call| !advertised_tool_names.contains(&tool_call.name))
        {
            return Err(HostManagedModelError::safe(
                HostManagedModelErrorKind::InvalidRequest,
                "model returned a tool call outside the advertised capability surface",
            ));
        }
        let mut candidates = Vec::with_capacity(response.tool_calls.len());
        let provider_turn_id = provider_turn_id(provider_turn_scope, &response.tool_calls);
        let provider_calls = response
            .tool_calls
            .into_iter()
            .map(|tool_call| {
                provider_tool_call_from_llm(
                    tool_call,
                    response.reasoning.clone(),
                    provider_turn_id.clone(),
                    replay_identity,
                )
            })
            .collect::<Vec<_>>();
        for provider_call in &provider_calls {
            capabilities
                .validate_provider_tool_call(provider_call)
                .map_err(map_capability_host_error)?;
        }
        for provider_call in provider_calls {
            let candidate = capabilities
                .register_provider_tool_call(provider_call)
                .await
                .map_err(map_capability_host_error)?;
            candidates.push(candidate);
        }
        debug!(
            capability_call_count = candidates.len(),
            "reborn model gateway classified provider response as capability calls"
        );
        return Ok(HostManagedModelResponse::capability_calls_with_reasoning(
            candidates,
            response.content.unwrap_or_default(),
            response.reasoning,
        ));
    }

    match response.finish_reason {
        FinishReason::Stop => {
            let content = response.content.unwrap_or_default();
            debug!(
                content_bytes = content.len(),
                "reborn model gateway classified tool-capable provider response as assistant reply"
            );
            Ok(HostManagedModelResponse::assistant_reply_with_reasoning(
                content,
                response.reasoning,
            ))
        }
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
            "model returned tool-use finish without tool calls",
        )),
        FinishReason::Unknown => Err(HostManagedModelError::safe(
            HostManagedModelErrorKind::Unavailable,
            "model response did not complete cleanly",
        )),
    }
}

fn provider_tool_call_from_llm(
    tool_call: ToolCall,
    response_reasoning: Option<String>,
    provider_turn_id: String,
    replay_identity: &ProviderReplayIdentity,
) -> ProviderToolCall {
    ProviderToolCall {
        provider_id: replay_identity.provider_id.clone(),
        provider_model_id: replay_identity.provider_model_id.clone(),
        turn_id: Some(provider_turn_id),
        id: tool_call.id,
        name: tool_call.name,
        arguments: tool_call.arguments,
        response_reasoning,
        reasoning: tool_call.reasoning,
        signature: tool_call.signature,
    }
}

fn provider_turn_id(provider_turn_scope: &str, tool_calls: &[ToolCall]) -> String {
    let mut stable = String::new();
    stable.push_str(provider_turn_scope);
    stable.push('\0');
    for tool_call in tool_calls {
        stable.push_str(tool_call.id.as_str());
        stable.push('\0');
        stable.push_str(tool_call.name.as_str());
        stable.push('\0');
    }
    format!("provider_turn:{}", sha256_hex_prefix(stable.as_bytes(), 32))
}

fn sha256_hex_prefix(input: &[u8], len: usize) -> String {
    let digest = sha256_digest_token(input);
    digest
        .strip_prefix("sha256:")
        .unwrap_or(&digest)
        .chars()
        .take(len)
        .collect()
}

fn response_to_host_reply(
    response: CompletionResponse,
) -> Result<HostManagedModelResponse, HostManagedModelError> {
    match response.finish_reason {
        FinishReason::Stop => Ok(HostManagedModelResponse::assistant_reply_with_reasoning(
            response.content,
            response.reasoning,
        )),
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

fn map_capability_host_error(error: AgentLoopHostError) -> HostManagedModelError {
    let kind = match error.kind {
        AgentLoopHostErrorKind::CredentialUnavailable => {
            HostManagedModelErrorKind::CredentialUnavailable
        }
        AgentLoopHostErrorKind::Unauthorized | AgentLoopHostErrorKind::PolicyDenied => {
            HostManagedModelErrorKind::PolicyDenied
        }
        AgentLoopHostErrorKind::BudgetExceeded
        | AgentLoopHostErrorKind::BudgetApprovalRequired
        | AgentLoopHostErrorKind::BudgetAccountingFailed => {
            HostManagedModelErrorKind::BudgetExceeded
        }
        AgentLoopHostErrorKind::Cancelled => HostManagedModelErrorKind::Cancelled,
        AgentLoopHostErrorKind::Invalid
        | AgentLoopHostErrorKind::InvalidInvocation
        | AgentLoopHostErrorKind::ScopeMismatch
        | AgentLoopHostErrorKind::StaleSurface => HostManagedModelErrorKind::InvalidRequest,
        AgentLoopHostErrorKind::Unavailable
        | AgentLoopHostErrorKind::CheckpointRejected
        | AgentLoopHostErrorKind::TranscriptWriteFailed
        | AgentLoopHostErrorKind::Internal => HostManagedModelErrorKind::Unavailable,
    };
    HostManagedModelError::safe(kind, error.safe_summary)
}

fn convert_messages(
    messages: Vec<HostManagedModelMessage>,
    replay_identity: &ProviderReplayIdentity,
) -> Result<Vec<ChatMessage>, HostManagedModelError> {
    let mut converted = Vec::with_capacity(messages.len());
    let mut index = 0;
    while index < messages.len() {
        let message = &messages[index];
        match message.role {
            HostManagedModelMessageRole::System => {
                converted.push(ChatMessage::system(message.content.clone()))
            }
            HostManagedModelMessageRole::User => {
                converted.push(ChatMessage::user(message.content.clone()))
            }
            HostManagedModelMessageRole::Assistant => {
                converted.push(ChatMessage::assistant(message.content.clone()));
            }
            HostManagedModelMessageRole::ToolResult => {
                let replay = tool_result_replay_message(message)?;
                let Some(provider_call) = replay.provider_call else {
                    converted.push(ChatMessage::system(replay.safe_summary));
                    index += 1;
                    continue;
                };
                validate_provider_replay_identity(&provider_call, replay_identity)?;
                let provider_turn_id = provider_call.provider_turn_id.clone();
                let mut provider_results = vec![(provider_call, replay.safe_summary)];
                let mut plain_tool_summaries = Vec::new();
                index += 1;
                while index < messages.len()
                    && messages[index].role == HostManagedModelMessageRole::ToolResult
                {
                    let next = tool_result_replay_message(&messages[index])?;
                    let Some(next_provider_call) = next.provider_call else {
                        plain_tool_summaries.push(next.safe_summary);
                        index += 1;
                        continue;
                    };
                    validate_provider_replay_identity(&next_provider_call, replay_identity)?;
                    if next_provider_call.provider_turn_id != provider_turn_id {
                        break;
                    }
                    provider_results.push((next_provider_call, next.safe_summary));
                    index += 1;
                }
                converted.extend(provider_tool_roundtrip_messages(provider_results));
                converted.extend(plain_tool_summaries.into_iter().map(ChatMessage::system));
                continue;
            }
        }
        index += 1;
    }
    Ok(converted)
}

fn validate_provider_replay_identity(
    provider_call: &ProviderToolCallReferenceEnvelope,
    expected: &ProviderReplayIdentity,
) -> Result<(), HostManagedModelError> {
    provider_call.validate().map_err(|error| {
        ironclaw_loop_support::raw_host_managed_model_error(
            "provider_tool_replay",
            "validate_provider_call",
            HostManagedModelErrorKind::InvalidRequest,
            "provider tool-call replay metadata is invalid",
            error,
        )
    })?;
    if provider_call.provider_id != expected.provider_id
        || provider_call.provider_model_id != expected.provider_model_id
    {
        return Err(HostManagedModelError::safe(
            HostManagedModelErrorKind::PolicyDenied,
            "provider tool-call replay metadata does not match the selected provider route",
        ));
    }
    Ok(())
}

struct ToolResultReplayMessage {
    provider_call: Option<ProviderToolCallReferenceEnvelope>,
    safe_summary: String,
}

fn tool_result_replay_message(
    message: &HostManagedModelMessage,
) -> Result<ToolResultReplayMessage, HostManagedModelError> {
    let envelope: ToolResultReferenceEnvelope =
        serde_json::from_str(&message.content).map_err(|error| {
            ironclaw_loop_support::raw_host_managed_model_error(
                "tool_result_replay",
                "decode_transcript_envelope",
                HostManagedModelErrorKind::InvalidRequest,
                "tool result reference transcript content is invalid",
                error,
            )
        })?;
    Ok(ToolResultReplayMessage {
        provider_call: message.tool_result_provider_call.clone(),
        safe_summary: envelope.safe_summary.as_str().to_string(),
    })
}

fn provider_tool_roundtrip_messages(
    provider_results: Vec<(ProviderToolCallReferenceEnvelope, String)>,
) -> Vec<ChatMessage> {
    let reasoning = provider_results
        .iter()
        .find_map(|(provider_call, _)| provider_call.response_reasoning.clone());
    let assistant = ChatMessage::assistant_with_tool_calls(
        None,
        provider_results
            .iter()
            .map(|(provider_call, _)| provider_tool_call_from_reference(provider_call))
            .collect(),
    )
    .with_reasoning(reasoning);
    std::iter::once(assistant)
        .chain(
            provider_results
                .into_iter()
                .map(|(provider_call, summary)| {
                    ChatMessage::tool_result(
                        provider_call.provider_call_id,
                        provider_call.provider_tool_name,
                        summary,
                    )
                }),
        )
        .collect()
}

fn provider_tool_call_from_reference(
    provider_call: &ProviderToolCallReferenceEnvelope,
) -> ToolCall {
    ToolCall {
        id: provider_call.provider_call_id.clone(),
        name: provider_call.provider_tool_name.clone(),
        arguments: provider_call.arguments.clone(),
        reasoning: provider_call.reasoning.clone(),
        signature: provider_call.signature.clone(),
    }
}

fn map_provider_error(error: LlmError) -> HostManagedModelError {
    tracing::warn!(
        component = "model_provider",
        operation = "complete",
        error = %error,
        error_debug = ?error,
        "reborn model provider error mapped to safe summary"
    );
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
                HostManagedModelErrorKind::CredentialUnavailable,
                "model credentials are unavailable",
            )
        }
        _ => HostManagedModelError::safe(
            HostManagedModelErrorKind::Unavailable,
            "model service is unavailable",
        ),
    }
}
