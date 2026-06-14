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
    ChatMessage, CompletionRequest, CompletionResponse, FinishReason, LlmError, LlmProvider, Role,
    ToolCall, ToolCompletionRequest, ToolCompletionResponse, ToolDefinition, clean_response,
    contains_codex_text_tool_call_syntax,
    costs::{default_cost, model_cost},
    recover_codex_text_tool_calls_from_tool_names,
};
use ironclaw_loop_support::{
    HostManagedModelError, HostManagedModelErrorKind, HostManagedModelGateway,
    HostManagedModelMessage, HostManagedModelMessageRole, HostManagedModelRequest,
    HostManagedModelResponse, HostManagedModelRouteSnapshot, HostManagedToolResultContent,
    ModelCost, StaticModelCostTable, ThreadBackedLoopContextPort, ThreadBackedLoopModelPort,
    ThreadContextWindowCache,
};
use ironclaw_safety::{
    is_provider_arguments_too_large_summary, provider_arguments_exceed_max_bytes,
};
use ironclaw_threads::{ProviderToolCallReferenceEnvelope, SessionThreadService, ThreadScope};
use ironclaw_turns::run_profile::LoopModelUsage;
use ironclaw_turns::{
    TurnId, TurnRunId,
    run_profile::{
        AgentLoopHostError, AgentLoopHostErrorKind, HostManagedLoopPromptPort,
        InMemoryInstructionMaterializationStore, InMemoryLoopHostMilestoneSink,
        InstructionMaterializationStore, InstructionSafetyContext, LoopModelGateway,
        LoopModelGatewayError, LoopModelGatewayRequest, LoopModelPort, LoopModelRequest,
        LoopModelResponse, LoopPromptBundleRequest, LoopPromptPort, LoopRunContext,
        LoopSafeSummary, ModelProfileId, PromptMode, ProviderToolCall, ProviderToolDefinition,
    },
};
use tracing::debug;

use crate::{
    failure_categories::MODEL_CREDITS_EXHAUSTED_REASON_KIND,
    model_routes::{
        ModelRoute, ModelRouteError, ModelRouteErrorKind, ModelRouteProviderKey,
        ModelRouteResolver, ModelSelectionMode, ModelSlot, ResolvedModelRouteSnapshot,
    },
};

const MODEL_CREDITS_EXHAUSTED_SUMMARY: &str = "model provider account is out of credits";
const PROVIDER_TOOL_ARGUMENTS_OMITTED_MARKER: &str =
    "arguments omitted because they exceeded the host provider-tool limit";

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

    /// Build a [`StaticModelCostTable`] mapping every allowed `ModelProfileId`
    /// to its per-token price via [`ironclaw_llm::costs::model_cost`].
    /// Profiles whose `model_override` is unknown to the LLM cost table
    /// fall back to [`ironclaw_llm::costs::default_cost`] (roughly GPT-4o
    /// pricing) so the accountant always reconciles to a non-zero spend
    /// for an unknown provider — fail-safe, not silent.
    pub fn build_cost_table(&self) -> StaticModelCostTable {
        let mut table = StaticModelCostTable::new();
        for (profile_id, route) in &self.routes {
            let cost = route
                .model_override
                .as_deref()
                .and_then(model_cost)
                .unwrap_or_else(default_cost);
            table.insert(
                profile_id.clone(),
                ModelCost {
                    input_per_token: cost.0,
                    output_per_token: cost.1,
                    // 0 = unknown; accountant falls back to its
                    // `DEFAULT_MAX_OUTPUT_TOKENS` (8 KiB) for the
                    // upfront reservation estimate.
                    max_output_tokens: 0,
                },
            );
        }
        table
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
    safety_context: InstructionSafetyContext,
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
        safety_context: InstructionSafetyContext,
    ) -> Self {
        Self {
            thread_service,
            thread_scope,
            host_gateway,
            max_messages,
            safety_context,
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
        let instruction_materialization_store: Arc<dyn InstructionMaterializationStore> =
            Arc::new(InMemoryInstructionMaterializationStore::default());
        let context_window_cache = Arc::new(ThreadContextWindowCache::default());
        self.issue_host_prompt_bundle(
            &request.context,
            &request.request,
            Arc::clone(&instruction_materialization_store),
            Arc::clone(&context_window_cache),
        )
        .await?;
        ThreadBackedLoopModelPort::new(
            Arc::clone(&self.thread_service),
            self.thread_scope.clone(),
            request.context,
            Arc::clone(&self.host_gateway),
            self.max_messages,
        )
        .with_instruction_materialization_store(instruction_materialization_store)
        .with_context_window_cache(context_window_cache)
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
        instruction_materialization_store: Arc<dyn InstructionMaterializationStore>,
        context_window_cache: Arc<ThreadContextWindowCache>,
    ) -> Result<(), LoopModelGatewayError> {
        let context_port = Arc::new(
            ThreadBackedLoopContextPort::new(
                Arc::clone(&self.thread_service),
                self.thread_scope.clone(),
                context.clone(),
                self.max_messages,
            )
            .with_context_window_cache(context_window_cache),
        );
        let prompt_port = HostManagedLoopPromptPort::new(
            context.clone(),
            context_port,
            Arc::new(InMemoryLoopHostMilestoneSink::default()),
        )
        .with_safety_context(self.safety_context.clone())
        .with_instruction_materialization_store(instruction_materialization_store);
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
        let model_override = request_model_override(route, self.provider.as_ref())?;
        let model_profile_id = request.model_profile_id.clone();
        let run_id = request.run_id;
        let turn_id = request.turn_id;
        let replay_identity = ProviderReplayIdentity::new(&self.provider_id, &model_override)?;
        let mut completion =
            CompletionRequest::new(convert_messages(request.messages, &replay_identity)?);
        completion.model = Some(model_override);
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
        let model_override = request_model_override(route, self.provider.as_ref())?;
        let model_profile_id = request.model_profile_id.clone();
        let run_id = request.run_id;
        let turn_id = request.turn_id;
        let replay_identity = ProviderReplayIdentity::new(&self.provider_id, &model_override)?;
        let mut completion =
            CompletionRequest::new(convert_messages(request.messages, &replay_identity)?);
        completion.model = Some(model_override);
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
    let reason_kind = error.reason_kind;
    let mut converted = match LoopModelGatewayError::new(error.kind, error.safe_summary) {
        Ok(error) => error,
        Err(_) => LoopModelGatewayError {
            kind: error.kind,
            safe_summary: LoopSafeSummary::model_gateway_failed(),
            reason_kind: None,
            diagnostic_ref: None,
        },
    };
    if let Some(reason_kind) = reason_kind {
        converted = converted.with_reason_kind(reason_kind);
    }
    if let Some(diagnostic_ref) = diagnostic_ref {
        converted = converted.with_diagnostic_ref(diagnostic_ref);
    }
    converted
}

fn request_model_override<P>(
    route: &LlmModelProfileRoute,
    provider: &P,
) -> Result<String, HostManagedModelError>
where
    P: LlmProvider + ?Sized,
{
    let model_override = route
        .model_override
        .as_deref()
        .map(str::to_string)
        .unwrap_or_else(|| provider.active_model_name());
    let trimmed = model_override.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("default") {
        return Err(HostManagedModelError::safe(
            HostManagedModelErrorKind::PolicyDenied,
            "model profile route must resolve to a concrete provider model",
        ));
    }
    Ok(trimmed.to_string())
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
            let mut recovery_tool_names = Vec::with_capacity(tool_definitions.len());
            let llm_tool_definitions = tool_definitions
                .into_iter()
                .map(|definition| {
                    recovery_tool_names.push(definition.name.clone());
                    provider_tool_definition_to_llm(definition)
                })
                .collect::<Vec<_>>();
            let tool_request =
                ToolCompletionRequest::from_completion_request(completion, llm_tool_definitions);
            debug!("reborn model gateway dispatching tool-capable provider request");
            let response = provider
                .complete_with_tools(tool_request.clone())
                .await
                .map_err(map_provider_error)?;
            let response =
                recover_textual_tool_calls_from_tool_response(response, &recovery_tool_names)?;
            match tool_response_to_host(
                response.clone(),
                Arc::clone(&capabilities),
                provider_turn_scope
                    .as_deref()
                    .unwrap_or("model_call=unknown"),
                &replay_identity,
            )
            .await
            {
                Ok(response) => return Ok(response),
                Err(error) if is_repairable_provider_tool_output_error(&error) => {
                    debug!(
                        safe_summary = error.safe_summary.as_str(),
                        "reborn model gateway retrying after repairable provider tool output"
                    );
                    let mut repair_request = tool_request;
                    repair_request
                        .messages
                        .extend(provider_tool_repair_messages(
                            &response,
                            error.safe_summary.as_str(),
                        ));
                    let rejected_response = response;
                    let response = provider
                        .complete_with_tools(repair_request)
                        .await
                        .map_err(map_provider_error)?;
                    let mut response = recover_textual_tool_calls_from_tool_response(
                        response,
                        &recovery_tool_names,
                    )?;
                    accumulate_tool_response_usage(&mut response, &rejected_response);
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
                Err(error) => return Err(error),
            }
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

fn accumulate_tool_response_usage(
    response: &mut ToolCompletionResponse,
    additional: &ToolCompletionResponse,
) {
    response.input_tokens = response
        .input_tokens
        .saturating_add(additional.input_tokens);
    response.output_tokens = response
        .output_tokens
        .saturating_add(additional.output_tokens);
    response.cache_read_input_tokens = response
        .cache_read_input_tokens
        .saturating_add(additional.cache_read_input_tokens);
    response.cache_creation_input_tokens = response
        .cache_creation_input_tokens
        .saturating_add(additional.cache_creation_input_tokens);
}

fn recover_textual_tool_calls_from_tool_response(
    response: ToolCompletionResponse,
    tool_names: &[String],
) -> Result<ToolCompletionResponse, HostManagedModelError> {
    if !response.tool_calls.is_empty() {
        return Ok(response);
    }
    let Some(content) = response.content.as_deref() else {
        return Ok(response);
    };
    let recovered_tool_calls = recover_codex_text_tool_calls_from_tool_names(content, tool_names);
    if recovered_tool_calls.is_empty() {
        if contains_codex_text_tool_call_syntax(content) {
            debug!("reborn model gateway rejected unrecovered textual provider tool-call syntax");
            return Err(HostManagedModelError::safe(
                HostManagedModelErrorKind::InvalidOutput,
                "model returned textual tool-call syntax instead of structured tool calls",
            ));
        }
        return Ok(response);
    }

    debug!(
        recovered_tool_call_count = recovered_tool_calls.len(),
        "reborn model gateway recovered capability calls from textual provider response"
    );
    Ok(ToolCompletionResponse {
        content: Some(clean_response(content)),
        tool_calls: recovered_tool_calls,
        input_tokens: response.input_tokens,
        output_tokens: response.output_tokens,
        finish_reason: FinishReason::ToolUse,
        cache_read_input_tokens: response.cache_read_input_tokens,
        cache_creation_input_tokens: response.cache_creation_input_tokens,
        reasoning: response.reasoning,
    })
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
                HostManagedModelErrorKind::InvalidOutput,
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
                .map_err(map_provider_tool_output_error)?;
        }
        for provider_call in provider_calls {
            let candidate = capabilities
                .register_provider_tool_call(provider_call)
                .await
                .map_err(map_provider_tool_output_error)?;
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
        )
        .with_usage(LoopModelUsage {
            input_tokens: response.input_tokens,
            output_tokens: response.output_tokens,
        }));
    }

    match response.finish_reason {
        FinishReason::Stop => {
            let content = clean_response(&response.content.unwrap_or_default());
            if content.trim().is_empty() {
                return Err(HostManagedModelError::safe(
                    HostManagedModelErrorKind::InvalidOutput,
                    "model returned an empty assistant response",
                ));
            }
            debug!(
                content_bytes = content.len(),
                "reborn model gateway classified tool-capable provider response as assistant reply"
            );
            Ok(HostManagedModelResponse::assistant_reply_with_reasoning(
                content,
                response.reasoning,
            )
            .with_usage(LoopModelUsage {
                input_tokens: response.input_tokens,
                output_tokens: response.output_tokens,
            }))
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
            HostManagedModelErrorKind::InvalidOutput,
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
    let usage = LoopModelUsage {
        input_tokens: response.input_tokens,
        output_tokens: response.output_tokens,
    };
    match response.finish_reason {
        FinishReason::Stop => {
            let content = clean_response(&response.content);
            Ok(HostManagedModelResponse::assistant_reply_with_reasoning(
                content,
                response.reasoning,
            )
            .with_usage(usage))
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
            HostManagedModelErrorKind::InvalidOutput,
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

fn map_provider_tool_output_error(error: AgentLoopHostError) -> HostManagedModelError {
    match error.kind {
        AgentLoopHostErrorKind::Invalid | AgentLoopHostErrorKind::InvalidInvocation => {
            HostManagedModelError::safe(
                HostManagedModelErrorKind::InvalidOutput,
                error.safe_summary,
            )
        }
        _ => map_capability_host_error(error),
    }
}

fn is_repairable_provider_tool_output_error(error: &HostManagedModelError) -> bool {
    error.kind == HostManagedModelErrorKind::InvalidOutput
        && is_provider_arguments_too_large_summary(&error.safe_summary)
}

fn provider_tool_repair_messages(
    response: &ToolCompletionResponse,
    safe_summary: &str,
) -> Vec<ChatMessage> {
    if response.tool_calls.is_empty() {
        return Vec::new();
    }

    let assistant = ChatMessage::assistant_with_tool_calls(
        response.content.clone(),
        response
            .tool_calls
            .iter()
            .map(provider_tool_call_for_repair)
            .collect(),
    )
    .with_reasoning(response.reasoning.clone());
    std::iter::once(assistant)
        .chain(response.tool_calls.iter().map(|tool_call| {
            ChatMessage::tool_result(
                tool_call.id.clone(),
                tool_call.name.clone(),
                format!(
                    "Tool call batch rejected by host: {safe_summary}. None of this response's tool calls were executed. Retry with smaller arguments or answer directly without this tool if it is not needed."
                ),
            )
        }))
        .collect()
}

fn provider_tool_call_for_repair(tool_call: &ToolCall) -> ToolCall {
    let arguments = if provider_arguments_exceed_max_bytes(&tool_call.arguments) {
        serde_json::json!({
            "error": PROVIDER_TOOL_ARGUMENTS_OMITTED_MARKER,
        })
    } else {
        tool_call.arguments.clone()
    };

    ToolCall {
        id: tool_call.id.clone(),
        name: tool_call.name.clone(),
        arguments,
        reasoning: tool_call.reasoning.clone(),
        signature: tool_call.signature.clone(),
        arguments_parse_error: tool_call.arguments_parse_error.clone(),
    }
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
                let Some(provider_call) = replay.provider_call.clone() else {
                    converted.push(ChatMessage::user(tool_summary_message(
                        replay.plain_fallback_content(),
                    )));
                    index += 1;
                    continue;
                };
                if !provider_replay_matches_identity(&provider_call, replay_identity) {
                    converted.push(ChatMessage::user(tool_summary_message(
                        replay.plain_fallback_content(),
                    )));
                    index += 1;
                    continue;
                }
                validate_provider_replay_identity(&provider_call, replay_identity)?;
                let provider_turn_id = provider_call.provider_turn_id.clone();
                let mut provider_results = vec![(provider_call, replay.model_content)];
                let mut plain_tool_results = Vec::new();
                index += 1;
                while index < messages.len()
                    && messages[index].role == HostManagedModelMessageRole::ToolResult
                {
                    let next = tool_result_replay_message(&messages[index])?;
                    let Some(next_provider_call) = next.provider_call.clone() else {
                        plain_tool_results.push(next.plain_fallback_content());
                        index += 1;
                        continue;
                    };
                    if !provider_replay_matches_identity(&next_provider_call, replay_identity) {
                        plain_tool_results.push(next.plain_fallback_content());
                        index += 1;
                        continue;
                    }
                    validate_provider_replay_identity(&next_provider_call, replay_identity)?;
                    if next_provider_call.provider_turn_id != provider_turn_id {
                        break;
                    }
                    provider_results.push((next_provider_call, next.model_content));
                    index += 1;
                }
                converted.extend(provider_tool_roundtrip_messages(provider_results));
                converted.extend(
                    plain_tool_results
                        .into_iter()
                        .map(tool_summary_message)
                        .map(ChatMessage::user),
                );
                continue;
            }
        }
        index += 1;
    }
    Ok(coalesce_system_messages_at_start(converted))
}

fn coalesce_system_messages_at_start(messages: Vec<ChatMessage>) -> Vec<ChatMessage> {
    let mut system_content = Vec::new();
    let mut transcript = Vec::with_capacity(messages.len());
    for message in messages {
        if message.role == Role::System {
            system_content.push(message.content);
        } else {
            transcript.push(message);
        }
    }
    if system_content.is_empty() {
        return transcript;
    }

    let mut normalized = Vec::with_capacity(transcript.len() + 1);
    normalized.push(ChatMessage::system(system_content.join("\n\n")));
    normalized.extend(transcript);
    normalized
}

fn tool_summary_message(summary: String) -> String {
    format!("[Tool result summary]: {summary}")
}

fn provider_replay_matches_identity(
    provider_call: &ProviderToolCallReferenceEnvelope,
    expected: &ProviderReplayIdentity,
) -> bool {
    provider_call.provider_id == expected.provider_id
        && provider_call.provider_model_id == expected.provider_model_id
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
    model_content: String,
    model_content_is_plain_fallback_safe: bool,
}

impl ToolResultReplayMessage {
    fn plain_fallback_content(self) -> String {
        if self.model_content_is_plain_fallback_safe {
            self.model_content
        } else {
            self.safe_summary
        }
    }
}

fn tool_result_replay_message(
    message: &HostManagedModelMessage,
) -> Result<ToolResultReplayMessage, HostManagedModelError> {
    let (safe_summary, model_content, model_content_is_plain_fallback_safe) =
        match message.tool_result_content.as_ref() {
            Some(HostManagedToolResultContent::Reference { envelope }) => {
                let safe_summary = envelope.safe_summary.as_str().to_string();
                let model_content = envelope.model_visible_content_or_safe_summary();
                (safe_summary, model_content, true)
            }
            Some(HostManagedToolResultContent::Resolved { safe_summary }) => (
                safe_summary.as_str().to_string(),
                message.content.clone(),
                false,
            ),
            None => {
                return Err(HostManagedModelError::safe(
                    HostManagedModelErrorKind::InvalidRequest,
                    "tool result replay content is missing",
                ));
            }
        };
    Ok(ToolResultReplayMessage {
        provider_call: message.tool_result_provider_call.clone(),
        safe_summary,
        model_content,
        model_content_is_plain_fallback_safe,
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
        arguments_parse_error: None,
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
    if is_credit_exhaustion_error(&error) {
        return HostManagedModelError::safe(
            HostManagedModelErrorKind::CredentialUnavailable,
            MODEL_CREDITS_EXHAUSTED_SUMMARY,
        )
        .with_reason_kind(MODEL_CREDITS_EXHAUSTED_REASON_KIND);
    }
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

fn is_credit_exhaustion_error(error: &LlmError) -> bool {
    let LlmError::RequestFailed { reason, .. } = error else {
        return false;
    };
    let lower = reason.to_ascii_lowercase();
    lower.contains("http 402")
        || lower.contains("402 payment required")
        || lower.contains("payment required")
        || lower.contains("insufficient credit")
        || lower.contains("insufficient credits")
        || lower.contains("not enough credit")
        || lower.contains("not enough credits")
        || lower.contains("credits exhausted")
        || lower.contains("out of credits")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request_failed(reason: &str) -> LlmError {
        LlmError::RequestFailed {
            provider: "test_provider".to_string(),
            reason: reason.to_string(),
        }
    }

    #[test]
    fn is_credit_exhaustion_error_matches_all_trigger_phrases() {
        let phrases = [
            "HTTP 402",
            "402 Payment Required",
            "Payment Required",
            "insufficient credit",
            "insufficient credits",
            "not enough credit",
            "not enough credits",
            "credits exhausted",
            "out of credits",
        ];
        for phrase in &phrases {
            let err = request_failed(&format!("error: {phrase}: some detail"));
            assert!(
                is_credit_exhaustion_error(&err),
                "should match phrase: {phrase}"
            );
        }
        // Case-insensitive
        let err = request_failed("HTTP 402 payment required");
        assert!(is_credit_exhaustion_error(&err), "should match lowercase");
    }

    #[test]
    fn is_credit_exhaustion_error_returns_false_for_non_request_failed_variants() {
        let non_request_failed = [
            LlmError::ContextLengthExceeded {
                used: 1000,
                limit: 500,
            },
            LlmError::ModelNotAvailable {
                provider: "p".to_string(),
                model: "m".to_string(),
            },
            LlmError::AuthFailed {
                provider: "p".to_string(),
            },
            LlmError::SessionExpired {
                provider: "p".to_string(),
            },
        ];
        for err in &non_request_failed {
            assert!(
                !is_credit_exhaustion_error(err),
                "should not match: {err:?}"
            );
        }
    }

    #[test]
    fn is_credit_exhaustion_error_returns_false_for_non_matching_request_failed() {
        let err = request_failed("Internal server error");
        assert!(!is_credit_exhaustion_error(&err));

        let err = request_failed("rate limit exceeded");
        assert!(!is_credit_exhaustion_error(&err));
    }

    #[test]
    fn tool_result_replay_prefers_model_observation_over_safe_summary() {
        let observation = serde_json::json!({
            "schema_version": 1,
            "status": "error",
            "summary": "Tool input failed schema validation.",
            "detail": {
                "kind": "invalid_input",
                "issues": [{
                    "path": "file_path",
                    "code": "missing_required"
                }]
            },
            "trust": "untrusted_tool_output"
        });
        let envelope = ironclaw_threads::ToolResultReferenceEnvelope::with_model_observation(
            "result:tool-error",
            ironclaw_threads::ToolResultSafeSummary::new("tool failed").expect("safe summary"),
            observation.clone(),
        )
        .expect("valid observation envelope");
        let message = HostManagedModelMessage {
            role: HostManagedModelMessageRole::ToolResult,
            content: "tool failed".to_string(),
            content_ref: ironclaw_turns::LoopMessageRef::new(
                "msg:11111111-1111-1111-1111-111111111111",
            )
            .expect("valid message ref"),
            tool_result_provider_call: None,
            tool_result_content: Some(HostManagedToolResultContent::Reference { envelope }),
        };

        let replay = tool_result_replay_message(&message).expect("replay message");

        assert_eq!(replay.safe_summary, "tool failed");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&replay.model_content).unwrap(),
            observation
        );
    }
}
