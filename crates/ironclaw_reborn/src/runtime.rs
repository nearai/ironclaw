//! Default Reborn runtime-loop composition.

use std::{error::Error, fmt, sync::Arc};

use ironclaw_events::SecurityAuditSink;
use ironclaw_host_api::CapabilityId;
use ironclaw_loop_support::{
    CapabilitySurfaceProfileResolver, CompositeTurnRunWakeNotifier,
    DecoratingLoopCapabilityPortFactory, HostIdentityContextSource, HostInputQueue,
    HostManagedModelGateway, HostSkillContextSource, LoopAttachmentReadPort,
    LoopCapabilityPortDecorator, LoopCapabilityPortFactory, LoopCapabilityResultWriter,
    ProductLiveCancellationReadiness, RunCancellationFactory, SpawnSubagentFlavorDescriptor,
    SpawnSubagentInputCodec, SubagentDefinitionResolver, SubagentPromptComposer,
    SubagentPromptMaterialSource, SubagentSpawnCapabilityPort, SubagentSpawnDeps,
    SubagentSpawnGoalStore, SubagentSpawnLimits, verify_product_live_cancellation_probe,
};
use ironclaw_threads::{SessionThreadService, ThreadScope};
use ironclaw_turns::{
    AgentLoopDriverError, CheckpointStateStore, DefaultTurnCoordinator,
    DefaultTurnLifecycleEventBus, LifecyclePublicationErrorPort, LifecyclePublishingTurnStateStore,
    LoopCheckpointStore, RunProfileResolver, TurnCommittedEventObserver, TurnEventSink,
    TurnLifecycleEventBus, TurnRunWakeNotifier, TurnSpawnTreePort, TurnSpawnTreeStateStore,
    TurnStateStore,
    loop_exit::LoopExitEvidencePort,
    run_profile::{
        AgentLoopHostError, CommunicationContextProvider, InstructionSafetyContext,
        LoopCapabilityPort, LoopHostMilestoneSink, LoopModelBudgetAccountant, LoopModelPolicyGuard,
        LoopRunContext,
    },
    runner::TurnRunTransitionPort,
};

use crate::{
    app_loop_family::build_loop_family_registry,
    driver_registry::{DriverRegistry, DriverRegistryError},
    loop_driver_host::{
        HookDispatcherBuilderFactory, RebornLoopDriverHostFactory, TextOnlyLoopHostConfig,
    },
    loop_exit_applier::{LoopExitApplier, ThreadCheckpointLoopExitEvidencePort},
    model_routes::ModelRouteResolver,
    planned_driver_factory::{
        DefaultPlannedDriverRegistrationError, default_planned_run_profile_resolver,
        register_default_planned_driver, register_default_text_only_driver,
        register_subagent_planned_driver,
    },
    subagent::{
        capability_surface::SubagentCapabilitySurfaceResolver,
        completion_observer::SubagentCompletionObserver, flavors,
        gate_resolution::BoundedSubagentGateResolutionStore, goal_store::SubagentGoalStore,
        prompt_material::GateBackedSubagentPromptMaterialSource,
    },
    text_loop_driver::TextOnlyModelReplyDriverConfig,
    turn_runner::{
        TurnRunnerWakeReceiver, TurnRunnerWakeSender, TurnRunnerWorker, TurnRunnerWorkerConfig,
    },
};

type PlannedRuntimeWakeChannel = (TurnRunnerWakeSender, TurnRunnerWakeReceiver);

#[derive(Debug, Clone, Default)]
pub struct DefaultPlannedRuntimeConfig {
    pub worker: TurnRunnerWorkerConfig,
    pub text_only_driver: TextOnlyModelReplyDriverConfig,
    pub host: TextOnlyLoopHostConfig,
}

pub trait RuntimeTurnStateStore:
    TurnSpawnTreeStateStore
    + TurnRunTransitionPort
    + ironclaw_turns::TurnEventProjectionSource
    + Send
    + Sync
{
}

impl<T> RuntimeTurnStateStore for T where
    T: TurnSpawnTreeStateStore
        + TurnRunTransitionPort
        + ironclaw_turns::TurnEventProjectionSource
        + Send
        + Sync
{
}

pub struct DefaultPlannedRuntimeParts<G>
where
    G: HostManagedModelGateway + ?Sized + Send + Sync + 'static,
{
    pub turn_state: Arc<dyn RuntimeTurnStateStore>,
    pub thread_service: Arc<dyn SessionThreadService>,
    pub thread_scope: ThreadScope,
    pub model_gateway: Arc<G>,
    pub checkpoint_state_store: Arc<dyn CheckpointStateStore>,
    pub loop_checkpoint_store: Arc<dyn LoopCheckpointStore>,
    pub milestone_sink: Arc<dyn LoopHostMilestoneSink>,
    pub capability_factory: Arc<dyn LoopCapabilityPortFactory>,
    pub capability_surface_resolver: Arc<dyn CapabilitySurfaceProfileResolver>,
    pub capability_result_writer: Arc<dyn LoopCapabilityResultWriter>,
    pub subagent_goal_store: Arc<dyn RuntimeSubagentGoalStore>,
    pub subagent_gate_store: Arc<BoundedSubagentGateResolutionStore>,
    pub subagent_definition_resolver: Arc<dyn SubagentDefinitionResolver>,
    pub subagent_spawn_input_codec: Arc<dyn SpawnSubagentInputCodec>,
    pub subagent_spawn_limits: SubagentSpawnLimits,
    pub loop_exit_evidence: Arc<dyn LoopExitEvidencePort>,
    pub config: DefaultPlannedRuntimeConfig,
    pub model_route_resolver: Option<Arc<dyn ModelRouteResolver>>,
    pub cancellation_factory: Option<Arc<dyn RunCancellationFactory>>,
    pub skill_context_source: Option<Arc<dyn HostSkillContextSource>>,
    /// Reads landed attachment bytes so the model port can build multimodal
    /// image parts for vision-capable models. Genuinely optional, not a
    /// fail-closed gap: a reader can only exist where a local runtime composed a
    /// workspace filesystem to read landed bytes back from. Compositions without
    /// one have nothing to read, so `None` correctly degrades to the transcript's
    /// textual `<attachments>` pointer (the same fallback a text-only model
    /// gets) rather than failing the turn.
    pub attachment_read_port: Option<Arc<dyn LoopAttachmentReadPort>>,
    pub input_queue: Option<Arc<dyn HostInputQueue>>,
    /// Required by live planned-runtime composition. Helper-level tests may use
    /// a no-op implementation, but the type signature always requires a valid
    /// identity context source.
    pub identity_context_source: Arc<dyn HostIdentityContextSource>,
    /// Product-live readiness extensions. `RebornLoopDriverHostFactory`
    /// defaults these to no-op implementations so helper tests keep compiling.
    /// `build_product_live_planned_runtime` fails closed when any of them is
    /// `None`, matching the cancellation/identity contract.
    pub model_policy_guard: Option<Arc<dyn LoopModelPolicyGuard>>,
    pub model_budget_accountant: Option<Arc<dyn LoopModelBudgetAccountant>>,
    pub safety_context: Option<InstructionSafetyContext>,
    pub hook_security_audit_sink: Option<Arc<dyn SecurityAuditSink>>,
    pub turn_event_sink: Option<Arc<dyn TurnEventSink>>,
    /// Per-run hook dispatcher builder factory. `None` (the default) leaves
    /// the hook framework dormant: no dispatcher is composed and the runtime
    /// behaves exactly as it did before hooks existed.
    pub hook_dispatcher_builder_factory: Option<HookDispatcherBuilderFactory>,
    pub communication_context_provider: Option<Arc<dyn CommunicationContextProvider>>,
}

pub trait RuntimeSubagentGoalStore:
    SubagentGoalStore + SubagentSpawnGoalStore + Send + Sync
{
}

impl<T> RuntimeSubagentGoalStore for T where
    T: SubagentGoalStore + SubagentSpawnGoalStore + Send + Sync
{
}

pub struct RebornRuntimeLoopComposition<S, G>
where
    S: SessionThreadService + ?Sized + Send + Sync + 'static,
    G: HostManagedModelGateway + ?Sized + Send + Sync + 'static,
{
    pub driver_registry: Arc<DriverRegistry>,
    pub run_profile_resolver: Arc<dyn RunProfileResolver>,
    pub coordinator: Arc<dyn ironclaw_turns::TurnCoordinator>,
    pub host_factory: Arc<RebornLoopDriverHostFactory<S, G>>,
    pub worker: Arc<TurnRunnerWorker>,
    pub wake_sender: TurnRunnerWakeSender,
}

#[derive(Debug)]
pub enum DefaultPlannedRuntimeBuildError {
    DriverRegistry(DriverRegistryError),
    PlannedDriver(DefaultPlannedDriverRegistrationError),
    RunProfile(String),
    SubagentCompletion(String),
}

impl fmt::Display for DefaultPlannedRuntimeBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DriverRegistry(error) => write!(formatter, "driver registry failed: {error}"),
            Self::PlannedDriver(error) => write!(formatter, "planned driver failed: {error}"),
            Self::RunProfile(error) => write!(formatter, "run profile resolver failed: {error}"),
            Self::SubagentCompletion(error) => {
                write!(formatter, "subagent completion wiring failed: {error}")
            }
        }
    }
}

impl Error for DefaultPlannedRuntimeBuildError {}

impl From<DriverRegistryError> for DefaultPlannedRuntimeBuildError {
    fn from(error: DriverRegistryError) -> Self {
        Self::DriverRegistry(error)
    }
}

impl From<DefaultPlannedDriverRegistrationError> for DefaultPlannedRuntimeBuildError {
    fn from(error: DefaultPlannedDriverRegistrationError) -> Self {
        Self::PlannedDriver(error)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProductLiveRuntimeReadinessComponent {
    ModelRouteResolver,
    InputQueue,
    CancellationFactory,
    IdentityContextSource,
    ModelPolicyGuard,
    ModelBudgetAccountant,
    SafetyContext,
}

impl ProductLiveRuntimeReadinessComponent {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ModelRouteResolver => "model_route_resolver",
            Self::InputQueue => "input_queue",
            Self::CancellationFactory => "cancellation_factory",
            Self::IdentityContextSource => "identity_context_source",
            Self::ModelPolicyGuard => "model_policy_guard",
            Self::ModelBudgetAccountant => "model_budget_accountant",
            Self::SafetyContext => "safety_context",
        }
    }
}

#[derive(Debug)]
pub enum ProductLiveRuntimeBuildError {
    Missing(ProductLiveRuntimeReadinessComponent),
    Inert(ProductLiveRuntimeReadinessComponent),
    Probe {
        component: ProductLiveRuntimeReadinessComponent,
        source: AgentLoopHostError,
    },
    Runtime(DefaultPlannedRuntimeBuildError),
}

impl fmt::Display for ProductLiveRuntimeBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing(component) => {
                write!(
                    formatter,
                    "product live runtime missing {}",
                    component.as_str()
                )
            }
            Self::Inert(component) => {
                write!(
                    formatter,
                    "product live runtime has inert {}",
                    component.as_str()
                )
            }
            Self::Probe { component, source } => {
                write!(
                    formatter,
                    "product live runtime could not probe {}: {}",
                    component.as_str(),
                    source,
                )
            }
            Self::Runtime(error) => write!(formatter, "{error}"),
        }
    }
}

impl Error for ProductLiveRuntimeBuildError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Probe { source, .. } => Some(source),
            Self::Runtime(error) => Some(error),
            Self::Missing(_) | Self::Inert(_) => None,
        }
    }
}

pub fn build_product_live_planned_runtime<G>(
    mut parts: DefaultPlannedRuntimeParts<G>,
) -> Result<RebornRuntimeLoopComposition<dyn SessionThreadService, G>, ProductLiveRuntimeBuildError>
where
    G: HostManagedModelGateway + ?Sized + Send + Sync + 'static,
{
    if parts.model_route_resolver.is_none() {
        return Err(ProductLiveRuntimeBuildError::Missing(
            ProductLiveRuntimeReadinessComponent::ModelRouteResolver,
        ));
    }
    if parts.input_queue.is_none() {
        return Err(ProductLiveRuntimeBuildError::Missing(
            ProductLiveRuntimeReadinessComponent::InputQueue,
        ));
    }
    if parts.model_policy_guard.is_none() {
        return Err(ProductLiveRuntimeBuildError::Missing(
            ProductLiveRuntimeReadinessComponent::ModelPolicyGuard,
        ));
    }
    if parts.model_budget_accountant.is_none() {
        return Err(ProductLiveRuntimeBuildError::Missing(
            ProductLiveRuntimeReadinessComponent::ModelBudgetAccountant,
        ));
    }
    if parts.safety_context.is_none() {
        return Err(ProductLiveRuntimeBuildError::Missing(
            ProductLiveRuntimeReadinessComponent::SafetyContext,
        ));
    }
    let Some(cancellation_factory) = parts.cancellation_factory.clone() else {
        return Err(ProductLiveRuntimeBuildError::Missing(
            ProductLiveRuntimeReadinessComponent::CancellationFactory,
        ));
    };
    let readiness =
        verify_product_live_cancellation_probe(cancellation_factory.as_ref()).map_err(|error| {
            ProductLiveRuntimeBuildError::Probe {
                component: ProductLiveRuntimeReadinessComponent::CancellationFactory,
                source: error,
            }
        })?;
    if readiness != ProductLiveCancellationReadiness::ExternallyControllable {
        return Err(ProductLiveRuntimeBuildError::Inert(
            ProductLiveRuntimeReadinessComponent::CancellationFactory,
        ));
    }
    let turn_state_store: Arc<dyn TurnStateStore> = parts.turn_state.clone();
    parts.loop_exit_evidence = Arc::new(
        ThreadCheckpointLoopExitEvidencePort::new_with_thread_scope(
            Arc::clone(&parts.thread_service),
            turn_state_store,
            Arc::clone(&parts.loop_checkpoint_store),
            parts.thread_scope.clone(),
        )
        .with_checkpoint_state_store(Arc::clone(&parts.checkpoint_state_store))
        .with_cancellation_factory(cancellation_factory),
    );
    build_default_planned_runtime(parts).map_err(ProductLiveRuntimeBuildError::Runtime)
}

fn local_development_noop_safety_context() -> InstructionSafetyContext {
    tracing::debug!(
        "using local-development no-op instruction safety context; configure a real instruction safety scanner before product-live use"
    );
    InstructionSafetyContext::local_development_noop()
}

pub fn build_default_planned_runtime<G>(
    parts: DefaultPlannedRuntimeParts<G>,
) -> Result<
    RebornRuntimeLoopComposition<dyn SessionThreadService, G>,
    DefaultPlannedRuntimeBuildError,
>
where
    G: HostManagedModelGateway + ?Sized + Send + Sync + 'static,
{
    build_default_planned_runtime_with_optional_wake_channel(parts, None)
}

pub fn build_default_planned_runtime_with_wake_channel<G>(
    parts: DefaultPlannedRuntimeParts<G>,
    wake_channel: PlannedRuntimeWakeChannel,
) -> Result<
    RebornRuntimeLoopComposition<dyn SessionThreadService, G>,
    DefaultPlannedRuntimeBuildError,
>
where
    G: HostManagedModelGateway + ?Sized + Send + Sync + 'static,
{
    build_default_planned_runtime_with_optional_wake_channel(parts, Some(wake_channel))
}

fn build_default_planned_runtime_with_optional_wake_channel<G>(
    parts: DefaultPlannedRuntimeParts<G>,
    wake_channel: Option<PlannedRuntimeWakeChannel>,
) -> Result<
    RebornRuntimeLoopComposition<dyn SessionThreadService, G>,
    DefaultPlannedRuntimeBuildError,
>
where
    G: HostManagedModelGateway + ?Sized + Send + Sync + 'static,
{
    let mut registry = DriverRegistry::new();
    register_default_text_only_driver(&mut registry, parts.config.text_only_driver)?;
    let family_registry = build_loop_family_registry().map_err(|error| {
        DefaultPlannedRuntimeBuildError::PlannedDriver(
            DefaultPlannedDriverRegistrationError::DriverBuild(
                AgentLoopDriverError::InvalidRequest {
                    reason: error.to_string(),
                },
            ),
        )
    })?;
    register_default_planned_driver(&mut registry, Arc::clone(&family_registry))?;
    register_subagent_planned_driver(&mut registry, family_registry)?;
    let driver_registry = Arc::new(registry);

    let resolver = Arc::new(
        default_planned_run_profile_resolver()
            .map_err(|error| DefaultPlannedRuntimeBuildError::RunProfile(error.to_string()))?,
    );
    let run_profile_resolver: Arc<dyn RunProfileResolver> = resolver;

    let (wake_sender, wake_receiver) = wake_channel.unwrap_or_else(TurnRunnerWakeReceiver::new);
    let worker_wake_notifier: Arc<dyn TurnRunWakeNotifier> = Arc::new(wake_sender.clone());
    // When a cancellation factory is supplied, fan-out each coordinator wake to
    // BOTH the worker AND the factory's `notify_run_wake` observer. Without
    // this composite, the worker still wakes but retained product run handles
    // never flip on `cancel_run` — breaking end-to-end product-live
    // cancellation observation.
    let wake_notifier: Arc<dyn TurnRunWakeNotifier> = match parts.cancellation_factory.clone() {
        Some(factory) => Arc::new(CompositeTurnRunWakeNotifier::new(
            worker_wake_notifier,
            factory,
        )),
        None => worker_wake_notifier,
    };
    let turn_state_for_observer: Arc<dyn TurnSpawnTreeStateStore> = parts.turn_state.clone();
    let completion_observer = Arc::new(SubagentCompletionObserver::new_unbound(
        Arc::clone(&parts.subagent_gate_store),
        Arc::clone(&parts.subagent_goal_store) as Arc<dyn SubagentSpawnGoalStore>,
        turn_state_for_observer,
        Arc::clone(&parts.capability_result_writer),
        Arc::clone(&parts.thread_service),
    ));
    let subagent_completion_observer: Arc<dyn TurnCommittedEventObserver> =
        completion_observer.clone();
    let lifecycle_bus = Arc::new(DefaultTurnLifecycleEventBus::new());
    lifecycle_bus
        .subscribe_required(Arc::clone(&subagent_completion_observer))
        .map_err(|error| DefaultPlannedRuntimeBuildError::SubagentCompletion(error.to_string()))?;
    if let Some(turn_event_sink) = parts.turn_event_sink.clone() {
        lifecycle_bus
            .subscribe_best_effort(turn_event_sink)
            .map_err(|error| {
                DefaultPlannedRuntimeBuildError::SubagentCompletion(error.to_string())
            })?;
    }
    let turn_state = Arc::new(LifecyclePublishingTurnStateStore::new(
        Arc::clone(&parts.turn_state),
        lifecycle_bus,
    ));
    let publication_error_port: Arc<dyn LifecyclePublicationErrorPort> = turn_state.clone();
    let base_coordinator = DefaultTurnCoordinator::new(Arc::clone(&turn_state))
        .with_run_profile_resolver(Arc::clone(&run_profile_resolver))
        .with_wake_notifier(Arc::clone(&wake_notifier))
        .with_lifecycle_publication_error_port(publication_error_port);
    let base_coordinator_arc = Arc::new(base_coordinator);
    let child_runs: Arc<dyn TurnSpawnTreePort> = base_coordinator_arc.clone();
    let coordinator: Arc<dyn ironclaw_turns::TurnCoordinator> = base_coordinator_arc;
    completion_observer
        .bind_coordinator(Arc::clone(&coordinator))
        .map_err(|error| DefaultPlannedRuntimeBuildError::SubagentCompletion(error.to_string()))?;

    let turn_state_store: Arc<dyn TurnStateStore> = turn_state.clone();
    let subagent_prompt_source: Arc<dyn SubagentPromptMaterialSource> =
        Arc::new(GateBackedSubagentPromptMaterialSource::new(
            Arc::clone(&parts.subagent_goal_store),
            Arc::clone(&parts.subagent_gate_store),
            Arc::clone(&parts.thread_service),
        ));
    let subagent_prompt_composer = SubagentPromptComposer::new(Arc::clone(&subagent_prompt_source));
    let spawn_decorator = Arc::new(SubagentSpawnCapabilityDecorator::new(
        SubagentSpawnDeps {
            coordinator: Arc::clone(&coordinator) as Arc<dyn ironclaw_turns::TurnCoordinator>,
            child_runs,
            turn_state_store: Arc::clone(&parts.turn_state) as Arc<dyn TurnSpawnTreeStateStore>,
            thread_service: Arc::clone(&parts.thread_service),
            goal_store: Arc::clone(&parts.subagent_goal_store) as Arc<dyn SubagentSpawnGoalStore>,
            gate_store: Arc::clone(&parts.subagent_gate_store)
                as Arc<dyn ironclaw_loop_support::SubagentGateResolutionStore>,
            definition_resolver: Arc::clone(&parts.subagent_definition_resolver),
            spawn_input_codec: Arc::clone(&parts.subagent_spawn_input_codec),
            result_writer: Arc::clone(&parts.capability_result_writer),
        },
        parts.subagent_spawn_limits,
        flavors::builtin_flavor_catalog(),
    )?);
    let capability_factory: Arc<dyn LoopCapabilityPortFactory> = Arc::new(
        DecoratingLoopCapabilityPortFactory::new(parts.capability_factory)
            .with_decorator(spawn_decorator),
    );
    let capability_surface_resolver: Arc<dyn CapabilitySurfaceProfileResolver> =
        Arc::new(SubagentCapabilitySurfaceResolver::new(
            parts.capability_surface_resolver,
            Arc::clone(&subagent_prompt_source),
        ));
    let safety_context = parts
        .safety_context
        .unwrap_or_else(local_development_noop_safety_context);
    let mut host_factory = RebornLoopDriverHostFactory::new(
        Arc::clone(&parts.thread_service),
        parts.thread_scope,
        Arc::clone(&parts.model_gateway),
        parts.checkpoint_state_store,
        turn_state_store,
        Arc::clone(&parts.loop_checkpoint_store),
        parts.milestone_sink,
        parts.config.host,
        safety_context,
    )
    .with_profiled_capability_port_factory(capability_factory, capability_surface_resolver)
    .with_subagent_prompt_composer(subagent_prompt_composer)
    .with_driver_requirements(driver_registry.requirements_snapshot());
    if let Some(resolver) = parts.model_route_resolver {
        host_factory = host_factory.with_model_route_resolver(resolver);
    }
    if let Some(factory) = parts.cancellation_factory {
        host_factory = host_factory.with_cancellation_factory(factory);
    }
    if let Some(port) = parts.attachment_read_port {
        host_factory = host_factory.with_attachment_read_port(port);
    }
    if let Some(source) = parts.skill_context_source {
        host_factory = host_factory.with_skill_context_source(source);
    }
    if let Some(queue) = parts.input_queue {
        host_factory = host_factory.with_input_queue(queue);
    }
    if let Some(guard) = parts.model_policy_guard {
        host_factory = host_factory.with_model_policy_guard(guard);
    }
    if let Some(accountant) = parts.model_budget_accountant {
        host_factory = host_factory.with_model_budget_accountant(accountant);
    }
    if let Some(factory) = parts.hook_dispatcher_builder_factory {
        host_factory = host_factory.with_hook_dispatcher_builder_factory(move || factory());
    }
    if let Some(provider) = parts.communication_context_provider {
        host_factory = host_factory.with_communication_context_provider(provider);
    }
    if let Some(sink) = parts.hook_security_audit_sink {
        host_factory = host_factory.with_hook_security_audit_sink(sink);
    }
    host_factory = host_factory.with_identity_context_source(parts.identity_context_source);
    let host_factory = Arc::new(host_factory);

    let transition_port: Arc<dyn TurnRunTransitionPort> = turn_state;
    let loop_exit_applier = Arc::new(LoopExitApplier::new(
        Arc::clone(&transition_port),
        parts.loop_exit_evidence,
    ));
    let worker = Arc::new(TurnRunnerWorker::new(
        parts.config.worker,
        transition_port,
        loop_exit_applier,
        Arc::clone(&driver_registry),
        host_factory.clone(),
        wake_receiver,
    ));

    Ok(
        RebornRuntimeLoopComposition::<dyn SessionThreadService, G> {
            driver_registry,
            run_profile_resolver,
            coordinator,
            host_factory,
            worker,
            wake_sender,
        },
    )
}

struct SubagentSpawnCapabilityDecorator {
    spawn_deps: Arc<SubagentSpawnDeps>,
    spawn_id: CapabilityId,
    spawn_limits: SubagentSpawnLimits,
    /// Schema precomputed once at construction time so `decorate()` does not
    /// rebuild it on every loop run.
    parameters_schema: Arc<serde_json::Value>,
}

impl SubagentSpawnCapabilityDecorator {
    fn new(
        spawn_deps: SubagentSpawnDeps,
        spawn_limits: SubagentSpawnLimits,
        flavor_catalog: Vec<SpawnSubagentFlavorDescriptor>,
    ) -> Result<Self, DefaultPlannedRuntimeBuildError> {
        let spawn_id =
            CapabilityId::new(ironclaw_loop_support::DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID)
                .map_err(|error| DefaultPlannedRuntimeBuildError::RunProfile(error.to_string()))?;
        let parameters_schema = Arc::new(
            ironclaw_loop_support::build_spawn_subagent_parameters_schema(&flavor_catalog),
        );
        Ok(Self {
            spawn_deps: Arc::new(spawn_deps),
            spawn_id,
            spawn_limits,
            parameters_schema,
        })
    }
}

impl LoopCapabilityPortDecorator for SubagentSpawnCapabilityDecorator {
    fn decorate(
        &self,
        run_context: &LoopRunContext,
        inner: Arc<dyn LoopCapabilityPort>,
    ) -> Arc<dyn LoopCapabilityPort> {
        // Arc::clone is a cheap ref-count bump — avoids deep-cloning the JSON
        // schema tree on every decorate() call (the schema is rendered to a
        // serde_json::Value only at the single render site in
        // spawn_tool_definition / spawn_descriptor when the model requests it).
        Arc::new(SubagentSpawnCapabilityPort::new_with_schema(
            inner,
            run_context.clone(),
            self.spawn_id.clone(),
            self.spawn_limits,
            Arc::clone(&self.spawn_deps),
            Arc::clone(&self.parameters_schema),
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    use async_trait::async_trait;
    use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId};
    use ironclaw_turns::{
        InMemoryRunProfileResolver, RunProfileResolver, TurnId, TurnRunId, TurnScope,
        run_profile::{
            AgentLoopHostError, AgentLoopHostErrorKind, CapabilityBatchInvocation,
            CapabilityBatchOutcome, CapabilityInvocation, CapabilityOutcome, LoopCapabilityPort,
            LoopRunContext, RunProfileResolutionRequest, VisibleCapabilityRequest,
            VisibleCapabilitySurface,
        },
    };

    use ironclaw_loop_support::{
        DecoratingLoopCapabilityPortFactory, LoopCapabilityPortDecorator, LoopCapabilityPortFactory,
    };

    async fn test_run_context() -> LoopRunContext {
        let tenant_id = TenantId::new("tenant-runtime-test").unwrap();
        let agent_id = AgentId::new("agent-runtime-test").unwrap();
        let project_id = ProjectId::new("project-runtime-test").unwrap();
        let thread_id = ThreadId::new("thread-runtime-test").unwrap();
        let turn_scope = TurnScope::new(tenant_id, Some(agent_id), Some(project_id), thread_id);
        let resolved = InMemoryRunProfileResolver::default()
            .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
            .await
            .unwrap();
        LoopRunContext::new(turn_scope, TurnId::new(), TurnRunId::new(), resolved)
    }

    struct FailingFactory {
        error: AgentLoopHostError,
    }

    #[async_trait]
    impl LoopCapabilityPortFactory for FailingFactory {
        async fn create_capability_port(
            &self,
            _run_context: &LoopRunContext,
        ) -> Result<Arc<dyn LoopCapabilityPort>, AgentLoopHostError> {
            Err(self.error.clone())
        }
    }

    struct InnerPort {
        label: &'static str,
        log: Arc<Mutex<Vec<&'static str>>>,
    }

    #[async_trait]
    impl LoopCapabilityPort for InnerPort {
        async fn visible_capabilities(
            &self,
            _request: VisibleCapabilityRequest,
        ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
            self.log.lock().unwrap().push(self.label);
            Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::Unavailable,
                format!("{label} failed", label = self.label),
            ))
        }

        async fn invoke_capability(
            &self,
            _request: CapabilityInvocation,
        ) -> Result<CapabilityOutcome, AgentLoopHostError> {
            Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::Unavailable,
                format!("{label} unused", label = self.label),
            ))
        }

        async fn invoke_capability_batch(
            &self,
            _request: CapabilityBatchInvocation,
        ) -> Result<CapabilityBatchOutcome, AgentLoopHostError> {
            Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::Unavailable,
                format!("{label} unused", label = self.label),
            ))
        }
    }

    struct LoggingDecorator {
        label: &'static str,
        log: Arc<Mutex<Vec<&'static str>>>,
    }

    impl LoopCapabilityPortDecorator for LoggingDecorator {
        fn decorate(
            &self,
            _run_context: &LoopRunContext,
            inner: Arc<dyn LoopCapabilityPort>,
        ) -> Arc<dyn LoopCapabilityPort> {
            Arc::new(LoggingDecoratorPort {
                label: self.label,
                log: Arc::clone(&self.log),
                inner,
            })
        }
    }

    struct LoggingDecoratorPort {
        label: &'static str,
        log: Arc<Mutex<Vec<&'static str>>>,
        inner: Arc<dyn LoopCapabilityPort>,
    }

    #[async_trait]
    impl LoopCapabilityPort for LoggingDecoratorPort {
        async fn visible_capabilities(
            &self,
            request: VisibleCapabilityRequest,
        ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
            self.log.lock().unwrap().push(self.label);
            self.inner.visible_capabilities(request).await
        }

        async fn invoke_capability(
            &self,
            request: CapabilityInvocation,
        ) -> Result<CapabilityOutcome, AgentLoopHostError> {
            self.log.lock().unwrap().push(self.label);
            self.inner.invoke_capability(request).await
        }

        async fn invoke_capability_batch(
            &self,
            request: CapabilityBatchInvocation,
        ) -> Result<CapabilityBatchOutcome, AgentLoopHostError> {
            self.log.lock().unwrap().push(self.label);
            self.inner.invoke_capability_batch(request).await
        }
    }

    #[tokio::test]
    async fn decorating_factory_applies_layers_in_order() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let inner = Arc::new(InnerPort {
            label: "inner",
            log: Arc::clone(&log),
        });
        let factory =
            DecoratingLoopCapabilityPortFactory::new(Arc::new(StaticFactory { port: inner }))
                .with_decorator(Arc::new(LoggingDecorator {
                    label: "first",
                    log: Arc::clone(&log),
                }))
                .with_decorator(Arc::new(LoggingDecorator {
                    label: "second",
                    log: Arc::clone(&log),
                }));

        let port = factory
            .create_capability_port(&test_run_context().await)
            .await
            .expect("decorated capability port");

        let error = match port.visible_capabilities(VisibleCapabilityRequest).await {
            Ok(_) => panic!("inner port should fail"),
            Err(error) => error,
        };

        assert_eq!(error.kind, AgentLoopHostErrorKind::Unavailable);
        assert_eq!(&*log.lock().unwrap(), &["second", "first", "inner"]);
    }

    #[tokio::test]
    async fn decorating_factory_propagates_inner_error() {
        let decorate_calls = Arc::new(AtomicUsize::new(0));
        let factory = DecoratingLoopCapabilityPortFactory::new(Arc::new(FailingFactory {
            error: AgentLoopHostError::new(
                AgentLoopHostErrorKind::Unavailable,
                "inner factory failed",
            ),
        }))
        .with_decorator(Arc::new(NoopDecorator {
            decorate_calls: Arc::clone(&decorate_calls),
        }));

        let error = match factory
            .create_capability_port(&test_run_context().await)
            .await
        {
            Ok(_) => panic!("inner error should propagate"),
            Err(error) => error,
        };

        assert_eq!(error.kind, AgentLoopHostErrorKind::Unavailable);
        assert_eq!(error.safe_summary, "inner factory failed");
        assert_eq!(decorate_calls.load(Ordering::SeqCst), 0);
    }

    struct StaticFactory {
        port: Arc<dyn LoopCapabilityPort>,
    }

    #[async_trait]
    impl LoopCapabilityPortFactory for StaticFactory {
        async fn create_capability_port(
            &self,
            _run_context: &LoopRunContext,
        ) -> Result<Arc<dyn LoopCapabilityPort>, AgentLoopHostError> {
            Ok(Arc::clone(&self.port))
        }
    }

    struct NoopDecorator {
        decorate_calls: Arc<AtomicUsize>,
    }

    impl LoopCapabilityPortDecorator for NoopDecorator {
        fn decorate(
            &self,
            _run_context: &LoopRunContext,
            inner: Arc<dyn LoopCapabilityPort>,
        ) -> Arc<dyn LoopCapabilityPort> {
            self.decorate_calls.fetch_add(1, Ordering::SeqCst);
            inner
        }
    }

    // ── Gap 3: decorator non-empty catalog → schema enum present ─────────────

    #[test]
    fn builtin_flavor_catalog_threads_enum_into_schema() {
        // Verifies that `builtin_flavor_catalog()` — the source-of-truth
        // function the decorator wires into `SubagentSpawnCapabilityPort` — is
        // non-empty AND that the resulting `build_spawn_subagent_parameters_schema`
        // output includes an `enum` key containing all four expected flavor IDs
        // in registry order.
        //
        // This indirectly proves the threading: if the decorator passes a
        // non-empty catalog, the produced schema will have a satisfiable enum
        // constraint. The companion empty-catalog test (gap 1, loop_support)
        // confirms the absent-enum guard on the other side.
        use ironclaw_loop_support::build_spawn_subagent_parameters_schema;

        let catalog = crate::subagent::flavors::builtin_flavor_catalog();

        assert!(
            !catalog.is_empty(),
            "builtin_flavor_catalog must be non-empty"
        );
        assert_eq!(catalog.len(), 4, "expected exactly 4 builtin flavors");

        let schema = build_spawn_subagent_parameters_schema(&catalog);

        let enum_vals = schema["properties"]["subagent_type"]["enum"]
            .as_array()
            .expect("schema must have an 'enum' key when catalog is non-empty");

        assert_eq!(enum_vals.len(), 4);
        assert_eq!(enum_vals[0], serde_json::json!("general"));
        assert_eq!(enum_vals[1], serde_json::json!("explorer"));
        assert_eq!(enum_vals[2], serde_json::json!("coder"));
        assert_eq!(enum_vals[3], serde_json::json!("planner"));
    }
}
