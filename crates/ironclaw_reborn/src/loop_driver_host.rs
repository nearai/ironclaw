use std::{
    collections::HashMap,
    error::Error,
    fmt,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use ironclaw_loop_support::{
    CapabilityResolveError, CapabilitySurfaceProfileFilter, CapabilitySurfaceProfileResolver,
    EmptyLoopCapabilityPort, HostIdentityContextSource, HostInputQueue, HostManagedModelGateway,
    HostQueueLoopInputPort, HostSkillContextSource, RunCancellationFactory,
    RunCancellationObservationKind, RunStateLoopCancellationPort, ThreadBackedLoopContextPort,
    ThreadBackedLoopModelPort, ThreadBackedLoopTranscriptPort, TurnStateRunCancellationFactory,
};
use ironclaw_threads::{SessionThreadService, ThreadScope};

use crate::driver_registry::{DriverRequirements, LoopDriverRegistryKey, RequirementLevel};
use crate::model_routes::{ModelRouteError, ModelRouteResolver, ModelSlot};
use crate::text_loop_driver::{TEXT_ONLY_DRIVER_ID, TEXT_ONLY_DRIVER_VERSION};

// Pre-WS-14 text-only driver key used by `is_text_only_driver_key`'s
// fail-closed allowlist. Kept alongside the WS-7 `TEXT_ONLY_DRIVER_ID` so
// legacy registry entries still resolve through the text-only host path.
// Retire once no callers register or persist the `lightweight_loop` key —
// after the WS-17 product cutover and any downstream migrations are
// confirmed complete.
const LEGACY_TEXT_ONLY_DRIVER_ID: &str = "lightweight_loop";
const LEGACY_TEXT_ONLY_DRIVER_VERSION: u64 = 1;
const LEGACY_TEXT_ONLY_CHECKPOINT_SCHEMA_ID: &str = "interactive_checkpoint_v1";
const LEGACY_TEXT_ONLY_CHECKPOINT_SCHEMA_VERSION: u64 = 1;

use ironclaw_turns::{
    CheckpointStateStore, GetCheckpointStateRequest, GetLoopCheckpointRequest,
    LoopCheckpointStateRef, LoopCheckpointStore, PutCheckpointStateRequest,
    PutLoopCheckpointRequest, RunProfileId, TurnCheckpointId, TurnError, TurnRunWake,
    TurnRunWakeNotifier, TurnRunWakeNotifyError, TurnStateStore, TurnStatus,
    run_profile::{
        AgentLoopHostError, AgentLoopHostErrorKind, AppendCapabilityResultRef, BeginAssistantDraft,
        CapabilityBatchInvocation, CapabilityBatchOutcome, CapabilityInvocation, CapabilityOutcome,
        FinalizeAssistantMessage, HostManagedLoopModelPort, HostManagedLoopPromptPort,
        InMemoryInstructionMaterializationStore, InstructionMaterializationStore,
        InstructionSafetyContext, LoadCheckpointPayloadRequest, LoadedCheckpointPayload,
        LoopCancellationPort, LoopCancellationSignal, LoopCapabilityPort, LoopCheckpointPort,
        LoopCheckpointRequest, LoopContextBundle, LoopContextPort, LoopContextRequest,
        LoopHostMilestoneEmitter, LoopHostMilestoneSink, LoopInputAckToken, LoopInputBatch,
        LoopInputCursor, LoopInputPort, LoopModelBudgetAccountant, LoopModelGateway,
        LoopModelGatewayError, LoopModelGatewayRequest, LoopModelPolicyGuard, LoopModelPort,
        LoopModelRequest, LoopModelResponse, LoopProgressEvent, LoopProgressPort, LoopPromptBundle,
        LoopPromptBundleAuthority, LoopPromptBundleRequest, LoopPromptPort, LoopRunContext,
        LoopRunInfoPort, LoopSafeSummary, LoopTranscriptPort, NoOpBudgetAccountant,
        NoOpPolicyGuard, StageCheckpointPayloadRequest, UpdateAssistantDraft,
        VisibleCapabilityRequest, VisibleCapabilitySurface,
    },
    runner::ClaimedTurnRun,
};

#[async_trait]
pub trait LoopCapabilityPortFactory: Send + Sync {
    async fn create_capability_port(
        &self,
        run_context: &LoopRunContext,
    ) -> Result<Arc<dyn LoopCapabilityPort>, AgentLoopHostError>;
}

struct ProfiledCapabilityHostRuntime {
    capability_factory: Arc<dyn LoopCapabilityPortFactory>,
    surface_resolver: Arc<dyn CapabilitySurfaceProfileResolver>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextOnlyLoopHostConfig {
    pub max_messages: usize,
    pub require_model_route_snapshot: bool,
}

impl Default for TextOnlyLoopHostConfig {
    fn default() -> Self {
        Self {
            max_messages: 16,
            require_model_route_snapshot: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RebornLoopDriverHostError {
    ScopeMismatch { reason: String },
    InvalidRequest { reason: String },
}

impl fmt::Display for RebornLoopDriverHostError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ScopeMismatch { reason } => {
                write!(formatter, "loop driver host scope mismatch: {reason}")
            }
            Self::InvalidRequest { reason } => {
                write!(formatter, "invalid loop driver host request: {reason}")
            }
        }
    }
}

impl Error for RebornLoopDriverHostError {}

#[derive(Debug, Clone)]
pub struct RebornLoopDriverHostRequest {
    pub claimed_run: ClaimedTurnRun,
    pub loop_run_context: LoopRunContext,
}

#[derive(Default)]
struct CapabilitySurfaceState {
    current: Mutex<Option<VisibleCapabilitySurface>>,
}

impl CapabilitySurfaceState {
    fn set_current(&self, surface: VisibleCapabilitySurface) -> Result<(), AgentLoopHostError> {
        let mut current = self.current.lock().map_err(|_| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::Unavailable,
                "capability surface state is unavailable",
            )
        })?;
        *current = Some(surface);
        Ok(())
    }

    fn current(&self) -> Result<Option<VisibleCapabilitySurface>, AgentLoopHostError> {
        self.current
            .lock()
            .map(|current| current.clone())
            .map_err(|_| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Unavailable,
                    "capability surface state is unavailable",
                )
            })
    }
}

struct SurfaceTrackingLoopCapabilityPort {
    inner: Arc<dyn LoopCapabilityPort>,
    surface_state: Arc<CapabilitySurfaceState>,
}

impl SurfaceTrackingLoopCapabilityPort {
    fn new(inner: Arc<dyn LoopCapabilityPort>, surface_state: Arc<CapabilitySurfaceState>) -> Self {
        Self {
            inner,
            surface_state,
        }
    }
}

#[async_trait]
impl LoopCapabilityPort for SurfaceTrackingLoopCapabilityPort {
    async fn visible_capabilities(
        &self,
        request: VisibleCapabilityRequest,
    ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
        let surface = self.inner.visible_capabilities(request).await?;
        self.surface_state.set_current(surface.clone())?;
        Ok(surface)
    }

    async fn invoke_capability(
        &self,
        request: CapabilityInvocation,
    ) -> Result<CapabilityOutcome, AgentLoopHostError> {
        self.inner.invoke_capability(request).await
    }

    async fn invoke_capability_batch(
        &self,
        request: CapabilityBatchInvocation,
    ) -> Result<CapabilityBatchOutcome, AgentLoopHostError> {
        self.inner.invoke_capability_batch(request).await
    }
}

pub struct RebornLoopDriverHostFactory<S, G>
where
    S: SessionThreadService + ?Sized,
    G: HostManagedModelGateway + ?Sized,
{
    thread_service: Arc<S>,
    thread_scope: ThreadScope,
    model_gateway: Arc<G>,
    model_route_resolver: Option<Arc<dyn ModelRouteResolver>>,
    checkpoint_state_store: Arc<dyn CheckpointStateStore>,
    loop_checkpoint_store: Arc<dyn LoopCheckpointStore>,
    milestone_sink: Arc<dyn LoopHostMilestoneSink>,
    model_accountant: Arc<dyn LoopModelBudgetAccountant>,
    model_policy_guard: Arc<dyn LoopModelPolicyGuard>,
    cancellation_factory: Arc<dyn RunCancellationFactory>,
    config: TextOnlyLoopHostConfig,
    skill_context_source: Option<Arc<dyn HostSkillContextSource>>,
    safety_context: Option<InstructionSafetyContext>,
    identity_context_source: Option<Arc<dyn HostIdentityContextSource>>,
    input_queue: Option<Arc<dyn HostInputQueue>>,
    profiled_capabilities: Option<ProfiledCapabilityHostRuntime>,
    driver_requirements: HashMap<LoopDriverRegistryKey, DriverRequirements>,
}

impl<S, G> RebornLoopDriverHostFactory<S, G>
where
    S: SessionThreadService + ?Sized + Send + Sync + 'static,
    G: HostManagedModelGateway + ?Sized + Send + Sync + 'static,
{
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        thread_service: Arc<S>,
        thread_scope: ThreadScope,
        model_gateway: Arc<G>,
        checkpoint_state_store: Arc<dyn CheckpointStateStore>,
        turn_state_store: Arc<dyn TurnStateStore>,
        loop_checkpoint_store: Arc<dyn LoopCheckpointStore>,
        milestone_sink: Arc<dyn LoopHostMilestoneSink>,
        config: TextOnlyLoopHostConfig,
    ) -> Self {
        let cancellation_factory: Arc<dyn RunCancellationFactory> = Arc::new(
            TurnStateRunCancellationFactory::new(Arc::clone(&turn_state_store)),
        );
        Self {
            thread_service,
            thread_scope,
            model_gateway,
            model_route_resolver: None,
            checkpoint_state_store,
            loop_checkpoint_store,
            milestone_sink,
            model_accountant: Arc::new(NoOpBudgetAccountant),
            model_policy_guard: Arc::new(NoOpPolicyGuard),
            cancellation_factory,
            config,
            skill_context_source: None,
            safety_context: None,
            identity_context_source: None,
            input_queue: None,
            profiled_capabilities: None,
            driver_requirements: HashMap::new(),
        }
    }

    pub fn with_cancellation_factory(mut self, factory: Arc<dyn RunCancellationFactory>) -> Self {
        self.cancellation_factory = factory;
        self
    }

    pub fn cancellation_observation_kind(&self) -> RunCancellationObservationKind {
        self.cancellation_factory.observation_kind()
    }

    pub fn with_skill_context_source(mut self, source: Arc<dyn HostSkillContextSource>) -> Self {
        self.skill_context_source = Some(source);
        self
    }

    pub fn with_safety_context(mut self, safety_context: InstructionSafetyContext) -> Self {
        self.safety_context = Some(safety_context);
        self
    }

    // Note: the WS-11 brief specifies input_queue on PlannedDriverConfig; the implementation
    // puts it here on RebornLoopDriverHostFactory instead, which is the factory pattern already
    // used for capability/context ports. PlannedDriver delegates fully to the host for
    // input port construction. This deviation is intentional; update the brief if keeping.
    pub fn with_input_queue(mut self, queue: Arc<dyn HostInputQueue>) -> Self {
        self.input_queue = Some(queue);
        self
    }

    pub fn with_identity_context_source(
        mut self,
        source: Arc<dyn HostIdentityContextSource>,
    ) -> Self {
        self.identity_context_source = Some(source);
        self
    }

    pub fn with_profiled_capability_port_factory(
        mut self,
        capability_factory: Arc<dyn LoopCapabilityPortFactory>,
        surface_resolver: Arc<dyn CapabilitySurfaceProfileResolver>,
    ) -> Self {
        self.profiled_capabilities = Some(ProfiledCapabilityHostRuntime {
            capability_factory,
            surface_resolver,
        });
        self
    }

    pub fn with_driver_requirements(
        mut self,
        driver_requirements: HashMap<LoopDriverRegistryKey, DriverRequirements>,
    ) -> Self {
        self.driver_requirements = driver_requirements;
        self
    }

    pub fn with_model_route_resolver(mut self, resolver: Arc<dyn ModelRouteResolver>) -> Self {
        self.model_route_resolver = Some(resolver);
        self
    }

    pub fn with_model_budget_accountant(
        mut self,
        accountant: Arc<dyn LoopModelBudgetAccountant>,
    ) -> Self {
        self.model_accountant = accountant;
        self
    }

    pub fn with_model_policy_guard(mut self, policy_guard: Arc<dyn LoopModelPolicyGuard>) -> Self {
        self.model_policy_guard = policy_guard;
        self
    }

    pub async fn build_text_only_host(
        &self,
        request: RebornLoopDriverHostRequest,
    ) -> Result<RebornLoopDriverHost, RebornLoopDriverHostError> {
        self.build_text_only_host_with_capabilities(request, Arc::new(EmptyLoopCapabilityPort))
            .await
    }

    pub async fn build_text_only_host_with_profiled_capabilities(
        &self,
        request: RebornLoopDriverHostRequest,
        capabilities: Arc<dyn LoopCapabilityPort>,
        surface_resolver: Arc<dyn CapabilitySurfaceProfileResolver>,
    ) -> Result<RebornLoopDriverHost, RebornLoopDriverHostError> {
        validate_claimed_run_context(&request.claimed_run, &request.loop_run_context)?;
        validate_thread_scope(&self.thread_scope, &request.loop_run_context)?;
        let allow_set = Arc::new(
            surface_resolver
                .resolve(&request.loop_run_context)
                .await
                .map_err(capability_resolve_error_to_host_error)?,
        );
        let capabilities: Arc<dyn LoopCapabilityPort> =
            Arc::new(CapabilitySurfaceProfileFilter::new(capabilities, allow_set));
        self.build_text_only_host_with_capabilities(request, capabilities)
            .await
    }

    pub async fn build_text_only_host_with_capabilities(
        &self,
        request: RebornLoopDriverHostRequest,
        capabilities: Arc<dyn LoopCapabilityPort>,
    ) -> Result<RebornLoopDriverHost, RebornLoopDriverHostError> {
        validate_claimed_run_context(&request.claimed_run, &request.loop_run_context)?;
        validate_thread_scope(&self.thread_scope, &request.loop_run_context)?;

        let max_messages = self.config.max_messages.max(1);
        let run_context = self.attach_model_route_snapshot(request.loop_run_context)?;
        let mut context_adapter = ThreadBackedLoopContextPort::new(
            Arc::clone(&self.thread_service),
            self.thread_scope.clone(),
            run_context.clone(),
            max_messages,
        );
        if let Some(source) = self.skill_context_source.as_ref() {
            context_adapter = context_adapter.with_skill_context_source(source.clone());
        }
        if let Some(source) = self.identity_context_source.as_ref() {
            context_adapter = context_adapter.with_identity_context_source(source.clone());
        }
        let context: Arc<dyn LoopContextPort> = Arc::new(context_adapter);
        let instruction_materialization_store: Arc<dyn InstructionMaterializationStore> =
            Arc::new(InMemoryInstructionMaterializationStore::default());
        let surface_state = Arc::new(CapabilitySurfaceState::default());
        let capabilities: Arc<dyn LoopCapabilityPort> = Arc::new(
            SurfaceTrackingLoopCapabilityPort::new(capabilities, Arc::clone(&surface_state)),
        );
        capabilities
            .visible_capabilities(VisibleCapabilityRequest)
            .await
            .map_err(|error| RebornLoopDriverHostError::InvalidRequest {
                reason: error.safe_summary,
            })?;
        let prompt_authority = LoopPromptBundleAuthority::shared();
        let surface_state_for_prompt = Arc::clone(&surface_state);
        let mut prompt_port = HostManagedLoopPromptPort::new(
            run_context.clone(),
            Arc::clone(&context),
            Arc::clone(&self.milestone_sink),
        )
        .with_prompt_bundle_authority(prompt_authority.clone())
        .with_default_message_limit(max_messages)
        .with_current_surface_lookup(move || surface_state_for_prompt.current())
        .with_instruction_materialization_store(Arc::clone(&instruction_materialization_store));
        if let Some(safety_context) = self.safety_context.clone() {
            prompt_port = prompt_port.with_safety_context(safety_context);
        }
        let prompt: Arc<dyn LoopPromptPort> = Arc::new(prompt_port);
        let input: Arc<dyn LoopInputPort> = match self.input_queue.as_ref() {
            Some(queue) => Arc::new(HostQueueLoopInputPort::new(
                queue.clone(),
                run_context.clone(),
            )),
            None => Arc::new(NoExtraLoopInputPort::new(run_context.clone())),
        };
        let model_gateway = Arc::new(ThreadResolvingLoopModelGateway::new(
            ThreadResolvingLoopModelGatewayConfig {
                thread_service: Arc::clone(&self.thread_service),
                thread_scope: self.thread_scope.clone(),
                host_gateway: Arc::clone(&self.model_gateway),
                max_messages,
                skill_context_source: self.skill_context_source.clone(),
                identity_context_source: self.identity_context_source.clone(),
                instruction_materialization_store: Some(Arc::clone(
                    &instruction_materialization_store,
                )),
                prompt_authority,
            },
        ));
        let model: Arc<dyn LoopModelPort> = Arc::new(HostManagedLoopModelPort::with_guards(
            run_context.clone(),
            model_gateway,
            Arc::clone(&self.milestone_sink),
            Arc::clone(&self.model_accountant),
            Arc::clone(&self.model_policy_guard),
        ));
        let checkpoint: Arc<dyn LoopCheckpointPort> = Arc::new(HostManagedLoopCheckpointPort::new(
            run_context.clone(),
            Arc::clone(&self.checkpoint_state_store),
            Arc::clone(&self.loop_checkpoint_store),
            Arc::clone(&self.milestone_sink),
        ));
        let transcript: Arc<dyn LoopTranscriptPort> =
            Arc::new(ThreadBackedLoopTranscriptPort::with_milestone_sink(
                Arc::clone(&self.thread_service),
                self.thread_scope.clone(),
                run_context.clone(),
                Arc::clone(&self.milestone_sink),
            ));
        let progress: Arc<dyn LoopProgressPort> = Arc::new(HostManagedLoopProgressPort::new(
            run_context.clone(),
            Arc::clone(&self.milestone_sink),
        ));
        let cancellation_handle = self
            .cancellation_factory
            .handle_for_run(&run_context.scope, run_context.run_id)
            .await
            .map_err(|error| RebornLoopDriverHostError::InvalidRequest {
                reason: error.safe_summary,
            })?;
        let cancellation: Arc<dyn LoopCancellationPort> =
            Arc::new(RunStateLoopCancellationPort::new(cancellation_handle));

        Ok(RebornLoopDriverHost {
            run_context,
            context,
            prompt,
            input,
            model,
            checkpoint,
            capabilities,
            transcript,
            progress,
            cancellation,
        })
    }

    fn attach_model_route_snapshot(
        &self,
        run_context: LoopRunContext,
    ) -> Result<LoopRunContext, RebornLoopDriverHostError> {
        if let Some(snapshot) = &run_context.resolved_model_route {
            snapshot
                .validate()
                .map_err(|reason| RebornLoopDriverHostError::InvalidRequest { reason })?;
            let Some(resolver) = &self.model_route_resolver else {
                return Err(RebornLoopDriverHostError::InvalidRequest {
                    reason: "model route resolver is required for this host".to_string(),
                });
            };
            let slot = slot_for_model_profile(&run_context)?;
            let route = crate::model_routes::ModelRoute::new(
                snapshot.provider_id.clone(),
                snapshot.model_id.clone(),
            )
            .map_err(model_route_error_to_host_error)?;
            resolver
                .validate_model_route(slot, &route)
                .map_err(model_route_error_to_host_error)?;
            return Ok(run_context);
        }
        let Some(resolver) = &self.model_route_resolver else {
            if self.config.require_model_route_snapshot {
                return Err(RebornLoopDriverHostError::InvalidRequest {
                    reason: "model route resolver is required for this host".to_string(),
                });
            }
            return Ok(run_context);
        };
        let slot = slot_for_model_profile(&run_context)?;
        let snapshot = resolver
            .resolve_model_route(slot)
            .map_err(model_route_error_to_host_error)?;
        Ok(run_context.with_resolved_model_route(snapshot.to_loop_model_route_snapshot()))
    }
}

impl<S, G> TurnRunWakeNotifier for RebornLoopDriverHostFactory<S, G>
where
    S: SessionThreadService + ?Sized + Send + Sync + 'static,
    G: HostManagedModelGateway + ?Sized + Send + Sync + 'static,
{
    fn notify_queued_run(&self, wake: TurnRunWake) -> Result<(), TurnRunWakeNotifyError> {
        self.cancellation_factory.notify_run_wake(&wake);
        Ok(())
    }
}

struct ThreadResolvingLoopModelGateway<S, G>
where
    S: SessionThreadService + ?Sized,
    G: HostManagedModelGateway + ?Sized,
{
    thread_service: Arc<S>,
    thread_scope: ThreadScope,
    host_gateway: Arc<G>,
    max_messages: usize,
    skill_context_source: Option<Arc<dyn HostSkillContextSource>>,
    identity_context_source: Option<Arc<dyn HostIdentityContextSource>>,
    instruction_materialization_store: Option<Arc<dyn InstructionMaterializationStore>>,
    prompt_authority: LoopPromptBundleAuthority,
}

struct ThreadResolvingLoopModelGatewayConfig<S, G>
where
    S: SessionThreadService + ?Sized,
    G: HostManagedModelGateway + ?Sized,
{
    thread_service: Arc<S>,
    thread_scope: ThreadScope,
    host_gateway: Arc<G>,
    max_messages: usize,
    skill_context_source: Option<Arc<dyn HostSkillContextSource>>,
    identity_context_source: Option<Arc<dyn HostIdentityContextSource>>,
    instruction_materialization_store: Option<Arc<dyn InstructionMaterializationStore>>,
    prompt_authority: LoopPromptBundleAuthority,
}

impl<S, G> ThreadResolvingLoopModelGateway<S, G>
where
    S: SessionThreadService + ?Sized,
    G: HostManagedModelGateway + ?Sized,
{
    fn new(config: ThreadResolvingLoopModelGatewayConfig<S, G>) -> Self {
        Self {
            thread_service: config.thread_service,
            thread_scope: config.thread_scope,
            host_gateway: config.host_gateway,
            max_messages: config.max_messages,
            skill_context_source: config.skill_context_source,
            identity_context_source: config.identity_context_source,
            instruction_materialization_store: config.instruction_materialization_store,
            prompt_authority: config.prompt_authority,
        }
    }
}

#[async_trait]
impl<S, G> LoopModelGateway for ThreadResolvingLoopModelGateway<S, G>
where
    S: SessionThreadService + ?Sized + Send + Sync,
    G: HostManagedModelGateway + ?Sized + Send + Sync,
{
    async fn stream_model(
        &self,
        request: LoopModelGatewayRequest,
    ) -> Result<LoopModelResponse, LoopModelGatewayError> {
        let mut model_port = ThreadBackedLoopModelPort::new(
            Arc::clone(&self.thread_service),
            self.thread_scope.clone(),
            request.context,
            Arc::clone(&self.host_gateway),
            self.max_messages,
        )
        .with_prompt_bundle_authority(self.prompt_authority.clone());
        if let Some(source) = self.skill_context_source.as_ref() {
            model_port = model_port.with_skill_context_source(source.clone());
        }
        if let Some(source) = self.identity_context_source.as_ref() {
            model_port = model_port.with_identity_context_source(source.clone());
        }
        if let Some(store) = self.instruction_materialization_store.as_ref() {
            model_port = model_port.with_instruction_materialization_store(Arc::clone(store));
        }
        model_port
            .stream_model(request.request)
            .await
            .map_err(host_error_to_model_gateway_error)
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

pub struct RebornLoopDriverHost {
    run_context: LoopRunContext,
    context: Arc<dyn LoopContextPort>,
    prompt: Arc<dyn LoopPromptPort>,
    input: Arc<dyn LoopInputPort>,
    model: Arc<dyn LoopModelPort>,
    checkpoint: Arc<dyn LoopCheckpointPort>,
    capabilities: Arc<dyn LoopCapabilityPort>,
    transcript: Arc<dyn LoopTranscriptPort>,
    progress: Arc<dyn LoopProgressPort>,
    cancellation: Arc<dyn LoopCancellationPort>,
}

impl fmt::Debug for RebornLoopDriverHost {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RebornLoopDriverHost")
            .field("scope", &self.run_context.scope)
            .field("turn_id", &self.run_context.turn_id)
            .field("run_id", &self.run_context.run_id)
            .field("loop_driver_id", &self.run_context.loop_driver_id)
            .finish()
    }
}

impl LoopRunInfoPort for RebornLoopDriverHost {
    fn run_context(&self) -> &LoopRunContext {
        &self.run_context
    }
}

impl LoopCancellationPort for RebornLoopDriverHost {
    fn observe_cancellation(&self) -> Option<LoopCancellationSignal> {
        self.cancellation.observe_cancellation()
    }
}

#[async_trait]
impl LoopContextPort for RebornLoopDriverHost {
    async fn load_loop_context(
        &self,
        request: LoopContextRequest,
    ) -> Result<LoopContextBundle, AgentLoopHostError> {
        self.context.load_loop_context(request).await
    }
}

#[async_trait]
impl LoopPromptPort for RebornLoopDriverHost {
    async fn build_prompt_bundle(
        &self,
        request: LoopPromptBundleRequest,
    ) -> Result<LoopPromptBundle, AgentLoopHostError> {
        self.prompt.build_prompt_bundle(request).await
    }
}

#[async_trait]
impl LoopInputPort for RebornLoopDriverHost {
    async fn poll_inputs(
        &self,
        after: LoopInputCursor,
        limit: usize,
    ) -> Result<LoopInputBatch, AgentLoopHostError> {
        self.input.poll_inputs(after, limit).await
    }

    async fn ack_inputs(&self, tokens: Vec<LoopInputAckToken>) -> Result<(), AgentLoopHostError> {
        self.input.ack_inputs(tokens).await
    }
}

#[async_trait]
impl LoopModelPort for RebornLoopDriverHost {
    async fn stream_model(
        &self,
        request: LoopModelRequest,
    ) -> Result<LoopModelResponse, AgentLoopHostError> {
        self.model.stream_model(request).await
    }
}

#[async_trait]
impl LoopCapabilityPort for RebornLoopDriverHost {
    async fn visible_capabilities(
        &self,
        request: VisibleCapabilityRequest,
    ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
        self.capabilities.visible_capabilities(request).await
    }

    async fn invoke_capability(
        &self,
        request: CapabilityInvocation,
    ) -> Result<CapabilityOutcome, AgentLoopHostError> {
        self.capabilities.invoke_capability(request).await
    }

    async fn invoke_capability_batch(
        &self,
        request: CapabilityBatchInvocation,
    ) -> Result<CapabilityBatchOutcome, AgentLoopHostError> {
        self.capabilities.invoke_capability_batch(request).await
    }
}

#[async_trait]
impl LoopTranscriptPort for RebornLoopDriverHost {
    async fn begin_assistant_draft(
        &self,
        request: BeginAssistantDraft,
    ) -> Result<ironclaw_turns::LoopMessageRef, AgentLoopHostError> {
        self.transcript.begin_assistant_draft(request).await
    }

    async fn update_assistant_draft(
        &self,
        request: UpdateAssistantDraft,
    ) -> Result<(), AgentLoopHostError> {
        self.transcript.update_assistant_draft(request).await
    }

    async fn finalize_assistant_message(
        &self,
        request: FinalizeAssistantMessage,
    ) -> Result<ironclaw_turns::LoopMessageRef, AgentLoopHostError> {
        self.transcript.finalize_assistant_message(request).await
    }

    async fn append_capability_result_ref(
        &self,
        request: AppendCapabilityResultRef,
    ) -> Result<ironclaw_turns::LoopMessageRef, AgentLoopHostError> {
        self.transcript.append_capability_result_ref(request).await
    }
}

#[async_trait]
impl LoopCheckpointPort for RebornLoopDriverHost {
    async fn checkpoint(
        &self,
        request: LoopCheckpointRequest,
    ) -> Result<TurnCheckpointId, AgentLoopHostError> {
        self.checkpoint.checkpoint(request).await
    }

    async fn stage_checkpoint_payload(
        &self,
        request: StageCheckpointPayloadRequest,
    ) -> Result<LoopCheckpointStateRef, AgentLoopHostError> {
        self.checkpoint.stage_checkpoint_payload(request).await
    }

    async fn load_checkpoint_payload(
        &self,
        request: LoadCheckpointPayloadRequest,
    ) -> Result<LoadedCheckpointPayload, AgentLoopHostError> {
        self.checkpoint.load_checkpoint_payload(request).await
    }
}

#[async_trait]
impl LoopProgressPort for RebornLoopDriverHost {
    async fn emit_loop_progress(&self, event: LoopProgressEvent) -> Result<(), AgentLoopHostError> {
        self.progress.emit_loop_progress(event).await
    }
}

#[derive(Clone)]
pub struct NoExtraLoopInputPort {
    run_context: LoopRunContext,
}

impl NoExtraLoopInputPort {
    pub fn new(run_context: LoopRunContext) -> Self {
        Self { run_context }
    }

    fn validate_cursor(&self, cursor: &LoopInputCursor) -> Result<(), AgentLoopHostError> {
        if cursor.is_for_run(&self.run_context) {
            Ok(())
        } else {
            Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::ScopeMismatch,
                "input cursor is not scoped to this loop run",
            ))
        }
    }
}

impl LoopRunInfoPort for NoExtraLoopInputPort {
    fn run_context(&self) -> &LoopRunContext {
        &self.run_context
    }
}

#[async_trait]
impl LoopInputPort for NoExtraLoopInputPort {
    async fn poll_inputs(
        &self,
        after: LoopInputCursor,
        _limit: usize,
    ) -> Result<LoopInputBatch, AgentLoopHostError> {
        self.validate_cursor(&after)?;
        Ok(LoopInputBatch {
            inputs: Vec::new(),
            input_acks: Vec::new(),
            next_cursor: after,
        })
    }

    async fn ack_inputs(&self, tokens: Vec<LoopInputAckToken>) -> Result<(), AgentLoopHostError> {
        if tokens.is_empty() {
            Ok(())
        } else {
            Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "input ack token was not issued by this host",
            ))
        }
    }
}

#[derive(Clone)]
pub struct HostManagedLoopCheckpointPort {
    run_context: LoopRunContext,
    checkpoint_state_store: Arc<dyn CheckpointStateStore>,
    loop_checkpoint_store: Arc<dyn LoopCheckpointStore>,
    milestone_sink: Arc<dyn LoopHostMilestoneSink>,
}

impl HostManagedLoopCheckpointPort {
    pub fn new(
        run_context: LoopRunContext,
        checkpoint_state_store: Arc<dyn CheckpointStateStore>,
        loop_checkpoint_store: Arc<dyn LoopCheckpointStore>,
        milestone_sink: Arc<dyn LoopHostMilestoneSink>,
    ) -> Self {
        Self {
            run_context,
            checkpoint_state_store,
            loop_checkpoint_store,
            milestone_sink,
        }
    }
}

impl LoopRunInfoPort for HostManagedLoopCheckpointPort {
    fn run_context(&self) -> &LoopRunContext {
        &self.run_context
    }
}

#[async_trait]
impl LoopCheckpointPort for HostManagedLoopCheckpointPort {
    async fn checkpoint(
        &self,
        request: LoopCheckpointRequest,
    ) -> Result<TurnCheckpointId, AgentLoopHostError> {
        // `stage_checkpoint_payload` returns a run-scoped ref of the form
        // `checkpoint:{run_id}:{token}`. The underlying store indexed the payload
        // under the original `checkpoint:{token}` key (which `new_state_ref()`
        // generated). Unwrap to the store key so the look-up succeeds, then pass
        // the caller-supplied (run-scoped) ref through to the loop-checkpoint
        // record so `is_for_run` validators see the correct form.
        let store_ref = checkpoint_state_store_ref(&self.run_context, &request.state_ref)?;

        let loaded = self
            .checkpoint_state_store
            .get_checkpoint_state(GetCheckpointStateRequest {
                scope: self.run_context.scope.clone(),
                turn_id: self.run_context.turn_id,
                run_id: self.run_context.run_id,
                state_ref: store_ref,
                schema_id: self.run_context.checkpoint_schema_id.clone(),
                schema_version: self.run_context.checkpoint_schema_version,
                kind: request.kind,
            })
            .await
            .map_err(turn_error_to_host_error)?;
        if loaded.is_none() {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::CheckpointRejected,
                "checkpoint state ref is unavailable for this loop run",
            ));
        }

        let checkpoint = self
            .loop_checkpoint_store
            .put_loop_checkpoint(PutLoopCheckpointRequest {
                scope: self.run_context.scope.clone(),
                turn_id: self.run_context.turn_id,
                run_id: self.run_context.run_id,
                state_ref: request.state_ref,
                schema_id: self.run_context.checkpoint_schema_id.clone(),
                schema_version: self.run_context.checkpoint_schema_version,
                kind: request.kind,
            })
            .await
            .map_err(turn_error_to_host_error)?;
        LoopHostMilestoneEmitter::new(self.run_context.clone(), Arc::clone(&self.milestone_sink))
            .checkpoint_created(checkpoint.checkpoint_id, request.kind)
            .await?;
        Ok(checkpoint.checkpoint_id)
    }

    async fn stage_checkpoint_payload(
        &self,
        request: StageCheckpointPayloadRequest,
    ) -> Result<LoopCheckpointStateRef, AgentLoopHostError> {
        // Reject staged payloads whose schema_id disagrees with the run
        // profile's resolved checkpoint schema — the read-side
        // `get_checkpoint_state` checks `(state_ref, schema_id, kind)` as a
        // unit, so mismatches here would lead to phantom resume rejections.
        if request.schema_id != self.run_context.checkpoint_schema_id.as_str() {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::CheckpointRejected,
                "staged checkpoint payload schema_id does not match the run profile's checkpoint schema",
            ));
        }

        let record = self
            .checkpoint_state_store
            .put_checkpoint_state(PutCheckpointStateRequest::new(
                self.run_context.scope.clone(),
                self.run_context.turn_id,
                self.run_context.run_id,
                self.run_context.checkpoint_schema_id.clone(),
                self.run_context.checkpoint_schema_version,
                request.kind,
                request.payload,
            ))
            .await
            .map_err(turn_error_to_host_error)?;

        // The store produces `checkpoint:{uuid}` refs. Wrap into the run-scoped
        // form `checkpoint:{run_id}:{token}` so that `LoopCheckpointStateRef::
        // is_for_run` validators accept the returned ref without treating it as
        // a cross-run ref. The token is the opaque UUID the store already minted.
        let raw = record.state_ref.as_str();
        let token = raw.strip_prefix("checkpoint:").ok_or_else(|| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::Internal,
                "checkpoint state store returned ref without expected `checkpoint:` prefix",
            )
        })?;
        LoopCheckpointStateRef::for_run(&self.run_context, token).map_err(|reason| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::Internal,
                format!("could not build run-scoped checkpoint state ref: {reason}"),
            )
        })
    }

    async fn load_checkpoint_payload(
        &self,
        request: LoadCheckpointPayloadRequest,
    ) -> Result<LoadedCheckpointPayload, AgentLoopHostError> {
        let metadata = self
            .loop_checkpoint_store
            .get_loop_checkpoint(GetLoopCheckpointRequest {
                scope: self.run_context.scope.clone(),
                turn_id: self.run_context.turn_id,
                run_id: self.run_context.run_id,
                checkpoint_id: request.checkpoint_id,
            })
            .await
            .map_err(turn_error_to_host_error)?
            .ok_or_else(|| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Unavailable,
                    "checkpoint metadata was not found for this loop run",
                )
            })?;

        if metadata.schema_id != request.expected_schema_id
            || metadata.schema_version != request.expected_schema_version
        {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::Invalid,
                "checkpoint schema id/version does not match the resume request",
            ));
        }

        let state_ref = checkpoint_state_store_ref(&self.run_context, &metadata.state_ref)?;
        let state_record = self
            .checkpoint_state_store
            .get_checkpoint_state(GetCheckpointStateRequest {
                scope: self.run_context.scope.clone(),
                turn_id: self.run_context.turn_id,
                run_id: self.run_context.run_id,
                state_ref,
                schema_id: metadata.schema_id.clone(),
                schema_version: metadata.schema_version,
                kind: metadata.kind,
            })
            .await
            .map_err(turn_error_to_host_error)?
            .ok_or_else(|| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Unavailable,
                    "checkpoint payload was not found for this loop run",
                )
            })?;

        Ok(LoadedCheckpointPayload {
            kind: state_record.kind,
            schema_id: state_record.schema_id,
            schema_version: state_record.schema_version,
            payload: state_record.payload,
        })
    }
}

fn checkpoint_state_store_ref(
    run_context: &LoopRunContext,
    state_ref: &LoopCheckpointStateRef,
) -> Result<LoopCheckpointStateRef, AgentLoopHostError> {
    let run_scoped_prefix = format!("checkpoint:{}:", run_context.run_id);
    if let Some(token) = state_ref.as_str().strip_prefix(&run_scoped_prefix) {
        return LoopCheckpointStateRef::new(format!("checkpoint:{token}")).map_err(|reason| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::Internal,
                format!("could not rebuild store key from run-scoped checkpoint ref: {reason}"),
            )
        });
    }
    Ok(state_ref.clone())
}

#[derive(Clone)]
pub struct HostManagedLoopProgressPort {
    run_context: LoopRunContext,
    milestone_sink: Arc<dyn LoopHostMilestoneSink>,
}

impl HostManagedLoopProgressPort {
    pub fn new(
        run_context: LoopRunContext,
        milestone_sink: Arc<dyn LoopHostMilestoneSink>,
    ) -> Self {
        Self {
            run_context,
            milestone_sink,
        }
    }
}

impl LoopRunInfoPort for HostManagedLoopProgressPort {
    fn run_context(&self) -> &LoopRunContext {
        &self.run_context
    }
}

#[async_trait]
impl LoopProgressPort for HostManagedLoopProgressPort {
    async fn emit_loop_progress(&self, event: LoopProgressEvent) -> Result<(), AgentLoopHostError> {
        let emitter = LoopHostMilestoneEmitter::new(
            self.run_context.clone(),
            Arc::clone(&self.milestone_sink),
        );
        match event {
            LoopProgressEvent::DriverNote { kind, safe_summary } => {
                emitter.driver_note(kind, safe_summary).await
            }
            LoopProgressEvent::IterationStarted { iteration } => {
                emitter.iteration_started(iteration).await
            }
            // Prompt construction already emits the canonical
            // `PromptBundleBuilt` milestone from `HostManagedLoopPromptPort`,
            // including the bundle ref and redacted skill-context metadata.
            // Treat the executor progress echo as advisory to avoid duplicate
            // prompt milestones for the same bundle.
            LoopProgressEvent::PromptBundleBuilt { .. } => Ok(()),
            LoopProgressEvent::CapabilityBatchStarted {
                iteration,
                call_count,
                policy,
            } => {
                emitter
                    .capability_batch_started(iteration, call_count, policy)
                    .await
            }
            LoopProgressEvent::CapabilityBatchCompleted {
                iteration,
                result_count,
                denied_count,
                gated_count,
                failed_count,
            } => {
                emitter
                    .capability_batch_completed(
                        iteration,
                        result_count,
                        denied_count,
                        gated_count,
                        failed_count,
                    )
                    .await
            }
            LoopProgressEvent::GateBlocked {
                iteration,
                gate_kind,
            } => emitter.gate_blocked(iteration, gate_kind).await,
            // `HostManagedLoopCheckpointPort::checkpoint` publishes the
            // canonical checkpoint milestone with the durable checkpoint id.
            // `CheckpointWritten` carries only the checkpoint kind/iteration,
            // so emitting it here would either duplicate or weaken that record.
            LoopProgressEvent::CheckpointWritten { .. } => Ok(()),
        }
    }
}

fn validate_claimed_run_context(
    claimed_run: &ClaimedTurnRun,
    run_context: &LoopRunContext,
) -> Result<(), RebornLoopDriverHostError> {
    if claimed_run.state.status != TurnStatus::Running {
        return Err(RebornLoopDriverHostError::InvalidRequest {
            reason: "claimed run must be running".to_string(),
        });
    }
    if claimed_run.state.scope != run_context.scope
        || claimed_run.state.turn_id != run_context.turn_id
        || claimed_run.state.run_id != run_context.run_id
    {
        return Err(RebornLoopDriverHostError::ScopeMismatch {
            reason: "claimed run state does not match loop run context".to_string(),
        });
    }
    if claimed_run.resolved_run_profile != run_context.resolved_run_profile {
        return Err(RebornLoopDriverHostError::ScopeMismatch {
            reason: "claimed run profile does not match loop run context".to_string(),
        });
    }
    match (
        &claimed_run.state.resolved_model_route,
        &run_context.resolved_model_route,
    ) {
        (Some(expected), Some(actual)) if expected != actual => {
            return Err(RebornLoopDriverHostError::ScopeMismatch {
                reason: "loop run context model route does not match claimed run".to_string(),
            });
        }
        (Some(_), None) => {
            return Err(RebornLoopDriverHostError::ScopeMismatch {
                reason: "loop run context is missing claimed run model route".to_string(),
            });
        }
        (None, Some(_)) => {
            return Err(RebornLoopDriverHostError::ScopeMismatch {
                reason: "loop run context model route was not persisted on claimed run".to_string(),
            });
        }
        _ => {}
    }
    let expected_profile_id = persisted_profile_id(&run_context.resolved_run_profile.profile_id);
    if claimed_run.state.resolved_run_profile_id != expected_profile_id
        || claimed_run.state.resolved_run_profile_version
            != run_context.resolved_run_profile.profile_version
    {
        return Err(RebornLoopDriverHostError::ScopeMismatch {
            reason: "claimed run persisted profile identity does not match loop run context"
                .to_string(),
        });
    }
    if run_context.loop_driver_id != run_context.resolved_run_profile.loop_driver.id
        || run_context.loop_driver_version != run_context.resolved_run_profile.loop_driver.version
    {
        return Err(RebornLoopDriverHostError::ScopeMismatch {
            reason: "loop driver identity does not match resolved profile".to_string(),
        });
    }
    if run_context.thread_id != run_context.scope.thread_id {
        return Err(RebornLoopDriverHostError::ScopeMismatch {
            reason: "loop run context thread does not match scope thread".to_string(),
        });
    }
    if run_context.checkpoint_schema_id != run_context.resolved_run_profile.checkpoint_schema_id
        || run_context.checkpoint_schema_version
            != run_context.resolved_run_profile.checkpoint_schema_version
    {
        return Err(RebornLoopDriverHostError::ScopeMismatch {
            reason: "loop run context checkpoint identity does not match resolved profile"
                .to_string(),
        });
    }
    Ok(())
}

#[async_trait]
impl<S, G> crate::turn_runner::HostFactory for RebornLoopDriverHostFactory<S, G>
where
    S: SessionThreadService + ?Sized + Send + Sync + 'static,
    G: HostManagedModelGateway + ?Sized + Send + Sync + 'static,
{
    async fn create_host(
        &self,
        claimed: &ClaimedTurnRun,
    ) -> Result<
        Box<dyn ironclaw_turns::run_profile::AgentLoopDriverHost + Send + Sync>,
        crate::turn_runner::HostFactoryError,
    > {
        let mut loop_run_context = LoopRunContext::new(
            claimed.state.scope.clone(),
            claimed.state.turn_id,
            claimed.state.run_id,
            claimed.resolved_run_profile.clone(),
        );
        if let Some(snapshot) = claimed.state.resolved_model_route.clone() {
            loop_run_context = loop_run_context.with_resolved_model_route(snapshot);
        }
        let request = RebornLoopDriverHostRequest {
            claimed_run: claimed.clone(),
            loop_run_context,
        };
        let capability_requirement = self.capability_requirement(claimed)?;
        let host_result = if capability_requirement.requires_profiled_capabilities() {
            let Some(profiled) = self.profiled_capabilities.as_ref() else {
                return Err(crate::turn_runner::HostFactoryError::new(
                    "profiled capability port factory is required for capability-required driver host",
                ));
            };
            let capabilities = profiled
                .capability_factory
                .create_capability_port(&request.loop_run_context)
                .await
                .map_err(|error| crate::turn_runner::HostFactoryError::new(error.safe_summary))?;
            self.build_text_only_host_with_profiled_capabilities(
                request,
                capabilities,
                Arc::clone(&profiled.surface_resolver),
            )
            .await
        } else {
            self.build_text_only_host(request).await
        };
        host_result
            .map(|host| {
                Box::new(host)
                    as Box<dyn ironclaw_turns::run_profile::AgentLoopDriverHost + Send + Sync>
            })
            .map_err(|error| crate::turn_runner::HostFactoryError::new(error.to_string()))
    }
}

impl<S, G> RebornLoopDriverHostFactory<S, G>
where
    S: SessionThreadService + ?Sized + Send + Sync + 'static,
    G: HostManagedModelGateway + ?Sized + Send + Sync + 'static,
{
    fn capability_requirement(
        &self,
        claimed: &ClaimedTurnRun,
    ) -> Result<DriverCapabilityRequirement, crate::turn_runner::HostFactoryError> {
        let key = LoopDriverRegistryKey::from_descriptor(&claimed.resolved_run_profile.loop_driver)
            .map_err(|reason| {
                crate::turn_runner::HostFactoryError::new(format!(
                    "invalid loop driver descriptor: {reason}"
                ))
            })?;
        let Some(requirements) = self.driver_requirements.get(&key) else {
            // Older text-only factory paths predate driver requirement snapshots.
            // Keep only those known descriptors on the no-capability host path.
            if is_text_only_driver_key(&key) {
                return Ok(DriverCapabilityRequirement::ExplicitlyTextOnly);
            }
            return Err(crate::turn_runner::HostFactoryError::new(
                "loop driver requirements metadata is unavailable; cannot determine capability requirements",
            ));
        };
        Ok(DriverCapabilityRequirement::from_requirements(requirements))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DriverCapabilityRequirement {
    ExplicitlyTextOnly,
    ProfiledCapabilitiesRequired,
    ProfiledCapabilitiesNotRequired,
}

impl DriverCapabilityRequirement {
    fn from_requirements(requirements: &DriverRequirements) -> Self {
        if matches!(requirements.capabilities, RequirementLevel::Required) {
            Self::ProfiledCapabilitiesRequired
        } else {
            Self::ProfiledCapabilitiesNotRequired
        }
    }

    fn requires_profiled_capabilities(self) -> bool {
        matches!(self, Self::ProfiledCapabilitiesRequired)
    }
}

fn is_text_only_driver_key(key: &LoopDriverRegistryKey) -> bool {
    is_reborn_text_only_driver_key(key) || is_legacy_text_only_driver_key(key)
}

fn is_reborn_text_only_driver_key(key: &LoopDriverRegistryKey) -> bool {
    key.id.as_str() == TEXT_ONLY_DRIVER_ID
        && key.version.as_u64() == TEXT_ONLY_DRIVER_VERSION
        && key.checkpoint_schema_id.is_none()
        && key.checkpoint_schema_version.is_none()
}

fn is_legacy_text_only_driver_key(key: &LoopDriverRegistryKey) -> bool {
    key.id.as_str() == LEGACY_TEXT_ONLY_DRIVER_ID
        && key.version.as_u64() == LEGACY_TEXT_ONLY_DRIVER_VERSION
        && key
            .checkpoint_schema_id
            .as_ref()
            .is_some_and(|schema_id| schema_id.as_str() == LEGACY_TEXT_ONLY_CHECKPOINT_SCHEMA_ID)
        && key
            .checkpoint_schema_version
            .is_some_and(|version| version.as_u64() == LEGACY_TEXT_ONLY_CHECKPOINT_SCHEMA_VERSION)
}

fn capability_resolve_error_to_host_error(
    error: CapabilityResolveError,
) -> RebornLoopDriverHostError {
    let reason = match error {
        CapabilityResolveError::Unavailable { .. } => "capability surface profile is unavailable",
        CapabilityResolveError::Internal { .. } => {
            "capability surface profile could not be resolved"
        }
        _ => "capability surface profile resolution failed",
    };
    RebornLoopDriverHostError::InvalidRequest {
        reason: reason.to_string(),
    }
}

fn model_route_error_to_host_error(error: ModelRouteError) -> RebornLoopDriverHostError {
    RebornLoopDriverHostError::InvalidRequest {
        reason: format!("model route resolution failed: {}", error.kind().as_str()),
    }
}

fn slot_for_model_profile(
    run_context: &LoopRunContext,
) -> Result<ModelSlot, RebornLoopDriverHostError> {
    ModelSlot::from_model_profile_id(&run_context.resolved_run_profile.model_profile_id).ok_or_else(
        || RebornLoopDriverHostError::InvalidRequest {
            reason: "model profile is not supported by the model route resolver".to_string(),
        },
    )
}

fn persisted_profile_id(profile_id: &RunProfileId) -> RunProfileId {
    if profile_id.is_interactive_default() {
        RunProfileId::default_profile()
    } else {
        profile_id.clone()
    }
}

fn validate_thread_scope(
    thread_scope: &ThreadScope,
    run_context: &LoopRunContext,
) -> Result<(), RebornLoopDriverHostError> {
    // Reborn text-only hosts currently wrap `ironclaw_threads::ThreadScope`,
    // whose production transcript boundary is agent-scoped. Agentless turn
    // scopes are rejected here until that lower thread boundary grows an
    // explicit agentless thread scope.
    if run_context.scope.agent_id.as_ref() != Some(&thread_scope.agent_id) {
        return Err(RebornLoopDriverHostError::ScopeMismatch {
            reason: "text-only loop host requires a matching agent-scoped thread".to_string(),
        });
    }
    if thread_scope.tenant_id != run_context.scope.tenant_id
        || thread_scope.project_id != run_context.scope.project_id
    {
        return Err(RebornLoopDriverHostError::ScopeMismatch {
            reason: "thread scope does not match loop run scope".to_string(),
        });
    }
    Ok(())
}

fn turn_error_to_host_error(error: TurnError) -> AgentLoopHostError {
    match error {
        TurnError::Unauthorized => AgentLoopHostError::new(
            AgentLoopHostErrorKind::Unauthorized,
            "checkpoint state access was unauthorized",
        ),
        TurnError::InvalidRequest { .. } => AgentLoopHostError::new(
            AgentLoopHostErrorKind::InvalidInvocation,
            "checkpoint state request is invalid",
        ),
        TurnError::Unavailable { .. } => AgentLoopHostError::new(
            AgentLoopHostErrorKind::Unavailable,
            "checkpoint state store is unavailable",
        ),
        TurnError::ScopeNotFound => AgentLoopHostError::new(
            AgentLoopHostErrorKind::CheckpointRejected,
            "checkpoint state scope was not found for this loop run",
        ),
        TurnError::Conflict { .. } => AgentLoopHostError::new(
            AgentLoopHostErrorKind::CheckpointRejected,
            "checkpoint state write conflicted with current turn state",
        ),
        TurnError::InvalidTransition { .. } => AgentLoopHostError::new(
            AgentLoopHostErrorKind::CheckpointRejected,
            "checkpoint state write was invalid for current turn state",
        ),
        TurnError::LeaseMismatch => AgentLoopHostError::new(
            AgentLoopHostErrorKind::CheckpointRejected,
            "checkpoint state write lease no longer matches current run",
        ),
        TurnError::ThreadBusy(_) | TurnError::AdmissionRejected(_) => AgentLoopHostError::new(
            AgentLoopHostErrorKind::Unavailable,
            "checkpoint state store returned unsupported turn admission status",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId};
    use ironclaw_turns::{
        InMemoryCheckpointStateStore, InMemoryLoopCheckpointStore, InMemoryRunProfileResolver,
        RunProfileResolver, TurnCheckpointId, TurnId, TurnRunId, TurnScope,
        run_profile::{
            AgentLoopHostErrorKind, CheckpointSchemaId, InMemoryLoopHostMilestoneSink,
            LoadCheckpointPayloadRequest, LoopCheckpointKind, LoopCheckpointRequest,
            LoopRunContext, RunProfileResolutionRequest, StageCheckpointPayloadRequest,
        },
    };

    async fn test_run_context() -> LoopRunContext {
        let tenant_id = TenantId::new("tenant-surf-prompt-test").unwrap();
        let agent_id = AgentId::new("agent-surf-prompt-test").unwrap();
        let project_id = ProjectId::new("project-surf-prompt-test").unwrap();
        let thread_id = ThreadId::new("thread-surf-prompt-test").unwrap();
        let turn_scope = TurnScope::new(tenant_id, Some(agent_id), Some(project_id), thread_id);
        let resolved = InMemoryRunProfileResolver::default()
            .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
            .await
            .unwrap();
        LoopRunContext::new(turn_scope, TurnId::new(), TurnRunId::new(), resolved)
    }

    fn test_checkpoint_port(
        context: LoopRunContext,
    ) -> (
        HostManagedLoopCheckpointPort,
        Arc<InMemoryCheckpointStateStore>,
        Arc<InMemoryLoopCheckpointStore>,
    ) {
        let state_store = Arc::new(InMemoryCheckpointStateStore::default());
        let checkpoint_store = Arc::new(InMemoryLoopCheckpointStore::default());
        let milestone_sink = Arc::new(InMemoryLoopHostMilestoneSink::default());
        let port = HostManagedLoopCheckpointPort::new(
            context,
            state_store.clone(),
            checkpoint_store.clone(),
            milestone_sink,
        );
        (port, state_store, checkpoint_store)
    }

    #[tokio::test]
    async fn checkpoint_port_load_payload_roundtrips_staged_payload() {
        let context = test_run_context().await;
        let expected_schema_id = context.checkpoint_schema_id.clone();
        let expected_schema_version = context.checkpoint_schema_version;
        let (port, _state_store, _checkpoint_store) = test_checkpoint_port(context);
        let payload = br#"{"iteration":3}"#.to_vec();

        let state_ref = port
            .stage_checkpoint_payload(StageCheckpointPayloadRequest {
                kind: LoopCheckpointKind::BeforeSideEffect,
                schema_id: expected_schema_id.as_str().to_string(),
                payload: payload.clone(),
            })
            .await
            .expect("stage checkpoint payload");
        let checkpoint_id = port
            .checkpoint(LoopCheckpointRequest {
                kind: LoopCheckpointKind::BeforeSideEffect,
                state_ref,
            })
            .await
            .expect("write checkpoint metadata");

        let loaded = port
            .load_checkpoint_payload(LoadCheckpointPayloadRequest {
                checkpoint_id,
                expected_schema_id: expected_schema_id.clone(),
                expected_schema_version,
            })
            .await
            .expect("load checkpoint payload");

        assert_eq!(loaded.kind, LoopCheckpointKind::BeforeSideEffect);
        assert_eq!(loaded.schema_id, expected_schema_id);
        assert_eq!(loaded.schema_version, expected_schema_version);
        assert_eq!(loaded.payload.as_bytes(), payload.as_slice());
    }

    #[tokio::test]
    async fn checkpoint_port_load_payload_rejects_schema_mismatch() {
        let context = test_run_context().await;
        let expected_schema_id = context.checkpoint_schema_id.clone();
        let expected_schema_version = context.checkpoint_schema_version;
        let (port, _state_store, _checkpoint_store) = test_checkpoint_port(context);
        let state_ref = port
            .stage_checkpoint_payload(StageCheckpointPayloadRequest {
                kind: LoopCheckpointKind::BeforeModel,
                schema_id: expected_schema_id.as_str().to_string(),
                payload: b"{}".to_vec(),
            })
            .await
            .expect("stage checkpoint payload");
        let checkpoint_id = port
            .checkpoint(LoopCheckpointRequest {
                kind: LoopCheckpointKind::BeforeModel,
                state_ref,
            })
            .await
            .expect("write checkpoint metadata");

        let error = port
            .load_checkpoint_payload(LoadCheckpointPayloadRequest {
                checkpoint_id,
                expected_schema_id: CheckpointSchemaId::new("different_checkpoint_schema")
                    .expect("valid schema"),
                expected_schema_version,
            })
            .await
            .expect_err("schema mismatch must reject");

        assert_eq!(error.kind, AgentLoopHostErrorKind::Invalid);
    }

    #[tokio::test]
    async fn checkpoint_port_load_payload_rejects_schema_version_mismatch() {
        let context = test_run_context().await;
        let expected_schema_id = context.checkpoint_schema_id.clone();
        let stored_schema_version = context.checkpoint_schema_version;
        let (port, _state_store, _checkpoint_store) = test_checkpoint_port(context);
        let state_ref = port
            .stage_checkpoint_payload(StageCheckpointPayloadRequest {
                kind: LoopCheckpointKind::BeforeModel,
                schema_id: expected_schema_id.as_str().to_string(),
                payload: b"{}".to_vec(),
            })
            .await
            .expect("stage checkpoint payload");
        let checkpoint_id = port
            .checkpoint(LoopCheckpointRequest {
                kind: LoopCheckpointKind::BeforeModel,
                state_ref,
            })
            .await
            .expect("write checkpoint metadata");

        // Load with a bumped schema version — stored = N, expected = N+1.
        let bumped_version =
            ironclaw_turns::RunProfileVersion::new(stored_schema_version.as_u64() + 1);

        let error = port
            .load_checkpoint_payload(LoadCheckpointPayloadRequest {
                checkpoint_id,
                expected_schema_id,
                expected_schema_version: bumped_version,
            })
            .await
            .expect_err("schema version mismatch must reject");

        assert_eq!(error.kind, AgentLoopHostErrorKind::Invalid);
    }

    #[tokio::test]
    async fn checkpoint_port_load_payload_missing_metadata_is_unavailable() {
        let context = test_run_context().await;
        let expected_schema_id = context.checkpoint_schema_id.clone();
        let expected_schema_version = context.checkpoint_schema_version;
        let (port, _state_store, _checkpoint_store) = test_checkpoint_port(context);

        let error = port
            .load_checkpoint_payload(LoadCheckpointPayloadRequest {
                checkpoint_id: TurnCheckpointId::new(),
                expected_schema_id,
                expected_schema_version,
            })
            .await
            .expect_err("missing metadata must reject");

        assert_eq!(error.kind, AgentLoopHostErrorKind::Unavailable);
    }

    #[tokio::test]
    async fn checkpoint_port_load_payload_missing_state_record_is_unavailable() {
        let context = test_run_context().await;
        let expected_schema_id = context.checkpoint_schema_id.clone();
        let expected_schema_version = context.checkpoint_schema_version;
        let (port, _state_store, checkpoint_store) = test_checkpoint_port(context.clone());
        let missing_state_ref =
            LoopCheckpointStateRef::for_run(&context, "missing-state").expect("valid ref");
        let metadata = checkpoint_store
            .put_loop_checkpoint(PutLoopCheckpointRequest {
                scope: context.scope.clone(),
                turn_id: context.turn_id,
                run_id: context.run_id,
                state_ref: missing_state_ref,
                schema_id: expected_schema_id.clone(),
                schema_version: expected_schema_version,
                kind: LoopCheckpointKind::BeforeBlock,
            })
            .await
            .expect("write checkpoint metadata");

        let error = port
            .load_checkpoint_payload(LoadCheckpointPayloadRequest {
                checkpoint_id: metadata.checkpoint_id,
                expected_schema_id,
                expected_schema_version,
            })
            .await
            .expect_err("missing state payload must reject");

        assert_eq!(error.kind, AgentLoopHostErrorKind::Unavailable);
    }
}
