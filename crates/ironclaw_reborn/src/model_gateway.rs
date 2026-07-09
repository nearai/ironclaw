//! LLM provider-backed Reborn model gateway wiring.
//!
//! The loop-support crate owns the host-facing model gateway contract. This
//! adapter lives in the standalone Reborn composition crate because it bridges
//! that contract to the shared `ironclaw_llm` provider abstraction.

use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};

use async_trait::async_trait;
use ironclaw_host_api::{CapabilityId, ProviderToolName, sha256_digest_token};
use ironclaw_llm::{
    ChatMessage, CompletionRequest, CompletionResponse, CompletionStreamSink, ContentPart,
    FinishReason, ImageUrl, LlmError, LlmProvider, Role, ToolCall, ToolCompletionRequest,
    ToolCompletionResponse, ToolDefinition, clean_response, contains_codex_text_tool_call_syntax,
    costs::{default_cost, model_cost},
    recover_codex_text_tool_calls_from_tool_names,
    vision_models::is_vision_model,
};
use ironclaw_loop_support::{
    HostManagedModelError, HostManagedModelErrorKind, HostManagedModelGateway,
    HostManagedModelMessage, HostManagedModelMessageRole, HostManagedModelRequest,
    HostManagedModelResponse, HostManagedModelRouteSnapshot, HostManagedModelStreamSink,
    HostManagedToolResultContent, ModelCost, StaticModelCostTable, ThreadBackedLoopContextPort,
    ThreadBackedLoopModelPort, ThreadContextWindowCache,
};
use ironclaw_observability::live_latency_started_at;
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
        LoopModelGatewayError, LoopModelGatewayRequest, LoopModelPort, LoopModelProgressSink,
        LoopModelRequest, LoopModelResponse, LoopPromptBundleRequest, LoopPromptPort,
        LoopRunContext, LoopSafeSummary, ModelProfileId, PromptMode, ProviderToolCall,
        ProviderToolDefinition, RegisterProviderToolCallRequest, sanitize_model_visible_text,
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
const CONTEXT_SHADOW_TARGET: &str = "ironclaw::reborn::context_shadow";
const UNAVAILABLE_CAPABILITY_REPLY: &str = "That capability is unavailable or disabled for this request, so I will not route it through another tool.";

fn trace_model_latency_ok(
    operation: &'static str,
    replay_identity: &ProviderReplayIdentity,
    provider_turn_scope: Option<&str>,
    started_at: Option<Instant>,
) {
    ironclaw_observability::live_latency_trace_ok!(
        "model_gateway",
        operation,
        started_at,
        provider_id = %replay_identity.provider_id,
        provider_model_id = %replay_identity.provider_model_id,
        provider_turn_scope = provider_turn_scope.unwrap_or(""),
        "model gateway operation completed",
    );
}

fn trace_model_latency_error<E: ?Sized>(
    operation: &'static str,
    replay_identity: &ProviderReplayIdentity,
    provider_turn_scope: Option<&str>,
    started_at: Option<Instant>,
    _error: &E,
) {
    ironclaw_observability::live_latency_trace_error!(
        "model_gateway",
        operation,
        started_at,
        "model_gateway_error",
        provider_id = %replay_identity.provider_id,
        provider_model_id = %replay_identity.provider_model_id,
        provider_turn_scope = provider_turn_scope.unwrap_or(""),
        "model gateway operation failed",
    );
}

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
        self.stream_model_inner(request, None).await
    }

    async fn stream_model_with_progress(
        &self,
        request: LoopModelGatewayRequest,
        progress_sink: Arc<dyn LoopModelProgressSink>,
    ) -> Result<LoopModelResponse, LoopModelGatewayError> {
        self.stream_model_inner(request, Some(progress_sink)).await
    }
}

impl<S, G> ThreadBackedLoopModelGateway<S, G>
where
    S: SessionThreadService + ?Sized + Send + Sync,
    G: HostManagedModelGateway + ?Sized + Send + Sync,
{
    async fn stream_model_inner(
        &self,
        request: LoopModelGatewayRequest,
        progress_sink: Option<Arc<dyn LoopModelProgressSink>>,
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
        let mut port = ThreadBackedLoopModelPort::new(
            Arc::clone(&self.thread_service),
            self.thread_scope.clone(),
            request.context,
            Arc::clone(&self.host_gateway),
            self.max_messages,
        )
        .with_instruction_materialization_store(instruction_materialization_store)
        .with_context_window_cache(context_window_cache);
        if let Some(progress_sink) = progress_sink {
            port = port.with_stream_sink(Arc::new(LoopProgressHostStreamSink {
                inner: progress_sink,
            }));
        }
        port.stream_model(request.request)
            .await
            .map_err(host_error_to_model_gateway_error)
    }
}

struct LoopProgressHostStreamSink {
    inner: Arc<dyn LoopModelProgressSink>,
}

#[async_trait]
impl HostManagedModelStreamSink for LoopProgressHostStreamSink {
    async fn safe_text_update(&self, safe_text: String) {
        self.inner.model_text_update(safe_text).await;
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
                inline_messages: request.inline_messages.clone(),
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
            None,
            replay_identity,
        )
        .await
    }

    async fn stream_model_with_progress(
        &self,
        request: HostManagedModelRequest,
        sink: Arc<dyn HostManagedModelStreamSink>,
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
            Some(sink),
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
            None,
            replay_identity,
        )
        .await
    }

    async fn stream_model_with_capabilities_and_progress(
        &self,
        request: HostManagedModelRequest,
        capabilities: Arc<dyn ironclaw_turns::run_profile::LoopCapabilityPort>,
        sink: Arc<dyn HostManagedModelStreamSink>,
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
            Some(sink),
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

        complete_model_request(
            provider.as_ref(),
            completion,
            None,
            None,
            None,
            replay_identity,
        )
        .await
    }

    async fn stream_model_with_progress(
        &self,
        request: HostManagedModelRequest,
        sink: Arc<dyn HostManagedModelStreamSink>,
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

        complete_model_request(
            provider.as_ref(),
            completion,
            None,
            None,
            Some(sink),
            replay_identity,
        )
        .await
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
            None,
            replay_identity,
        )
        .await
    }

    async fn stream_model_with_capabilities_and_progress(
        &self,
        request: HostManagedModelRequest,
        capabilities: Arc<dyn ironclaw_turns::run_profile::LoopCapabilityPort>,
        sink: Arc<dyn HostManagedModelStreamSink>,
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
            Some(sink),
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
    let gate_ref = error.gate_ref;
    let mut converted = match LoopModelGatewayError::new(error.kind, error.safe_summary) {
        Ok(error) => error,
        Err(_) => LoopModelGatewayError {
            kind: error.kind,
            safe_summary: LoopSafeSummary::model_gateway_failed(),
            reason_kind: None,
            gate_ref: None,
            diagnostic_ref: None,
        },
    };
    if let Some(reason_kind) = reason_kind {
        converted = converted.with_reason_kind(reason_kind);
    }
    if let Some(gate_ref) = gate_ref {
        converted = converted.with_gate_ref(gate_ref);
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

struct ProviderStreamSink {
    inner: Arc<dyn HostManagedModelStreamSink>,
    accumulated_text: Mutex<String>,
}

impl ProviderStreamSink {
    fn new(inner: Arc<dyn HostManagedModelStreamSink>) -> Self {
        Self {
            inner,
            accumulated_text: Mutex::new(String::new()),
        }
    }
}

#[async_trait]
impl CompletionStreamSink for ProviderStreamSink {
    async fn text_delta(&self, delta: String) {
        if delta.is_empty() {
            return;
        }
        let safe_text = {
            let mut guard = match self.accumulated_text.lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            guard.push_str(&delta);
            sanitize_model_visible_text(guard.clone())
        };
        self.inner.safe_text_update(safe_text).await;
    }
}

#[tracing::instrument(
    level = "debug",
    skip(provider, completion, capabilities, stream_sink, replay_identity),
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
    stream_sink: Option<Arc<dyn HostManagedModelStreamSink>>,
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
        if tracing::enabled!(target: CONTEXT_SHADOW_TARGET, tracing::Level::DEBUG) {
            let est_tool_schema_tokens = estimate_tool_schema_tokens(&tool_definitions);
            debug!(
                target: CONTEXT_SHADOW_TARGET,
                tool_definition_count = tool_definitions.len(),
                est_tool_schema_tokens,
                "reborn tool surface shadow measurement"
            );
        }
        if !tool_definitions.is_empty() {
            let unavailable_capability_guard =
                unavailable_requested_capability_guard(&completion.messages, &tool_definitions);
            let mut recovery_tool_names = Vec::with_capacity(tool_definitions.len());
            let llm_tool_definitions = tool_definitions
                .into_iter()
                .map(|definition| {
                    recovery_tool_names.push(definition.name.as_str().to_string());
                    provider_tool_definition_to_llm(definition)
                })
                .collect::<Vec<_>>();
            let tool_request =
                ToolCompletionRequest::from_completion_request(completion, llm_tool_definitions);
            debug!("reborn model gateway dispatching tool-capable provider request");
            let provider_started_at = live_latency_started_at();
            let response = match if let Some(stream_sink) = stream_sink.as_ref() {
                provider
                    .complete_with_tools_streaming(
                        tool_request.clone(),
                        Arc::new(ProviderStreamSink::new(Arc::clone(stream_sink))),
                    )
                    .await
            } else {
                provider.complete_with_tools(tool_request.clone()).await
            } {
                Ok(response) => {
                    trace_model_latency_ok(
                        "provider_complete_with_tools",
                        &replay_identity,
                        provider_turn_scope.as_deref(),
                        provider_started_at,
                    );
                    response
                }
                Err(error) => {
                    trace_model_latency_error(
                        "provider_complete_with_tools",
                        &replay_identity,
                        provider_turn_scope.as_deref(),
                        provider_started_at,
                        &error,
                    );
                    return Err(map_provider_error(error));
                }
            };
            let response =
                recover_textual_tool_calls_from_tool_response(response, &recovery_tool_names)?;
            let host_response_started_at = live_latency_started_at();
            match tool_response_to_host(
                response.clone(),
                Arc::clone(&capabilities),
                provider_turn_scope
                    .as_deref()
                    .unwrap_or("model_call=unknown"),
                &replay_identity,
                unavailable_capability_guard.as_ref(),
            )
            .await
            {
                Ok(response) => {
                    trace_model_latency_ok(
                        "tool_response_to_host",
                        &replay_identity,
                        provider_turn_scope.as_deref(),
                        host_response_started_at,
                    );
                    return Ok(response);
                }
                Err(error) if is_repairable_provider_tool_output_error(&error) => {
                    trace_model_latency_error(
                        "tool_response_to_host",
                        &replay_identity,
                        provider_turn_scope.as_deref(),
                        host_response_started_at,
                        &error,
                    );
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
                    let retry_started_at = live_latency_started_at();
                    let response = match provider.complete_with_tools(repair_request).await {
                        Ok(response) => {
                            trace_model_latency_ok(
                                "provider_complete_with_tools_repair",
                                &replay_identity,
                                provider_turn_scope.as_deref(),
                                retry_started_at,
                            );
                            response
                        }
                        Err(error) => {
                            trace_model_latency_error(
                                "provider_complete_with_tools_repair",
                                &replay_identity,
                                provider_turn_scope.as_deref(),
                                retry_started_at,
                                &error,
                            );
                            return Err(map_provider_error(error));
                        }
                    };
                    let mut response = recover_textual_tool_calls_from_tool_response(
                        response,
                        &recovery_tool_names,
                    )?;
                    accumulate_tool_response_usage(&mut response, &rejected_response);
                    let repair_host_started_at = live_latency_started_at();
                    let result = tool_response_to_host(
                        response,
                        capabilities,
                        provider_turn_scope
                            .as_deref()
                            .unwrap_or("model_call=unknown"),
                        &replay_identity,
                        unavailable_capability_guard.as_ref(),
                    )
                    .await;
                    match &result {
                        Ok(_) => trace_model_latency_ok(
                            "tool_response_to_host_repair",
                            &replay_identity,
                            provider_turn_scope.as_deref(),
                            repair_host_started_at,
                        ),
                        Err(error) => trace_model_latency_error(
                            "tool_response_to_host_repair",
                            &replay_identity,
                            provider_turn_scope.as_deref(),
                            repair_host_started_at,
                            error,
                        ),
                    }
                    return result;
                }
                Err(error) => {
                    trace_model_latency_error(
                        "tool_response_to_host",
                        &replay_identity,
                        provider_turn_scope.as_deref(),
                        host_response_started_at,
                        &error,
                    );
                    return Err(error);
                }
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

    let provider_started_at = live_latency_started_at();
    let response = match if let Some(stream_sink) = stream_sink.as_ref() {
        provider
            .complete_streaming(
                completion,
                Arc::new(ProviderStreamSink::new(Arc::clone(stream_sink))),
            )
            .await
    } else {
        provider.complete(completion).await
    } {
        Ok(response) => {
            trace_model_latency_ok(
                "provider_complete",
                &replay_identity,
                provider_turn_scope.as_deref(),
                provider_started_at,
            );
            response
        }
        Err(error) => {
            trace_model_latency_error(
                "provider_complete",
                &replay_identity,
                provider_turn_scope.as_deref(),
                provider_started_at,
                &error,
            );
            return Err(map_provider_error(error));
        }
    };
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
        reasoning_details: response.reasoning_details,
    })
}

fn provider_tool_definition_to_llm(definition: ProviderToolDefinition) -> ToolDefinition {
    ToolDefinition {
        name: definition.name.into_string(),
        description: definition.description,
        parameters: definition.parameters,
    }
}

fn estimate_tool_schema_tokens(definitions: &[ProviderToolDefinition]) -> u32 {
    definitions.iter().fold(0_u32, |total, definition| {
        let schema = serde_json::json!({
            "name": definition.name.as_str(),
            "description": definition.description.as_str(),
            "parameters": &definition.parameters,
        });
        total.saturating_add(crate::context_shadow::estimate_tokens(&schema.to_string()))
    })
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
    unavailable_capability_guard: Option<&UnavailableCapabilityGuard>,
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
        if let Some(guard) = unavailable_capability_guard {
            debug!(
                requested_capability_id = %guard.capability_id,
                tool_call_count = response.tool_calls.len(),
                "reborn model gateway suppressed provider tool calls after unavailable named capability request"
            );
            return Ok(HostManagedModelResponse::assistant_reply_with_reasoning(
                UNAVAILABLE_CAPABILITY_REPLY,
                response.reasoning,
            )
            .with_usage(LoopModelUsage {
                input_tokens: response.input_tokens,
                output_tokens: response.output_tokens,
            }));
        }
        let advertised_tool_names = capabilities
            .tool_definitions()
            .map_err(map_capability_host_error)?
            .into_iter()
            .map(|definition| definition.name)
            .collect::<HashSet<_>>();
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
            .collect::<Result<Vec<_>, HostManagedModelError>>()?;
        if !provider_calls_are_advertised_or_resolvable(
            &advertised_tool_names,
            capabilities.as_ref(),
            &provider_calls,
        ) {
            return Err(HostManagedModelError::safe(
                HostManagedModelErrorKind::InvalidOutput,
                "model returned a tool call outside the advertised capability surface",
            ));
        }
        for provider_call in &provider_calls {
            if let Err(error) = capabilities.validate_provider_tool_call(provider_call) {
                // Fail loud: this rejection otherwise discards the whole response
                // (budget released, no dispatch) and the run eventually fails with
                // no trace of which call or why. Log before mapping/propagating.
                debug!(
                    tool_name = provider_call.name.as_str(),
                    provider_call_id = provider_call.id.as_str(),
                    error_kind = ?error.kind,
                    // The safe_summary is layer-distinct ("outside the
                    // model-visible capability view" = visible filter, "targets a
                    // disabled capability" = deny filter, etc.), so it names which
                    // port in the chain rejected the call.
                    reason = error.safe_summary.as_str(),
                    "reborn model gateway rejected provider tool call during validation"
                );
                return Err(map_provider_tool_output_error(error));
            }
        }
        for provider_call in provider_calls {
            let rejected_tool_name = provider_call.name.clone();
            let rejected_provider_call_id = provider_call.id.clone();
            match capabilities
                .register_provider_tool_call(RegisterProviderToolCallRequest::new(provider_call))
                .await
            {
                Ok(candidate) => candidates.push(candidate),
                Err(error) => {
                    debug!(
                        tool_name = rejected_tool_name.as_str(),
                        provider_call_id = rejected_provider_call_id.as_str(),
                        error_kind = ?error.kind,
                        reason = error.safe_summary.as_str(),
                        "reborn model gateway rejected provider tool call during registration"
                    );
                    return Err(map_provider_tool_output_error(error));
                }
            }
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

fn provider_calls_are_advertised_or_resolvable(
    advertised_tool_names: &HashSet<ProviderToolName>,
    capabilities: &dyn ironclaw_turns::run_profile::LoopCapabilityPort,
    provider_calls: &[ProviderToolCall],
) -> bool {
    for provider_call in provider_calls {
        if advertised_tool_names.contains(&provider_call.name) {
            continue;
        }
        match capabilities.provider_tool_call_capability_ids(provider_call) {
            Ok(ids) => {
                debug!(
                    tool_name = provider_call.name.as_str(),
                    provider_capability_id = ids.provider_capability_id.as_str(),
                    "reborn model gateway accepted resolvable unadvertised provider tool call"
                );
            }
            Err(error) => {
                debug!(
                    tool_name = provider_call.name.as_str(),
                    safe_summary = error.safe_summary.as_str(),
                    "reborn model gateway rejected unresolved unadvertised provider tool call"
                );
                return false;
            }
        }
    }
    true
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct UnavailableCapabilityGuard {
    capability_id: CapabilityId,
}

fn unavailable_requested_capability_guard(
    messages: &[ChatMessage],
    tool_definitions: &[ProviderToolDefinition],
) -> Option<UnavailableCapabilityGuard> {
    let latest_user = messages
        .iter()
        .rev()
        .find(|message| message.role == Role::User)?;
    let visible_capability_ids = tool_definitions
        .iter()
        .map(|definition| definition.capability_id.as_str())
        .collect::<HashSet<_>>();
    // Namespaces the agent actually has (a visible capability shares the prefix,
    // e.g. `builtin`). Used only to rescue backticked references to REAL
    // capability namespaces from the inline-code skip — a backticked `builtin.echo`
    // is still a request, whereas a backticked `playwright.sync_api` (a library
    // whose namespace this agent doesn't have) is a code reference.
    let visible_namespaces = visible_capability_ids
        .iter()
        .filter_map(|id| id.split('.').next())
        .collect::<HashSet<_>>();

    extract_explicit_capability_request_ids(&latest_user.content, &visible_namespaces)
        .into_iter()
        .find(|capability_id| !visible_capability_ids.contains(capability_id.as_str()))
        .map(|capability_id| UnavailableCapabilityGuard { capability_id })
}

fn extract_explicit_capability_request_ids(
    content: &str,
    visible_namespaces: &HashSet<&str>,
) -> Vec<CapabilityId> {
    let mut ids = Vec::new();
    let mut token_start = None;
    // Track Markdown inline-code parity (per line) in this same single pass so we
    // never rescan the line for each token — one long user line with many
    // capability-shaped tokens would otherwise be O(n^2).
    let mut in_inline_code = false;
    let mut token_in_code = false;
    for (index, character) in content.char_indices() {
        if is_capability_token_char(character) {
            if token_start.is_none() {
                token_start = Some(index);
                token_in_code = in_inline_code;
            }
            continue;
        }
        if let Some(start) = token_start.take() {
            push_explicit_capability_request_token(
                content,
                start,
                index,
                token_in_code,
                visible_namespaces,
                &mut ids,
            );
        }
        match character {
            '\n' => in_inline_code = false,
            '`' => in_inline_code = !in_inline_code,
            _ => {}
        }
    }
    if let Some(start) = token_start {
        push_explicit_capability_request_token(
            content,
            start,
            content.len(),
            token_in_code,
            visible_namespaces,
            &mut ids,
        );
    }
    ids
}

fn is_capability_token_char(character: char) -> bool {
    character.is_ascii_lowercase()
        || character.is_ascii_digit()
        || matches!(character, '_' | '-' | '.')
}

fn push_explicit_capability_request_token(
    content: &str,
    start: usize,
    end: usize,
    in_inline_code: bool,
    visible_namespaces: &HashSet<&str>,
    ids: &mut Vec<CapabilityId>,
) {
    let token = &content[start..end];
    if !is_likely_capability_reference(token)
        || !is_explicit_capability_request_token(content, start, end)
    {
        return;
    }
    // Tokens written in Markdown inline code (e.g. "use `playwright.sync_api`", a
    // Python module) are code references, not capability requests — ignore them.
    // Two exceptions keep genuine requests covered even when backticked:
    //  - the prompt explicitly labels the token a tool/capability
    //    ("use the `builtin.http` capability"), or
    //  - the token names a real capability namespace this agent has
    //    (`builtin.echo` — `builtin` is a live namespace, unlike `playwright`).
    if in_inline_code
        && !has_capability_noun_context(content, start, end)
        && !token_namespace_is_visible(token, visible_namespaces)
    {
        return;
    }
    if let Ok(capability_id) = CapabilityId::new(token)
        && !ids.iter().any(|existing| existing == &capability_id)
    {
        ids.push(capability_id);
    }
}

fn is_likely_capability_reference(token: &str) -> bool {
    token.starts_with("builtin.") || token.split('.').count() == 2
}

/// True when the token's namespace (its first dotted segment) is one the agent
/// actually has — a backticked reference to a real capability namespace is still
/// a request, unlike a library reference (`playwright.sync_api`).
fn token_namespace_is_visible(token: &str, visible_namespaces: &HashSet<&str>) -> bool {
    token
        .split('.')
        .next()
        .is_some_and(|namespace| visible_namespaces.contains(namespace))
}

/// The request-word immediately before `start` (alphanumeric/`_`/`-` run).
fn previous_request_word(content: &str, start: usize) -> Option<&str> {
    content[..start]
        .trim_end()
        .rsplit(|character: char| !is_capability_request_word_char(character))
        .find(|word| !word.is_empty())
}

/// True when the word right before or after the token is an explicit "tool" /
/// "capability" noun — the prompt is calling the token out as a capability, so
/// it's a genuine request even when written in backticks.
fn has_capability_noun_context(content: &str, start: usize, end: usize) -> bool {
    let next_word = content[end..]
        .trim_start()
        .split(|character: char| !is_capability_request_word_char(character))
        .find(|word| !word.is_empty());
    previous_request_word(content, start).is_some_and(is_capability_request_noun)
        || next_word.is_some_and(is_capability_request_noun)
}

fn is_explicit_capability_request_token(content: &str, start: usize, end: usize) -> bool {
    previous_request_word(content, start).is_some_and(is_capability_request_verb)
        || has_capability_noun_context(content, start, end)
}

fn is_capability_request_word_char(character: char) -> bool {
    character.is_ascii_alphanumeric() || matches!(character, '_' | '-')
}

fn is_capability_request_verb(word: &str) -> bool {
    matches!(
        word.to_ascii_lowercase().as_str(),
        "use" | "using" | "call" | "run" | "execute" | "invoke"
    )
}

fn is_capability_request_noun(word: &str) -> bool {
    matches!(word.to_ascii_lowercase().as_str(), "tool" | "capability")
}

fn provider_tool_call_from_llm(
    tool_call: ToolCall,
    response_reasoning: Option<String>,
    provider_turn_id: String,
    replay_identity: &ProviderReplayIdentity,
) -> Result<ProviderToolCall, HostManagedModelError> {
    let name = ProviderToolName::new(tool_call.name).map_err(|error| {
        debug!(%error, "reborn model gateway rejected invalid provider tool name");
        HostManagedModelError::safe(
            HostManagedModelErrorKind::InvalidOutput,
            "model returned an invalid provider tool name",
        )
    })?;
    Ok(ProviderToolCall {
        provider_id: replay_identity.provider_id.clone(),
        provider_model_id: replay_identity.provider_model_id.clone(),
        turn_id: Some(provider_turn_id),
        id: tool_call.id,
        name,
        arguments: tool_call.arguments,
        response_reasoning,
        reasoning: tool_call.reasoning,
        signature: tool_call.signature,
    })
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
        AgentLoopHostErrorKind::BudgetExceeded | AgentLoopHostErrorKind::BudgetAccountingFailed => {
            HostManagedModelErrorKind::BudgetExceeded
        }
        AgentLoopHostErrorKind::BudgetApprovalRequired => {
            HostManagedModelErrorKind::BudgetApprovalRequired
        }
        AgentLoopHostErrorKind::Cancelled => HostManagedModelErrorKind::Cancelled,
        AgentLoopHostErrorKind::Invalid
        | AgentLoopHostErrorKind::InvalidInvocation
        | AgentLoopHostErrorKind::ScopeMismatch
        | AgentLoopHostErrorKind::StaleSurface => HostManagedModelErrorKind::InvalidRequest,
        AgentLoopHostErrorKind::Unavailable
        | AgentLoopHostErrorKind::InvalidOutput
        | AgentLoopHostErrorKind::CheckpointRejected
        | AgentLoopHostErrorKind::TranscriptWriteFailed
        | AgentLoopHostErrorKind::Internal => HostManagedModelErrorKind::Unavailable,
    };
    let mut converted = HostManagedModelError::safe(kind, error.safe_summary);
    if let Some(gate_ref) = error.gate_ref {
        converted = converted.with_gate_ref(gate_ref);
    }
    converted
}

fn map_provider_tool_output_error(error: AgentLoopHostError) -> HostManagedModelError {
    match error.kind {
        AgentLoopHostErrorKind::Invalid
        | AgentLoopHostErrorKind::InvalidInvocation
        | AgentLoopHostErrorKind::InvalidOutput => HostManagedModelError::safe(
            HostManagedModelErrorKind::InvalidOutput,
            error.safe_summary,
        ),
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
    .with_reasoning_details(response.reasoning_details.clone())
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

/// Encode raw image bytes as a base64 `data:` URL a vision model can read
/// inline. The model port carries undecorated bytes; this provider-format
/// concern lives at the gateway boundary.
fn image_data_url(mime_type: &str, bytes: &[u8]) -> String {
    use base64::Engine;
    format!(
        "data:{};base64,{}",
        mime_type,
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
}

/// Env flag gating [`collapse_repeated_failure_observations`].
///
/// Defaults **off**: unset / empty / unrecognized leaves the replayed context
/// byte-identical to the pre-feature path. An operator opts in with `on`, `1`,
/// or `true`. Kept as a separate knob from `REBORN_TOOL_DISCLOSURE` because this
/// context-dedup pass runs in the shared `convert_messages` path independently of
/// tool disclosure.
pub const REBORN_COLLAPSE_REPEATED_FAILURES_ENV: &str = "REBORN_COLLAPSE_REPEATED_FAILURES";

fn collapse_repeated_failures_enabled() -> bool {
    collapse_repeated_failures_from_raw(std::env::var(REBORN_COLLAPSE_REPEATED_FAILURES_ENV).ok())
}

/// Pure resolution of the collapse flag from a raw env value, so the default-off
/// contract is testable without mutating process env.
fn collapse_repeated_failures_from_raw(raw: Option<impl AsRef<str>>) -> bool {
    match raw {
        Some(value) => {
            let value = value.as_ref().trim();
            value.eq_ignore_ascii_case("on")
                || value.eq_ignore_ascii_case("1")
                || value.eq_ignore_ascii_case("true")
        }
        None => false,
    }
}

/// Collapse runs of identical *error* tool observations in the replayed context.
///
/// A model that repeats the same failing call accumulates byte-for-byte identical
/// error observations — one per attempt — and every one is replayed into every
/// later prompt. That both bloats context and drowns the model in copies of its
/// own failure so it cannot tell it is looping. Keep the FIRST and LAST occurrence
/// of each identical error intact (first for original detail, last because it is
/// most recent and carries any repair hints) and replace the ones in between with
/// a compact marker. Nothing is dropped — every tool-result message stays, so
/// provider tool-call/result pairing is preserved; only the observation *content*
/// of interior duplicates shrinks. Success observations and a lone repeat are
/// never touched (the 3+ threshold leaves the first/last-only case alone).
fn collapse_repeated_failure_observations(messages: &mut [HostManagedModelMessage]) {
    let mut occurrences: std::collections::HashMap<String, Vec<usize>> =
        std::collections::HashMap::new();
    for (index, message) in messages.iter().enumerate() {
        if let Some(HostManagedToolResultContent::Reference { envelope }) =
            message.tool_result_content.as_ref()
            && let Some(fingerprint) = envelope.error_observation_fingerprint()
        {
            occurrences.entry(fingerprint).or_default().push(index);
        }
    }
    for indices in occurrences.values() {
        if indices.len() < 3 {
            continue;
        }
        for &index in &indices[1..indices.len() - 1] {
            if let Some(HostManagedToolResultContent::Reference { envelope }) =
                messages[index].tool_result_content.as_mut()
            {
                envelope.collapse_to_repeated_error_marker();
            }
        }
    }
}

fn convert_messages(
    mut messages: Vec<HostManagedModelMessage>,
    replay_identity: &ProviderReplayIdentity,
) -> Result<Vec<ChatMessage>, HostManagedModelError> {
    // Off by default (see REBORN_COLLAPSE_REPEATED_FAILURES_ENV): only collapse
    // interior duplicate error observations when an operator opts in, so the
    // replayed context is otherwise byte-identical to the pre-feature path.
    if collapse_repeated_failures_enabled() {
        collapse_repeated_failure_observations(&mut messages);
    }
    let mut converted = Vec::with_capacity(messages.len());
    let mut index = 0;
    while index < messages.len() {
        let message = &messages[index];
        match message.role {
            HostManagedModelMessageRole::System => {
                converted.push(ChatMessage::system(message.content.clone()))
            }
            HostManagedModelMessageRole::User => {
                // Attach images only for a vision-capable model. A text-only
                // model can't accept image parts (it would error or ignore
                // them), so it keeps just the text — the durable transcript
                // still carries the `<attachments>` pointer for those models.
                let vision = is_vision_model(&replay_identity.provider_model_id);
                if message.image_parts.is_empty() || !vision {
                    converted.push(ChatMessage::user(message.content.clone()));
                } else {
                    // Multimodal: the text rides in `content`; `content_parts`
                    // carries only the image parts (the provider adapters
                    // prepend the text). Encoding to a base64 `data:` URL is a
                    // provider-format concern, so it happens here at the gateway
                    // — the model port carries only the raw bytes.
                    let parts = message
                        .image_parts
                        .iter()
                        .map(|image| ContentPart::ImageUrl {
                            image_url: ImageUrl {
                                url: image_data_url(&image.mime_type, &image.bytes),
                                detail: None,
                            },
                        })
                        .collect();
                    converted.push(ChatMessage::user_with_parts(message.content.clone(), parts));
                }
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
                        provider_call.provider_tool_name.into_string(),
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
        name: provider_call.provider_tool_name.as_str().to_string(),
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
    // Tier 2b: carry the provider's real message (status line + body snippet)
    // on the model-visible detail channel so the failure explainer can describe
    // the actual fault. `safe_with_detail` scrubs credential-looking tokens
    // (api_key=…, sk-…, access_token=…) before the text is stored; the safe
    // summary stays a fixed host-authored category string.
    let provider_detail = error.to_string();
    if is_credit_exhaustion_error(&error) {
        return HostManagedModelError::safe(
            HostManagedModelErrorKind::CredentialUnavailable,
            MODEL_CREDITS_EXHAUSTED_SUMMARY,
        )
        .with_reason_kind(MODEL_CREDITS_EXHAUSTED_REASON_KIND)
        .safe_with_detail(provider_detail.clone());
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
    .safe_with_detail(provider_detail)
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

    fn tool_def(capability_id: &str, name: &str) -> ProviderToolDefinition {
        ProviderToolDefinition {
            capability_id: CapabilityId::new(capability_id).unwrap(),
            name: ProviderToolName::new(name).unwrap(),
            description: String::new(),
            parameters: serde_json::json!({}),
        }
    }

    #[test]
    fn guard_ignores_incidental_code_references() {
        // The playwright/browser tasks literally instruct: "use `playwright.sync_api`"
        // — a Python module named right after a request verb. That is NOT a
        // capability request; the guard must not fire and suppress the model's
        // legitimate write_file calls.
        let messages = vec![ChatMessage::user(
            "Read form.html, then use `playwright.sync_api` (Python sync API) to \
             write an end-to-end test saved as test_form.py.",
        )];
        let tools = vec![
            tool_def("builtin.write_file", "builtin__write_file"),
            tool_def("builtin.read_file", "builtin__read_file"),
        ];
        assert!(
            unavailable_requested_capability_guard(&messages, &tools).is_none(),
            "guard must not misfire on the code reference `playwright.sync_api`"
        );
    }

    #[test]
    fn guard_still_fires_on_real_disabled_capability() {
        // A genuine, un-backticked request for a capability that isn't visible must
        // still fire (`builtin.http` is gated off here).
        let messages = vec![ChatMessage::user(
            "Fetch the page using the builtin.http capability.",
        )];
        let tools = vec![tool_def("builtin.write_file", "builtin__write_file")];
        let guard = unavailable_requested_capability_guard(&messages, &tools);
        assert!(
            guard.is_some(),
            "guard should still fire for a real builtin capability that is disabled"
        );
        assert_eq!(guard.unwrap().capability_id.as_str(), "builtin.http");
    }

    #[test]
    fn guard_fires_on_backticked_capability_with_explicit_noun() {
        // Backticks alone don't excuse a request the prompt explicitly labels a
        // capability/tool — the inline-code skip must not swallow a genuine
        // request. Here `builtin.http` is backticked but called a "capability".
        let messages = vec![ChatMessage::user(
            "Fetch the page using the `builtin.http` capability.",
        )];
        let tools = vec![tool_def("builtin.write_file", "builtin__write_file")];
        let guard = unavailable_requested_capability_guard(&messages, &tools);
        assert!(
            guard.is_some(),
            "explicitly-labeled capability must still fire even when backticked"
        );
        assert_eq!(guard.unwrap().capability_id.as_str(), "builtin.http");
    }

    #[test]
    fn guard_fires_on_backticked_known_namespace_capability() {
        // A backticked reference to a REAL capability namespace this agent has
        // (`builtin`) is still a request, even with only a request verb and no
        // tool/capability noun — unlike a library ref such as `playwright.sync_api`.
        let messages = vec![ChatMessage::user("Use `builtin.echo` to print the banner.")];
        let tools = vec![tool_def("builtin.write_file", "builtin__write_file")];
        let guard = unavailable_requested_capability_guard(&messages, &tools);
        assert!(
            guard.is_some(),
            "backticked known-namespace capability must still fire"
        );
        assert_eq!(guard.unwrap().capability_id.as_str(), "builtin.echo");
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
            image_parts: Vec::new(),
        };

        let replay = tool_result_replay_message(&message).expect("replay message");

        assert_eq!(replay.safe_summary, "tool failed");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&replay.model_content).unwrap(),
            observation
        );
    }

    fn error_tool_result_message(
        result_ref: &str,
        observation: serde_json::Value,
    ) -> HostManagedModelMessage {
        let envelope = ironclaw_threads::ToolResultReferenceEnvelope::with_model_observation(
            result_ref,
            ironclaw_threads::ToolResultSafeSummary::new("tool failed").expect("safe summary"),
            observation,
        )
        .expect("valid observation envelope");
        HostManagedModelMessage {
            role: HostManagedModelMessageRole::ToolResult,
            content: "tool failed".to_string(),
            content_ref: ironclaw_turns::LoopMessageRef::new(
                "msg:11111111-1111-1111-1111-111111111111",
            )
            .expect("valid message ref"),
            tool_result_provider_call: None,
            tool_result_content: Some(HostManagedToolResultContent::Reference { envelope }),
            image_parts: Vec::new(),
        }
    }

    fn tool_result_observation(message: &HostManagedModelMessage) -> serde_json::Value {
        match message
            .tool_result_content
            .as_ref()
            .expect("tool result content")
        {
            HostManagedToolResultContent::Reference { envelope } => envelope
                .model_observation
                .clone()
                .expect("model observation"),
            other => panic!("expected reference, got {other:?}"),
        }
    }

    fn generic_error_observation() -> serde_json::Value {
        serde_json::json!({
            "schema_version": 1,
            "status": "error",
            "summary": "Capability failed with invalid_input.",
            "detail": {"kind": "generic_failure", "failure_kind": "invalid_input"},
            "trust": "untrusted_tool_output",
        })
    }

    #[test]
    fn collapse_repeated_failures_flag_defaults_off_and_opts_in_explicitly() {
        // Unset / empty / unrecognized => off (byte-identical replayed context).
        assert!(!collapse_repeated_failures_from_raw(None::<&str>));
        assert!(!collapse_repeated_failures_from_raw(Some("")));
        assert!(!collapse_repeated_failures_from_raw(Some("off")));
        assert!(!collapse_repeated_failures_from_raw(Some("garbage")));
        // Explicit truthy values opt in.
        assert!(collapse_repeated_failures_from_raw(Some("on")));
        assert!(collapse_repeated_failures_from_raw(Some("1")));
        assert!(collapse_repeated_failures_from_raw(Some("true")));
        assert!(collapse_repeated_failures_from_raw(Some(" TRUE ")));
    }

    #[test]
    fn collapse_repeated_failure_observations_keeps_first_and_last_only() {
        let error_obs = generic_error_observation();
        let success_obs = serde_json::json!({
            "schema_version": 1,
            "status": "success",
            "summary": "ok",
            "detail": {"kind": "generic_failure", "failure_kind": "none"},
            "trust": "untrusted_tool_output",
        });
        // Four identical failures (each its own result_ref) plus a success.
        let mut messages = vec![
            error_tool_result_message("result:err_1.1", error_obs.clone()),
            error_tool_result_message("result:err_1.2", error_obs.clone()),
            error_tool_result_message("result:err_1.3", error_obs.clone()),
            error_tool_result_message("result:err_1.4", error_obs.clone()),
            error_tool_result_message("result:ok_1.5", success_obs.clone()),
        ];

        collapse_repeated_failure_observations(&mut messages);

        // First and last identical errors keep full detail.
        assert_eq!(tool_result_observation(&messages[0]), error_obs);
        assert_eq!(tool_result_observation(&messages[3]), error_obs);
        // Interior duplicates collapse to the compact, schema-valid marker.
        for index in [1usize, 2] {
            let failure_kind = tool_result_observation(&messages[index])
                .get("detail")
                .and_then(|detail| detail.get("failure_kind"))
                .and_then(|kind| kind.as_str())
                .map(str::to_string);
            assert_eq!(failure_kind.as_deref(), Some("repeated_error_elided"));
        }
        // Success observation is never touched.
        assert_eq!(tool_result_observation(&messages[4]), success_obs);
    }

    #[test]
    fn collapse_repeated_failure_observations_leaves_a_single_repeat_alone() {
        let error_obs = generic_error_observation();
        let mut messages = vec![
            error_tool_result_message("result:err_2.1", error_obs.clone()),
            error_tool_result_message("result:err_2.2", error_obs.clone()),
        ];

        collapse_repeated_failure_observations(&mut messages);

        // Below the 3+ threshold: both copies stay intact.
        assert_eq!(tool_result_observation(&messages[0]), error_obs);
        assert_eq!(tool_result_observation(&messages[1]), error_obs);
    }

    fn user_message_with_images(
        content: &str,
        image_parts: Vec<ironclaw_loop_support::HostManagedModelImagePart>,
    ) -> HostManagedModelMessage {
        HostManagedModelMessage {
            role: HostManagedModelMessageRole::User,
            content: content.to_string(),
            content_ref: ironclaw_turns::LoopMessageRef::new(
                "msg:11111111-1111-1111-1111-111111111111",
            )
            .expect("valid message ref"),
            tool_result_provider_call: None,
            tool_result_content: None,
            image_parts,
        }
    }

    #[test]
    fn convert_messages_emits_image_url_parts_for_user_image_attachments() {
        let message = user_message_with_images(
            "what is in this image?",
            vec![ironclaw_loop_support::HostManagedModelImagePart {
                mime_type: "image/png".to_string(),
                bytes: vec![1, 2, 3, 4],
            }],
        );
        let identity = ProviderReplayIdentity::new("openai", "gpt-4o").expect("identity");

        let converted = convert_messages(vec![message], &identity).expect("convert");

        assert_eq!(converted.len(), 1);
        let chat = &converted[0];
        assert_eq!(chat.role, Role::User);
        // Text rides in `content`; the raw bytes are base64-encoded here at the
        // gateway into a `data:` ImageUrl part.
        assert_eq!(chat.content, "what is in this image?");
        assert_eq!(chat.content_parts.len(), 1);
        match &chat.content_parts[0] {
            ContentPart::ImageUrl { image_url } => {
                assert_eq!(image_url.url, "data:image/png;base64,AQIDBA==");
            }
            other => panic!("expected an ImageUrl part, got {other:?}"),
        }
    }

    #[test]
    fn convert_messages_text_only_user_carries_no_content_parts() {
        let message = user_message_with_images("hello", Vec::new());
        let identity = ProviderReplayIdentity::new("openai", "gpt-4o").expect("identity");

        let converted = convert_messages(vec![message], &identity).expect("convert");

        assert_eq!(converted[0].content, "hello");
        assert!(converted[0].content_parts.is_empty());
    }

    #[test]
    fn convert_messages_drops_image_parts_for_non_vision_model() {
        // Even with image bytes present, a text-only model must not receive
        // image content (it would error or ignore it); it keeps the text and
        // relies on the transcript's `<attachments>` pointer.
        let message = user_message_with_images(
            "what is in this image?",
            vec![ironclaw_loop_support::HostManagedModelImagePart {
                mime_type: "image/png".to_string(),
                bytes: vec![1, 2, 3, 4],
            }],
        );
        let identity =
            ProviderReplayIdentity::new("mistral", "mistral-7b-instruct").expect("identity");

        let converted = convert_messages(vec![message], &identity).expect("convert");

        assert_eq!(converted[0].content, "what is in this image?");
        assert!(
            converted[0].content_parts.is_empty(),
            "a non-vision model must not receive image parts"
        );
    }

    #[test]
    fn gateway_recovers_capability_calls_from_textual_tool_syntax_preserves_reasoning_details() {
        use ironclaw_llm::{ReasoningDetail, ReasoningDetails};

        let expected_reasoning = ReasoningDetails {
            id: Some("thinking_123".to_string()),
            content: vec![ReasoningDetail::Text {
                text: "Let me call the echo tool.".to_string(),
                signature: Some("sig_abc".to_string()),
            }],
        };

        let response = ToolCompletionResponse {
            content: Some(
                "Searching now.\nto=demo__echo weirdjson\n{\"message\":\"hello\"}".to_string(),
            ),
            tool_calls: Vec::new(),
            input_tokens: 1,
            output_tokens: 1,
            finish_reason: FinishReason::Stop,
            cache_read_input_tokens: 0,
            cache_creation_input_tokens: 0,
            reasoning: Some("text reasoning".to_string()),
            reasoning_details: Some(expected_reasoning.clone()),
        };

        let recovered =
            recover_textual_tool_calls_from_tool_response(response, &["demo__echo".to_string()])
                .expect("textual tool call recovery succeeded");

        assert_eq!(
            recovered.tool_calls.len(),
            1,
            "recovery must extract the textual tool call"
        );
        assert_eq!(recovered.tool_calls[0].name, "demo__echo");
        assert_eq!(
            recovered.reasoning_details,
            Some(expected_reasoning),
            "recovery must preserve typed reasoning_details onto the recovered response"
        );
    }

    #[test]
    fn provider_tool_repair_messages_preserves_reasoning_details_on_assistant_message() {
        use ironclaw_llm::{ReasoningDetail, ReasoningDetails};

        let expected_reasoning = ReasoningDetails {
            id: Some("thinking_456".to_string()),
            content: vec![ReasoningDetail::Encrypted(
                "encrypted_thinking_data".to_string(),
            )],
        };

        let response = ToolCompletionResponse {
            content: Some("Calling tool.".to_string()),
            tool_calls: vec![ToolCall {
                id: "call_1".to_string(),
                name: "demo__echo".to_string(),
                arguments: serde_json::json!({"message": "hello"}),
                reasoning: None,
                signature: None,
                arguments_parse_error: None,
            }],
            input_tokens: 1,
            output_tokens: 1,
            finish_reason: FinishReason::ToolUse,
            cache_read_input_tokens: 0,
            cache_creation_input_tokens: 0,
            reasoning: Some("text reasoning".to_string()),
            reasoning_details: Some(expected_reasoning.clone()),
        };

        let messages = provider_tool_repair_messages(&response, "tool arguments exceeded limit");

        let repair_assistant = messages
            .iter()
            .find(|m| m.role == Role::Assistant && m.tool_calls.is_some())
            .expect("repair messages must include an assistant tool call replay");

        assert_eq!(
            repair_assistant.reasoning_details,
            Some(expected_reasoning),
            "repaired assistant message must preserve typed reasoning_details"
        );
    }
}
