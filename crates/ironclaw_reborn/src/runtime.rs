//! Default Reborn runtime-loop composition.

use std::{error::Error, fmt, sync::Arc};

use async_trait::async_trait;
use ironclaw_loop_support::{
    CapabilitySurfaceProfileResolver, CompositeTurnRunWakeNotifier, HostIdentityContextSource,
    HostInputQueue, HostManagedModelGateway, HostRuntimeLoopCapabilityPortFactory,
    HostSkillContextSource, ProductLiveCancellationReadiness, RunCancellationFactory,
    verify_product_live_cancellation_probe,
};
use ironclaw_threads::{SessionThreadService, ThreadScope};
use ironclaw_turns::{
    AgentLoopDriverError, CheckpointStateStore, DefaultTurnCoordinator, LoopCheckpointStore,
    RunProfileResolver, TurnRunWakeNotifier, TurnStateStore,
    loop_exit::LoopExitEvidencePort,
    run_profile::{
        AgentLoopHostError, InstructionSafetyContext, LoopCapabilityPort, LoopHostMilestoneSink,
        LoopModelBudgetAccountant, LoopModelPolicyGuard, LoopRunContext,
    },
    runner::TurnRunTransitionPort,
};

use crate::{
    app_loop_family::build_loop_family_registry,
    driver_registry::{DriverRegistry, DriverRegistryError},
    loop_driver_host::{
        HookDispatcherBuilderFactory, LoopCapabilityPortFactory, RebornLoopDriverHostFactory,
        TextOnlyLoopHostConfig,
    },
    loop_exit_applier::{LoopExitApplier, ThreadCheckpointLoopExitEvidencePort},
    model_routes::ModelRouteResolver,
    planned_driver_factory::{
        DefaultPlannedDriverRegistrationError, default_planned_run_profile_resolver,
        register_default_planned_driver, register_default_text_only_driver,
    },
    text_loop_driver::TextOnlyModelReplyDriverConfig,
    turn_runner::{
        TurnRunnerWakeReceiver, TurnRunnerWakeSender, TurnRunnerWorker, TurnRunnerWorkerConfig,
    },
};

#[derive(Debug, Clone, Default)]
pub struct DefaultPlannedRuntimeConfig {
    pub worker: TurnRunnerWorkerConfig,
    pub text_only_driver: TextOnlyModelReplyDriverConfig,
    pub host: TextOnlyLoopHostConfig,
}

pub struct DefaultPlannedRuntimeParts<T, S, G>
where
    T: TurnStateStore + TurnRunTransitionPort + Send + Sync + 'static,
    S: SessionThreadService + ?Sized + Send + Sync + 'static,
    G: HostManagedModelGateway + ?Sized + Send + Sync + 'static,
{
    pub turn_state: Arc<T>,
    pub thread_service: Arc<S>,
    pub thread_scope: ThreadScope,
    pub model_gateway: Arc<G>,
    pub checkpoint_state_store: Arc<dyn CheckpointStateStore>,
    pub loop_checkpoint_store: Arc<dyn LoopCheckpointStore>,
    pub milestone_sink: Arc<dyn LoopHostMilestoneSink>,
    pub capability_factory: Arc<dyn LoopCapabilityPortFactory>,
    pub capability_surface_resolver: Arc<dyn CapabilitySurfaceProfileResolver>,
    pub loop_exit_evidence: Arc<dyn LoopExitEvidencePort>,
    pub config: DefaultPlannedRuntimeConfig,
    pub model_route_resolver: Option<Arc<dyn ModelRouteResolver>>,
    pub cancellation_factory: Option<Arc<dyn RunCancellationFactory>>,
    pub skill_context_source: Option<Arc<dyn HostSkillContextSource>>,
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
    /// Per-run hook dispatcher builder factory. `None` (the default) leaves
    /// the hook framework dormant: no dispatcher is composed and the runtime
    /// behaves exactly as it did before hooks existed — the rollout-safety
    /// contract for the activation behind `HOOKS_ENABLED`. `Some(factory)`
    /// (only when the flag is ON) wires the factory through
    /// [`RebornLoopDriverHostFactory::with_hook_dispatcher_builder_factory`],
    /// so each host build mints a fresh dispatcher (per-run isolation of
    /// dispatcher-local state — slot-poisoning, registry edits; predicate
    /// counters remain tenant-scoped, shared across runs by design) and the
    /// host attaches a run-scoped milestone sink internally (per-run telemetry
    /// attribution, the #3573 lesson).
    pub hook_dispatcher_builder_factory: Option<HookDispatcherBuilderFactory>,
}

pub struct RebornRuntimeLoopComposition<T, S, G>
where
    T: TurnStateStore + TurnRunTransitionPort + Send + Sync + 'static,
    S: SessionThreadService + ?Sized + Send + Sync + 'static,
    G: HostManagedModelGateway + ?Sized + Send + Sync + 'static,
{
    pub driver_registry: Arc<DriverRegistry>,
    pub run_profile_resolver: Arc<dyn RunProfileResolver>,
    pub coordinator: Arc<DefaultTurnCoordinator<T>>,
    pub host_factory: Arc<RebornLoopDriverHostFactory<S, G>>,
    pub worker: Arc<TurnRunnerWorker>,
    pub wake_sender: TurnRunnerWakeSender,
}

#[derive(Debug)]
pub enum DefaultPlannedRuntimeBuildError {
    DriverRegistry(DriverRegistryError),
    PlannedDriver(DefaultPlannedDriverRegistrationError),
    RunProfile(String),
}

impl fmt::Display for DefaultPlannedRuntimeBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DriverRegistry(error) => write!(formatter, "driver registry failed: {error}"),
            Self::PlannedDriver(error) => write!(formatter, "planned driver failed: {error}"),
            Self::RunProfile(error) => write!(formatter, "run profile resolver failed: {error}"),
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

pub fn build_product_live_planned_runtime<T, S, G>(
    mut parts: DefaultPlannedRuntimeParts<T, S, G>,
) -> Result<RebornRuntimeLoopComposition<T, S, G>, ProductLiveRuntimeBuildError>
where
    T: TurnStateStore + TurnRunTransitionPort + Send + Sync + 'static,
    S: SessionThreadService + ?Sized + Send + Sync + 'static,
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
        .with_cancellation_factory(cancellation_factory),
    );
    build_default_planned_runtime(parts).map_err(ProductLiveRuntimeBuildError::Runtime)
}

pub fn build_default_planned_runtime<T, S, G>(
    parts: DefaultPlannedRuntimeParts<T, S, G>,
) -> Result<RebornRuntimeLoopComposition<T, S, G>, DefaultPlannedRuntimeBuildError>
where
    T: TurnStateStore + TurnRunTransitionPort + Send + Sync + 'static,
    S: SessionThreadService + ?Sized + Send + Sync + 'static,
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
    register_default_planned_driver(&mut registry, family_registry)?;
    let driver_registry = Arc::new(registry);

    let resolver = Arc::new(
        default_planned_run_profile_resolver()
            .map_err(|error| DefaultPlannedRuntimeBuildError::RunProfile(error.to_string()))?,
    );
    let run_profile_resolver: Arc<dyn RunProfileResolver> = resolver;

    let (wake_sender, wake_receiver) = TurnRunnerWakeReceiver::new();
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
    let coordinator = Arc::new(
        DefaultTurnCoordinator::new(Arc::clone(&parts.turn_state))
            .with_run_profile_resolver(Arc::clone(&run_profile_resolver))
            .with_wake_notifier(wake_notifier),
    );

    let turn_state_store: Arc<dyn TurnStateStore> = parts.turn_state.clone();
    let mut host_factory = RebornLoopDriverHostFactory::new(
        Arc::clone(&parts.thread_service),
        parts.thread_scope,
        Arc::clone(&parts.model_gateway),
        parts.checkpoint_state_store,
        turn_state_store,
        Arc::clone(&parts.loop_checkpoint_store),
        parts.milestone_sink,
        parts.config.host,
    )
    .with_profiled_capability_port_factory(
        parts.capability_factory,
        parts.capability_surface_resolver,
    )
    .with_driver_requirements(driver_registry.requirements_snapshot());
    if let Some(resolver) = parts.model_route_resolver {
        host_factory = host_factory.with_model_route_resolver(resolver);
    }
    if let Some(factory) = parts.cancellation_factory {
        host_factory = host_factory.with_cancellation_factory(factory);
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
    if let Some(safety) = parts.safety_context {
        host_factory = host_factory.with_safety_context(safety);
    }
    // Hook framework activation (item 6 of #3934). When the builder factory is
    // present (flag ON), wire it so each host build mints a fresh dispatcher
    // with per-run isolation of dispatcher-local state (slot-poisoning,
    // registry edits); predicate counters stay tenant-scoped and shared across
    // runs by design. The host factory attaches the run-scoped milestone sink
    // internally for per-run telemetry attribution. When `None` (flag OFF /
    // default), no dispatcher is composed and behavior is identical to the
    // pre-hooks runtime.
    if let Some(factory) = parts.hook_dispatcher_builder_factory {
        host_factory = host_factory.with_hook_dispatcher_builder_factory(move || factory());
    }
    host_factory = host_factory.with_identity_context_source(parts.identity_context_source);
    let host_factory = Arc::new(host_factory);

    let transition_port: Arc<dyn TurnRunTransitionPort> = parts.turn_state;
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

    Ok(RebornRuntimeLoopComposition {
        driver_registry,
        run_profile_resolver,
        coordinator,
        host_factory,
        worker,
        wake_sender,
    })
}

#[async_trait]
impl LoopCapabilityPortFactory for HostRuntimeLoopCapabilityPortFactory {
    async fn create_capability_port(
        &self,
        run_context: &LoopRunContext,
    ) -> Result<Arc<dyn LoopCapabilityPort>, AgentLoopHostError> {
        Ok(self.for_run_context(run_context.clone()))
    }
}
