//! Default Reborn runtime-loop composition.

use std::{error::Error, fmt, sync::Arc};

use async_trait::async_trait;
use ironclaw_loop_support::{
    CapabilitySurfaceProfileResolver, HostIdentityContextSource, HostInputQueue,
    HostManagedModelGateway, HostRuntimeLoopCapabilityPortFactory, HostSkillContextSource,
    RunCancellationFactory,
};
use ironclaw_threads::{SessionThreadService, ThreadScope};
use ironclaw_turns::{
    AgentLoopDriverError, CheckpointStateStore, DefaultTurnCoordinator, LoopCheckpointStore,
    RunProfileResolver, TurnRunWakeNotifier, TurnStateStore,
    loop_exit::LoopExitEvidencePort,
    run_profile::{AgentLoopHostError, LoopCapabilityPort, LoopHostMilestoneSink, LoopRunContext},
    runner::TurnRunTransitionPort,
};

use crate::{
    build_loop_family_registry,
    driver_registry::{DriverRegistry, DriverRegistryError},
    loop_driver_host::{
        LoopCapabilityPortFactory, RebornLoopDriverHostFactory, TextOnlyLoopHostConfig,
    },
    loop_exit_applier::LoopExitApplier,
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

#[derive(Debug, Clone)]
pub struct DefaultPlannedRuntimeConfig {
    pub worker: TurnRunnerWorkerConfig,
    pub text_only_driver: TextOnlyModelReplyDriverConfig,
    pub host: TextOnlyLoopHostConfig,
}

impl Default for DefaultPlannedRuntimeConfig {
    fn default() -> Self {
        Self {
            worker: TurnRunnerWorkerConfig::default(),
            text_only_driver: TextOnlyModelReplyDriverConfig::default(),
            host: TextOnlyLoopHostConfig::default(),
        }
    }
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
    /// Required by the WS-14 planned-driver brief for the WS-16 runtime smoke
    /// and WS-17 product cutover. `None` is only acceptable for helper-level
    /// WS-14 unit tests; live composition must always supply identity context.
    pub identity_context_source: Arc<dyn HostIdentityContextSource>,
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
    let wake_notifier: Arc<dyn TurnRunWakeNotifier> = Arc::new(wake_sender.clone());
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
